//! Structured mail search query parser aligned with Linux Hylki (`src/ui/message_list/query.rs`).
//!
//! Supports:
//! - Multiple bare keywords (AND semantics, full Unicode / Chinese support)
//! - Quoted exact phrases: `"exact phrase"`
//! - Field operators: `from:<val>`, `to:<val>` (and `cc:`, `bcc:`), `subject:<val>`,
//!   `in:<val>` (and `folder:`)
//! - Flag operators: `has:attachment` (`has:attachments`), `is:unread`, `is:read`, `is:starred`

use crate::MessageSummary;

/// Maximum number of bare terms kept in a single parsed query.
pub const MAX_TERMS: usize = 63;

/// Parsed search query with all values normalized to lowercase.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Bare words and quoted phrases, each of which must match somewhere in
    /// subject, sender, recipients, preview, or body_text.
    pub terms: Vec<String>,
    /// `from:<val>` — must match the sender field.
    pub from: Vec<String>,
    /// `to:<val>` / `cc:<val>` / `bcc:<val>` — must match the recipients field.
    pub to: Vec<String>,
    /// `subject:<val>` — must match the subject field.
    pub subject: Vec<String>,
    /// `in:<val>` / `folder:<val>` — must match the folder name or path.
    pub folder: Vec<String>,
    /// `has:attachment` / `has:attachments`.
    pub attachment: bool,
    /// `is:unread` (`Some(true)`) or `is:read` (`Some(false)`).
    pub unread: Option<bool>,
    /// `is:starred`.
    pub starred: bool,
}

/// Split raw query text into tokens, keeping quoted stretches intact (with quotes removed)
/// even when attached to an operator like `from:"Ann Lee"` or `from:"张 三"`.
/// Each returned item is `(token, had_quotes)`.
fn split_pieces(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut piece = String::new();
    let mut quoted = false;
    let mut in_quotes = false;

    for c in text.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                quoted = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if !piece.is_empty() {
                    out.push((std::mem::take(&mut piece), quoted));
                }
                quoted = false;
            }
            c => piece.push(c),
        }
    }
    if !piece.is_empty() {
        out.push((piece, quoted));
    }
    out
}

impl SearchQuery {
    /// Parse a user-entered search string into a structured [`SearchQuery`].
    pub fn parse(text: &str) -> Self {
        let mut q = Self::default();
        for (piece, quoted) in split_pieces(&text.to_lowercase()) {
            let operator = piece
                .split_once(':')
                .filter(|(_, value)| !value.is_empty() || quoted);
            let clean = |v: &str| v.trim().to_string();

            match operator {
                Some(("from", v)) => q.from.push(clean(v)),
                Some(("to" | "cc" | "bcc", v)) => q.to.push(clean(v)),
                Some(("subject", v)) => q.subject.push(clean(v)),
                Some(("in" | "folder", v)) => q.folder.push(clean(v)),
                Some(("has", "attachment" | "attachments")) => q.attachment = true,
                Some(("is", "unread")) => q.unread = Some(true),
                Some(("is", "read")) => q.unread = Some(false),
                Some(("is", "starred")) => q.starred = true,
                _ => {
                    let term = piece.trim().to_string();
                    if !term.is_empty() && q.terms.len() < MAX_TERMS && !q.terms.contains(&term) {
                        q.terms.push(term);
                    }
                }
            }
        }
        q.from.retain(|v| !v.is_empty());
        q.to.retain(|v| !v.is_empty());
        q.subject.retain(|v| !v.is_empty());
        q.folder.retain(|v| !v.is_empty());
        q
    }

    /// Return `true` when the query contains neither terms nor operators.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Extract a clean text string suitable for remote IMAP / Graph server search.
    pub fn server_search_text(&self) -> String {
        if !self.terms.is_empty() {
            return self.terms.join(" ");
        }
        if !self.subject.is_empty() {
            return self.subject.join(" ");
        }
        if !self.from.is_empty() {
            return self.from.join(" ");
        }
        if !self.to.is_empty() {
            return self.to.join(" ");
        }
        String::new()
    }

    /// In-memory matcher used by `DemoStore` and unit tests.
    pub fn matches_summary(
        &self,
        msg: &MessageSummary,
        folder_name: &str,
        recipients: &str,
        body_text: Option<&str>,
    ) -> bool {
        if self.attachment && !msg.has_attachment {
            return false;
        }
        if let Some(want_unread) = self.unread {
            if msg.unread != want_unread {
                return false;
            }
        }
        if self.starred && !msg.flagged {
            return false;
        }

        let sender_l = msg.sender.to_lowercase();
        let recipients_l = recipients.to_lowercase();
        let subject_l = msg.subject.to_lowercase();
        let preview_l = msg.preview.to_lowercase();
        let body_l = body_text.unwrap_or("").to_lowercase();
        let folder_name_l = folder_name.to_lowercase();
        let folder_path_l = msg.folder_path.to_lowercase();

        if !self.from.iter().all(|v| sender_l.contains(v.as_str())) {
            return false;
        }
        if !self.to.iter().all(|v| recipients_l.contains(v.as_str())) {
            return false;
        }
        if !self.subject.iter().all(|v| subject_l.contains(v.as_str())) {
            return false;
        }
        if !self
            .folder
            .iter()
            .all(|v| folder_name_l.contains(v.as_str()) || folder_path_l.contains(v.as_str()))
        {
            return false;
        }

        self.terms.iter().all(|term| {
            subject_l.contains(term.as_str())
                || sender_l.contains(term.as_str())
                || recipients_l.contains(term.as_str())
                || preview_l.contains(term.as_str())
                || body_l.contains(term.as_str())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_summary() -> MessageSummary {
        MessageSummary {
            id: 1,
            account_id: 1,
            folder_id: 10,
            folder_path: "Archive/2026".to_string(),
            uid: 101,
            sender: "张伟 <zhangwei@example.com>".to_string(),
            subject: "2026 年度合同与发票确认".to_string(),
            preview: "请查收附件中的电子发票".to_string(),
            date_ts: 1_700_000_000,
            date_display: "11-20".to_string(),
            unread: true,
            flagged: true,
            has_attachment: true,
        }
    }

    #[test]
    fn parses_terms_phrases_and_all_operators() {
        let q = SearchQuery::parse(
            r#"Rossi 发票 "exact phrase" from:ann to:"Lee K" subject:Q4 in:Archive has:attachment is:unread is:starred re: 10:30"#,
        );
        assert_eq!(
            q.terms,
            vec!["rossi", "发票", "exact phrase", "re:", "10:30"]
        );
        assert_eq!(q.from, vec!["ann"]);
        assert_eq!(q.to, vec!["lee k"]);
        assert_eq!(q.subject, vec!["q4"]);
        assert_eq!(q.folder, vec!["archive"]);
        assert!(q.attachment);
        assert_eq!(q.unread, Some(true));
        assert!(q.starred);
    }

    #[test]
    fn empty_and_bare_colon_edge_cases() {
        assert!(SearchQuery::parse("   ").is_empty());
        let q = SearchQuery::parse("from:");
        assert_eq!(q.terms, vec!["from:"]);
        assert!(q.from.is_empty());

        let q_read = SearchQuery::parse("is:read has:attachments cc:bob folder:sent");
        assert_eq!(q_read.unread, Some(false));
        assert!(q_read.attachment);
        assert_eq!(q_read.to, vec!["bob"]);
        assert_eq!(q_read.folder, vec!["sent"]);
    }

    #[test]
    fn matches_summary_handles_chinese_and_operators() {
        let msg = sample_summary();
        let body = "本次付款包含两张增值税专用发票，合同编号 HT-2026-009。";
        let rcpt = "财务部 <finance@company.cn>";

        let q1 = SearchQuery::parse(r#"from:张伟 to:finance in:archive has:attachment is:unread is:starred "合同编号 ht-2026-009""#);
        assert!(q1.matches_summary(&msg, "归档", rcpt, Some(body)));

        let q2 = SearchQuery::parse("is:read");
        assert!(!q2.matches_summary(&msg, "归档", rcpt, Some(body)));

        let q3 = SearchQuery::parse("from:李四");
        assert!(!q3.matches_summary(&msg, "归档", rcpt, Some(body)));

        let q4 = SearchQuery::parse("增值税 发票");
        assert!(q4.matches_summary(&msg, "归档", rcpt, Some(body)));
    }
}
