//! Cross-account scatter-gather query merge, deduplication, and parallel search.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::account_db::{AccountDb, MergedMessageCandidate};
use super::split_address;
use crate::{MessageSort, MessageSummary};

/// Total order matching SQLite's [`MessageSort`] ORDER BY clauses across accounts.
pub(crate) fn compare_message_summaries(
    sort: MessageSort,
    a: &MessageSummary,
    b: &MessageSummary,
) -> Ordering {
    let newest = || b.date_ts.cmp(&a.date_ts).then_with(|| b.uid.cmp(&a.uid));
    let oldest = || a.date_ts.cmp(&b.date_ts).then_with(|| a.uid.cmp(&b.uid));
    match sort {
        MessageSort::DateNewest => newest(),
        MessageSort::DateOldest => oldest(),
        MessageSort::SenderAsc => a
            .sender
            .to_lowercase()
            .cmp(&b.sender.to_lowercase())
            .then_with(newest),
        MessageSort::SenderDesc => b
            .sender
            .to_lowercase()
            .cmp(&a.sender.to_lowercase())
            .then_with(newest),
        MessageSort::SubjectAsc => a
            .subject
            .to_lowercase()
            .cmp(&b.subject.to_lowercase())
            .then_with(newest),
        MessageSort::SubjectDesc => b
            .subject
            .to_lowercase()
            .cmp(&a.subject.to_lowercase())
            .then_with(newest),
        MessageSort::UnreadFirst => b.unread.cmp(&a.unread).then_with(newest),
        MessageSort::StarredFirst => b.flagged.cmp(&a.flagged).then_with(newest),
        MessageSort::AccountAsc => a.account_id.cmp(&b.account_id).then_with(newest),
        MessageSort::AccountDesc => b.account_id.cmp(&a.account_id).then_with(newest),
        MessageSort::AttachmentFirst => b.has_attachment.cmp(&a.has_attachment).then_with(newest),
        MessageSort::ImportanceFirst => b.importance.cmp(&a.importance).then_with(newest),
        MessageSort::TagsAsc => a
            .tags
            .join(",")
            .to_lowercase()
            .cmp(&b.tags.join(",").to_lowercase())
            .then_with(newest),
    }
}

/// Deduplicate cross-account search hits by non-empty RFC `message_id`,
/// preserving the highest folder priority (`Inbox > normal > Archive > Trash/Junk`)
/// and breaking ties by smaller external `id`.
pub(crate) fn dedup_search_candidates(
    candidates: Vec<MergedMessageCandidate>,
) -> Vec<MessageSummary> {
    let mut out: Vec<MergedMessageCandidate> = Vec::with_capacity(candidates.len());
    let mut by_msg_id: HashMap<String, usize> = HashMap::new();

    for cand in candidates {
        if cand.rfc_message_id.is_empty() {
            out.push(cand);
            continue;
        }
        if let Some(&idx) = by_msg_id.get(&cand.rfc_message_id) {
            let existing = &out[idx];
            let replace = (cand.folder_priority, cand.summary.id)
                < (existing.folder_priority, existing.summary.id);
            if replace {
                out[idx] = cand;
            }
        } else {
            let idx = out.len();
            by_msg_id.insert(cand.rfc_message_id.clone(), idx);
            out.push(cand);
        }
    }

    out.into_iter().map(|c| c.summary).collect()
}

/// Aggregate `(counts, sent_counts, names, emails)` from scanned `(sender, recipients, is_sent)` rows.
pub(crate) fn fold_contact_rows(
    rows_iter: impl IntoIterator<Item = (String, String, bool)>,
    counts: &mut HashMap<String, usize>,
    sent_counts: &mut HashMap<String, usize>,
    names: &mut HashMap<String, String>,
    emails: &mut HashMap<String, String>,
) {
    for (sender, recipients, is_sent) in rows_iter {
        let mut seen_in_message: HashSet<String> = HashSet::new();
        let mut pairs = vec![split_address(&sender)];
        for part in recipients.split(',') {
            pairs.push(split_address(part));
        }
        for (name, email) in pairs {
            if !email.contains('@') {
                continue;
            }
            let key = email.to_lowercase();
            if !seen_in_message.insert(key.clone()) {
                continue;
            }
            emails.entry(key.clone()).or_insert(email);
            let name_slot = names.entry(key.clone()).or_default();
            if name_slot.is_empty() && !name.is_empty() {
                *name_slot = name;
            }
            *counts.entry(key.clone()).or_insert(0) += 1;
            if is_sent {
                *sent_counts.entry(key).or_insert(0) += 1;
            }
        }
    }
}

/// Execute parallel per-account disk work across `(account_id, slot, account_dir)` tuples
/// using short-lived read-only connections on SQLite WAL mode.
pub(crate) fn parallel_account_disk_query<T: Send>(
    targets: &[(u32, u8, &Path)],
    worker_fn: &(dyn Fn(&AccountDb) -> Result<T, String> + Sync),
) -> Result<Vec<(u32, T)>, String> {
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(targets.len());
        for &(account_id, slot, dir) in targets {
            handles.push(scope.spawn(move || -> Result<(u32, T), String> {
                let Some(ro_db) = AccountDb::open_read_only_in_dir(account_id, slot, dir)? else {
                    // Caller ensures mail.db exists or handles missing gracefully via Option;
                    // fallback open in-memory empty schema would hide errors, so use Err only on real DB error.
                    let empty = AccountDb::open_in_memory(account_id, slot)?;
                    return Ok((account_id, worker_fn(&empty)?));
                };
                Ok((account_id, worker_fn(&ro_db)?))
            }));
        }
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            let res = handle
                .join()
                .map_err(|_| "并行跨账号查询线程异常中止".to_string())??;
            results.push(res);
        }
        Ok(results)
    })
}
