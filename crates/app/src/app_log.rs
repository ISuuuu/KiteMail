//! 轻量本地运行日志（写入 `<data_dir>/logs/kitemail.log`，带大小轮转）。
//!
//! 仅记录 Toast 通知、应用锁屏状态变化与 Panic 信息，绝不记录密码明文、凭据或邮件正文。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Local;
use kitemail_mail_core::config;

use crate::ui::toast::ToastKind;

const LOG_DIR_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "kitemail.log";
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

static LOG_MUTEX: Mutex<()> = Mutex::new(());

fn log_file_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(LOG_DIR_NAME).join(LOG_FILE_NAME)
}

fn rotate_if_needed(path: &Path, max_bytes: u64) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() < max_bytes {
        return;
    }
    let backup = path.with_extension("log.1");
    let _ = fs::remove_file(&backup);
    let _ = fs::rename(path, &backup);
}

fn sanitize_line_part(text: &str) -> String {
    text.replace('\r', "\\r").replace('\n', "\\n")
}

fn write_record_in(data_dir: &Path, max_bytes: u64, level: &str, category: &str, message: &str) {
    let path = log_file_path_in(data_dir);
    let _guard = match LOG_MUTEX.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    rotate_if_needed(&path, max_bytes);
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!(
        "[{ts}] [{level}] [{category}] {}\n",
        sanitize_line_part(message)
    );
    let _ = file.write_all(line.as_bytes());
}

/// 写入一条结构化运行日志。
pub(crate) fn log(level: &str, category: &str, message: &str) {
    write_record_in(&config::data_dir(), MAX_LOG_BYTES, level, category, message);
}

/// 记录所有展示给用户的 Toast 通知（含弹窗 key、正文与可选 detail）。
pub(crate) fn log_toast(
    channel: &str,
    key: &str,
    kind: ToastKind,
    message: &str,
    detail: Option<&str>,
) {
    let level = match kind {
        ToastKind::Error => "ERROR",
        ToastKind::Warning => "WARN",
        ToastKind::Info | ToastKind::Success => "INFO",
    };
    let text = match detail.filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("({channel}) key={key} msg={message} | detail={d}"),
        None => format!("({channel}) key={key} msg={message}"),
    };
    log(level, "toast", &text);
}

/// 安装全局 Panic Hook，崩溃时将线程与位置信息写入本地日志。
pub(crate) fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        log(
            "PANIC",
            "panic",
            &format!("thread={thread_name} location={location} payload={payload}"),
        );
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hylki-app-log-{label}-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_record_creates_dir_and_sanitizes_newlines() {
        let dir = temp_dir("write");
        write_record_in(&dir, 1024 * 1024, "ERROR", "toast", "line1\nline2");

        let content = fs::read_to_string(log_file_path_in(&dir)).expect("log file exists");
        assert!(content.contains("[ERROR] [toast] line1\\nline2"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_record_rotates_when_exceeding_max_bytes() {
        let dir = temp_dir("rotate");
        write_record_in(&dir, 64, "INFO", "test", &"a".repeat(80));
        write_record_in(&dir, 64, "WARN", "test", "after-rotate");

        let primary = fs::read_to_string(log_file_path_in(&dir)).expect("primary log exists");
        let backup =
            fs::read_to_string(log_file_path_in(&dir).with_extension("log.1")).expect("backup log");
        assert!(primary.contains("[WARN] [test] after-rotate"));
        assert!(backup.contains("[INFO] [test]"));
        let _ = fs::remove_dir_all(&dir);
    }
}
