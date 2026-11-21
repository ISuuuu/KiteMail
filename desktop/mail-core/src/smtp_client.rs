//! Async SMTP client supporting TLS/STARTTLS, attachments, and uncertain-delivery detection.

use std::path::Path;
use std::time::Duration;

use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::{AccountConfig, SecurityMode};

const SMTP_TIMEOUT: Duration = Duration::from_secs(25);

/// Outcome of an SMTP send attempt.
#[derive(Debug)]
pub enum SendOutcome {
    Accepted,
    /// Clear pre-send or server rejection (safe to edit/retry).
    Failed(String),
    /// Connection dropped or timed out after DATA; server state is unknown.
    Uncertain(String),
}

fn build_transport(
    account: &AccountConfig,
    password: &str,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    if account.smtp_host.trim().is_empty() {
        return Err("SMTP 主机地址不能为空".to_string());
    }

    let username = if account.smtp_separate_credentials && !account.smtp_username.trim().is_empty()
    {
        account.smtp_username.clone()
    } else if !account.imap_username.trim().is_empty() {
        account.imap_username.clone()
    } else {
        account.email.clone()
    };

    let creds = Credentials::new(username, password.to_string());
    let builder = match account.smtp_security {
        SecurityMode::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&account.smtp_host)
            .map_err(|e| format!("初始化 SMTP TLS 传输失败: {e}"))?
            .port(account.smtp_port),
        SecurityMode::StartTls => {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&account.smtp_host)
                .map_err(|e| format!("初始化 SMTP STARTTLS 传输失败: {e}"))?
                .port(account.smtp_port)
        }
    };

    Ok(builder
        .credentials(creds)
        .timeout(Some(SMTP_TIMEOUT))
        .build())
}

/// Verify SMTP connectivity and authentication.
pub async fn verify_login(account: &AccountConfig, password: &str) -> Result<(), String> {
    let transport = build_transport(account, password)?;
    let ok = transport
        .test_connection()
        .await
        .map_err(|e| format!("SMTP 连接或认证失败: {e}"))?;
    if !ok {
        return Err("SMTP 服务器拒绝了连接测试".to_string());
    }
    Ok(())
}

/// Parse a comma-separated address list into mailboxes; empty parts are skipped.
fn parse_mailbox_list(raw: &str, label: &str) -> Result<Vec<Mailbox>, String> {
    let mut mailboxes = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mailbox: Mailbox = part
            .parse()
            .map_err(|e| format!("{label}地址格式无效 ({part}): {e}"))?;
        mailboxes.push(mailbox);
    }
    Ok(mailboxes)
}

/// Build and send an email (with optional reply headers and file attachments).
pub async fn send_message(
    account: &AccountConfig,
    password: &str,
    to_addr: &str,
    cc_addrs: &str,
    bcc_addrs: &str,
    subject: &str,
    body_text: &str,
    in_reply_to: &str,
    attachment_paths: &[String],
) -> SendOutcome {
    send_message_with_raw(
        account,
        password,
        to_addr,
        cc_addrs,
        bcc_addrs,
        subject,
        body_text,
        in_reply_to,
        attachment_paths,
    )
    .await
    .0
}

/// Build and send an email, also returning the serialized RFC 822 bytes when accepted
/// so the caller can independently append a Sent copy via IMAP.
#[allow(clippy::too_many_arguments)]
pub async fn send_message_with_raw(
    account: &AccountConfig,
    password: &str,
    to_addr: &str,
    cc_addrs: &str,
    bcc_addrs: &str,
    subject: &str,
    body_text: &str,
    in_reply_to: &str,
    attachment_paths: &[String],
) -> (SendOutcome, Option<Vec<u8>>) {
    let from_str = if account.display_name.trim().is_empty() {
        account.email.clone()
    } else {
        format!("{} <{}>", account.display_name.trim(), account.email.trim())
    };
    let from_mailbox: Mailbox = match from_str.parse() {
        Ok(m) => m,
        Err(e) => {
            return (
                SendOutcome::Failed(format!("发件人地址格式无效: {e}")),
                None,
            )
        }
    };
    let to_mailboxes = match parse_mailbox_list(to_addr, "收件人") {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => return (SendOutcome::Failed("收件人地址不能为空".to_string()), None),
        Err(e) => return (SendOutcome::Failed(e), None),
    };
    let cc_mailboxes = match parse_mailbox_list(cc_addrs, "抄送") {
        Ok(list) => list,
        Err(e) => return (SendOutcome::Failed(e), None),
    };
    let bcc_mailboxes = match parse_mailbox_list(bcc_addrs, "密送") {
        Ok(list) => list,
        Err(e) => return (SendOutcome::Failed(e), None),
    };

    let mut builder = Message::builder().from(from_mailbox).subject(subject);
    for mailbox in to_mailboxes {
        builder = builder.to(mailbox);
    }
    for mailbox in cc_mailboxes {
        builder = builder.cc(mailbox);
    }
    for mailbox in bcc_mailboxes {
        builder = builder.bcc(mailbox);
    }

    if !in_reply_to.trim().is_empty() {
        builder = builder
            .in_reply_to(in_reply_to.trim().to_string())
            .references(in_reply_to.trim().to_string());
    }

    let message = if attachment_paths.is_empty() {
        builder.body(body_text.to_string())
    } else {
        let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(body_text.to_string()));
        for path_str in attachment_paths {
            let path = Path::new(path_str);
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("attachment.bin")
                .to_string();
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    return (
                        SendOutcome::Failed(format!("读取附件失败 ({}): {e}", path.display())),
                        None,
                    )
                }
            };
            let content_type = "application/octet-stream"
                .parse()
                .expect("valid fallback mime");
            multipart = multipart.singlepart(Attachment::new(filename).body(bytes, content_type));
        }
        builder.multipart(multipart)
    };

    let email = match message {
        Ok(m) => m,
        Err(e) => {
            return (
                SendOutcome::Failed(format!("构建邮件 MIME 失败: {e}")),
                None,
            )
        }
    };
    let raw_rfc822 = email.formatted();

    let transport = match build_transport(account, password) {
        Ok(t) => t,
        Err(e) => return (SendOutcome::Failed(e), None),
    };

    match transport.send(email).await {
        Ok(_) => (SendOutcome::Accepted, Some(raw_rfc822)),
        Err(e) => {
            if e.is_transient() || e.is_timeout() || e.is_client() {
                (
                    SendOutcome::Uncertain(format!("发送结果待确认（传输未收到最终回执）: {e}")),
                    None,
                )
            } else {
                (SendOutcome::Failed(format!("SMTP 发送被拒绝: {e}")), None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mailbox_list_supports_multiple_comma_separated_addresses() {
        let list = parse_mailbox_list("Alice <alice@example.com>, bob@example.com, ", "收件人")
            .expect("multiple recipients should parse");
        assert_eq!(list.len(), 2);
        assert!(parse_mailbox_list("   ,  ", "收件人")
            .expect("empty list parses as empty")
            .is_empty());
        assert!(parse_mailbox_list("not-an-email", "收件人").is_err());
    }
}
