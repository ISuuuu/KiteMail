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
#[allow(clippy::too_many_arguments)]
pub async fn send_message(
    account: &AccountConfig,
    password: &str,
    to_addr: &str,
    cc_addrs: &str,
    bcc_addrs: &str,
    subject: &str,
    body_text: &str,
    in_reply_to: &str,
    from_addr: &str,
    reply_to: &str,
    body_html: Option<&str>,
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
        from_addr,
        reply_to,
        body_html,
        attachment_paths,
    )
    .await
    .0
}

/// 组装信体：有 HTML 正文时按 `multipart/alternative` 发送（`text/plain` 兜底），
/// 有附件时再套一层 `multipart/mixed`。
///
/// WP-10-3：写信器的富文本 / Markdown / HTML 三种格式都在应用层转成 HTML 后
/// 传进来；纯文本模式传 `None`，保持「一个 `text/plain` 单段」的老行为。
fn assemble_body(
    builder: lettre::message::MessageBuilder,
    body_text: &str,
    body_html: Option<&str>,
    attachment_paths: &[String],
) -> Result<Message, String> {
    // 正文里的 data: 图片提升成 cid: 内联段（multipart/related）。没有这一步，
    // Gmail / Outlook 会把 base64 图当外链拦掉，签名 logo 显示红叉。
    let (body_html, inline_images) = match body_html {
        Some(html) => {
            let (html, images) = crate::inline::extract_data_images(html);
            (Some(html), images)
        }
        None => (None, Vec::new()),
    };

    let alternative = body_html.map(|html| {
        let alternative = MultiPart::alternative()
            .singlepart(SinglePart::plain(body_text.to_string()))
            .singlepart(SinglePart::html(html));
        if inline_images.is_empty() {
            return alternative;
        }
        let mut related = MultiPart::related().multipart(alternative);
        for image in &inline_images {
            let content_type = image.mime.parse().unwrap_or_else(|_| {
                "application/octet-stream"
                    .parse()
                    .expect("valid fallback mime")
            });
            related = related.singlepart(
                Attachment::new_inline_with_name(image.content_id.clone(), image.filename.clone())
                    .body(image.data.clone(), content_type),
            );
        }
        related
    });

    let built = if attachment_paths.is_empty() {
        match alternative {
            Some(alt) => builder.multipart(alt),
            None => builder.body(body_text.to_string()),
        }
    } else {
        let mut multipart = match alternative {
            Some(alt) => MultiPart::mixed().multipart(alt),
            None => MultiPart::mixed().singlepart(SinglePart::plain(body_text.to_string())),
        };
        for path_str in attachment_paths {
            let path = Path::new(path_str);
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("attachment.bin")
                .to_string();
            let bytes = std::fs::read(path)
                .map_err(|e| format!("读取附件失败 ({}): {e}", path.display()))?;
            let content_type = "application/octet-stream"
                .parse()
                .expect("valid fallback mime");
            multipart = multipart.singlepart(Attachment::new(filename).body(bytes, content_type));
        }
        builder.multipart(multipart)
    };
    built.map_err(|e| format!("构建邮件 MIME 失败: {e}"))
}

/// 构造信封与信头（From / To / Cc / Bcc / In-Reply-To / Reply-To）。
///
/// 抽成独立函数以便单测直接核对头字段，无需真正连 SMTP。
/// `from_addr` 非空时覆盖账号默认身份；`reply_to` 非空时写入 `Reply-To`。
#[allow(clippy::too_many_arguments)]
fn build_envelope(
    account: &AccountConfig,
    to_addr: &str,
    cc_addrs: &str,
    bcc_addrs: &str,
    subject: &str,
    in_reply_to: &str,
    from_addr: &str,
    reply_to: &str,
) -> Result<lettre::message::MessageBuilder, String> {
    let from_str = if from_addr.trim().is_empty() {
        if account.display_name.trim().is_empty() {
            account.email.clone()
        } else {
            format!("{} <{}>", account.display_name.trim(), account.email.trim())
        }
    } else {
        from_addr.trim().to_string()
    };
    let from_mailbox: Mailbox = from_str
        .parse()
        .map_err(|e| format!("发件人地址格式无效: {e}"))?;
    let to_mailboxes = parse_mailbox_list(to_addr, "收件人")?;
    if to_mailboxes.is_empty() {
        return Err("收件人地址不能为空".to_string());
    }
    let cc_mailboxes = parse_mailbox_list(cc_addrs, "抄送")?;
    let bcc_mailboxes = parse_mailbox_list(bcc_addrs, "密送")?;

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

    // Reply-To 指向的是「回信落到哪里」，与信封 From 独立，多个地址按逗号分隔。
    for mailbox in parse_mailbox_list(reply_to, "Reply-To")? {
        builder = builder.reply_to(mailbox);
    }

    Ok(builder)
}

/// Build and send an email, also returning the serialized RFC 822 bytes when accepted
/// so the caller can independently append a Sent copy via IMAP.
///
/// `from_addr` 非空时覆盖账号默认身份（对齐原版写信器的自定义 From / 别名），
/// `reply_to` 非空时写入 `Reply-To` 信头。
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
    from_addr: &str,
    reply_to: &str,
    body_html: Option<&str>,
    attachment_paths: &[String],
) -> (SendOutcome, Option<Vec<u8>>) {
    let builder = match build_envelope(
        account,
        to_addr,
        cc_addrs,
        bcc_addrs,
        subject,
        in_reply_to,
        from_addr,
        reply_to,
    ) {
        Ok(builder) => builder,
        Err(e) => return (SendOutcome::Failed(e), None),
    };

    let email = match assemble_body(builder, body_text, body_html, attachment_paths) {
        Ok(m) => m,
        Err(e) => return (SendOutcome::Failed(e), None),
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

    /// WP-10-2：自定义 From 与 Reply-To 必须真的写进 RFC822 头。
    #[test]
    fn custom_from_and_reply_to_land_in_headers() {
        let account = AccountConfig {
            email: "me@example.com".to_string(),
            display_name: "账号默认名".to_string(),
            ..Default::default()
        };
        let builder = build_envelope(
            &account,
            "to@example.com",
            "cc@example.com",
            "",
            "主题",
            "<prev@example.com>",
            "别名 <alias@example.com>",
            "replies@example.com",
        )
        .expect("envelope builds");
        let raw = String::from_utf8(builder.body("正文".to_string()).unwrap().formatted())
            .expect("utf8 message");
        assert!(raw.contains("From: "), "缺 From 头: {raw}");
        assert!(
            raw.contains("alias@example.com"),
            "自定义 From 未生效: {raw}"
        );
        assert!(
            !raw.contains("me@example.com>"),
            "仍在使用账号默认地址: {raw}"
        );
        assert!(
            raw.contains("Reply-To: replies@example.com"),
            "缺 Reply-To 头: {raw}"
        );
        assert!(
            raw.contains("In-Reply-To: <prev@example.com>"),
            "缺 In-Reply-To: {raw}"
        );
    }

    /// WP-10-3：带 HTML 正文时发出 multipart/alternative（纯文本兜底 + HTML）。
    #[test]
    fn html_body_produces_multipart_alternative() {
        let account = AccountConfig {
            email: "me@example.com".to_string(),
            display_name: "我".to_string(),
            ..Default::default()
        };
        let builder = build_envelope(&account, "to@example.com", "", "", "主题", "", "", "")
            .expect("envelope builds");
        let message = assemble_body(
            builder,
            "纯文本兜底",
            Some("<p>富文本<strong>正文</strong></p>"),
            &[],
        )
        .expect("body assembles");
        let raw = String::from_utf8(message.formatted()).expect("utf8 message");
        assert!(raw.contains("multipart/alternative"), "未发多段正文: {raw}");
        assert!(raw.contains("text/plain"), "缺纯文本兜底段: {raw}");
        assert!(raw.contains("text/html"), "缺 HTML 段: {raw}");
    }

    /// 正文里的 data: 图片提升成 multipart/related 里的 cid: 内联段，
    /// 收件端才会显示图片而不是红叉。
    #[test]
    fn inline_data_image_becomes_related_part() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let account = AccountConfig {
            email: "me@example.com".to_string(),
            display_name: "我".to_string(),
            ..Default::default()
        };
        let builder = build_envelope(&account, "to@example.com", "", "", "主题", "", "", "")
            .expect("envelope builds");
        let html = format!(r#"<p>正文<img src="data:image/png;base64,{PNG}"></p>"#);
        let message = assemble_body(builder, "正文", Some(&html), &[]).expect("body assembles");
        let raw = String::from_utf8(message.formatted()).expect("utf8 message");
        assert!(raw.contains("multipart/related"), "缺内联段容器: {raw}");
        assert!(raw.contains("Content-ID: <hylki-"), "缺 Content-ID: {raw}");
        assert!(raw.contains("cid:hylki-"), "HTML 未改写成 cid: {raw}");
        assert!(!raw.contains("data:image/png"), "base64 仍留在正文: {raw}");
        assert!(raw.contains("image/png"), "缺图片类型: {raw}");
    }

    /// 纯文本模式仍发单段 text/plain（不引入 multipart 开销）。
    #[test]
    fn plain_body_keeps_single_part() {
        let account = AccountConfig {
            email: "me@example.com".to_string(),
            display_name: "我".to_string(),
            ..Default::default()
        };
        let builder = build_envelope(&account, "to@example.com", "", "", "主题", "", "", "")
            .expect("envelope builds");
        let message = assemble_body(builder, "只有纯文本", None, &[]).expect("body assembles");
        let raw = String::from_utf8(message.formatted()).expect("utf8 message");
        // 单段纯文本不写 Content-Type（RFC 默认即 text/plain）。
        assert!(
            !raw.contains("multipart/alternative"),
            "不该有多段正文: {raw}"
        );
        assert!(!raw.contains("text/html"), "不该有 HTML 段: {raw}");
        assert!(!raw.contains("Content-Type:"), "不该写 Content-Type: {raw}");
    }

    /// 未指定自定义 From / Reply-To 时沿用账号身份，且不产生 Reply-To 头。
    #[test]
    fn empty_overrides_fall_back_to_account_identity() {
        let account = AccountConfig {
            email: "me@example.com".to_string(),
            display_name: "我的名字".to_string(),
            ..Default::default()
        };
        let builder = build_envelope(&account, "to@example.com", "", "", "s", "", "", "")
            .expect("envelope builds");
        let raw = String::from_utf8(builder.body("b".to_string()).unwrap().formatted())
            .expect("utf8 message");
        assert!(raw.contains("me@example.com"), "缺默认 From: {raw}");
        assert!(!raw.contains("Reply-To"), "不该有 Reply-To 头: {raw}");
    }

    /// 自定义 From 非法时构造函数直接报错，避免带着坏地址发信。
    #[test]
    fn invalid_custom_from_is_rejected() {
        let account = AccountConfig {
            email: "me@example.com".to_string(),
            ..Default::default()
        };
        assert!(build_envelope(
            &account,
            "to@example.com",
            "",
            "",
            "s",
            "",
            "not-an-address",
            ""
        )
        .is_err());
    }
}
