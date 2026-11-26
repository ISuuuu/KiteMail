//! Unit & integration tests for the split SQLite cache (`global.db` + per-account DBs).

use super::*;
use crate::config::{self, AccountConfig};

fn sample_header(uid: u32, subject: &str) -> CachedHeaderInput {
    CachedHeaderInput {
        uid,
        message_id: format!("<msg-{uid}@example.com>"),
        in_reply_to: String::new(),
        references_header: String::new(),
        sender: "Alice <alice@example.com>".to_string(),
        recipients: "bob@example.com".to_string(),
        cc: String::new(),
        subject: subject.to_string(),
        preview: "正文摘要".to_string(),
        date_ts: 1_700_000_000 + uid as i64,
        date_display: "15:00".to_string(),
        unread: true,
        flagged: false,
        has_attachment: false,
    }
}

fn folder(name: &str, path: &str) -> RemoteFolder {
    RemoteFolder {
        name: name.to_string(),
        path: path.to_string(),
        unread_count: 0,
        total_count: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn header_with(
    uid: u32,
    subject: &str,
    sender: &str,
    recipients: &str,
    flagged: bool,
    unread: bool,
    date_ts: i64,
) -> CachedHeaderInput {
    CachedHeaderInput {
        uid,
        message_id: format!("<msg-{uid}@example.com>"),
        in_reply_to: String::new(),
        references_header: String::new(),
        sender: sender.to_string(),
        recipients: recipients.to_string(),
        cc: String::new(),
        subject: subject.to_string(),
        preview: String::new(),
        date_ts,
        date_display: String::new(),
        unread,
        flagged,
        has_attachment: false,
    }
}

fn temp_cache_dir(suffix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hylki-cache-test-{suffix}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn collapsed_inbox_cache_invalidates_for_other_connection_and_local_writes() {
    let dir = temp_cache_dir("collapsed-version");
    let mut writer = CacheStore::open_for_data_dir(&dir).unwrap();
    let inbox = writer
        .sync_folders(1, &[folder("收件箱", "INBOX")])
        .unwrap()[0]
        .id;
    writer
        .commit_folder_headers(1, inbox, 1, &[sample_header(1, "first")], None)
        .unwrap();
    let mut reader = CacheStore::open_for_data_dir_existing(&dir).unwrap();
    assert_eq!(
        reader
            .list_messages_page(inbox, "", 0, 10)
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        reader
            .list_messages_page(inbox, "", 0, 10)
            .unwrap()
            .items
            .len(),
        1
    );
    writer
        .commit_folder_headers(1, inbox, 1, &[sample_header(2, "second")], None)
        .unwrap();
    assert_eq!(
        reader
            .list_messages_page(inbox, "", 0, 10)
            .unwrap()
            .items
            .len(),
        2
    );
    reader
        .commit_folder_headers(1, inbox, 1, &[sample_header(3, "third")], None)
        .unwrap();
    assert_eq!(
        reader
            .list_messages_page(inbox, "", 0, 10)
            .unwrap()
            .items
            .len(),
        3
    );
    writer
        .commit_folder_headers(1, inbox, 1, &[], Some(&[1, 3]))
        .unwrap();
    assert_eq!(
        reader
            .list_messages_page(inbox, "", 0, 10)
            .unwrap()
            .items
            .len(),
        2
    );
    drop(reader);
    drop(writer);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn attachments_index_is_created_when_existing_database_reopens() {
    let dir = temp_cache_dir("attachment-index");
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();
    cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    cache.with_account_mail_conn(1, |conn| {
        conn.execute("DROP INDEX idx_attachments_message_id", [])
            .unwrap();
    });
    drop(cache);
    let mut cache = CacheStore::open_for_data_dir_existing(&dir).unwrap();
    cache.list_folders(1).unwrap();
    cache.with_account_mail_conn(1, |conn| {
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT * FROM attachments WHERE message_id = 1",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("idx_attachments_message_id"), "{plan}");
    });
    drop(cache);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn graph_remote_id_collision_keeps_distinct_messages_and_stable_identity() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folder = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap()[0].id;
    let a = sample_header(42, "first");
    let mut b = sample_header(42, "second");
    b.message_id = "<different@example.com>".into();
    cache
        .commit_folder_headers_with_remote(
            1,
            folder,
            1,
            &[
                (a.clone(), "remote-a".into()),
                (b.clone(), "remote-b".into()),
            ],
            None,
        )
        .unwrap();
    let page = cache.list_messages_page(folder, "", 0, 10).unwrap();
    assert_eq!(page.items.len(), 2);
    let ids: std::collections::HashMap<_, _> = page
        .items
        .iter()
        .map(|m| (m.subject.clone(), m.id))
        .collect();
    assert_eq!(
        cache
            .get_message_remote_id(ids["first"])
            .unwrap()
            .as_deref(),
        Some("remote-a")
    );
    assert_eq!(
        cache
            .get_message_remote_id(ids["second"])
            .unwrap()
            .as_deref(),
        Some("remote-b")
    );
    cache
        .store_message_body(ids["first"], "body-a", &[])
        .unwrap();
    cache
        .store_message_body(ids["second"], "body-b", &[])
        .unwrap();
    cache
        .commit_folder_headers_with_remote(
            1,
            folder,
            1,
            &[(b, "remote-b".into()), (a, "remote-a".into())],
            None,
        )
        .unwrap();
    let page = cache.list_messages_page(folder, "", 0, 10).unwrap();
    assert_eq!(page.items.len(), 2);
    for m in page.items {
        assert_eq!(m.id, ids[&m.subject]);
    }
    assert_eq!(
        cache
            .get_message_detail(ids["first"])
            .unwrap()
            .unwrap()
            .0
            .body_text,
        "body-a"
    );
    assert_eq!(
        cache
            .get_message_detail(ids["second"])
            .unwrap()
            .unwrap()
            .0
            .body_text,
        "body-b"
    );
}

#[test]
fn header_cc_is_cached_and_exposed_in_detail() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;
    let mut header = sample_header(1, "抄送可见");
    header.cc = "Carol <carol@example.com>".to_string();
    cache
        .commit_folder_headers(1, inbox, 1, &[header], None)
        .unwrap();
    let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let id = page.items[0].id;
    let (detail, _, _) = cache.get_message_detail(id).unwrap().unwrap();
    assert_eq!(detail.recipients, "bob@example.com");
    assert_eq!(detail.cc, "Carol <carol@example.com>");
}

#[test]
fn same_uid_across_two_folders_stays_isolated() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache
        .sync_folders(
            1,
            &[
                RemoteFolder {
                    name: "收件箱".to_string(),
                    path: "INBOX".to_string(),
                    unread_count: 1,
                    total_count: 1,
                },
                RemoteFolder {
                    name: "已发送".to_string(),
                    path: "Sent".to_string(),
                    unread_count: 0,
                    total_count: 1,
                },
            ],
        )
        .expect("sync folders");

    let inbox_id = folders[0].id;
    let sent_id = folders[1].id;

    cache
        .commit_folder_headers(
            1,
            inbox_id,
            100,
            &[sample_header(42, "收件箱 UID 42")],
            None,
        )
        .expect("commit inbox");
    cache
        .commit_folder_headers(1, sent_id, 200, &[sample_header(42, "已发送 UID 42")], None)
        .expect("commit sent");

    let inbox_page = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
    let sent_page = cache.list_messages_page(sent_id, "", 0, 10).unwrap();
    assert_eq!(inbox_page.items[0].subject, "收件箱 UID 42");
    assert_eq!(sent_page.items[0].subject, "已发送 UID 42");
}

#[test]
fn uidvalidity_change_rebuilds_folder_cache() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache
        .sync_folders(
            1,
            &[RemoteFolder {
                name: "收件箱".to_string(),
                path: "INBOX".to_string(),
                unread_count: 1,
                total_count: 1,
            }],
        )
        .unwrap();
    let inbox_id = folders[0].id;

    cache
        .commit_folder_headers(1, inbox_id, 1001, &[sample_header(1, "旧纪元邮件")], None)
        .unwrap();
    cache
        .commit_folder_headers(1, inbox_id, 1002, &[sample_header(1, "新纪元邮件")], None)
        .unwrap();

    let page = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].subject, "新纪元邮件");
}

#[test]
fn templates_round_trip_and_delete() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    assert!(cache.list_templates().unwrap().is_empty());

    let id = cache
        .upsert_template(&MailTemplate {
            id: 0,
            name: "客户回访".into(),
            to_addr: "a@x.com".into(),
            cc_addr: String::new(),
            bcc_addr: String::new(),
            subject: "回访".into(),
            body_text: "您好".into(),
            body_format: "markdown".into(),
            attachment_paths: vec!["C:////tmp////a.pdf".into()],
            created_at: 100,
        })
        .unwrap();
    assert!(id > 0);

    let list = cache.list_templates().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "客户回访");
    assert_eq!(list[0].body_format, "markdown");
    assert_eq!(
        list[0].attachment_paths,
        vec!["C:////tmp////a.pdf".to_string()]
    );

    let mut updated = list[0].clone();
    updated.name = "新名字".into();
    assert_eq!(cache.upsert_template(&updated).unwrap(), id);
    let list = cache.list_templates().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "新名字");

    cache.delete_template(id).unwrap();
    assert!(cache.list_templates().unwrap().is_empty());
}

#[test]
fn outbox_preserves_uncertain_state_without_auto_resend() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let id = cache
        .enqueue_outbox(
            1,
            "to@example.com",
            "",
            "",
            "带附件回复",
            "测试正文",
            "<orig@example.com>",
            &["C:\\temp\\test.txt".to_string()],
            1_700_000_123,
            None,
        )
        .unwrap();

    cache
        .update_outbox_status(id, OutboxStatus::Uncertain, "SMTP 响应超时")
        .unwrap();

    let items = cache.list_outbox(1).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, OutboxStatus::Uncertain);
    assert_eq!(items[0].attachment_paths, vec!["C:\\temp\\test.txt"]);
}

#[test]
fn outbox_round_trips_cc_and_bcc() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    cache
        .enqueue_outbox(
            1,
            "to@example.com",
            "c1@example.com, Carol <carol@c.com>",
            "secret@example.com",
            "带抄送的邮件",
            "正文",
            "",
            &[],
            1_700_000_200,
            None,
        )
        .unwrap();

    let items = cache.list_outbox(1).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].to_addr, "to@example.com");
    assert_eq!(items[0].cc_addr, "c1@example.com, Carol <carol@c.com>");
    assert_eq!(items[0].bcc_addr, "secret@example.com");
}

#[test]
fn outbox_claim_requires_due_and_is_idempotent() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let now = 1_700_000_000i64;
    let due_id = cache
        .enqueue_outbox(
            1,
            "a@x.com",
            "",
            "",
            "已到期",
            "正文",
            "",
            &[],
            now - 60,
            Some(now - 30),
        )
        .unwrap();
    let not_due_id = cache
        .enqueue_outbox(
            1,
            "b@x.com",
            "",
            "",
            "未到期",
            "正文",
            "",
            &[],
            now - 10,
            Some(now + 300),
        )
        .unwrap();

    assert_eq!(
        cache.list_due_queued_ids(Some(1), now).unwrap(),
        vec![(1, due_id)]
    );

    assert!(cache
        .claim_outbox_for_delivery(not_due_id, now)
        .unwrap()
        .is_none());
    let claimed = cache
        .claim_outbox_for_delivery(due_id, now)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, due_id);
    assert_eq!(claimed.status, OutboxStatus::Sending);
    assert!(cache
        .claim_outbox_for_delivery(due_id, now)
        .unwrap()
        .is_none());
    assert!(cache.claim_outbox_for_retry(due_id).unwrap().is_none());

    assert!(cache.list_due_queued_ids(Some(1), now).unwrap().is_empty());
    assert!(cache.list_due_queued_ids(None, now).unwrap().is_empty());
}

#[test]
fn next_queued_send_at_tracks_earliest_pending() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let now = 1_700_000_000i64;
    assert_eq!(cache.next_queued_send_at(now).unwrap(), None);

    let later = cache
        .enqueue_outbox(
            1,
            "late@x.com",
            "",
            "",
            "晚上发",
            "正文",
            "",
            &[],
            now,
            Some(now + 3600),
        )
        .unwrap();
    let sooner = cache
        .enqueue_outbox(
            1,
            "soon@x.com",
            "",
            "",
            "一小时后发",
            "正文",
            "",
            &[],
            now,
            Some(now + 600),
        )
        .unwrap();
    assert_eq!(cache.next_queued_send_at(now).unwrap(), Some(now + 600));

    assert!(cache
        .claim_outbox_for_delivery(sooner, now + 600)
        .unwrap()
        .is_some());
    assert_eq!(cache.next_queued_send_at(now).unwrap(), Some(now + 3600));

    cache.delete_outbox(later).unwrap();
    assert_eq!(cache.next_queued_send_at(now).unwrap(), None);

    let stale = cache
        .enqueue_outbox(
            1,
            "stale@x.com",
            "",
            "",
            "过期未发",
            "正文",
            "",
            &[],
            now,
            Some(now - 30),
        )
        .unwrap();
    assert_eq!(cache.next_queued_send_at(now).unwrap(), None);
    assert_eq!(
        cache.list_due_queued_ids(None, now).unwrap(),
        vec![(1, stale)]
    );
}

#[test]
fn outbox_disabled_account_stays_queued_and_resumes() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let now = 1_700_000_000i64;
    let id = cache
        .enqueue_outbox(
            1,
            "a@x.com",
            "c@x.com",
            "",
            "停用期间排队",
            "正文",
            "<r@x.com>",
            &[],
            now,
            Some(now - 5),
        )
        .unwrap();

    let claimed = cache.claim_outbox_for_delivery(id, now).unwrap().unwrap();
    cache
        .update_outbox_status(id, OutboxStatus::Queued, "账号已停用，暂缓投递")
        .unwrap();

    let items = cache.list_outbox(1).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, OutboxStatus::Queued);
    assert_eq!(items[0].to_addr, claimed.to_addr);
    assert_eq!(items[0].body_text, claimed.body_text);
    assert_eq!(items[0].subject, claimed.subject);
    assert_eq!(items[0].last_error, "账号已停用，暂缓投递");

    assert!(cache.claim_outbox_for_delivery(id, now).unwrap().is_some());
    assert!(cache.list_due_queued_ids(Some(1), now).unwrap().is_empty());
}

#[test]
fn outbox_auto_delivery_never_claims_failed_or_uncertain() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let now = 1_700_000_000i64;
    let failed_id = cache
        .enqueue_outbox(1, "a@x.com", "", "", "失败", "正文", "", &[], now, None)
        .unwrap();
    let uncertain_id = cache
        .enqueue_outbox(1, "b@x.com", "", "", "不确定", "正文", "", &[], now, None)
        .unwrap();
    cache
        .update_outbox_status(failed_id, OutboxStatus::Failed, "SMTP 拒绝")
        .unwrap();
    cache
        .update_outbox_status(uncertain_id, OutboxStatus::Uncertain, "连接中断")
        .unwrap();

    assert!(cache.list_due_queued_ids(Some(1), now).unwrap().is_empty());
    assert!(cache
        .claim_outbox_for_delivery(failed_id, now)
        .unwrap()
        .is_none());
    assert!(cache
        .claim_outbox_for_delivery(uncertain_id, now)
        .unwrap()
        .is_none());
    assert!(cache.claim_outbox_for_retry(failed_id).unwrap().is_some());
    assert!(cache
        .claim_outbox_for_retry(uncertain_id)
        .unwrap()
        .is_none());
}

#[test]
fn enqueue_and_draft_handoff_are_atomic() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let draft = LocalDraft {
        id: 0,
        account_id: 1,
        to_addr: "to".into(),
        cc_addr: "cc".into(),
        bcc_addr: "bcc".into(),
        subject: "subject".into(),
        body_text: "body".into(),
        in_reply_to: "ref".into(),
        from_addr: "Me <me@example.com>".into(),
        reply_to: "replies@example.com".into(),
        body_format: "rich".into(),
        attachment_paths: vec!["file".into()],
        updated_at: 10,
    };
    let id = cache.upsert_draft(&draft).unwrap();
    cache
        .global_conn()
        .execute_batch("CREATE TRIGGER reject_handoff BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT, 'write failed'); END;")
        .unwrap();
    assert!(cache
        .enqueue_outbox_from_draft(
            1,
            "to",
            "cc",
            "bcc",
            "subject",
            "body",
            "ref",
            "Me <me@example.com>",
            "replies@example.com",
            "rich",
            None,
            &["file".into()],
            10,
            Some(20),
            Some(id)
        )
        .is_err());
    assert_eq!(cache.list_drafts().unwrap().len(), 1);
    assert!(cache.list_outbox(1).unwrap().is_empty());
    cache
        .global_conn()
        .execute_batch("DROP TRIGGER reject_handoff")
        .unwrap();
    cache
        .enqueue_outbox_from_draft(
            1,
            "to",
            "cc",
            "bcc",
            "subject",
            "body",
            "ref",
            "Me <me@example.com>",
            "replies@example.com",
            "markdown",
            Some("<p>body</p>"),
            &["file".into()],
            10,
            Some(20),
            Some(id),
        )
        .unwrap();
    assert!(cache.list_drafts().unwrap().is_empty());
    let outbox = cache.list_outbox(1).unwrap();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].from_addr, "Me <me@example.com>");
    assert_eq!(outbox[0].reply_to, "replies@example.com");
    assert_eq!(outbox[0].body_format, "markdown");
    assert_eq!(outbox[0].send_at, Some(20));
}

#[test]
fn undo_outbox_is_atomic_and_preserves_all_draft_fields() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let id = cache
        .enqueue_outbox(
            1,
            "to",
            "cc",
            "bcc",
            "subject",
            "body",
            "ref",
            &["file".into()],
            10,
            Some(100),
        )
        .unwrap();
    assert_eq!(cache.undo_queued_outbox(2, id).unwrap(), None);
    let draft_id = cache.undo_queued_outbox(1, id).unwrap().unwrap();
    let drafts = cache.list_drafts().unwrap();
    let d = drafts.iter().find(|d| d.id == draft_id).unwrap();
    assert_eq!(
        (
            &*d.to_addr,
            &*d.cc_addr,
            &*d.bcc_addr,
            &*d.subject,
            &*d.body_text,
            &*d.in_reply_to
        ),
        ("to", "cc", "bcc", "subject", "body", "ref")
    );
    assert_eq!(d.attachment_paths, vec!["file"]);
    assert!(cache.list_outbox(1).unwrap().is_empty());
    assert!(cache.claim_outbox_for_retry(id).unwrap().is_none());
    assert_eq!(cache.undo_queued_outbox(1, id).unwrap(), None);

    let claimed = cache
        .enqueue_outbox(1, "to", "", "", "", "", "", &[], 10, None)
        .unwrap();
    assert!(cache.claim_outbox_for_retry(claimed).unwrap().is_some());
    assert_eq!(cache.undo_queued_outbox(1, claimed).unwrap(), None);
    assert_eq!(cache.list_drafts().unwrap().len(), 1);
}

#[test]
fn undo_rolls_back_when_draft_write_fails() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let id = cache
        .enqueue_outbox(1, "to", "", "", "subject", "body", "", &[], 10, Some(100))
        .unwrap();
    cache
        .global_conn()
        .execute_batch("CREATE TRIGGER reject_draft BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'write failed'); END;")
        .unwrap();
    assert!(cache.undo_queued_outbox(1, id).is_err());
    assert_eq!(cache.list_outbox(1).unwrap().len(), 1);
    assert!(cache.list_drafts().unwrap().is_empty());
}

#[test]
fn undo_outbox_preserves_all_body_formats_and_legacy_fallback() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    for fmt in ["rich", "markdown", "html", "plain", ""] {
        let id = cache
            .enqueue_outbox_from_draft(
                1,
                "to@example.com",
                "",
                "",
                "主题",
                "正文",
                "",
                "",
                "",
                fmt,
                None,
                &[],
                10,
                Some(100),
                None,
            )
            .unwrap();
        let draft_id = cache.undo_queued_outbox(1, id).unwrap().unwrap();
        let draft = cache.load_draft(draft_id).unwrap().unwrap();
        assert_eq!(draft.body_format, fmt);
    }
}

#[test]
fn quit_preserves_queued_send_at_for_undo_and_scheduled_items() {
    let dir = std::env::temp_dir().join(format!(
        "hylki-quit-preserve-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();
    let now = chrono::Utc::now().timestamp() + 3600;
    let undo_due = now + 5;
    let scheduled_due = now + 3600;
    let undo_id = cache
        .enqueue_outbox(
            1,
            "undo@x.com",
            "",
            "",
            "缓冲",
            "正文",
            "",
            &[],
            now,
            Some(undo_due),
        )
        .unwrap();
    let sched_id = cache
        .enqueue_outbox(
            1,
            "sched@x.com",
            "",
            "",
            "定时",
            "正文",
            "",
            &[],
            now,
            Some(scheduled_due),
        )
        .unwrap();
    drop(cache);

    let mut before_restart = CacheStore::open_for_data_dir_existing(&dir).unwrap();
    let interrupted_id = before_restart
        .enqueue_outbox(
            1,
            "recipient@example.invalid",
            "",
            "",
            "中断",
            "",
            "",
            &[],
            now,
            None,
        )
        .unwrap();
    assert!(before_restart
        .claim_outbox_for_delivery(interrupted_id, now)
        .unwrap()
        .is_some());
    drop(before_restart);

    let worker = crate::worker::MailWorkerHandle::spawn(dir.clone()).unwrap();
    worker.prepare_quit_now().unwrap();
    worker
        .send_checked(crate::MailRequest::PrepareQuit)
        .unwrap();
    let started = std::time::Instant::now();
    loop {
        if worker
            .drain_events()
            .into_iter()
            .any(|event| matches!(event, crate::WorkerEvent::QuitPrepared { result: Ok(()) }))
        {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "worker did not acknowledge shutdown"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    drop(worker);

    let reopened = CacheStore::open_for_data_dir_existing(&dir).unwrap();
    let items = reopened.list_outbox(1).unwrap();
    assert_eq!(
        items
            .iter()
            .find(|i| i.id == interrupted_id)
            .unwrap()
            .status,
        OutboxStatus::Uncertain
    );
    assert_eq!(
        items.iter().find(|i| i.id == undo_id).unwrap().send_at,
        Some(undo_due)
    );
    assert_eq!(
        items.iter().find(|i| i.id == sched_id).unwrap().send_at,
        Some(scheduled_due)
    );
    // 早于缓冲截止时刻重启：两者都不应被提前领取。
    assert!(reopened
        .list_due_queued_ids(None, now + 3)
        .unwrap()
        .is_empty());
    // 晚于缓冲截止时刻重启：仅缓冲件到期，定时件仍保留计划时间。
    assert_eq!(
        reopened.list_due_queued_ids(None, undo_due).unwrap(),
        vec![(1, undo_id)]
    );
    drop(reopened);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn outbox_undo_deletes_only_queued_records() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let now = 1_700_000_000i64;
    let queued_id = cache
        .enqueue_outbox(
            1,
            "a@x.com",
            "",
            "",
            "撤销目标",
            "正文",
            "",
            &[],
            now,
            Some(now + 60),
        )
        .unwrap();
    let sending_id = cache
        .enqueue_outbox(1, "b@x.com", "", "", "投递中", "正文", "", &[], now, None)
        .unwrap();
    cache
        .update_outbox_status(sending_id, OutboxStatus::Sending, "")
        .unwrap();

    assert!(cache.delete_queued_outbox(queued_id).unwrap());
    assert!(cache
        .list_outbox(1)
        .unwrap()
        .iter()
        .all(|i| i.id != queued_id));
    assert!(!cache.delete_queued_outbox(sending_id).unwrap());
    assert!(cache
        .list_outbox(1)
        .unwrap()
        .iter()
        .any(|i| i.id == sending_id));
}

#[test]
fn legacy_db_without_cc_bcc_columns_is_migrated() {
    let dir = temp_cache_dir("legacy-db-nocc");
    let db_path = dir.join("legacy-cache.db");
    {
        let conn = Connection::open(&db_path).expect("open legacy db");
        conn.execute_batch(
            "
            CREATE TABLE folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                uid_validity INTEGER NOT NULL DEFAULT 0,
                unread_count INTEGER NOT NULL DEFAULT 0,
                total_count INTEGER NOT NULL DEFAULT 0,
                UNIQUE(account_id, path)
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                folder_id INTEGER NOT NULL,
                uid INTEGER NOT NULL,
                message_id TEXT NOT NULL DEFAULT '',
                sender TEXT NOT NULL,
                recipients TEXT NOT NULL DEFAULT '',
                subject TEXT NOT NULL,
                preview TEXT NOT NULL DEFAULT '',
                date_ts INTEGER NOT NULL DEFAULT 0,
                date_display TEXT NOT NULL DEFAULT '',
                unread INTEGER NOT NULL DEFAULT 1,
                flagged INTEGER NOT NULL DEFAULT 0,
                has_attachment INTEGER NOT NULL DEFAULT 0,
                body_text TEXT,
                UNIQUE(folder_id, uid)
            );
            CREATE TABLE attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id INTEGER NOT NULL,
                filename TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                content BLOB NOT NULL
            );
            CREATE TABLE outbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                to_addr TEXT NOT NULL,
                subject TEXT NOT NULL,
                body_text TEXT NOT NULL,
                in_reply_to TEXT NOT NULL DEFAULT '',
                attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL DEFAULT 'queued',
                last_error TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );
            CREATE TABLE drafts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                to_addr TEXT NOT NULL,
                subject TEXT NOT NULL,
                body_text TEXT NOT NULL,
                in_reply_to TEXT NOT NULL DEFAULT '',
                attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                updated_at INTEGER NOT NULL
            );
            INSERT INTO outbox (account_id, to_addr, subject, body_text, in_reply_to,
                                attachment_paths_json, status, last_error, created_at)
            VALUES (1, 'old@example.com', '旧邮件', '正文', '', '[]', 'queued', '', 100);
            INSERT INTO drafts (account_id, to_addr, subject, body_text, in_reply_to,
                                attachment_paths_json, updated_at)
            VALUES (1, 'old-draft@example.com', '旧草稿', '正文', '', '[]', 100);
            ",
        )
        .expect("create legacy schema");
    }

    let mut cache = CacheStore::open(&db_path).expect("open legacy db through migration");
    let outbox = cache.list_outbox(1).unwrap();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].cc_addr, "");
    assert_eq!(outbox[0].bcc_addr, "");
    let drafts = cache.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].cc_addr, "");
    assert_eq!(drafts[0].bcc_addr, "");

    cache
        .enqueue_outbox(
            1,
            "to@example.com",
            "cc@example.com",
            "bcc@example.com",
            "迁移后新邮件",
            "正文",
            "",
            &[],
            200,
            None,
        )
        .unwrap();
    let outbox = cache.list_outbox(1).unwrap();
    assert_eq!(outbox.len(), 2);
    let fresh = outbox.iter().find(|i| i.subject == "迁移后新邮件").unwrap();
    assert_eq!(fresh.cc_addr, "cc@example.com");
    assert_eq!(fresh.bcc_addr, "bcc@example.com");

    drop(cache);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unified_inbox_and_chinese_two_char_search_keep_account_ownership() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let f1 = cache
        .sync_folders(
            1,
            &[RemoteFolder {
                name: "收件箱".to_string(),
                path: "INBOX".to_string(),
                unread_count: 1,
                total_count: 1,
            }],
        )
        .unwrap();
    let f2 = cache
        .sync_folders(
            2,
            &[RemoteFolder {
                name: "收件箱".to_string(),
                path: "INBOX".to_string(),
                unread_count: 1,
                total_count: 1,
            }],
        )
        .unwrap();

    cache
        .commit_folder_headers(
            1,
            f1[0].id,
            10,
            &[sample_header(1, "项目周报：Windows 移植进度")],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            2,
            f2[0].id,
            20,
            &[sample_header(2, "Invoice 发票报销单确认")],
            None,
        )
        .unwrap();

    let all = cache.list_unified_inbox("", 0, 10).unwrap();
    assert_eq!(all.total, 2);
    assert_eq!(all.items[0].account_id, 2);
    assert_eq!(all.items[1].account_id, 1);

    let zh_search = cache.list_unified_inbox("周报", 0, 10).unwrap();
    assert_eq!(zh_search.total, 1);
    assert_eq!(zh_search.items[0].account_id, 1);

    let mixed_search = cache.list_unified_inbox("Invoice 发票", 0, 10).unwrap();
    assert_eq!(mixed_search.total, 1);
    assert_eq!(mixed_search.items[0].account_id, 2);
}

#[test]
fn local_draft_persists_updates_and_deletes() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let id = cache
        .upsert_draft(&LocalDraft {
            id: 0,
            account_id: 1,
            to_addr: "alice@example.com".to_string(),
            cc_addr: "bob@example.com".to_string(),
            bcc_addr: String::new(),
            subject: "草稿主题".to_string(),
            body_text: "第一版内容".to_string(),
            in_reply_to: String::new(),
            from_addr: "别名 <alias@example.com>".to_string(),
            reply_to: "reply@example.com".to_string(),
            body_format: "markdown".to_string(),
            attachment_paths: vec!["C:\\tmp\\report.pdf".to_string()],
            updated_at: 100,
        })
        .unwrap();

    let drafts = cache.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].id, id);
    assert_eq!(drafts[0].cc_addr, "bob@example.com");
    assert_eq!(drafts[0].bcc_addr, "");
    assert_eq!(drafts[0].from_addr, "别名 <alias@example.com>");
    assert_eq!(drafts[0].reply_to, "reply@example.com");
    assert_eq!(drafts[0].attachment_paths, vec!["C:\\tmp\\report.pdf"]);

    let mut updated = drafts[0].clone();
    updated.bcc_addr = "secret@example.com".to_string();
    updated.updated_at = 200;
    cache.upsert_draft(&updated).unwrap();
    let drafts = cache.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].bcc_addr, "secret@example.com");

    cache.delete_draft(id).unwrap();
    assert!(cache.list_drafts().unwrap().is_empty());
}

#[test]
fn starred_view_filters_flagged_and_searches() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let f1 = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let f2 = cache.sync_folders(2, &[folder("收件箱", "INBOX")]).unwrap();

    cache
        .commit_folder_headers(
            1,
            f1[0].id,
            10,
            &[
                header_with(1, "星标周报", "Alice <alice@a.com>", "", true, false, 100),
                header_with(2, "普通邮件", "Bob <bob@b.com>", "", false, false, 200),
            ],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            2,
            f2[0].id,
            20,
            &[header_with(
                1,
                "另一账号星标",
                "Carol <carol@c.com>",
                "",
                true,
                false,
                300,
            )],
            None,
        )
        .unwrap();

    let starred = cache.list_starred_messages("", 0, 10).unwrap();
    assert_eq!(starred.total, 2);
    assert_eq!(starred.folder_id, 0);
    assert_eq!(starred.items[0].subject, "另一账号星标");
    assert_eq!(starred.items[0].account_id, 2);
    assert_eq!(starred.items[1].subject, "星标周报");
    assert!(starred.items.iter().all(|m| m.flagged));

    let search = cache.list_starred_messages("周报", 0, 10).unwrap();
    assert_eq!(search.total, 1);
    assert_eq!(search.items[0].subject, "星标周报");

    let paged = cache.list_starred_messages("", 1, 1).unwrap();
    assert_eq!(paged.total, 2);
    assert_eq!(paged.items.len(), 1);
    assert_eq!(paged.items[0].subject, "星标周报");
}

#[test]
fn sent_view_aggregates_by_name_and_path() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let f1 = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("已发送", "Sent"),
                folder("垃圾邮件", "Junk"),
            ],
        )
        .unwrap();
    let f2 = cache
        .sync_folders(2, &[folder("已发送", "AAMkGraphFolderId")])
        .unwrap();

    let inbox = f1.iter().find(|f| f.path == "INBOX").unwrap().id;
    let sent = f1.iter().find(|f| f.path == "Sent").unwrap().id;
    let junk = f1.iter().find(|f| f.path == "Junk").unwrap().id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            10,
            &[header_with(
                1,
                "收到的邮件",
                "x@x.com",
                "",
                false,
                false,
                50,
            )],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            sent,
            11,
            &[header_with(
                1,
                "发出的邮件",
                "me@me.com",
                "you@you.com",
                false,
                false,
                100,
            )],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            junk,
            12,
            &[header_with(
                1,
                "广告邮件",
                "ad@ad.com",
                "",
                false,
                false,
                200,
            )],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            2,
            f2[0].id,
            20,
            &[header_with(
                1,
                "Graph 已发送",
                "me2@me.com",
                "",
                false,
                false,
                300,
            )],
            None,
        )
        .unwrap();

    let sent_page = cache.list_sent_messages("", 0, 10).unwrap();
    assert_eq!(sent_page.total, 2);
    assert_eq!(sent_page.items[0].subject, "Graph 已发送");
    assert_eq!(sent_page.items[0].account_id, 2);
    assert_eq!(sent_page.items[1].subject, "发出的邮件");
    assert_eq!(sent_page.items[1].account_id, 1);

    let search = cache.list_sent_messages("发出", 0, 10).unwrap();
    assert_eq!(search.total, 1);
    assert_eq!(search.items[0].subject, "发出的邮件");
}

#[test]
fn unread_view_lists_unread_and_excludes_trash_and_junk() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let f1 = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("已删除", "Trash"),
                folder("垃圾邮件", "Junk"),
            ],
        )
        .unwrap();
    let inbox = f1.iter().find(|f| f.path == "INBOX").unwrap().id;
    let trash = f1.iter().find(|f| f.path == "Trash").unwrap().id;
    let junk = f1.iter().find(|f| f.path == "Junk").unwrap().id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            10,
            &[
                header_with(1, "未读邮件", "a@x.com", "", false, true, 100),
                header_with(2, "已读邮件", "b@x.com", "", false, false, 90),
            ],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            trash,
            10,
            &[header_with(3, "已删未读", "c@x.com", "", false, true, 95)],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            junk,
            10,
            &[header_with(4, "垃圾未读", "d@x.com", "", false, true, 96)],
            None,
        )
        .unwrap();

    let page = cache
        .list_unread_messages_for_accounts(
            &[1],
            "",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            20,
        )
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].subject, "未读邮件");
}

#[test]
fn archived_view_aggregates_by_name_and_path() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let f1 = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("归档", "Archive"),
                folder("Notes", "Notes"),
            ],
        )
        .unwrap();
    let f2 = cache
        .sync_folders(2, &[folder("Archive", "INBOX/Archive")])
        .unwrap();

    let inbox = f1.iter().find(|f| f.path == "INBOX").unwrap().id;
    let archive = f1.iter().find(|f| f.path == "Archive").unwrap().id;
    let notes = f1.iter().find(|f| f.path == "Notes").unwrap().id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            10,
            &[header_with(
                1,
                "收件箱邮件",
                "x@x.com",
                "",
                false,
                false,
                50,
            )],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            archive,
            11,
            &[header_with(
                1,
                "已归档邮件",
                "y@y.com",
                "",
                false,
                false,
                100,
            )],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            notes,
            12,
            &[header_with(1, "笔记", "z@z.com", "", false, false, 200)],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            2,
            f2[0].id,
            20,
            &[header_with(
                1,
                "INBOX 下归档",
                "w@w.com",
                "",
                false,
                false,
                300,
            )],
            None,
        )
        .unwrap();

    let archived = cache.list_archived_messages("", 0, 10).unwrap();
    assert_eq!(archived.total, 2);
    assert_eq!(archived.items[0].subject, "INBOX 下归档");
    assert_eq!(archived.items[1].subject, "已归档邮件");

    let found = cache.find_archive_folder(1).unwrap();
    assert_eq!(found, Some((archive, "Archive".to_string())));
}

#[test]
fn attachment_gallery_joins_messages_and_filters() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
    let sent = folders.iter().find(|f| f.path == "Sent").unwrap().id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            10,
            &[
                header_with(1, "照片分享", "Bob <bob@b.com>", "", false, false, 200),
                header_with(2, "季度报告", "Alice <alice@a.com>", "", false, false, 100),
            ],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            sent,
            20,
            &[header_with(
                1,
                "合同扫描件",
                "Carol <carol@c.com>",
                "",
                false,
                false,
                300,
            )],
            None,
        )
        .unwrap();

    let inbox_page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let photo_id = inbox_page.items[0].id;
    let sent_page = cache.list_messages_page(sent, "", 0, 10).unwrap();
    let contract_id = sent_page.items[0].id;

    cache
        .store_message_body(
            photo_id,
            "正文",
            &[CachedAttachmentInput::downloaded(
                "holiday.jpg",
                "image/jpeg",
                vec![1, 2, 3],
            )],
        )
        .unwrap();
    cache
        .store_message_body(
            contract_id,
            "正文",
            &[CachedAttachmentInput::downloaded(
                "contract.pdf",
                "application/pdf",
                vec![4],
            )],
        )
        .unwrap();

    let all = cache.list_cached_attachments("", 0, 10).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].filename, "contract.pdf");
    assert_eq!(all[0].subject, "合同扫描件");
    assert_eq!(all[0].sender, "Carol <carol@c.com>");
    assert_eq!(all[0].size_bytes, 1);
    assert_eq!(all[1].filename, "holiday.jpg");
    assert_eq!(all[1].message_id, photo_id);
    assert_eq!(all[1].account_id, 1);
    assert_eq!(all[1].mime_type, "image/jpeg");

    let by_name = cache.list_cached_attachments("contract", 0, 10).unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].filename, "contract.pdf");

    let by_subject = cache.list_cached_attachments("照片", 0, 10).unwrap();
    assert_eq!(by_subject.len(), 1);
    assert_eq!(by_subject[0].filename, "holiday.jpg");

    let by_sender = cache.list_cached_attachments("bob@", 0, 10).unwrap();
    assert_eq!(by_sender.len(), 1);
    assert_eq!(by_sender[0].filename, "holiday.jpg");

    let paged = cache.list_cached_attachments("", 1, 1).unwrap();
    assert_eq!(paged.len(), 1);
    assert_eq!(paged[0].filename, "holiday.jpg");
}

#[test]
fn known_contacts_dedupe_rank_and_filter() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();

    cache
        .commit_folder_headers(
            1,
            folders[0].id,
            10,
            &[
                header_with(
                    1,
                    "A",
                    "Alice <alice@a.com>",
                    "bob@b.com, Carol <carol@c.com>",
                    false,
                    false,
                    100,
                ),
                header_with(2, "B", "alice@a.com", "Bob <BOB@b.com>", false, false, 200),
                header_with(3, "C", "(未知)", "名字 <>", false, false, 300),
            ],
            None,
        )
        .unwrap();

    let contacts = cache.list_known_contacts("", 0, 50).unwrap();
    assert_eq!(contacts.len(), 3);
    assert_eq!(contacts[0].email, "alice@a.com");
    assert_eq!(contacts[0].name, "Alice");
    assert_eq!(contacts[0].message_count, 2);
    assert_eq!(contacts[1].email, "bob@b.com");
    assert_eq!(contacts[1].name, "Bob");
    assert_eq!(contacts[1].message_count, 2);
    assert_eq!(contacts[2].email, "carol@c.com");
    assert_eq!(contacts[2].name, "Carol");
    assert_eq!(contacts[2].message_count, 1);

    let filtered = cache.list_known_contacts("carol", 0, 50).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].email, "carol@c.com");

    let paged = cache.list_known_contacts("", 1, 1).unwrap();
    assert_eq!(paged.len(), 1);
    assert_eq!(paged[0].email, "bob@b.com");
}

#[test]
fn clear_message_bodies_keeps_headers_and_clears_attachments() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("归档", "Archive")])
        .unwrap();
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            10,
            &[
                sample_header(1, "带正文与附件"),
                sample_header(2, "仅邮件头"),
            ],
            None,
        )
        .unwrap();
    let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let with_body_id = page
        .items
        .iter()
        .find(|m| m.subject == "带正文与附件")
        .unwrap()
        .id;
    let header_only_id = page
        .items
        .iter()
        .find(|m| m.subject == "仅邮件头")
        .unwrap()
        .id;

    cache
        .store_message_body(
            with_body_id,
            "这是邮件正文",
            &[CachedAttachmentInput::downloaded(
                "report.pdf",
                "application/pdf",
                vec![9, 9, 9],
            )],
        )
        .unwrap();

    let (_, _, body_loaded) = cache.get_message_detail(with_body_id).unwrap().unwrap();
    assert!(body_loaded);
    assert_eq!(cache.list_cached_attachments("", 0, 10).unwrap().len(), 1);

    let cleared = cache.clear_message_bodies().unwrap();
    assert_eq!(cleared, 1);
    assert_eq!(cache.clear_message_bodies().unwrap(), 0);

    let (detail, _, body_loaded) = cache.get_message_detail(with_body_id).unwrap().unwrap();
    assert!(!body_loaded);
    assert!(detail.body_text.is_empty());
    assert!(detail.attachments.is_empty());
    assert_eq!(detail.subject, "带正文与附件");
    assert!(cache.list_cached_attachments("", 0, 10).unwrap().is_empty());

    let (_, _, header_body_loaded) = cache.get_message_detail(header_only_id).unwrap().unwrap();
    assert!(!header_body_loaded);

    let unified = cache.list_unified_inbox("", 0, 10).unwrap();
    assert_eq!(unified.total, 2);
    let subjects: Vec<&str> = unified.items.iter().map(|m| m.subject.as_str()).collect();
    assert!(subjects.contains(&"带正文与附件"));
    assert!(subjects.contains(&"仅邮件头"));
}

#[test]
fn rename_folder_moves_only_descendants_and_preserves_ids() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache
        .sync_folders(
            1,
            &[
                folder("A", "A"),
                folder("Child", "A/Child"),
                folder("Nested", "A/Child/Nested"),
                folder("AB", "AB/Child"),
            ],
        )
        .unwrap();
    let id = |path: &str| folders.iter().find(|f| f.path == path).unwrap().id;
    let renamed = cache
        .rename_folder_entry(1, id("A"), "A", "B", "B", Some("/"))
        .unwrap();
    for (path, original) in [
        ("B", "A"),
        ("B/Child", "A/Child"),
        ("B/Child/Nested", "A/Child/Nested"),
        ("AB/Child", "AB/Child"),
    ] {
        assert_eq!(
            renamed.iter().find(|f| f.path == path).unwrap().id,
            id(original)
        );
    }
}

#[test]
fn role_folder_finders_match_names_and_paths() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("已删除", "Trash"),
                folder("归档", "Archive"),
                folder("Notes", "Notes"),
            ],
        )
        .unwrap();
    let archive = folders.iter().find(|f| f.path == "Archive").unwrap();
    let trash = folders.iter().find(|f| f.path == "Trash").unwrap();

    assert_eq!(
        cache.find_archive_folder(1).unwrap(),
        Some((archive.id, "Archive".to_string()))
    );
    assert_eq!(
        cache.find_trash_folder(1).unwrap(),
        Some((trash.id, "Trash".to_string()))
    );
    assert_eq!(cache.find_archive_folder(2).unwrap(), None);
    assert_eq!(cache.find_trash_folder(2).unwrap(), None);
}

#[test]
fn remove_message_updates_counts_and_cascades_attachments() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            10,
            &[
                header_with(
                    1,
                    "被移除的邮件",
                    "Alice <alice@a.com>",
                    "",
                    false,
                    false,
                    100,
                ),
                header_with(2, "保留的未读邮件", "Bob <bob@b.com>", "", false, true, 200),
            ],
            None,
        )
        .unwrap();

    let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let target_id = page
        .items
        .iter()
        .find(|m| m.subject == "被移除的邮件")
        .unwrap()
        .id;
    cache
        .store_message_body(
            target_id,
            "正文",
            &[CachedAttachmentInput::downloaded(
                "gone.txt",
                "text/plain",
                vec![1],
            )],
        )
        .unwrap();
    assert_eq!(cache.list_cached_attachments("", 0, 10).unwrap().len(), 1);

    let removed = cache.remove_message(target_id).unwrap();
    assert_eq!(removed, Some(inbox));
    assert!(cache.get_message_detail(target_id).unwrap().is_none());
    assert!(cache.list_cached_attachments("", 0, 10).unwrap().is_empty());

    let folders_after = cache.list_folders(1).unwrap();
    assert_eq!(folders_after[0].total_count, 1);
    assert_eq!(folders_after[0].unread_count, 1);

    let page_after = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    assert_eq!(page_after.total, 1);
    assert_eq!(page_after.items[0].subject, "保留的未读邮件");

    assert_eq!(cache.remove_message(target_id).unwrap(), None);
}

#[test]
fn message_filter_queries_return_exact_totals_and_items() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;
    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[
                CachedHeaderInput {
                    uid: 1,
                    message_id: "<1@test>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Alice <a@test>".to_string(),
                    recipients: "me@test".to_string(),
                    cc: String::new(),
                    subject: "未读且星标".to_string(),
                    preview: String::new(),
                    date_ts: 300,
                    date_display: "10:00".to_string(),
                    unread: true,
                    flagged: true,
                    has_attachment: false,
                },
                CachedHeaderInput {
                    uid: 2,
                    message_id: "<2@test>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Bob <b@test>".to_string(),
                    recipients: "me@test".to_string(),
                    cc: String::new(),
                    subject: "仅未读".to_string(),
                    preview: String::new(),
                    date_ts: 200,
                    date_display: "09:00".to_string(),
                    unread: true,
                    flagged: false,
                    has_attachment: false,
                },
                CachedHeaderInput {
                    uid: 3,
                    message_id: "<3@test>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Carol <c@test>".to_string(),
                    recipients: "me@test".to_string(),
                    cc: String::new(),
                    subject: "已读星标".to_string(),
                    preview: String::new(),
                    date_ts: 100,
                    date_display: "08:00".to_string(),
                    unread: false,
                    flagged: true,
                    has_attachment: false,
                },
            ],
            Some(&[1, 2, 3]),
        )
        .unwrap();

    let unread = cache
        .list_messages_page_filtered(inbox, "", MessageFilter::Unread, 0, 10)
        .unwrap();
    assert_eq!(unread.total, 2);
    assert!(unread.items.iter().all(|m| m.unread));

    let starred = cache
        .list_unified_inbox_filtered("", MessageFilter::Starred, 0, 10)
        .unwrap();
    assert_eq!(starred.total, 2);
    assert!(starred.items.iter().all(|m| m.flagged));

    let unread_starred = cache
        .list_starred_messages_filtered("", MessageFilter::Unread, 0, 10)
        .unwrap();
    assert_eq!(unread_starred.total, 1);
    assert_eq!(unread_starred.items[0].subject, "未读且星标");
}

#[test]
fn thread_subject_normalization_and_grouping_work() {
    assert_eq!(
        normalize_thread_subject("Re: Fwd: 回复：Q3 Roadmap "),
        "q3 roadmap"
    );
    assert_eq!(normalize_thread_subject("转发: 项目进度"), "项目进度");

    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox = folders[0].id;
    assert_eq!(
        cache.find_sent_folder(1).unwrap().map(|(_, p)| p),
        Some("Sent".to_string())
    );

    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[
                CachedHeaderInput {
                    uid: 1,
                    message_id: "<m1@test>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Alice <a@test>".to_string(),
                    recipients: "me@test".to_string(),
                    cc: String::new(),
                    subject: "Q3 Roadmap".to_string(),
                    preview: "第一封".to_string(),
                    date_ts: 100,
                    date_display: "09:00".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                },
                CachedHeaderInput {
                    uid: 2,
                    message_id: "<m2@test>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Bob <b@test>".to_string(),
                    recipients: "me@test".to_string(),
                    cc: String::new(),
                    subject: "Re: Q3 Roadmap".to_string(),
                    preview: "第二封回复".to_string(),
                    date_ts: 200,
                    date_display: "10:00".to_string(),
                    unread: true,
                    flagged: false,
                    has_attachment: false,
                },
            ],
            Some(&[1, 2]),
        )
        .unwrap();

    let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let latest_id = page.items[0].id;

    let desc = cache
        .list_thread_messages(1, latest_id, "Re: Q3 Roadmap", true)
        .unwrap();
    assert_eq!(desc.len(), 2);
    assert_eq!(desc[0].sender, "Bob <b@test>");
    assert_eq!(desc[1].sender, "Alice <a@test>");

    let asc = cache
        .list_thread_messages(1, latest_id, "Re: Q3 Roadmap", false)
        .unwrap();
    assert_eq!(asc.len(), 2);
    assert_eq!(asc[0].sender, "Alice <a@test>");
    assert_eq!(asc[1].sender, "Bob <b@test>");
}

#[test]
fn rfc_threading_links_different_subjects_and_dedupes_cross_folder_copies() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("已发送", "Sent"),
                folder("已删除", "Trash"),
            ],
        )
        .unwrap();
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
    let sent = folders.iter().find(|f| f.path == "Sent").unwrap().id;
    let trash = folders.iter().find(|f| f.path == "Trash").unwrap().id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[
                CachedHeaderInput {
                    uid: 10,
                    message_id: "<root@example.com>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Alice <alice@example.com>".to_string(),
                    recipients: "me@example.com".to_string(),
                    cc: String::new(),
                    subject: "原始需求讨论".to_string(),
                    preview: "第一封".to_string(),
                    date_ts: 1000,
                    date_display: "09:00".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                },
                CachedHeaderInput {
                    uid: 12,
                    message_id: "<reply2@example.com>".to_string(),
                    in_reply_to: "<reply1@example.com>".to_string(),
                    references_header: "<root@example.com> <reply1@example.com>".to_string(),
                    sender: "Alice <alice@example.com>".to_string(),
                    recipients: "me@example.com".to_string(),
                    cc: String::new(),
                    subject: "全新主题名（依然同线程）".to_string(),
                    preview: "第三封".to_string(),
                    date_ts: 3000,
                    date_display: "11:00".to_string(),
                    unread: true,
                    flagged: false,
                    has_attachment: false,
                },
            ],
            None,
        )
        .unwrap();

    cache
        .commit_folder_headers(
            1,
            sent,
            2,
            &[CachedHeaderInput {
                uid: 20,
                message_id: "<reply1@example.com>".to_string(),
                in_reply_to: "<root@example.com>".to_string(),
                references_header: "<root@example.com>".to_string(),
                sender: "Me <me@example.com>".to_string(),
                recipients: "alice@example.com".to_string(),
                cc: String::new(),
                subject: "改过标题的回复".to_string(),
                preview: "第二封".to_string(),
                date_ts: 2000,
                date_display: "10:00".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
            }],
            None,
        )
        .unwrap();

    cache
        .commit_folder_headers(
            1,
            trash,
            3,
            &[CachedHeaderInput {
                uid: 99,
                message_id: "<root@example.com>".to_string(),
                in_reply_to: String::new(),
                references_header: String::new(),
                sender: "Alice <alice@example.com>".to_string(),
                recipients: "me@example.com".to_string(),
                cc: String::new(),
                subject: "原始需求讨论".to_string(),
                preview: "回收站副本".to_string(),
                date_ts: 1000,
                date_display: "09:00".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
            }],
            None,
        )
        .unwrap();

    let inbox_page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    assert_eq!(inbox_page.total, 1);
    assert_eq!(inbox_page.items.len(), 1);
    assert_eq!(inbox_page.items[0].uid, 12);
    assert_eq!(inbox_page.items[0].thread_count, 3);
    let reply2_id = inbox_page.items[0].id;
    let root_inbox_id = cache.list_summaries_by_uids(inbox, &[10]).unwrap()[0].id;

    let desc = cache
        .list_thread_messages(1, reply2_id, "全新主题名（依然同线程）", true)
        .unwrap();
    assert_eq!(desc.len(), 3);
    assert_eq!(desc[0].rfc_message_id, "<reply2@example.com>");
    assert_eq!(desc[1].rfc_message_id, "<reply1@example.com>");
    assert_eq!(desc[2].rfc_message_id, "<root@example.com>");
    assert_eq!(desc[2].id, root_inbox_id);

    let asc = cache
        .list_thread_messages(1, reply2_id, "全新主题名（依然同线程）", false)
        .unwrap();
    assert_eq!(asc.len(), 3);
    assert_eq!(asc[0].rfc_message_id, "<root@example.com>");
    assert_eq!(asc[1].rfc_message_id, "<reply1@example.com>");
    assert_eq!(asc[2].rfc_message_id, "<reply2@example.com>");

    let (detail, _, _) = cache.get_message_detail(reply2_id).unwrap().unwrap();
    assert_eq!(detail.thread_messages.len(), 3);
    assert_eq!(detail.thread_messages[0].id, reply2_id);
}

#[test]
fn list_thread_messages_excludes_trash_and_junk_unless_anchor_is_there() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("已删除", "Trash"),
                folder("垃圾邮件", "Junk"),
            ],
        )
        .unwrap();
    let folders = cache.list_folders(1).unwrap();
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
    let trash = folders.iter().find(|f| f.path == "Trash").unwrap().id;
    let junk = folders.iter().find(|f| f.path == "Junk").unwrap().id;

    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[CachedHeaderInput {
                uid: 1,
                message_id: "<root@example.com>".to_string(),
                in_reply_to: String::new(),
                references_header: String::new(),
                sender: "Alice <alice@example.com>".to_string(),
                recipients: "me@example.com".to_string(),
                cc: String::new(),
                subject: "Thread Cleanup".to_string(),
                preview: "root".to_string(),
                date_ts: 1000,
                date_display: "09:00".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
            }],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            trash,
            2,
            &[CachedHeaderInput {
                uid: 2,
                message_id: "<deleted-reply@example.com>".to_string(),
                in_reply_to: "<root@example.com>".to_string(),
                references_header: "<root@example.com>".to_string(),
                sender: "Bob <bob@example.com>".to_string(),
                recipients: "me@example.com".to_string(),
                cc: String::new(),
                subject: "Re: Thread Cleanup".to_string(),
                preview: "in trash".to_string(),
                date_ts: 2000,
                date_display: "10:00".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
            }],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            junk,
            3,
            &[CachedHeaderInput {
                uid: 3,
                message_id: "<spam-reply@example.com>".to_string(),
                in_reply_to: "<root@example.com>".to_string(),
                references_header: "<root@example.com>".to_string(),
                sender: "Spammer <spam@example.com>".to_string(),
                recipients: "me@example.com".to_string(),
                cc: String::new(),
                subject: "Re: Thread Cleanup".to_string(),
                preview: "in junk".to_string(),
                date_ts: 3000,
                date_display: "11:00".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
            }],
            None,
        )
        .unwrap();

    let root_id = cache.list_messages_page(inbox, "", 0, 10).unwrap().items[0].id;
    let from_inbox = cache
        .list_thread_messages(1, root_id, "Thread Cleanup", true)
        .unwrap();
    assert_eq!(from_inbox.len(), 1);
    assert_eq!(from_inbox[0].rfc_message_id, "<root@example.com>");

    let trash_id = cache.list_messages_page(trash, "", 0, 10).unwrap().items[0].id;
    let from_trash = cache
        .list_thread_messages(1, trash_id, "Re: Thread Cleanup", true)
        .unwrap();
    assert!(from_trash
        .iter()
        .any(|m| m.rfc_message_id == "<deleted-reply@example.com>"));
}

#[test]
fn reconcile_folder_flags_updates_older_messages_and_unread_count() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = cache.list_folders(1).unwrap()[0].id;

    let mk = |uid: u32, unread: bool, flagged: bool| CachedHeaderInput {
        uid,
        message_id: format!("<m-{uid}@example.com>"),
        in_reply_to: String::new(),
        references_header: String::new(),
        sender: "Alice <alice@example.com>".to_string(),
        recipients: "me@example.com".to_string(),
        cc: String::new(),
        subject: format!("Mail {uid}"),
        preview: "preview".to_string(),
        date_ts: 1000 + uid as i64,
        date_display: "10:00".to_string(),
        unread,
        flagged,
        has_attachment: false,
    };
    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[mk(1, true, false), mk(2, false, true), mk(3, true, true)],
            None,
        )
        .unwrap();

    let flagged: HashSet<u32> = [1, 3].into();
    let unseen: HashSet<u32> = [2, 3].into();
    let updated = cache
        .reconcile_folder_flags(inbox, &flagged, &unseen)
        .unwrap();
    assert_eq!(updated, 2);

    let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let m1 = page.items.iter().find(|m| m.uid == 1).unwrap();
    let m2 = page.items.iter().find(|m| m.uid == 2).unwrap();
    let m3 = page.items.iter().find(|m| m.uid == 3).unwrap();
    assert!(!m1.unread && m1.flagged);
    assert!(m2.unread && !m2.flagged);
    assert!(m3.unread && m3.flagged);

    let folder = cache.list_folders(1).unwrap().into_iter().next().unwrap();
    assert_eq!(folder.unread_count, 2);
}

#[test]
fn unique_attachment_path_appends_incrementing_suffix_on_collision() {
    let dir = temp_cache_dir("att-collision");
    let p0 = unique_attachment_path(&dir, "report.pdf");
    assert_eq!(p0, dir.join("report.pdf"));
    std::fs::write(&p0, b"first").unwrap();

    let p1 = unique_attachment_path(&dir, "report.pdf");
    assert_eq!(p1, dir.join("report (1).pdf"));
    std::fs::write(&p1, b"second").unwrap();

    let p2 = unique_attachment_path(&dir, "report.pdf");
    assert_eq!(p2, dir.join("report (2).pdf"));

    let no_ext0 = unique_attachment_path(&dir, "README");
    assert_eq!(no_ext0, dir.join("README"));
    std::fs::write(&no_ext0, b"readme").unwrap();
    let no_ext1 = unique_attachment_path(&dir, "README");
    assert_eq!(no_ext1, dir.join("README (1)"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn remote_cursor_and_incremental_older_batch_preserve_cached_history() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox = folders[0].id;
    assert!(!folders[0].has_more_remote);

    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[
                sample_header(101, "较新邮件1"),
                sample_header(102, "最新邮件2"),
            ],
            Some(&[50, 51, 101, 102]),
        )
        .unwrap();
    cache
        .set_folder_remote_cursor(inbox, true, "https://graph.example/next?skip=100")
        .unwrap();

    let (acc_id, path, has_more, next_link, min_uid, count) =
        cache.folder_remote_cursor(inbox).unwrap().unwrap();
    assert_eq!(acc_id, 1);
    assert_eq!(path, "INBOX");
    assert!(has_more);
    assert_eq!(next_link, "https://graph.example/next?skip=100");
    assert_eq!(min_uid, Some(101));
    assert_eq!(count, 2);

    let pending_inboxes = cache.list_inbox_folders_with_more_remote().unwrap();
    assert_eq!(pending_inboxes.len(), 1);
    assert_eq!(pending_inboxes[0].0, inbox);

    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[
                sample_header(50, "更早邮件1"),
                sample_header(51, "更早邮件2"),
            ],
            Some(&[50, 51, 101, 102]),
        )
        .unwrap();
    cache.set_folder_remote_cursor(inbox, false, "").unwrap();
    cache
        .commit_folder_headers(1, inbox, 1, &[sample_header(103, "新到达邮件")], None)
        .unwrap();

    let (_, _, has_more_after, next_after, min_uid_after, count_after) =
        cache.folder_remote_cursor(inbox).unwrap().unwrap();
    assert!(!has_more_after);
    assert!(next_after.is_empty());
    assert_eq!(min_uid_after, Some(50));
    assert_eq!(count_after, 5);
    assert!(cache
        .list_inbox_folders_with_more_remote()
        .unwrap()
        .is_empty());

    let reloaded = cache.list_folders(1).unwrap();
    assert!(!reloaded[0].has_more_remote);
    assert_eq!(reloaded[0].total_count, 5);
}

#[test]
fn background_search_text_covers_cached_archive_without_marking_body_loaded() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let archive = cache.sync_folders(1, &[folder("归档", "Archive")]).unwrap()[0].id;
    cache
        .commit_folder_headers(1, archive, 1, &[sample_header(9, "旧邮件")], None)
        .unwrap();

    let pending = cache.list_unindexed_messages(archive, 5).unwrap();
    assert_eq!(pending.len(), 1);
    let (id, uid, _) = pending[0];
    assert_eq!(uid, 9);
    assert_eq!(
        cache.list_backfill_folders().unwrap(),
        vec![(archive, 1, "Archive".to_string(), false)]
    );

    let text = format!("{}末尾合同 archivecode", "正文开头".repeat(1500));
    cache.store_index_text(id, &text).unwrap();
    assert!(cache
        .list_unindexed_messages(archive, 5)
        .unwrap()
        .is_empty());
    assert!(cache.list_backfill_folders().unwrap().is_empty());
    assert!(!cache.get_message_detail(id).unwrap().unwrap().2);

    for query in ["archivecode", "末尾合同", "archivecode 末尾合同"] {
        let page = cache
            .search_all_folders(
                &[1],
                query,
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                5,
            )
            .unwrap();
        assert_eq!(page.items[0].id, id);
    }

    assert_eq!(cache.clear_message_bodies().unwrap(), 1);
    assert_eq!(
        cache
            .list_messages_page(archive, "archivecode", 0, 5)
            .unwrap()
            .total,
        0
    );
    assert_eq!(cache.list_unindexed_messages(archive, 5).unwrap().len(), 1);

    cache.store_index_text(id, "stalesearchexample").unwrap();
    cache
        .store_message_body(id, "fullsearchexample", &[])
        .unwrap();
    assert!(cache.get_message_detail(id).unwrap().unwrap().2);
    assert_eq!(
        cache
            .list_messages_page(archive, "stalesearchexample", 0, 5)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        cache
            .list_messages_page(archive, "fullsearchexample", 0, 5)
            .unwrap()
            .total,
        1
    );
}

#[test]
fn fts5_and_chinese_body_search_with_structured_operators() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("归档", "Archive")])
        .unwrap();
    let inbox = folders[0].id;
    let archive = folders[1].id;

    let mut h1 = sample_header(1, "Q4 Financial Review");
    h1.sender = "Mário Rossi <mario@example.com>".to_string();
    h1.recipients = "财务部 <finance@company.cn>".to_string();
    h1.unread = true;
    h1.flagged = true;
    h1.has_attachment = true;
    h1.date_ts = 1_700_000_200;

    let mut h2 = sample_header(2, "Weekly Standup");
    h2.sender = "Bob Lee <bob@example.com>".to_string();
    h2.recipients = "team@company.cn".to_string();
    h2.unread = false;
    h2.flagged = false;
    h2.has_attachment = false;
    h2.date_ts = 1_700_000_100;

    let mut h3 = sample_header(3, "采购协议归档");
    h3.sender = "法务部 <legal@company.cn>".to_string();
    h3.recipients = "张总 <ceo@company.cn>".to_string();
    h3.unread = false;
    h3.flagged = true;
    h3.has_attachment = true;
    h3.date_ts = 1_700_000_300;

    cache
        .commit_folder_headers(1, inbox, 1, &[h1, h2], None)
        .unwrap();
    cache
        .commit_folder_headers(1, archive, 1, &[h3], None)
        .unwrap();

    let inbox_page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let m1_id = inbox_page.items.iter().find(|m| m.uid == 1).unwrap().id;
    let m2_id = inbox_page.items.iter().find(|m| m.uid == 2).unwrap().id;
    let archive_page = cache.list_messages_page(archive, "", 0, 10).unwrap();
    let m3_id = archive_page.items[0].id;

    cache
        .store_message_body(m1_id, "请核对本月增值税发票及 quarterly budget 报表。", &[])
        .unwrap();
    cache
        .store_message_body(m2_id, "Everything is on track for Friday release.", &[])
        .unwrap();
    cache
        .store_message_body(m3_id, "附件为最终签署的年度采购合同扫描件。", &[])
        .unwrap();

    let cn_invoice = cache.list_messages_page(inbox, "发票", 0, 10).unwrap();
    assert_eq!(cn_invoice.total, 1);
    assert_eq!(cn_invoice.items[0].uid, 1);

    let cn_contract = cache
        .search_all_folders(
            &[1],
            "合同",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(cn_contract.total, 1);
    assert_eq!(cn_contract.items[0].uid, 3);

    let fts_prefix = cache.list_messages_page(inbox, "quart", 0, 10).unwrap();
    assert_eq!(fts_prefix.total, 1);
    assert_eq!(fts_prefix.items[0].uid, 1);

    let fts_phrase = cache
        .list_messages_page(inbox, r#""quarterly budget""#, 0, 10)
        .unwrap();
    assert_eq!(fts_phrase.total, 1);
    assert_eq!(fts_phrase.items[0].uid, 1);

    let fts_diacritic = cache.list_messages_page(inbox, "mario", 0, 10).unwrap();
    assert_eq!(fts_diacritic.total, 1);
    assert_eq!(fts_diacritic.items[0].uid, 1);

    let op_from_to = cache
        .search_all_folders(
            &[1],
            "from:mario to:finance has:attachment is:unread is:starred",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(op_from_to.total, 1);
    assert_eq!(op_from_to.items[0].uid, 1);

    let op_read_in = cache
        .search_all_folders(
            &[1],
            "is:read in:archive",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(op_read_in.total, 1);
    assert_eq!(op_read_in.items[0].uid, 3);
}

#[test]
fn sort_orders_and_multi_quick_filters() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;
    let now_ts = chrono::Utc::now().timestamp();
    let old_ts = now_ts - 10 * 86_400;

    let mut a = sample_header(1, "Zebra");
    a.sender = "Charlie <c@x.com>".to_string();
    a.date_ts = old_ts;
    a.unread = false;
    a.flagged = true;
    a.has_attachment = false;

    let mut b = sample_header(2, "Alpha");
    b.sender = "Alice <a@x.com>".to_string();
    b.date_ts = now_ts;
    b.unread = true;
    b.flagged = true;
    b.has_attachment = true;

    let mut c = sample_header(3, "Middle");
    c.sender = "Bob <b@x.com>".to_string();
    c.date_ts = now_ts - 3600;
    c.unread = true;
    c.flagged = false;
    c.has_attachment = true;

    cache
        .commit_folder_headers(1, inbox, 1, &[a, b, c], None)
        .unwrap();

    let uids_for = |sort: MessageSort| -> Vec<u32> {
        cache
            .list_messages_page_with_options(inbox, "", QuickFilterSet::default(), sort, 0, 10)
            .unwrap()
            .items
            .into_iter()
            .map(|m| m.uid)
            .collect()
    };

    assert_eq!(uids_for(MessageSort::DateNewest), vec![2, 3, 1]);
    assert_eq!(uids_for(MessageSort::DateOldest), vec![1, 3, 2]);
    assert_eq!(uids_for(MessageSort::SenderAsc), vec![2, 3, 1]);
    assert_eq!(uids_for(MessageSort::SubjectAsc), vec![2, 3, 1]);
    assert_eq!(uids_for(MessageSort::UnreadFirst), vec![2, 3, 1]);
    assert_eq!(uids_for(MessageSort::StarredFirst), vec![2, 1, 3]);

    let multi = QuickFilterSet {
        unread: true,
        starred: true,
        has_attachment: true,
        today: true,
        ..QuickFilterSet::default()
    };
    let filtered = cache
        .list_messages_page_with_options(inbox, "", multi, MessageSort::DateNewest, 0, 10)
        .unwrap();
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.items[0].uid, 2);

    let msg_id = filtered.items[0].id;
    cache
        .set_message_tags(msg_id, &["重要".to_string(), "工作".to_string()])
        .unwrap();
    cache.snooze_message(msg_id, 1_900_000_000).unwrap();
    let tagged_page = cache
        .list_messages_page_with_options(
            inbox,
            "",
            QuickFilterSet {
                tagged: true,
                ..QuickFilterSet::default()
            },
            MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(tagged_page.total, 1);
    assert_eq!(tagged_page.items[0].tags, vec!["重要", "工作"]);
    assert_eq!(tagged_page.items[0].snoozed_until_ts, 1_900_000_000);
}

#[test]
fn cross_folder_search_excludes_waste_and_deduplicates_by_message_id() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("归档", "Archive"),
                folder("已删除", "Trash"),
                folder("垃圾邮件", "Junk"),
            ],
        )
        .unwrap();
    let inbox = folders[0].id;
    let archive = folders[1].id;
    let trash = folders[2].id;
    let junk = folders[3].id;

    let mut in_copy = sample_header(10, "重要合同通知");
    in_copy.message_id = "<dup-001@example.com>".to_string();
    let mut arch_copy = sample_header(20, "重要合同通知");
    arch_copy.message_id = "<dup-001@example.com>".to_string();
    let mut arch_only = sample_header(21, "归档独有合同");
    arch_only.message_id = "<arch-002@example.com>".to_string();
    let mut trash_msg = sample_header(30, "已删除合同");
    trash_msg.message_id = "<trash-003@example.com>".to_string();
    let mut junk_msg = sample_header(40, "垃圾合同广告");
    junk_msg.message_id = "<junk-004@example.com>".to_string();

    cache
        .commit_folder_headers(1, inbox, 1, &[in_copy], None)
        .unwrap();
    cache
        .commit_folder_headers(1, archive, 1, &[arch_copy, arch_only], None)
        .unwrap();
    cache
        .commit_folder_headers(1, trash, 1, &[trash_msg], None)
        .unwrap();
    cache
        .commit_folder_headers(1, junk, 1, &[junk_msg], None)
        .unwrap();

    let all = cache
        .search_all_folders(
            &[1],
            "合同",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            20,
        )
        .unwrap();
    assert_eq!(all.total, 2, "应排除 Trash/Junk 并按 message_id 去重");
    assert!(all
        .items
        .iter()
        .any(|m| m.folder_path == "INBOX" && m.uid == 10));
    assert!(all
        .items
        .iter()
        .any(|m| m.folder_path == "Archive" && m.uid == 21));

    let in_trash = cache
        .search_all_folders(
            &[1],
            "in:trash 合同",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            20,
        )
        .unwrap();
    assert_eq!(in_trash.total, 1);
    assert_eq!(in_trash.items[0].uid, 30);
}

#[test]
fn find_spam_folder_matches_junk_spam_and_chinese_names() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("垃圾邮件", "Junk")])
        .unwrap();
    let (_, path1) = cache
        .find_spam_folder(1)
        .unwrap()
        .expect("should find Junk");
    assert_eq!(path1, "Junk");

    cache
        .sync_folders(2, &[folder("Spam", "[Gmail]/Spam")])
        .unwrap();
    let (_, path2) = cache
        .find_spam_folder(2)
        .unwrap()
        .expect("should find Spam");
    assert_eq!(path2, "[Gmail]/Spam");

    assert!(cache.find_spam_folder(99).unwrap().is_none());
}

#[test]
fn export_message_attachments_writes_sanitized_unique_files() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;
    cache
        .commit_folder_headers(1, inbox, 1, &[sample_header(10, "带附件邮件")], None)
        .unwrap();
    let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let msg_id = page.items[0].id;
    cache
        .store_message_body(
            msg_id,
            "hello",
            &[
                CachedAttachmentInput::downloaded(
                    "report.pdf",
                    "application/pdf",
                    b"pdf-1".to_vec(),
                ),
                CachedAttachmentInput::downloaded(
                    "report.pdf",
                    "application/pdf",
                    b"pdf-2".to_vec(),
                ),
                CachedAttachmentInput::downloaded(
                    "../bad:name?.txt",
                    "text/plain",
                    b"txt".to_vec(),
                ),
            ],
        )
        .unwrap();

    let dest_dir = temp_cache_dir("forward-export");
    let paths = cache.export_message_attachments(msg_id, &dest_dir).unwrap();
    assert_eq!(paths.len(), 3);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"pdf-1");
    assert_eq!(std::fs::read(&paths[1]).unwrap(), b"pdf-2");
    assert_eq!(std::fs::read(&paths[2]).unwrap(), b"txt");
    assert_ne!(paths[0], paths[1]);
    assert!(paths[2].starts_with(&dest_dir));
    let _ = std::fs::remove_dir_all(&dest_dir);
}

#[test]
fn sanitize_filename_replaces_windows_illegal_chars() {
    assert_eq!(
        sanitize_filename("周报 Q3/四季度:评审?"),
        "周报 Q3_四季度_评审_"
    );
    assert_eq!(sanitize_filename("  a*b|c<d>e \"f\"  "), "a_b_c_d_e _f_");
    assert_eq!(sanitize_filename(" . .. "), "attachment");
    assert_eq!(sanitize_filename(""), "attachment");
    assert_eq!(sanitize_filename("普通主题"), "普通主题");
}

#[test]
fn list_page_fills_thread_counts_by_normalized_subject() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox_id = folders[0].id;
    let sent_id = folders[1].id;

    let mut h1 = sample_header(1, "项目排期讨论");
    h1.date_ts = 1_700_000_100;
    let mut h2 = sample_header(2, "Re: 项目排期讨论");
    h2.date_ts = 1_700_000_200;
    h2.recipients = "team@example.com".to_string();
    let mut h3 = sample_header(3, "Fwd: 项目排期讨论");
    h3.date_ts = 1_700_000_300;
    let mut h4 = sample_header(4, "独立主题邮件");
    h4.date_ts = 1_700_000_400;
    cache
        .commit_folder_headers(1, inbox_id, 100, &[h1, h2, h3, h4], None)
        .unwrap();
    let mut hs = sample_header(5, "Re: 项目排期讨论");
    hs.date_ts = 1_700_000_500;
    cache
        .commit_folder_headers(1, sent_id, 200, &[hs], None)
        .unwrap();

    let page = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items.len(), 2);
    let by_subject: HashMap<String, u32> = page
        .items
        .iter()
        .map(|m| (m.subject.clone(), m.thread_count))
        .collect();
    assert_eq!(by_subject["Fwd: 项目排期讨论"], 4);
    assert_eq!(by_subject["独立主题邮件"], 1);
    let rep = page
        .items
        .iter()
        .find(|m| m.subject == "Fwd: 项目排期讨论")
        .unwrap();
    let children = cache
        .list_thread_summaries(1, rep.id, &rep.subject)
        .unwrap();
    let with_recipients = children
        .iter()
        .find(|m| m.folder_id == inbox_id && m.subject == "Re: 项目排期讨论")
        .unwrap();
    assert_eq!(with_recipients.recipients, "team@example.com");
}

#[test]
fn extract_unsubscribe_url_picks_first_supported_handle() {
    assert_eq!(
        extract_unsubscribe_url(
            "<https://lists.example.com/unsub?id=1>, <mailto:leave@lists.example.com>"
        ),
        Some("https://lists.example.com/unsub?id=1".to_string())
    );
    assert_eq!(
        extract_unsubscribe_url("<mailto:leave@lists.example.com?subject=unsubscribe>"),
        Some("mailto:leave@lists.example.com?subject=unsubscribe".to_string())
    );
    assert_eq!(
        extract_unsubscribe_url("https://unsub.example.com/u?a=b"),
        Some("https://unsub.example.com/u?a=b".to_string())
    );
    assert_eq!(
        extract_unsubscribe_url("  HTTPS://Unsub.Example.com/x  "),
        Some("HTTPS://Unsub.Example.com/x".to_string())
    );
    assert_eq!(extract_unsubscribe_url(""), None);
    assert_eq!(extract_unsubscribe_url("<ftp://example.com/x>"), None);
    assert_eq!(
        extract_unsubscribe_url("<ftp://example.com/x>, <mailto:leave@x.example>"),
        Some("mailto:leave@x.example".to_string())
    );
}

#[test]
fn is_one_click_post_matches_rfc8058_value() {
    assert!(is_one_click_post("List-Unsubscribe=One-Click"));
    assert!(is_one_click_post(" list-unsubscribe = one-click "));
    assert!(is_one_click_post("\"List-Unsubscribe=One-Click\""));
    assert!(!is_one_click_post("List-Unsubscribe=Two-Click"));
    assert!(!is_one_click_post(""));
}

#[test]
fn user_contacts_crud_and_merge_with_aggregated_contacts() {
    let mut store = CacheStore::open_in_memory().unwrap();
    store
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox_id = store.find_inbox_folder(1).unwrap().unwrap().0;
    store
        .commit_folder_headers(
            1,
            inbox_id,
            1,
            &[
                sample_header(1, "季度会议通知"),
                sample_header(2, "会议纪要"),
            ],
            None,
        )
        .unwrap();

    let id_new = store
        .upsert_user_contact(&UserContact {
            id: 0,
            email: "custom@example.com".to_string(),
            display_name: "自定义联系人".to_string(),
            notes: "备注 A".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert!(id_new > 0);
    let alice_email = "alice@example.com".to_string();
    let id_alice = store
        .upsert_user_contact(&UserContact {
            id: 0,
            email: alice_email.clone(),
            display_name: "Alice 手工版".to_string(),
            notes: String::new(),
            ..Default::default()
        })
        .unwrap();

    assert!(store
        .upsert_user_contact(&UserContact {
            id: 0,
            email: alice_email.clone(),
            display_name: "重复".to_string(),
            notes: String::new(),
            ..Default::default()
        })
        .is_err());

    let contacts = store.list_known_contacts("", 0, 50).unwrap();
    let custom = contacts.iter().filter(|c| c.custom).count();
    assert_eq!(custom, 2);
    assert!(contacts[0].custom && contacts[1].custom);
    let merged = contacts
        .iter()
        .find(|c| c.email == alice_email)
        .expect("聚合邮箱应与自定义联系人合并");
    assert_eq!(merged.id, id_alice);
    assert_eq!(merged.name, "Alice 手工版");
    assert!(merged.message_count >= 2);
    let pure_custom = contacts
        .iter()
        .find(|c| c.email == "custom@example.com")
        .unwrap();
    assert_eq!(pure_custom.notes, "备注 A");
    assert_eq!(pure_custom.message_count, 0);

    let updated = store
        .upsert_user_contact(&UserContact {
            id: id_new,
            email: "custom@example.com".to_string(),
            display_name: "改名".to_string(),
            notes: String::new(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(updated, id_new);
    let renamed = store.list_user_contacts().unwrap();
    assert_eq!(renamed.len(), 2);
    assert_eq!(
        renamed
            .iter()
            .find(|c| c.id == id_new)
            .unwrap()
            .display_name,
        "改名"
    );
    assert!(store.delete_user_contact(id_new).unwrap());
    assert!(!store.delete_user_contact(id_new).unwrap());

    let (added, skipped) = store
        .import_user_contacts(&[
            UserContact {
                id: 0,
                email: "alice@example.com".to_string(),
                display_name: String::new(),
                notes: String::new(),
                ..Default::default()
            },
            UserContact {
                id: 0,
                email: "batch1@example.com".to_string(),
                display_name: "批量一".to_string(),
                notes: String::new(),
                ..Default::default()
            },
            UserContact {
                id: 0,
                email: "not-an-email".to_string(),
                display_name: String::new(),
                notes: String::new(),
                ..Default::default()
            },
        ])
        .unwrap();
    assert_eq!(added, 1);
    assert_eq!(skipped, 2);
}

#[test]
fn store_message_body_persists_unsubscribe_headers() {
    let mut store = CacheStore::open_in_memory().unwrap();
    store.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox_id = store.find_inbox_folder(1).unwrap().unwrap().0;
    store
        .commit_folder_headers(1, inbox_id, 1, &[sample_header(1, "周刊")], None)
        .unwrap();
    let msg_id = store.list_messages_page(inbox_id, "", 0, 10).unwrap().items[0].id;
    store
        .store_message_body_with_html(
            msg_id,
            "正文",
            None,
            &[],
            Some("https://lists.example.com/unsub"),
            true,
            None,
        )
        .unwrap();
    let (detail, _, _) = store.get_message_detail(msg_id).unwrap().unwrap();
    assert_eq!(
        detail.unsubscribe_url.as_deref(),
        Some("https://lists.example.com/unsub")
    );
    assert!(detail.unsubscribe_one_click);
}

#[test]
fn open_old_database_keeps_drafts_and_creates_user_contacts() {
    let dir = temp_cache_dir("old-drafts-compat");
    let db_path = dir.join("cache.db");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE drafts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                to_addr TEXT NOT NULL,
                cc_addr TEXT NOT NULL DEFAULT '',
                bcc_addr TEXT NOT NULL DEFAULT '',
                subject TEXT NOT NULL,
                body_text TEXT NOT NULL,
                in_reply_to TEXT NOT NULL DEFAULT '',
                attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                updated_at INTEGER NOT NULL
            );
            INSERT INTO drafts (account_id, to_addr, subject, body_text, updated_at)
            VALUES (1, 'old@example.com', '旧草稿', '旧内容', 123);
            ",
        )
        .unwrap();
    }
    let mut store = CacheStore::open(&db_path).unwrap();
    let drafts = store.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].subject, "旧草稿");

    let id = store
        .upsert_user_contact(&UserContact {
            id: 0,
            email: "new@example.com".to_string(),
            display_name: "新联系人".to_string(),
            notes: String::new(),
            ..UserContact::default()
        })
        .unwrap();
    assert!(id > 0);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unified_queries_filter_by_allowed_account_ids() {
    let mut store = CacheStore::open_in_memory().unwrap();
    store
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    store
        .sync_folders(2, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox_1 = store.find_inbox_folder(1).unwrap().unwrap().0;
    let inbox_2 = store.find_inbox_folder(2).unwrap().unwrap().0;
    store
        .commit_folder_headers(1, inbox_1, 1, &[sample_header(1, "账号一邮件")], None)
        .unwrap();
    store
        .commit_folder_headers(2, inbox_2, 1, &[sample_header(2, "账号二邮件")], None)
        .unwrap();

    let empty = store
        .list_unified_inbox_for_accounts(
            &[],
            "",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            50,
        )
        .unwrap();
    assert_eq!(empty.total, 0);
    assert!(empty.items.is_empty());

    let only_two = store
        .list_unified_inbox_for_accounts(
            &[2],
            "",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            50,
        )
        .unwrap();
    assert_eq!(only_two.total, 1);
    assert_eq!(only_two.items[0].account_id, 2);

    let both = store
        .list_unified_inbox_for_accounts(
            &[1, 2],
            "",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            50,
        )
        .unwrap();
    assert_eq!(both.total, 2);
}

#[test]
fn wp06_tags_crud_and_apply_filter_rules() {
    let mut store = CacheStore::open_in_memory().unwrap();
    let t1 = store.upsert_tag(None, "重要", "#e01b24", Some(1)).unwrap();
    let t2 = store.upsert_tag(None, "账单", "#3584e4", Some(1)).unwrap();
    let list = store.list_tags().unwrap();
    assert_eq!(list.len(), 2);
    let reloaded_t1 = list.iter().find(|t| t.id == t1.id).unwrap();
    let reloaded_t2 = list.iter().find(|t| t.id == t2.id).unwrap();
    assert_eq!(reloaded_t1.shortcut, None);
    assert_eq!(reloaded_t2.shortcut, Some(1));

    assert!(store.delete_tag(t1.id).unwrap());
    assert_eq!(store.list_tags().unwrap().len(), 1);

    let folders = store
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("归档", "Archive")])
        .unwrap();
    let inbox = folders[0].id;
    let archive = folders[1].id;
    store
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[header_with(
                101,
                "Monthly Invoice",
                "billing@example.com",
                "",
                false,
                true,
                1000,
            )],
            None,
        )
        .unwrap();

    let rules = vec![config::FilterRule {
        id: 1,
        name: "账单自动处理".to_string(),
        account_email: String::new(),
        enabled: true,
        match_all: true,
        field: config::FilterField::From,
        matcher: config::FilterMatch::Contains,
        value: "billing@".to_string(),
        move_to_folder: String::new(),
        archive: true,
        star: true,
        mark_read: true,
        apply_tag: String::new(),
        forward_to: String::new(),
    }];
    let updated = store.apply_filter_rules(&rules, &[]).unwrap();
    assert_eq!(updated, 1);

    let archived = store.list_messages_page(archive, "", 0, 10).unwrap();
    assert_eq!(archived.total, 1);
    assert!(!archived.items[0].unread);
    assert!(archived.items[0].flagged);
}

#[test]
fn disk_attachments_are_stored_as_files_and_cleaned_up_on_delete() {
    let dir = temp_cache_dir("disk-cleanup");
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;
    cache
        .commit_folder_headers(1, inbox, 1, &[sample_header(1, "附件落盘测试")], None)
        .unwrap();
    let msg_id = cache.list_messages_page(inbox, "", 0, 10).unwrap().items[0].id;

    cache
        .store_message_body(
            msg_id,
            "正文",
            &[CachedAttachmentInput::downloaded(
                "doc.pdf",
                "application/pdf",
                b"hello-disk-bytes".to_vec(),
            )],
        )
        .unwrap();

    let (detail, _, _) = cache.get_message_detail(msg_id).unwrap().unwrap();
    assert_eq!(detail.attachments.len(), 1);
    assert!(detail.attachments[0].downloaded);
    let att_id = detail.attachments[0].id;
    let (_, local_att_id) = decode_scoped_id(att_id).unwrap();
    let expected_rel = format!("{:02x}/{local_att_id}.bin", local_att_id & 0xff);
    let acc_dir = cache.account_dir_for(1).unwrap();
    let expected_file = acc_dir.join("attachments").join(&expected_rel);
    assert!(expected_file.exists());
    assert_eq!(std::fs::read(&expected_file).unwrap(), b"hello-disk-bytes");

    let (blob_len, storage_path): (i64, String) = cache.with_account_mail_conn(1, |conn| {
        conn.query_row(
            "SELECT length(content), storage_path FROM attachments WHERE id = ?1",
            params![local_att_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    });
    assert_eq!(blob_len, 0);
    assert_eq!(storage_path, expected_rel);

    let (_, _, bytes) = cache.get_attachment_bytes(att_id).unwrap().unwrap();
    assert_eq!(bytes, b"hello-disk-bytes");

    cache.remove_message(msg_id).unwrap();
    assert!(!expected_file.exists());

    cache
        .commit_folder_headers(1, inbox, 1, &[sample_header(2, "批量清理测试")], None)
        .unwrap();
    let msg2 = cache.list_messages_page(inbox, "", 0, 10).unwrap().items[0].id;
    cache
        .store_message_body(
            msg2,
            "正文2",
            &[CachedAttachmentInput::downloaded(
                "two.bin",
                "application/octet-stream",
                b"abc".to_vec(),
            )],
        )
        .unwrap();
    let att2 = cache
        .get_message_detail(msg2)
        .unwrap()
        .unwrap()
        .0
        .attachments[0]
        .id;
    let (_, local_att2) = decode_scoped_id(att2).unwrap();
    let rel2 = format!("{:02x}/{local_att2}.bin", local_att2 & 0xff);
    let file2 = acc_dir.join("attachments").join(&rel2);
    assert!(file2.exists());
    cache.clear_message_bodies().unwrap();
    assert!(!file2.exists());

    drop(cache);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_blob_attachment_still_readable_after_migration() {
    let dir = temp_cache_dir("legacy-blob");
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;
    cache
        .commit_folder_headers(1, inbox, 1, &[sample_header(1, "旧库 BLOB 兼容")], None)
        .unwrap();
    let msg_id = cache.list_messages_page(inbox, "", 0, 10).unwrap().items[0].id;
    let (slot, local_msg_id) = decode_scoped_id(msg_id).unwrap();

    let local_att_id: u32 = cache.with_account_mail_conn(1, |conn| {
        conn.execute(
            "INSERT INTO attachments (message_id, filename, mime_type, size_bytes, content, storage_path, downloaded)
             VALUES (?1, 'legacy.txt', 'text/plain', 6, ?2, '', 1)",
            params![local_msg_id, b"legacy".to_vec()],
        )
        .unwrap();
        u32::try_from(conn.last_insert_rowid()).unwrap()
    });
    let att_id = encode_scoped_id(slot, local_att_id).unwrap();
    let (name, mime, bytes) = cache.get_attachment_bytes(att_id).unwrap().unwrap();
    assert_eq!(name, "legacy.txt");
    assert_eq!(mime, "text/plain");
    assert_eq!(bytes, b"legacy");

    drop(cache);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pending_remote_attachment_transitions_to_local_after_download() {
    let dir = temp_cache_dir("pending-remote");
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;
    cache
        .commit_folder_headers(1, inbox, 1, &[sample_header(9, "按需附件")], None)
        .unwrap();
    let msg_id = cache.list_messages_page(inbox, "", 0, 10).unwrap().items[0].id;

    cache
        .store_message_body(
            msg_id,
            "正文",
            &[CachedAttachmentInput {
                filename: "large.zip".to_string(),
                mime_type: "application/zip".to_string(),
                size_bytes: 5_000_000,
                content: None,
                remote_part: "1.2".to_string(),
                transfer_encoding: "base64".to_string(),
            }],
        )
        .unwrap();

    let (detail, _, _) = cache.get_message_detail(msg_id).unwrap().unwrap();
    assert_eq!(detail.attachments.len(), 1);
    assert!(!detail.attachments[0].downloaded);
    assert_eq!(detail.attachments[0].size_bytes, 5_000_000);
    let att_id = detail.attachments[0].id;

    assert!(cache.get_attachment_bytes(att_id).unwrap().is_none());
    match cache.get_attachment_record(att_id).unwrap().unwrap() {
        AttachmentRecord::PendingRemote {
            uid,
            remote_part,
            transfer_encoding,
            size_bytes,
            ..
        } => {
            assert_eq!(uid, 9);
            assert_eq!(remote_part, "1.2");
            assert_eq!(transfer_encoding, "base64");
            assert_eq!(size_bytes, 5_000_000);
        }
        AttachmentRecord::Local { .. } => panic!("expected PendingRemote before download"),
    }

    let payload = vec![7u8; 42];
    let stored_len = cache
        .store_downloaded_attachment_bytes(att_id, &payload)
        .unwrap();
    assert_eq!(stored_len, 42);

    let (detail_after, _, _) = cache.get_message_detail(msg_id).unwrap().unwrap();
    assert!(detail_after.attachments[0].downloaded);
    assert_eq!(detail_after.attachments[0].size_bytes, 42);
    let (_, _, bytes) = cache.get_attachment_bytes(att_id).unwrap().unwrap();
    assert_eq!(bytes, payload);

    drop(cache);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prune_expired_downloads_drops_old_bodies_and_files_only() {
    let dir = temp_cache_dir("prune-expired");
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;

    let now_ts = 1_700_000_000i64;
    let old_ts = now_ts - 40 * 86_400;
    let fresh_ts = now_ts - 5 * 86_400;

    cache
        .commit_folder_headers(
            1,
            inbox,
            1,
            &[
                header_with(1, "旧邮件", "a@a.com", "b@b.com", false, false, old_ts),
                header_with(2, "新邮件", "a@a.com", "b@b.com", false, false, fresh_ts),
            ],
            None,
        )
        .unwrap();
    let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
    let old_id = page
        .items
        .iter()
        .find(|m| m.subject == "旧邮件")
        .unwrap()
        .id;
    let fresh_id = page
        .items
        .iter()
        .find(|m| m.subject == "新邮件")
        .unwrap()
        .id;

    cache
        .store_message_body(
            old_id,
            "旧正文",
            &[CachedAttachmentInput::downloaded(
                "old.pdf",
                "application/pdf",
                b"old-bytes".to_vec(),
            )],
        )
        .unwrap();
    cache
        .store_message_body(
            fresh_id,
            "新正文",
            &[CachedAttachmentInput::downloaded(
                "fresh.pdf",
                "application/pdf",
                b"fresh-bytes".to_vec(),
            )],
        )
        .unwrap();

    let acc_dir = cache.account_dir_for(1).unwrap();
    let old_att_id = cache
        .get_message_detail(old_id)
        .unwrap()
        .unwrap()
        .0
        .attachments[0]
        .id;
    let fresh_att_id = cache
        .get_message_detail(fresh_id)
        .unwrap()
        .unwrap()
        .0
        .attachments[0]
        .id;
    let (_, old_local_att) = decode_scoped_id(old_att_id).unwrap();
    let (_, fresh_local_att) = decode_scoped_id(fresh_att_id).unwrap();
    let old_file = acc_dir
        .join("attachments")
        .join(format!("{:02x}/{old_local_att}.bin", old_local_att & 0xff));
    let fresh_file = acc_dir.join("attachments").join(format!(
        "{:02x}/{fresh_local_att}.bin",
        fresh_local_att & 0xff
    ));
    assert!(old_file.exists() && fresh_file.exists());

    assert_eq!(cache.prune_expired_downloads(0, now_ts).unwrap(), 0);
    assert_eq!(cache.prune_expired_downloads(30, now_ts).unwrap(), 1);

    let (old_detail, _, old_loaded) = cache.get_message_detail(old_id).unwrap().unwrap();
    assert!(!old_loaded);
    assert!(old_detail.attachments.is_empty());
    assert!(!old_file.exists());

    let (_, old_local_msg) = decode_scoped_id(old_id).unwrap();
    let idx_is_null: bool = cache.with_account_mail_conn(1, |conn| {
        conn.query_row(
            "SELECT index_text IS NULL FROM messages WHERE id = ?1",
            params![old_local_msg],
            |r| r.get(0),
        )
        .unwrap()
    });
    assert!(!idx_is_null);

    let (fresh_detail, _, fresh_loaded) = cache.get_message_detail(fresh_id).unwrap().unwrap();
    assert!(fresh_loaded);
    assert_eq!(fresh_detail.attachments.len(), 1);
    assert!(fresh_file.exists());

    drop(cache);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thread_index_preserves_anchor_beyond_forty_messages_and_read_only_works() {
    let dir = temp_cache_dir("thread-readonly");
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox = folders[0].id;

    let mut batch = Vec::new();
    for uid in 1..=45u32 {
        let mut h = sample_header(uid, "Re: 长对话索引");
        h.message_id = String::new();
        h.date_ts = 1_700_000_000 + uid as i64;
        batch.push(h);
    }
    cache
        .commit_folder_headers(1, inbox, 1, &batch, None)
        .unwrap();

    let page = cache.list_messages_page(inbox, "", 0, 50).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].uid, 45);
    let anchor_id = cache.list_summaries_by_uids(inbox, &[1]).unwrap()[0].id;
    cache
        .store_message_body(anchor_id, "锚点正文已加载", &[])
        .unwrap();

    let ro = CacheStore::open_for_data_dir_read_only(&dir).unwrap();
    let (detail, _, loaded) = ro
        .get_message_detail_ordered(anchor_id, true)
        .unwrap()
        .unwrap();
    assert!(loaded);
    assert_eq!(detail.body_text, "锚点正文已加载");
    assert_eq!(detail.thread_messages.len(), 40);
    assert!(
        detail.thread_messages.iter().any(|m| m.id == anchor_id),
        "anchor beyond index 40 must be retained"
    );

    drop(ro);

    let mut rw = CacheStore::open_for_data_dir_existing(&dir).unwrap();
    rw.set_message_flagged(anchor_id, true).unwrap();
    let (detail_rw, _, _) = cache.get_message_detail(anchor_id).unwrap().unwrap();
    assert!(detail_rw.flagged);

    drop(rw);
    drop(cache);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_per_account_directories_isolate_mail_and_lazy_content_db() {
    let app_dir = temp_cache_dir("split-app");
    let mail_dir = temp_cache_dir("split-mail");

    let mut a1 = AccountConfig::default();
    a1.id = 1;
    a1.email = "Work@Example.com".to_string();
    let mut a2 = AccountConfig::default();
    a2.id = 2;
    a2.email = "personal@qq.com".to_string();
    config::save_accounts_in(&app_dir, &[a1, a2]).unwrap();
    config::save_storage_config_in(
        &app_dir,
        &config::StorageConfig {
            mail_data_dir: Some(mail_dir.to_string_lossy().into_owned()),
        },
    )
    .unwrap();

    let mut cache = CacheStore::open_for_data_dir(&app_dir).unwrap();
    let f1 = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    let f2 = cache.sync_folders(2, &[folder("收件箱", "INBOX")]).unwrap();
    assert_ne!(f1[0].id, f2[0].id);

    cache
        .commit_folder_headers(
            1,
            f1[0].id,
            10,
            &[header_with(
                1,
                "工作合同",
                "boss@example.com",
                "",
                false,
                true,
                100,
            )],
            None,
        )
        .unwrap();
    cache
        .commit_folder_headers(
            2,
            f2[0].id,
            20,
            &[header_with(
                2,
                "账单合同",
                "bank@qq.com",
                "",
                true,
                true,
                200,
            )],
            None,
        )
        .unwrap();

    let dir1 = mail_dir.join("work@example.com");
    let dir2 = mail_dir.join("personal@qq.com");
    assert!(dir1.join("mail.db").is_file());
    assert!(dir2.join("mail.db").is_file());
    // content.db 必须懒加载：仅同步邮件头和统一列表时不创建 content.db。
    assert!(!dir1.join("content.db").exists());
    assert!(!dir2.join("content.db").exists());

    // 全局搜索并发扫描两个磁盘库与统一收件箱。
    let unified = cache.list_unified_inbox("", 0, 10).unwrap();
    assert_eq!(unified.total, 2);
    assert_eq!(unified.items[0].account_id, 2);
    assert_eq!(unified.items[1].account_id, 1);

    let search = cache
        .search_all_folders(
            &[1, 2],
            "合同",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(search.total, 2);
    assert!(!dir1.join("content.db").exists());

    // 写入正文与联系人后，按需创建对应账号的 content.db；deleting account 1 仅清理其目录。
    let m1_id = unified.items[1].id;
    cache
        .store_message_body(m1_id, "工作合同正文", &[])
        .unwrap();
    assert!(dir1.join("content.db").is_file());
    assert!(!dir2.join("content.db").exists());

    cache.delete_account_data(1).unwrap();
    assert!(!dir1.exists());
    assert!(dir2.join("mail.db").is_file());

    let after_del = cache.list_unified_inbox("", 0, 10).unwrap();
    assert_eq!(after_del.total, 1);
    assert_eq!(after_del.items[0].account_id, 2);

    drop(cache);
    let _ = std::fs::remove_dir_all(&app_dir);
    let _ = std::fs::remove_dir_all(&mail_dir);
}

#[test]
fn portable_instance_reuses_installed_mail_dir_across_different_account_ids() {
    let installed_app = temp_cache_dir("inst-app");
    let portable_app = temp_cache_dir("port-app");
    let shared_mail = temp_cache_dir("shared-mail");

    let mut acc_inst = AccountConfig::default();
    acc_inst.id = 1;
    acc_inst.email = "Alice@Outlook.com".to_string();
    config::save_accounts_in(&installed_app, &[acc_inst]).unwrap();
    config::save_storage_config_in(
        &installed_app,
        &config::StorageConfig {
            mail_data_dir: Some(shared_mail.to_string_lossy().into_owned()),
        },
    )
    .unwrap();

    let mut inst_store = CacheStore::open_for_data_dir(&installed_app).unwrap();
    let f = inst_store
        .sync_folders(1, &[folder("收件箱", "INBOX")])
        .unwrap();
    inst_store
        .commit_folder_headers(1, f[0].id, 1, &[sample_header(55, "跨版本复用邮件")], None)
        .unwrap();
    let msg_id = inst_store
        .list_messages_page(f[0].id, "", 0, 10)
        .unwrap()
        .items[0]
        .id;
    inst_store
        .store_message_body(
            msg_id,
            "安装版保存的正文内容",
            &[CachedAttachmentInput::downloaded(
                "shared.txt",
                "text/plain",
                b"shared-att-bytes".to_vec(),
            )],
        )
        .unwrap();
    inst_store.checkpoint_and_close_all();
    drop(inst_store);

    // 便携版使用完全不同的 account_id = 7，但指向同一邮件目录：应零重新下载直接挂载读取。
    let mut acc_port = AccountConfig::default();
    acc_port.id = 7;
    acc_port.email = "alice@outlook.com".to_string();
    config::save_accounts_in(&portable_app, &[acc_port.clone()]).unwrap();
    config::save_storage_config_in(
        &portable_app,
        &config::StorageConfig {
            mail_data_dir: Some(shared_mail.to_string_lossy().into_owned()),
        },
    )
    .unwrap();

    let inspection = inspect_mail_data_dir(&shared_mail, &[acc_port]);
    assert_eq!(inspection.existing_accounts.len(), 1);
    assert_eq!(
        inspection.existing_accounts[0].matched_email.as_deref(),
        Some("alice@outlook.com")
    );

    let port_store = CacheStore::open_for_data_dir(&portable_app).unwrap();
    let pf = port_store.list_folders(7).unwrap();
    assert_eq!(pf.len(), 1);
    assert_eq!(pf[0].account_id, 7);

    let page = port_store.list_messages_page(pf[0].id, "", 0, 10).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].account_id, 7);
    assert_eq!(page.items[0].subject, "跨版本复用邮件");

    let (detail, _, loaded) = port_store
        .get_message_detail(page.items[0].id)
        .unwrap()
        .unwrap();
    assert!(loaded);
    assert_eq!(detail.account_id, 7);
    assert_eq!(detail.body_text, "安装版保存的正文内容");
    assert_eq!(detail.attachments.len(), 1);
    let (_, _, bytes) = port_store
        .get_attachment_bytes(detail.attachments[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"shared-att-bytes");

    drop(port_store);
    let _ = std::fs::remove_dir_all(&installed_app);
    let _ = std::fs::remove_dir_all(&portable_app);
    let _ = std::fs::remove_dir_all(&shared_mail);
}

#[test]
fn full_legacy_single_db_migration_preserves_messages_bodies_attachments_and_fts() {
    let app_dir = temp_cache_dir("full-legacy-migrate");
    let mail_dir = app_dir.join("mail");

    let mut acc1 = AccountConfig::default();
    acc1.id = 1;
    acc1.email = "u1@example.com".to_string();
    let mut acc2 = AccountConfig::default();
    acc2.id = 2;
    acc2.email = "u2@example.com".to_string();
    config::save_accounts_in(&app_dir, &[acc1, acc2]).unwrap();

    let legacy_db = config::cache_db_in(&app_dir);
    let legacy_att_dir = app_dir.join("attachments");
    std::fs::create_dir_all(&legacy_att_dir).unwrap();
    std::fs::write(legacy_att_dir.join("10.bin"), b"disk-attachment-payload").unwrap();

    {
        let conn = Connection::open(&legacy_db).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                uid_validity INTEGER NOT NULL DEFAULT 0,
                unread_count INTEGER NOT NULL DEFAULT 0,
                total_count INTEGER NOT NULL DEFAULT 0,
                is_inbox INTEGER NOT NULL DEFAULT 0,
                has_more_remote INTEGER NOT NULL DEFAULT 0,
                remote_next_link TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                folder_id INTEGER NOT NULL,
                uid INTEGER NOT NULL,
                remote_id TEXT NOT NULL DEFAULT '',
                message_id TEXT NOT NULL DEFAULT '',
                in_reply_to TEXT NOT NULL DEFAULT '',
                references_header TEXT NOT NULL DEFAULT '',
                sender TEXT NOT NULL,
                recipients TEXT NOT NULL DEFAULT '',
                cc TEXT NOT NULL DEFAULT '',
                subject TEXT NOT NULL,
                thread_subject TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                date_ts INTEGER NOT NULL DEFAULT 0,
                date_display TEXT NOT NULL DEFAULT '',
                unread INTEGER NOT NULL DEFAULT 1,
                flagged INTEGER NOT NULL DEFAULT 0,
                has_attachment INTEGER NOT NULL DEFAULT 0,
                body_text TEXT,
                body_html TEXT,
                index_text TEXT,
                list_unsubscribe TEXT NOT NULL DEFAULT '',
                list_unsubscribe_post INTEGER NOT NULL DEFAULT 0,
                auth_results TEXT NOT NULL DEFAULT '',
                importance INTEGER NOT NULL DEFAULT 0,
                tags TEXT NOT NULL DEFAULT '',
                answered INTEGER NOT NULL DEFAULT 0,
                forwarded INTEGER NOT NULL DEFAULT 0,
                snoozed_until_ts INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id INTEGER NOT NULL,
                filename TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                content BLOB NOT NULL,
                storage_path TEXT NOT NULL DEFAULT '',
                downloaded INTEGER NOT NULL DEFAULT 1,
                remote_part TEXT NOT NULL DEFAULT '',
                transfer_encoding TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                color TEXT NOT NULL DEFAULT '#3584e4',
                shortcut INTEGER
            );
            CREATE TABLE user_contacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                email TEXT NOT NULL UNIQUE,
                display_name TEXT NOT NULL DEFAULT '',
                notes TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE templates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                to_addr TEXT NOT NULL DEFAULT '',
                cc_addr TEXT NOT NULL DEFAULT '',
                bcc_addr TEXT NOT NULL DEFAULT '',
                subject TEXT NOT NULL DEFAULT '',
                body_text TEXT NOT NULL DEFAULT '',
                body_format TEXT NOT NULL DEFAULT '',
                attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL
            );
            CREATE TABLE outbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                to_addr TEXT NOT NULL,
                cc_addr TEXT NOT NULL DEFAULT '',
                bcc_addr TEXT NOT NULL DEFAULT '',
                subject TEXT NOT NULL,
                body_text TEXT NOT NULL,
                in_reply_to TEXT NOT NULL DEFAULT '',
                attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL DEFAULT 'queued',
                last_error TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );
            CREATE TABLE drafts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                to_addr TEXT NOT NULL,
                cc_addr TEXT NOT NULL DEFAULT '',
                bcc_addr TEXT NOT NULL DEFAULT '',
                subject TEXT NOT NULL,
                body_text TEXT NOT NULL,
                in_reply_to TEXT NOT NULL DEFAULT '',
                attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                updated_at INTEGER NOT NULL
            );
            INSERT INTO tags (id, name, color, shortcut) VALUES (1, 'VIP', '#e01b24', 1);
            INSERT INTO user_contacts (id, email, display_name, notes) VALUES (1, 'vip@test.com', 'VIP', 'note');
            INSERT INTO folders (id, account_id, name, path, is_inbox, total_count) VALUES (10, 1, '收件箱', 'INBOX', 1, 1);
            INSERT INTO folders (id, account_id, name, path, is_inbox, total_count) VALUES (20, 2, '收件箱', 'INBOX', 1, 1);
            INSERT INTO messages (id, account_id, folder_id, uid, sender, subject, preview, date_ts, unread, has_attachment, body_text, body_html)
            VALUES (100, 1, 10, 1, 'a@test.com', '老库迁移第一封', '摘要1', 1000, 1, 1, '请查收老库正文及 legacyfts 关键词', '<p>html</p>');
            INSERT INTO messages (id, account_id, folder_id, uid, sender, subject, preview, date_ts, unread, has_attachment, body_text)
            VALUES (200, 2, 20, 1, 'b@test.com', '账号二老邮件', '摘要2', 2000, 0, 1, '老库 BLOB 附件测试');
            INSERT INTO attachments (id, message_id, filename, mime_type, size_bytes, content, storage_path, downloaded)
            VALUES (10, 100, 'disk.txt', 'text/plain', 23, X'', '10.bin', 1);
            INSERT INTO attachments (id, message_id, filename, mime_type, size_bytes, content, storage_path, downloaded)
            VALUES (11, 200, 'blob.txt', 'text/plain', 9, X'626c6f626279746573', '', 1);
            ",
        )
        .unwrap();
    }

    let store = CacheStore::open_for_data_dir(&app_dir).unwrap();
    assert!(!legacy_db.exists());
    assert!(app_dir.join("kitemail-cache.db.migrated.bak").is_file());
    assert!(app_dir.join("attachments.migrated.bak").is_dir());
    assert!(mail_dir.join("u1@example.com").join("mail.db").is_file());
    assert!(mail_dir.join("u1@example.com").join("content.db").is_file());
    assert!(mail_dir.join("u2@example.com").join("mail.db").is_file());

    assert_eq!(store.list_tags().unwrap()[0].name, "VIP");
    assert_eq!(store.list_user_contacts().unwrap()[0].email, "vip@test.com");

    let hit_fts = store
        .search_all_folders(
            &[1, 2],
            "legacyfts",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(hit_fts.total, 1);
    let (d1, _, loaded1) = store
        .get_message_detail(hit_fts.items[0].id)
        .unwrap()
        .unwrap();
    assert!(loaded1);
    assert_eq!(d1.body_html.as_deref(), Some("<p>html</p>"));
    assert_eq!(
        store
            .get_attachment_bytes(d1.attachments[0].id)
            .unwrap()
            .unwrap()
            .2,
        b"disk-attachment-payload"
    );

    let hit_blob = store
        .search_all_folders(
            &[1, 2],
            "老库 BLOB",
            QuickFilterSet::default(),
            MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(hit_blob.total, 1);
    let (d2, _, _) = store
        .get_message_detail(hit_blob.items[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .get_attachment_bytes(d2.attachments[0].id)
            .unwrap()
            .unwrap()
            .2,
        b"blobbytes"
    );

    drop(store);
    // 再次打开不会二次迁移。
    let store2 = CacheStore::open_for_data_dir(&app_dir).unwrap();
    assert_eq!(store2.list_unified_inbox("", 0, 10).unwrap().total, 2);

    drop(store2);
    let _ = std::fs::remove_dir_all(&app_dir);
}

#[test]
fn validate_and_copy_mail_dir_handles_overlaps_and_filtered_copy() {
    let cur = temp_cache_dir("mail-switch-cur");
    let sub = cur.join("nested");
    let dst = temp_cache_dir("mail-switch-dst");

    assert!(validate_target_mail_dir(&cur, &cur).is_err());
    assert!(validate_target_mail_dir(&cur, &sub).is_err());
    assert!(validate_target_mail_dir(&sub, &cur).is_err());
    assert!(validate_target_mail_dir(&cur, &dst).is_ok());

    let app_dir = temp_cache_dir("mail-switch-app");
    config::save_storage_config_in(
        &app_dir,
        &config::StorageConfig {
            mail_data_dir: Some(cur.to_string_lossy().into_owned()),
        },
    )
    .unwrap();
    let mut cache = CacheStore::open_for_data_dir(&app_dir).unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    cache
        .commit_folder_headers(1, folders[0].id, 1, &[sample_header(1, "待迁移")], None)
        .unwrap();
    cache.checkpoint_and_close_all();
    drop(cache);

    let (files, bytes) = copy_mail_dir_contents(&cur, &dst).unwrap();
    assert!(files >= 1 && bytes > 0);
    assert!(dst.join("account_1").join("mail.db").is_file());

    let _ = std::fs::remove_dir_all(&cur);
    let _ = std::fs::remove_dir_all(&dst);
    let _ = std::fs::remove_dir_all(&app_dir);
}

#[test]
fn copy_mail_dir_contents_preserves_existing_target_and_sidecars() {
    let cur = temp_cache_dir("mail-wal-cur");
    let dst = temp_cache_dir("mail-wal-dst");
    let app_dir = temp_cache_dir("mail-wal-app");

    config::save_storage_config_in(
        &app_dir,
        &config::StorageConfig {
            mail_data_dir: Some(cur.to_string_lossy().into_owned()),
        },
    )
    .unwrap();

    let mut cache = CacheStore::open_for_data_dir(&app_dir).unwrap();
    let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
    cache
        .commit_folder_headers(
            1,
            folders[0].id,
            1,
            &[sample_header(1, "覆盖迁移后的新邮件")],
            None,
        )
        .unwrap();
    cache.checkpoint_and_close_all();
    drop(cache);

    let dst_acc = dst.join("account_1");
    std::fs::create_dir_all(&dst_acc).unwrap();
    std::fs::write(dst_acc.join("mail.db"), b"stale-db").unwrap();
    std::fs::write(dst_acc.join("mail.db-wal"), b"stale-wal").unwrap();
    std::fs::write(dst_acc.join("mail.db-shm"), b"stale-shm").unwrap();
    std::fs::write(dst_acc.join("content.db-wal"), b"stale-cwal").unwrap();
    std::fs::create_dir(dst_acc.join("attachments")).unwrap();
    std::fs::write(dst_acc.join("attachments/file"), b"attachment").unwrap();

    let error = copy_mail_dir_contents(&cur, &dst).unwrap_err();
    assert!(error.contains("复用"));
    assert_eq!(std::fs::read(dst_acc.join("mail.db")).unwrap(), b"stale-db");
    assert_eq!(
        std::fs::read(dst_acc.join("mail.db-wal")).unwrap(),
        b"stale-wal"
    );
    assert_eq!(
        std::fs::read(dst_acc.join("mail.db-shm")).unwrap(),
        b"stale-shm"
    );
    assert_eq!(
        std::fs::read(dst_acc.join("content.db-wal")).unwrap(),
        b"stale-cwal"
    );
    assert_eq!(
        std::fs::read(dst_acc.join("attachments/file")).unwrap(),
        b"attachment"
    );

    let _ = std::fs::remove_dir_all(&cur);
    let _ = std::fs::remove_dir_all(&dst);
    let _ = std::fs::remove_dir_all(&app_dir);
}

#[test]
fn mail_dir_migration_rolls_back_published_data_unless_committed() {
    let cur = temp_cache_dir("mail-rollback-cur");
    let dst = temp_cache_dir("mail-rollback-dst");
    std::fs::create_dir_all(cur.join("account_1")).unwrap();
    std::fs::write(cur.join("account_1/mail.db"), b"source").unwrap();
    std::fs::write(dst.join("unrelated"), b"keep").unwrap();
    {
        let migration = super::migrate::MailDirMigration::copy(&cur, &dst).unwrap();
        assert_eq!(migration.stats(), (1, 6));
        assert_eq!(
            std::fs::read(dst.join("account_1/mail.db")).unwrap(),
            b"source"
        );
    }
    assert!(!dst.join("account_1").exists());
    assert_eq!(std::fs::read(dst.join("unrelated")).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(cur.join("account_1/mail.db")).unwrap(),
        b"source"
    );
    assert!(copy_mail_dir_contents(&cur, &dst).is_ok());
    let _ = std::fs::remove_dir_all(&cur);
    let _ = std::fs::remove_dir_all(&dst);
}

#[cfg(windows)]
#[test]
fn mail_dir_migration_copy_failure_cleans_staging_and_allows_retry() {
    use std::os::windows::fs::OpenOptionsExt;

    let cur = temp_cache_dir("mail-copy-failure-cur");
    let dst = temp_cache_dir("mail-copy-failure-dst");
    let account = cur.join("account_1");
    std::fs::create_dir_all(account.join("attachments")).unwrap();
    std::fs::write(account.join("mail.db"), b"source-db").unwrap();
    let attachment = account.join("attachments/file");
    std::fs::write(&attachment, b"source-attachment").unwrap();
    std::fs::write(dst.join("unrelated"), b"keep").unwrap();
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&attachment)
        .unwrap();

    assert!(copy_mail_dir_contents(&cur, &dst).is_err());
    assert!(!dst.join("account_1").exists());
    assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 1);
    assert_eq!(std::fs::read(dst.join("unrelated")).unwrap(), b"keep");
    drop(locked);
    assert_eq!(
        std::fs::read(account.join("mail.db")).unwrap(),
        b"source-db"
    );
    assert_eq!(std::fs::read(&attachment).unwrap(), b"source-attachment");
    copy_mail_dir_contents(&cur, &dst).unwrap();
    assert_eq!(
        std::fs::read(dst.join("account_1/attachments/file")).unwrap(),
        b"source-attachment"
    );
    let _ = std::fs::remove_dir_all(&cur);
    let _ = std::fs::remove_dir_all(&dst);
}

#[test]
fn mail_dir_migration_conflict_prevents_all_publication() {
    let cur = temp_cache_dir("mail-conflict-cur");
    let dst = temp_cache_dir("mail-conflict-dst");
    for name in ["account_1", "account_2"] {
        std::fs::create_dir_all(cur.join(name)).unwrap();
        std::fs::write(cur.join(name).join("mail.db"), b"source").unwrap();
    }
    std::fs::write(dst.join("account_2"), b"existing").unwrap();
    assert!(copy_mail_dir_contents(&cur, &dst).is_err());
    assert!(!dst.join("account_1").exists());
    assert_eq!(std::fs::read(dst.join("account_2")).unwrap(), b"existing");
    assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&cur);
    let _ = std::fs::remove_dir_all(&dst);
}

#[test]
fn inspect_with_app_dir_discovers_external_accounts_and_current_mail_data() {
    let base = temp_cache_dir("mail-inspect-import");
    let installed_app = base.join("installed");
    let installed_mail = installed_app.join("mail");
    let portable_app = base.join("portable");
    let portable_mail = portable_app.join("mail");

    let mut acc1 = AccountConfig::default();
    acc1.id = 1;
    acc1.email = "alice@example.com".to_string();
    let mut acc2 = AccountConfig::default();
    acc2.id = 2;
    acc2.email = "bob@example.com".to_string();
    config::save_accounts_in(&installed_app, &[acc1.clone(), acc2.clone()]).unwrap();

    // 在安装版邮件目录为 acc1 创建本地库，acc2 仅存在配置中尚未缓存。
    let mut inst_cache = CacheStore::open_for_data_dir(&installed_app).unwrap();
    let f = inst_cache
        .sync_folders(1, &[folder("收件箱", "INBOX")])
        .unwrap();
    inst_cache
        .commit_folder_headers(1, f[0].id, 1, &[sample_header(1, "离线邮件")], None)
        .unwrap();
    inst_cache.checkpoint_and_close_all();
    drop(inst_cache);

    std::fs::create_dir_all(&portable_mail).unwrap();
    assert!(dir_has_account_mail_data(&installed_mail));
    assert!(!dir_has_account_mail_data(&portable_mail));

    // 便携版尚无任何账号与本地邮件：应探测出 2 个可导入账号，且 current_has_mail_data 为 false。
    let insp = inspect_mail_data_dir_with_app_dir(&portable_app, &installed_mail, &[]);
    assert!(!insp.current_has_mail_data);
    assert_eq!(
        insp.importable_config_dir.as_deref(),
        Some(installed_app.as_path())
    );
    assert_eq!(insp.importable_accounts.len(), 2);
    assert_eq!(
        insp.importable_accounts[0].storage_dir.as_deref(),
        Some("alice@example.com")
    );
    assert_eq!(insp.importable_accounts[1].storage_dir, None);

    // 若便携版已配置 alice@example.com（忽略大小写），仅剩余 bob@example.com 可导入。
    let mut existing = AccountConfig::default();
    existing.id = 5;
    existing.email = "Alice@Example.COM".to_string();
    let insp_partial =
        inspect_mail_data_dir_with_app_dir(&portable_app, &installed_mail, &[existing.clone()]);
    assert_eq!(insp_partial.importable_accounts.len(), 1);
    assert_eq!(insp_partial.importable_accounts[0].email, "bob@example.com");

    // 同配置目录内部不得自导入。
    let insp_self = inspect_mail_data_dir_with_app_dir(&installed_app, &installed_mail, &[]);
    assert!(insp_self.current_has_mail_data);
    assert!(insp_self.importable_accounts.is_empty());

    // merge_imported_accounts 跳过已存在邮箱并分配新 ID。
    let (merged, added) = merge_imported_accounts(&[existing], &insp.importable_accounts);
    assert_eq!(added, 1);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].id, 5);
    assert_eq!(merged[1].id, 6);
    assert_eq!(merged[1].email, "bob@example.com");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fill_thread_counts_matches_list_thread_messages_across_edge_cases() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("已发送", "Sent"),
                folder("所有邮件", "[Gmail]/All Mail"),
                folder("已删除", "Trash"),
                folder("垃圾邮件", "Junk"),
            ],
        )
        .unwrap();
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
    let sent = folders.iter().find(|f| f.path == "Sent").unwrap().id;
    let all_mail = folders
        .iter()
        .find(|f| f.path == "[Gmail]/All Mail")
        .unwrap()
        .id;
    let trash = folders.iter().find(|f| f.path == "Trash").unwrap().id;
    let junk = folders.iter().find(|f| f.path == "Junk").unwrap().id;

    // 1. Graph conversationId 隔离同主题独立邮件（核心复现：11 封同主题独立通知 + 2 封共享会话）
    let mut inbox_headers = Vec::new();
    for i in 1..=11u32 {
        let mut h = sample_header(i, "登录验证码");
        h.message_id = format!("<otp-{i}@graph.example>");
        h.references_header = format!("conv-otp-{i}");
        h.date_ts = 1_700_001_000 + i as i64;
        inbox_headers.push(h);
    }
    let mut conv_a = sample_header(20, "项目报价单");
    conv_a.message_id = "<conv-a-1@graph.example>".to_string();
    conv_a.references_header = "conv-shared-quote".to_string();
    conv_a.date_ts = 1_700_002_000;
    let mut conv_b = sample_header(21, "完全不同的跟进标题");
    conv_b.message_id = "<conv-a-2@graph.example>".to_string();
    conv_b.references_header = "conv-shared-quote".to_string();
    conv_b.date_ts = 1_700_002_100;
    inbox_headers.push(conv_a);
    inbox_headers.push(conv_b);

    // 2. IMAP 孤立 In-Reply-To 邮件（父邮件未缓存，但同主题有其他通知邮件）
    let mut orphan_reply = sample_header(30, "Re: 系统告警");
    orphan_reply.message_id = "<orphan-reply@imap.example>".to_string();
    orphan_reply.in_reply_to = "<missing-parent@imap.example>".to_string();
    orphan_reply.date_ts = 1_700_003_000;
    let mut plain_alert = sample_header(31, "系统告警");
    plain_alert.message_id = "<plain-alert@imap.example>".to_string();
    plain_alert.date_ts = 1_700_003_100;
    inbox_headers.push(orphan_reply);
    inbox_headers.push(plain_alert);

    // 3. 回收站 / 垃圾邮件排除（纯主题回退 + RFC 关联）
    let mut trash_subj_keep = sample_header(40, "账单提醒");
    trash_subj_keep.message_id = "<bill-keep@example.com>".to_string();
    trash_subj_keep.date_ts = 1_700_004_000;
    let mut trash_rfc_keep = sample_header(41, "审批通知");
    trash_rfc_keep.message_id = "<approve-root@example.com>".to_string();
    trash_rfc_keep.date_ts = 1_700_004_100;
    inbox_headers.push(trash_subj_keep);
    inbox_headers.push(trash_rfc_keep);

    // 4. 跨文件夹重复副本去重（INBOX + All Mail 同 message_id / sender / date_ts）
    let mut dup_inbox = sample_header(50, "跨文件夹副本");
    dup_inbox.message_id = "<dup-copy@example.com>".to_string();
    dup_inbox.sender = "Carol <carol@example.com>".to_string();
    dup_inbox.date_ts = 1_700_005_000;
    inbox_headers.push(dup_inbox.clone());

    // 5. 跨前缀归一化主题（仅原主题在 INBOX，Re: 在 Sent）+ 空归一化主题
    let mut prefix_inbox = sample_header(60, "季度复盘");
    prefix_inbox.message_id = "<review-root@example.com>".to_string();
    prefix_inbox.date_ts = 1_700_006_000;
    let mut empty_subj_1 = sample_header(61, "");
    empty_subj_1.message_id = "<empty-1@example.com>".to_string();
    empty_subj_1.date_ts = 1_700_006_100;
    let mut empty_subj_2 = sample_header(62, "Re: ");
    empty_subj_2.message_id = "<empty-2@example.com>".to_string();
    empty_subj_2.date_ts = 1_700_006_200;
    inbox_headers.push(prefix_inbox);
    inbox_headers.push(empty_subj_1);
    inbox_headers.push(empty_subj_2);

    cache
        .commit_folder_headers(1, inbox, 1, &inbox_headers, None)
        .unwrap();

    let mut prefix_sent = sample_header(100, "Re: 季度复盘");
    prefix_sent.message_id = "<review-reply@example.com>".to_string();
    prefix_sent.date_ts = 1_700_006_300;
    cache
        .commit_folder_headers(1, sent, 2, &[prefix_sent], None)
        .unwrap();

    cache
        .commit_folder_headers(1, all_mail, 3, &[dup_inbox], None)
        .unwrap();

    let mut trash_headers = Vec::new();
    for i in 1..=8u32 {
        let mut h = sample_header(200 + i, "账单提醒");
        h.message_id = format!("<bill-trash-{i}@example.com>");
        h.date_ts = 1_700_004_200 + i as i64;
        trash_headers.push(h);
    }
    let mut trash_rfc_reply = sample_header(220, "Re: 审批通知");
    trash_rfc_reply.message_id = "<approve-trash-reply@example.com>".to_string();
    trash_rfc_reply.in_reply_to = "<approve-root@example.com>".to_string();
    trash_rfc_reply.references_header = "<approve-root@example.com>".to_string();
    trash_rfc_reply.date_ts = 1_700_004_300;
    trash_headers.push(trash_rfc_reply);
    cache
        .commit_folder_headers(1, trash, 4, &trash_headers, None)
        .unwrap();

    let mut junk_headers = Vec::new();
    for i in 1..=2u32 {
        let mut h = sample_header(300 + i, "账单提醒");
        h.message_id = format!("<bill-junk-{i}@example.com>");
        h.date_ts = 1_700_004_400 + i as i64;
        junk_headers.push(h);
    }
    cache
        .commit_folder_headers(1, junk, 5, &junk_headers, None)
        .unwrap();

    let inbox_page = cache.list_messages_page(inbox, "", 0, 100).unwrap();
    for item in &inbox_page.items {
        let reader_count = cache
            .list_thread_messages(1, item.id, &item.subject, true)
            .unwrap()
            .len()
            .max(1) as u32;
        assert_eq!(
            item.thread_count, reader_count,
            "INBOX uid={} subject={:?} thread_count mismatch",
            item.uid, item.subject
        );
    }
    let by_uid = |uid: u32| {
        inbox_page
            .items
            .iter()
            .find(|m| m.uid == uid)
            .unwrap()
            .thread_count
    };
    for uid in 1..=11 {
        assert_eq!(by_uid(uid), 1);
    }
    assert!(!inbox_page.items.iter().any(|m| m.uid == 20));
    assert_eq!(by_uid(21), 2);
    assert_eq!(by_uid(30), 1);
    // uid 31 自身无回复头且无 RFC 引用，与 list_thread_messages 一样回退到 thread_subject 聚合（命中 30 与 31）
    assert_eq!(by_uid(31), 2);
    assert_eq!(by_uid(40), 1);
    assert_eq!(by_uid(41), 1);
    assert_eq!(by_uid(50), 1);
    assert_eq!(by_uid(60), 2);
    assert_eq!(by_uid(61), 1);
    assert_eq!(by_uid(62), 1);

    // 切到回收站视图时应与 list_thread_messages（包含回收站/垃圾邮件同会话）一致
    let trash_page = cache.list_messages_page(trash, "", 0, 50).unwrap();
    for item in &trash_page.items {
        let reader_count = cache
            .list_thread_messages(1, item.id, &item.subject, true)
            .unwrap()
            .len()
            .max(1) as u32;
        assert_eq!(
            item.thread_count, reader_count,
            "Trash uid={} subject={:?} thread_count mismatch",
            item.uid, item.subject
        );
    }
    assert_eq!(
        trash_page
            .items
            .iter()
            .find(|m| m.uid == 201)
            .unwrap()
            .thread_count,
        11
    );
    assert_eq!(
        trash_page
            .items
            .iter()
            .find(|m| m.uid == 220)
            .unwrap()
            .thread_count,
        2
    );
}

#[test]
fn fill_thread_counts_covers_rfc_renamed_threads_forty_cap_and_unified_inbox() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let folders_1 = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox_1 = folders_1[0].id;
    let sent_1 = folders_1[1].id;
    let folders_2 = cache.sync_folders(2, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox_2 = folders_2[0].id;

    // 1. RFC 改标题跨文件夹 3 封会话
    let mut root = sample_header(1, "原始需求讨论");
    root.message_id = "<rfc-root@example.com>".to_string();
    root.date_ts = 1_700_010_000;
    let mut reply2 = sample_header(2, "全新主题名（依然同线程）");
    reply2.message_id = "<rfc-reply2@example.com>".to_string();
    reply2.in_reply_to = "<rfc-reply1@example.com>".to_string();
    reply2.references_header = "<rfc-root@example.com> <rfc-reply1@example.com>".to_string();
    reply2.date_ts = 1_700_010_200;
    let mut reply1 = sample_header(10, "改过标题的回复");
    reply1.message_id = "<rfc-reply1@example.com>".to_string();
    reply1.in_reply_to = "<rfc-root@example.com>".to_string();
    reply1.references_header = "<rfc-root@example.com>".to_string();
    reply1.date_ts = 1_700_010_100;

    // 2. 65 封无 RFC 回复头的同主题邮件（应按 40 上限截断，且第 1 封超出 top 60 仍准确计数）
    let mut long_batch = vec![root, reply2];
    for i in 1..=65u32 {
        let mut h = sample_header(100 + i, "Re: 长对话计数上限");
        h.message_id = String::new();
        h.date_ts = 1_700_020_000 + i as i64;
        long_batch.push(h);
    }
    cache
        .commit_folder_headers(1, inbox_1, 1, &long_batch, None)
        .unwrap();
    cache
        .commit_folder_headers(1, sent_1, 2, &[reply1], None)
        .unwrap();

    // 账号 2 的统一收件箱独立测试（含 Graph conversationId 隔离）
    let mut acc2_a = sample_header(1, "周报");
    acc2_a.message_id = "<acc2-1@graph.example>".to_string();
    acc2_a.references_header = "acc2-conv-1".to_string();
    acc2_a.date_ts = 1_700_030_000;
    let mut acc2_b = sample_header(2, "周报");
    acc2_b.message_id = "<acc2-2@graph.example>".to_string();
    acc2_b.references_header = "acc2-conv-2".to_string();
    acc2_b.date_ts = 1_700_030_100;
    cache
        .commit_folder_headers(2, inbox_2, 1, &[acc2_a, acc2_b], None)
        .unwrap();

    let page_1 = cache.list_messages_page(inbox_1, "", 0, 100).unwrap();
    assert_eq!(page_1.total, 2);
    assert_eq!(page_1.items.len(), 2);
    let rfc_reply2 = page_1.items.iter().find(|m| m.uid == 2).unwrap();
    assert_eq!(rfc_reply2.thread_count, 3);
    let long_last = page_1.items.iter().find(|m| m.uid == 165).unwrap();
    assert_eq!(long_last.thread_count, 40);

    let older_members = cache.list_summaries_by_uids(inbox_1, &[1, 101]).unwrap();
    let rfc_root = older_members.iter().find(|m| m.uid == 1).unwrap();
    let long_first = older_members.iter().find(|m| m.uid == 101).unwrap();
    assert_eq!(rfc_root.thread_count, 3);
    assert_eq!(long_first.thread_count, 40);
    assert_eq!(
        cache
            .list_thread_messages(1, long_first.id, &long_first.subject, true)
            .unwrap()
            .len(),
        40
    );

    // 统一收件箱（经 fill_thread_counts_for_mixed）
    let unified = cache.list_unified_inbox("", 0, 100).unwrap();
    assert_eq!(unified.total, 4);
    assert_eq!(unified.items.len(), 4);
    for item in &unified.items {
        let reader_count = cache
            .list_thread_messages(item.account_id, item.id, &item.subject, true)
            .unwrap()
            .len()
            .max(1) as u32;
        assert_eq!(
            item.thread_count, reader_count,
            "Unified account={} uid={} subject={:?} thread_count mismatch",
            item.account_id, item.uid, item.subject
        );
    }
    let acc2_items: Vec<_> = unified.items.iter().filter(|m| m.account_id == 2).collect();
    assert_eq!(acc2_items.len(), 2);
    assert!(acc2_items.iter().all(|m| m.thread_count == 1));
}

#[test]
fn sync_folders_prunes_remotely_deleted_folders_and_cleans_bodies_and_attachments() {
    let dir = temp_cache_dir("prune-stale-folders");
    let mut cache = CacheStore::open_for_data_dir(&dir).unwrap();

    let folders = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("Trash", "custom-trash"),
                folder("已删除", "deleteditems"),
            ],
        )
        .unwrap();
    let inbox_id = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
    let trash_id = folders
        .iter()
        .find(|f| f.path == "custom-trash")
        .unwrap()
        .id;
    let deleted_id = folders
        .iter()
        .find(|f| f.path == "deleteditems")
        .unwrap()
        .id;

    cache
        .commit_folder_headers(1, inbox_id, 1, &[sample_header(1, "收件箱保留")], None)
        .unwrap();
    cache
        .commit_folder_headers(
            1,
            trash_id,
            1,
            &[sample_header(1, "待清理文件夹邮件")],
            None,
        )
        .unwrap();
    let trash_msg_id = cache.list_messages_page(trash_id, "", 0, 10).unwrap().items[0].id;

    cache
        .store_message_body(
            trash_msg_id,
            "旧 Trash 正文",
            &[CachedAttachmentInput::downloaded(
                "old.bin",
                "application/octet-stream",
                b"stale-attachment-bytes".to_vec(),
            )],
        )
        .unwrap();
    let (detail, _, _) = cache.get_message_detail(trash_msg_id).unwrap().unwrap();
    let att_id = detail.attachments[0].id;
    let (_, local_att_id) = decode_scoped_id(att_id).unwrap();
    let att_file = cache
        .account_dir_for(1)
        .unwrap()
        .join("attachments")
        .join(format!("{:02x}/{local_att_id}.bin", local_att_id & 0xff));
    assert!(att_file.exists());

    // 空列表保护：不会误删本地任何文件夹。
    let after_empty = cache.sync_folders(1, &[]).unwrap();
    assert_eq!(after_empty.len(), 3);
    assert!(cache.get_message_detail(trash_msg_id).unwrap().is_some());

    // 远端删除 custom-trash 后，仅保留收件箱与系统已删除，原 id 不变。
    let pruned = cache
        .sync_folders(
            1,
            &[folder("收件箱", "INBOX"), folder("已删除", "deleteditems")],
        )
        .unwrap();
    assert_eq!(pruned.len(), 2);
    assert_eq!(
        pruned
            .iter()
            .map(|f| (f.id, f.path.as_str()))
            .collect::<Vec<_>>(),
        vec![(inbox_id, "INBOX"), (deleted_id, "deleteditems")]
    );
    assert_eq!(cache.list_folders(1).unwrap().len(), 2);
    assert_eq!(
        cache.list_messages_page(inbox_id, "", 0, 10).unwrap().total,
        1
    );
    assert!(cache.get_message_detail(trash_msg_id).unwrap().is_none());
    assert!(!att_file.exists());

    drop(cache);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inbox_collapses_threads_and_aggregates_flags_across_single_and_unified_inbox() {
    let mut cache = CacheStore::open_in_memory().unwrap();
    let f1 = cache
        .sync_folders(
            1,
            &[
                folder("收件箱", "INBOX"),
                folder("已发送", "Sent"),
                folder("归档", "Archive"),
            ],
        )
        .unwrap();
    let inbox_1 = f1.iter().find(|f| f.path == "INBOX").unwrap().id;
    let sent_1 = f1.iter().find(|f| f.path == "Sent").unwrap().id;
    let archive_1 = f1.iter().find(|f| f.path == "Archive").unwrap().id;

    let f2 = cache.sync_folders(2, &[folder("收件箱", "INBOX")]).unwrap();
    let inbox_2 = f2[0].id;

    // 1. RFC 跨文件夹桥接会话：旧收件箱成员未读+加星+带标签，最新收件箱成员已读+带附件+带另一标签
    let mut rfc_old = sample_header(1, "项目审批");
    rfc_old.message_id = "<proj-root@a1>".to_string();
    rfc_old.date_ts = 1_700_100_000;
    rfc_old.unread = true;
    rfc_old.flagged = true;
    rfc_old.has_attachment = false;

    let mut rfc_sent = sample_header(2, "Re: 项目审批");
    rfc_sent.message_id = "<proj-reply1@a1>".to_string();
    rfc_sent.in_reply_to = "<proj-root@a1>".to_string();
    rfc_sent.references_header = "<proj-root@a1>".to_string();
    rfc_sent.date_ts = 1_700_200_000;
    rfc_sent.unread = false;

    let mut rfc_new = sample_header(3, "审批更新");
    rfc_new.message_id = "<proj-reply2@a1>".to_string();
    rfc_new.in_reply_to = "<proj-reply1@a1>".to_string();
    rfc_new.references_header = "<proj-root@a1> <proj-reply1@a1>".to_string();
    rfc_new.date_ts = 1_700_300_000;
    rfc_new.unread = false;
    rfc_new.flagged = false;
    rfc_new.has_attachment = true;

    // 2. 纯归一化主题会话：两条均未读，验证 unread filter 下也会折叠成一条
    let mut subj_old = sample_header(10, "周报同步");
    subj_old.message_id = String::new();
    subj_old.date_ts = 1_700_400_000;
    subj_old.unread = true;

    let mut subj_new = sample_header(11, "Re: 周报同步");
    subj_new.message_id = String::new();
    subj_new.date_ts = 1_700_500_000;
    subj_new.unread = true;

    cache
        .commit_folder_headers(1, inbox_1, 1, &[rfc_old, rfc_new, subj_old, subj_new], None)
        .unwrap();
    cache
        .commit_folder_headers(1, sent_1, 2, &[rfc_sent], None)
        .unwrap();

    // 同步合并 tags / importance 到成员行
    let members = cache.list_summaries_by_uids(inbox_1, &[1, 3]).unwrap();
    let old_id = members.iter().find(|m| m.uid == 1).unwrap().id;
    let new_id = members.iter().find(|m| m.uid == 3).unwrap().id;
    cache
        .set_message_tags(old_id, &["重要".to_string()])
        .unwrap();
    cache
        .set_message_tags(new_id, &["项目".to_string()])
        .unwrap();
    let (_, old_local) = decode_scoped_id(old_id).unwrap();
    cache.with_account_mail_conn(1, |conn| {
        conn.execute(
            "UPDATE messages SET importance = 1 WHERE id = ?1",
            params![old_local],
        )
        .unwrap();
    });

    // 非收件箱文件夹（归档）保持平铺不折叠
    let mut arc_1 = sample_header(20, "历史归档");
    arc_1.message_id = String::new();
    arc_1.date_ts = 1_700_600_000;
    let mut arc_2 = sample_header(21, "Re: 历史归档");
    arc_2.message_id = String::new();
    arc_2.date_ts = 1_700_700_000;
    cache
        .commit_folder_headers(1, archive_1, 3, &[arc_1, arc_2], None)
        .unwrap();

    // 账号 2 收件箱独立邮件
    let mut acc2_msg = sample_header(1, "跨账号通知");
    acc2_msg.message_id = "<acc2@a2>".to_string();
    acc2_msg.date_ts = 1_700_350_000;
    acc2_msg.unread = false;
    cache
        .commit_folder_headers(2, inbox_2, 1, &[acc2_msg], None)
        .unwrap();

    // 单账号收件箱折叠 + 状态聚合
    let inbox_page = cache.list_messages_page(inbox_1, "", 0, 10).unwrap();
    assert_eq!(inbox_page.total, 2);
    assert_eq!(inbox_page.items.len(), 2);
    assert_eq!(inbox_page.items[0].uid, 11);
    assert_eq!(inbox_page.items[0].thread_count, 2);
    let rep_rfc = &inbox_page.items[1];
    assert_eq!(rep_rfc.uid, 3);
    assert_eq!(rep_rfc.thread_count, 3);
    assert!(rep_rfc.unread);
    assert!(rep_rfc.flagged);
    assert!(rep_rfc.has_attachment);
    assert_eq!(rep_rfc.importance, 1);
    assert!(rep_rfc.tags.contains(&"项目".to_string()));
    assert!(rep_rfc.tags.contains(&"重要".to_string()));

    // 分页按折叠后的会话数切片
    let paged = cache.list_messages_page(inbox_1, "", 1, 1).unwrap();
    assert_eq!(paged.total, 2);
    assert_eq!(paged.items.len(), 1);
    assert_eq!(paged.items[0].uid, 3);

    // 展开子邮件仍包含同会话全部被折叠成员
    let children = cache
        .list_thread_summaries(1, rep_rfc.id, &rep_rfc.subject)
        .unwrap();
    assert_eq!(children.len(), 2);
    assert!(children
        .iter()
        .any(|c| c.uid == 1 && c.folder_path == "INBOX"));
    assert!(children
        .iter()
        .any(|c| c.uid == 2 && c.folder_path == "Sent"));

    // 未读快捷筛选下同会话仍去重，且旧未读成员可命中会话
    let mut unread_filter = crate::QuickFilterSet::default();
    unread_filter.unread = true;
    let unread_inbox = cache
        .list_messages_page_with_options(
            inbox_1,
            "",
            unread_filter,
            crate::MessageSort::DateNewest,
            0,
            10,
        )
        .unwrap();
    assert_eq!(unread_inbox.total, 2);
    assert_eq!(
        unread_inbox.items.iter().map(|m| m.uid).collect::<Vec<_>>(),
        vec![11, 1]
    );

    // 非收件箱文件夹（归档）保持逐封展示
    let archive_page = cache.list_messages_page(archive_1, "", 0, 10).unwrap();
    assert_eq!(archive_page.total, 2);
    assert_eq!(archive_page.items.len(), 2);

    // 统一收件箱同样折叠并保留聚合属性
    let unified = cache.list_unified_inbox("", 0, 10).unwrap();
    assert_eq!(unified.total, 3);
    assert_eq!(unified.items.len(), 3);
    assert_eq!(
        unified
            .items
            .iter()
            .map(|m| (m.account_id, m.uid))
            .collect::<Vec<_>>(),
        vec![(1, 11), (2, 1), (1, 3)]
    );
    let unified_rfc = &unified.items[2];
    assert!(unified_rfc.unread);
    assert!(unified_rfc.flagged);
    assert_eq!(unified_rfc.importance, 1);
    assert!(unified_rfc.tags.contains(&"重要".to_string()));
}

#[test]
fn list_thread_summaries_orders_descending_and_keeps_newest_when_truncated() {
    let mut cache = CacheStore::open_in_memory().expect("open memory db");
    let folders = cache
        .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
        .unwrap();
    let inbox_id = folders[0].id;
    let sent_id = folders[1].id;

    // 1. 同一会话跨文件夹且包含同时间戳多条记录，按 date_ts DESC, uid DESC, id DESC 排序
    let mut h1 = sample_header(1, "排序验证主题");
    h1.date_ts = 1_700_000_100;
    let mut h2 = sample_header(2, "Re: 排序验证主题");
    h2.date_ts = 1_700_000_200;
    h2.in_reply_to = "<msg-1@example.com>".to_string();
    let mut h3 = sample_header(3, "Re: 排序验证主题");
    h3.date_ts = 1_700_000_200;
    h3.in_reply_to = "<msg-2@example.com>".to_string();
    let mut h4 = sample_header(4, "Re: 排序验证主题");
    h4.date_ts = 1_700_000_400;
    h4.in_reply_to = "<msg-3@example.com>".to_string();
    cache
        .commit_folder_headers(1, inbox_id, 100, &[h1, h2, h3, h4], None)
        .unwrap();

    let mut hs = sample_header(5, "Re: 排序验证主题");
    hs.date_ts = 1_700_000_300;
    hs.in_reply_to = "<msg-3@example.com>".to_string();
    cache
        .commit_folder_headers(1, sent_id, 200, &[hs], None)
        .unwrap();

    let page = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
    assert_eq!(page.items.len(), 1);
    let head = &page.items[0];
    assert_eq!(head.uid, 4);

    let children = cache
        .list_thread_summaries(1, head.id, &head.subject)
        .unwrap();
    assert_eq!(
        children
            .iter()
            .map(|c| (c.date_ts, c.uid))
            .collect::<Vec<_>>(),
        vec![
            (1_700_000_300, 5),
            (1_700_000_200, 3),
            (1_700_000_200, 2),
            (1_700_000_100, 1),
        ]
    );

    // 2. 超长会话（45 封 > MAX_THREAD_MESSAGES=40）展开子邮件应保留最新的 39 封并按倒序返回
    let mut long_headers = Vec::new();
    for i in 1..=45u32 {
        let uid = 100 + i;
        let mut h = sample_header(uid, "超长倒序会话");
        h.date_ts = 1_700_100_000 + i64::from(i) * 10;
        if i > 1 {
            h.in_reply_to = format!("<msg-{}@example.com>", uid - 1);
            h.references_header = "<msg-101@example.com>".to_string();
        }
        long_headers.push(h);
    }
    cache
        .commit_folder_headers(1, inbox_id, 300, &long_headers, None)
        .unwrap();

    let page2 = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
    let long_head = page2
        .items
        .iter()
        .find(|m| m.subject == "超长倒序会话")
        .unwrap();
    assert_eq!(long_head.uid, 145);

    let long_children = cache
        .list_thread_summaries(1, long_head.id, &long_head.subject)
        .unwrap();
    assert_eq!(long_children.len(), 39);
    let expected_uids: Vec<u32> = (106..=144).rev().collect();
    assert_eq!(
        long_children.iter().map(|c| c.uid).collect::<Vec<_>>(),
        expected_uids
    );
}
