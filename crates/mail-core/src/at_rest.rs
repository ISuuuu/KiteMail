//! Local database encryption-at-rest key management & SQLCipher boundary.
//!
//! Note on offline build boundary:
//! `desktop/Cargo.lock` bundles `rusqlite` with standard `sqlite3` (`bundled`),
//! without `sqlcipher`. Therefore `is_sqlcipher_available()` returns `false`
//! unless built with a SQLCipher-enabled `libsqlite3-sys`. All key generation,
//! validation, Windows Credential Manager storage, and plain/encrypted file
//! state detection are fully implemented and ready.

use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

const PLAIN_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Whether the linked SQLite library supports SQLCipher PRAGMAs (`PRAGMA cipher_version`).
pub fn is_sqlcipher_available() -> bool {
    rusqlite::Connection::open_in_memory()
        .ok()
        .and_then(|conn| {
            conn.query_row("PRAGMA cipher_version", [], |r| r.get::<_, String>(0))
                .ok()
        })
        .is_some_and(|v| !v.trim().is_empty())
}

/// 32-byte random key encoded as 64 lowercase hex digits.
#[derive(Clone, PartialEq, Eq)]
pub struct Key(std::sync::Arc<str>);

impl Key {
    pub fn generate() -> Result<Key, String> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|e| format!("无法生成随机密钥: {e}"))?;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        bytes.fill(0);
        Ok(Key(hex.into()))
    }

    pub fn from_hex(text: &str) -> Option<Key> {
        let text = text.trim();
        (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Key(text.to_ascii_lowercase().into()))
    }

    pub fn hex(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Protection {
    Encrypted(Key),
    Plain,
}

static UNLOCKED: OnceLock<Protection> = OnceLock::new();

pub fn unlock(protection: Protection) {
    let _ = UNLOCKED.set(protection);
}

pub fn current_protection() -> Option<Protection> {
    UNLOCKED.get().cloned()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Missing,
    Plain,
    Encrypted,
}

pub fn file_state(db: &Path) -> std::io::Result<FileState> {
    let mut head = Vec::with_capacity(PLAIN_MAGIC.len());
    match std::fs::File::open(db) {
        Ok(f) => {
            f.take(PLAIN_MAGIC.len() as u64).read_to_end(&mut head)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileState::Missing),
        Err(e) => return Err(e),
    }
    if head.as_slice() == PLAIN_MAGIC {
        return Ok(FileState::Plain);
    }
    if !head.is_empty() {
        return Ok(FileState::Encrypted);
    }
    Ok(FileState::Missing)
}

pub trait KeyStore {
    fn read(&self) -> Result<Option<String>, String>;
    fn store(&self, key: &Key) -> Result<(), String>;
}

/// Windows Credential Manager backed [`KeyStore`] via `keyring`.
pub struct WindowsKeyringStore {
    pub service: String,
    pub account: String,
}

impl Default for WindowsKeyringStore {
    fn default() -> Self {
        Self {
            service: "kitemail-desktop-cache".to_string(),
            account: "sqlite-at-rest-key".to_string(),
        }
    }
}

impl KeyStore for WindowsKeyringStore {
    fn read(&self) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(&self.service, &self.account)
            .map_err(|e| format!("凭据管理器初始化失败: {e}"))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("读取凭据失败: {e}")),
        }
    }

    fn store(&self, key: &Key) -> Result<(), String> {
        let entry = keyring::Entry::new(&self.service, &self.account)
            .map_err(|e| format!("凭据管理器初始化失败: {e}"))?;
        entry
            .set_password(key.hex())
            .map_err(|e| format!("保存凭据失败: {e}"))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    KeyringUnavailable(String),
    KeyMissing,
    WrongKey(String),
    SqlCipherUnsupported,
    Storage(String),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::KeyringUnavailable(e) => write!(f, "凭据管理器不可用: {e}"),
            Problem::KeyMissing => write!(f, "本地缓存已加密，但凭据管理器中缺少密钥"),
            Problem::WrongKey(e) => write!(f, "密钥格式无效或无法解密缓存: {e}"),
            Problem::SqlCipherUnsupported => write!(
                f,
                "当前离线构建的 rusqlite 未启用 SQLCipher 特性，仅提供密钥管理与状态检测"
            ),
            Problem::Storage(e) => write!(f, "无法读取缓存文件: {e}"),
        }
    }
}

/// Ensure a key exists in `store` when `encrypt` is requested, and validate
/// whether the runtime can open encrypted databases.
pub fn prepare_with_store(
    db: &Path,
    encrypt: bool,
    store: &dyn KeyStore,
) -> Result<Protection, Problem> {
    let state = file_state(db).map_err(|e| Problem::Storage(e.to_string()))?;
    if !encrypt {
        if state == FileState::Encrypted && !is_sqlcipher_available() {
            return Err(Problem::SqlCipherUnsupported);
        }
        return Ok(Protection::Plain);
    }

    let key = match store.read().map_err(Problem::KeyringUnavailable)? {
        Some(raw) => Key::from_hex(&raw)
            .ok_or_else(|| Problem::WrongKey("密钥长度或十六进制格式不合法".to_string()))?,
        None => {
            if state == FileState::Encrypted {
                return Err(Problem::KeyMissing);
            }
            let fresh = Key::generate().map_err(Problem::Storage)?;
            store.store(&fresh).map_err(Problem::KeyringUnavailable)?;
            fresh
        }
    };

    if !is_sqlcipher_available() {
        return Err(Problem::SqlCipherUnsupported);
    }
    Ok(Protection::Encrypted(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MemStore(RefCell<Option<String>>);

    impl KeyStore for MemStore {
        fn read(&self) -> Result<Option<String>, String> {
            Ok(self.0.borrow().clone())
        }

        fn store(&self, key: &Key) -> Result<(), String> {
            *self.0.borrow_mut() = Some(key.hex().to_string());
            Ok(())
        }
    }

    #[test]
    fn key_generation_and_hex_validation() {
        let k = Key::generate().unwrap();
        assert_eq!(k.hex().len(), 64);
        assert_eq!(Key::from_hex(&k.hex().to_uppercase()), Some(k));
        assert!(Key::from_hex("too-short").is_none());
    }

    #[test]
    fn plain_file_detection_and_sqlcipher_boundary() {
        let dir = std::env::temp_dir().join(format!("hylki-at-rest-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("test.db");
        std::fs::write(&db, b"SQLite format 3\0rest").unwrap();
        assert_eq!(file_state(&db).unwrap(), FileState::Plain);

        let store = MemStore(RefCell::new(None));
        let res = prepare_with_store(&db, true, &store);
        assert_eq!(res, Err(Problem::SqlCipherUnsupported));
        // Key was still generated and persisted into KeyStore.
        assert!(store.read().unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
