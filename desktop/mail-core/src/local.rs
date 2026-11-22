//! Local Maildir & Mbox archive ("On This Computer") storage (#552).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Maildir {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaildirEntry {
    pub path: PathBuf,
    pub filename: String,
    pub unread: bool,
    pub flagged: bool,
}

impl Maildir {
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        let store = Self { root };
        store.ensure_folder("INBOX")?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn folder_dir(&self, folder: &str) -> PathBuf {
        let sanitized = folder.trim_matches('/').replace(['\\', ':'], "_");
        if sanitized.eq_ignore_ascii_case("INBOX") || sanitized.is_empty() {
            self.root.join("INBOX")
        } else {
            self.root.join(sanitized)
        }
    }

    pub fn ensure_folder(&self, folder: &str) -> std::io::Result<PathBuf> {
        let base = self.folder_dir(folder);
        std::fs::create_dir_all(base.join("cur"))?;
        std::fs::create_dir_all(base.join("new"))?;
        std::fs::create_dir_all(base.join("tmp"))?;
        Ok(base)
    }

    pub fn list_folders(&self) -> std::io::Result<Vec<String>> {
        let mut folders = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && path.join("cur").is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    folders.push(name.to_string());
                }
            }
        }
        folders.sort();
        Ok(folders)
    }

    pub fn deliver(&self, folder: &str, raw_rfc822: &[u8], seen: bool) -> std::io::Result<PathBuf> {
        let base = self.ensure_folder(folder)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pid = std::process::id();
        let sep = if cfg!(windows) { ";2," } else { ":2," };
        let flags = if seen {
            format!("{sep}S")
        } else {
            sep.to_string()
        };
        let filename = format!("{nanos}.{pid}.hylki{flags}");
        let tmp = base.join("tmp").join(&filename);
        let dest = base.join(if seen { "cur" } else { "new" }).join(&filename);
        std::fs::write(&tmp, raw_rfc822)?;
        std::fs::rename(&tmp, &dest)?;
        Ok(dest)
    }

    pub fn list_entries(&self, folder: &str) -> std::io::Result<Vec<MaildirEntry>> {
        let base = self.folder_dir(folder);
        let mut out = Vec::new();
        for sub in ["new", "cur"] {
            let dir = base.join(sub);
            if !dir.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let filename = entry.file_name().to_string_lossy().into_owned();
                let flags = filename
                    .split_once(":2,")
                    .or_else(|| filename.split_once(";2,"))
                    .map(|(_, f)| f)
                    .unwrap_or("");
                let seen = sub == "cur" && flags.contains('S');
                let flagged = flags.contains('F');
                out.push(MaildirEntry {
                    path,
                    filename,
                    unread: !seen,
                    flagged,
                });
            }
        }
        Ok(out)
    }
}

/// Split an Mboxrd/Mboxo file into individual RFC 822 raw messages.
pub fn parse_mbox(mbox_bytes: &[u8]) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(mbox_bytes);
    let mut messages = Vec::new();
    let mut current = String::new();
    let mut in_msg = false;

    for line in text.split_inclusive('\n') {
        if line.starts_with("From ") {
            if in_msg && !current.is_empty() {
                messages.push(current.as_bytes().to_vec());
                current.clear();
            }
            in_msg = true;
            continue;
        }
        if in_msg {
            let unescaped = line
                .strip_prefix('>')
                .filter(|r| r.starts_with("From "))
                .unwrap_or(line);
            current.push_str(unescaped);
        }
    }
    if in_msg && !current.is_empty() {
        messages.push(current.into_bytes());
    }
    messages
}

/// Export raw RFC 822 messages into Mbox format.
pub fn export_mbox(messages: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for raw in messages {
        out.extend_from_slice(b"From MAILER-DAEMON Thu Jan  1 00:00:00 1970\n");
        let text = String::from_utf8_lossy(raw);
        for line in text.split_inclusive('\n') {
            if line.starts_with("From ") {
                out.push(b'>');
            }
            out.extend_from_slice(line.as_bytes());
        }
        if !text.ends_with('\n') {
            out.push(b'\n');
        }
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maildir_deliver_and_list_and_mbox_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hylki-maildir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let md = Maildir::open(&dir).unwrap();
        let raw = b"From: a@example.com\r\nSubject: Test\r\n\r\nHello\r\n";
        md.deliver("Archive", raw, true).unwrap();

        let folders = md.list_folders().unwrap();
        assert!(folders.contains(&"INBOX".to_string()));
        assert!(folders.contains(&"Archive".to_string()));

        let entries = md.list_entries("Archive").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].unread);

        let mbox = export_mbox(&[raw]);
        let parsed = parse_mbox(&mbox);
        assert_eq!(parsed.len(), 1);
        assert!(String::from_utf8_lossy(&parsed[0]).contains("Subject: Test"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
