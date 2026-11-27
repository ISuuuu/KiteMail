//! 应用锁屏密码校验（PBKDF2-HMAC-SHA256）、防暴力破解退避与自动锁定状态机。

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

/// 锁屏密码最小字符数。
pub const PASSCODE_MIN_LEN: usize = 4;
/// 锁屏密码最大字节数（防止超长输入造成哈希 DoS）。
pub const PASSCODE_MAX_LEN: usize = 128;

const PBKDF2_SCHEME: &str = "pbkdf2_sha256";
const PBKDF2_ITERATIONS: u32 = 200_000;
const MIN_VERIFY_ITERATIONS: u32 = 10_000;
const MIN_SALT_LEN: usize = 8;
const SALT_LEN: usize = 16;
const DERIVED_KEY_LEN: usize = 32;

/// 后台「立即锁定」的防抖宽限时长，避免弹出菜单或系统瞬时失焦触发误锁。
pub const BACKGROUND_IMMEDIATE_GRACE: Duration = Duration::from_millis(1500);

const LOCK_STATE_FILE_NAME: &str = "app_lock.state";

/// 查询数据目录中是否记录了「处于锁定状态」（用于防止锁定后重启绕过）。
pub fn is_lock_persisted_in(data_dir: &Path) -> bool {
    data_dir.join(LOCK_STATE_FILE_NAME).is_file()
}

/// 持久化或清除「处于锁定状态」标记（仅在锁定后重启时要求重新解锁）。
pub fn set_lock_persisted_in(data_dir: &Path, locked: bool) -> Result<(), String> {
    let path = data_dir.join(LOCK_STATE_FILE_NAME);
    if locked {
        fs::create_dir_all(data_dir)
            .map_err(|e| format!("创建锁屏状态目录失败 ({}): {e}", data_dir.display()))?;
        fs::write(&path, b"locked\n")
            .map_err(|e| format!("写入锁屏状态失败 ({}): {e}", path.display()))
    } else {
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("清除锁屏状态失败 ({}): {e}", path.display())),
        }
    }
}

/// 校验新设置的锁屏密码是否满足最小/最大长度与非纯空白要求。
pub fn validate_new_passcode(passcode: &str) -> Result<(), String> {
    if passcode.is_empty() || passcode.trim().is_empty() {
        return Err("密码不能为空或仅包含空白字符".to_string());
    }
    if passcode.chars().count() < PASSCODE_MIN_LEN {
        return Err(format!("密码至少需要 {PASSCODE_MIN_LEN} 个字符"));
    }
    if passcode.len() > PASSCODE_MAX_LEN {
        return Err(format!("密码长度不能超过 {PASSCODE_MAX_LEN} 字节"));
    }
    Ok(())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let digest = Sha256::digest(key);
        key_block[..32].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let outer_digest = outer.finalize();

    let mut out = [0u8; 32];
    out.copy_from_slice(&outer_digest);
    out
}

/// 标准 RFC 2898 PBKDF2-HMAC-SHA256（单块 32 字节输出，对应 `dkLen = 32`）。
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; DERIVED_KEY_LEN] {
    let mut first_input = Vec::with_capacity(salt.len() + 4);
    first_input.extend_from_slice(salt);
    first_input.extend_from_slice(&1u32.to_be_bytes());

    let mut u = hmac_sha256(password, &first_input);
    let mut dk = u;
    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for (acc, byte) in dk.iter_mut().zip(u.iter()) {
            *acc ^= *byte;
        }
    }
    dk
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 生成随机盐并使用 PBKDF2-HMAC-SHA256 派生密码校验摘要。
pub fn hash_passcode(passcode: &str) -> Result<String, String> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt).map_err(|e| format!("无法生成随机盐值: {e}"))?;
    let dk = pbkdf2_hmac_sha256(passcode.as_bytes(), &salt, PBKDF2_ITERATIONS);
    Ok(format!(
        "{PBKDF2_SCHEME}${PBKDF2_ITERATIONS}${}${}",
        BASE64_STANDARD.encode(salt),
        BASE64_STANDARD.encode(dk)
    ))
}

/// 使用常量时间比较验证用户输入密码是否匹配已存哈希。
pub fn verify_passcode(passcode: &str, stored_hash: &str) -> bool {
    if passcode.is_empty() || passcode.len() > PASSCODE_MAX_LEN {
        return false;
    }
    let mut parts = stored_hash.trim().split('$');
    let Some(scheme) = parts.next() else {
        return false;
    };
    let Some(iter_str) = parts.next() else {
        return false;
    };
    let Some(salt_b64) = parts.next() else {
        return false;
    };
    let Some(dk_b64) = parts.next() else {
        return false;
    };
    if parts.next().is_some() || scheme != PBKDF2_SCHEME {
        return false;
    }
    let Ok(iterations) = iter_str.parse::<u32>() else {
        return false;
    };
    if iterations < MIN_VERIFY_ITERATIONS {
        return false;
    }
    let Ok(salt) = BASE64_STANDARD.decode(salt_b64) else {
        return false;
    };
    let Ok(expected_dk) = BASE64_STANDARD.decode(dk_b64) else {
        return false;
    };
    if salt.len() < MIN_SALT_LEN || expected_dk.len() != DERIVED_KEY_LEN {
        return false;
    }
    let computed = pbkdf2_hmac_sha256(passcode.as_bytes(), &salt, iterations);
    constant_time_eq(&computed, &expected_dk)
}

/// 根据连续输错次数计算防暴力破解冷却秒数。
pub fn unlock_cooldown_secs(failed_attempts: u32) -> u64 {
    match failed_attempts {
        0..=4 => 0,
        5 => 5,
        6 => 10,
        7 => 30,
        _ => 60,
    }
}

/// 切到后台 / 最小化 / 隐藏托盘后的自动锁定策略。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundLockPolicy {
    Immediate,
    Minutes(u32),
    Never,
}

impl BackgroundLockPolicy {
    /// 从持久化偏好字符串解析后台锁定策略；未知值回落默认 5 分钟。
    pub fn from_pref_str(s: &str) -> Self {
        match s.trim() {
            "immediate" => Self::Immediate,
            "never" => Self::Never,
            other => match other.parse::<u32>() {
                Ok(mins) if mins > 0 => Self::Minutes(mins),
                _ => Self::Minutes(5),
            },
        }
    }

    pub fn as_pref_str(self) -> String {
        match self {
            Self::Immediate => "immediate".to_string(),
            Self::Minutes(m) => m.to_string(),
            Self::Never => "never".to_string(),
        }
    }
}

/// 触发自动锁屏的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoLockTrigger {
    Idle,
    Background,
}

/// 单次自动锁屏状态评估输入。
#[derive(Clone, Copy, Debug)]
pub struct AutoLockTickInput {
    pub now: Instant,
    pub has_passcode: bool,
    pub is_locked: bool,
    pub in_foreground: bool,
    pub file_dialog_open: bool,
    pub system_idle: Option<Duration>,
    pub idle_minutes: u32,
    pub background_policy: BackgroundLockPolicy,
}

/// 纯逻辑自动锁屏计时追踪器（不直接调用 OS API，便于单元测试）。
#[derive(Clone, Debug)]
pub struct AutoLockTracker {
    unlocked_at: Instant,
    background_since: Option<Instant>,
}

impl Default for AutoLockTracker {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl AutoLockTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            unlocked_at: now,
            background_since: None,
        }
    }

    /// 解锁成功或修改锁屏密码后重置计时基线，防止系统历史空闲时间导致立刻重锁。
    pub fn reset_after_unlock(&mut self, now: Instant) {
        self.unlocked_at = now;
        self.background_since = None;
    }

    /// 推进一次计时评估，返回是否到达自动锁定阈值。
    pub fn evaluate(&mut self, input: AutoLockTickInput) -> Option<AutoLockTrigger> {
        if !input.has_passcode || input.is_locked {
            self.background_since = None;
            return None;
        }

        if input.in_foreground {
            self.background_since = None;
            if input.idle_minutes > 0 {
                if let Some(sys_idle) = input.system_idle {
                    let since_unlock = input.now.saturating_duration_since(self.unlocked_at);
                    let effective_idle = sys_idle.min(since_unlock);
                    if effective_idle >= Duration::from_secs(u64::from(input.idle_minutes) * 60) {
                        return Some(AutoLockTrigger::Idle);
                    }
                }
            }
            return None;
        }

        if input.file_dialog_open {
            return None;
        }

        let bg_start = *self.background_since.get_or_insert(input.now);
        let bg_elapsed = input.now.saturating_duration_since(bg_start);
        match input.background_policy {
            BackgroundLockPolicy::Immediate => {
                if bg_elapsed >= BACKGROUND_IMMEDIATE_GRACE {
                    return Some(AutoLockTrigger::Background);
                }
            }
            BackgroundLockPolicy::Minutes(m) => {
                if m > 0 && bg_elapsed >= Duration::from_secs(u64::from(m) * 60) {
                    return Some(AutoLockTrigger::Background);
                }
            }
            BackgroundLockPolicy::Never => {
                if input.idle_minutes > 0 {
                    if let Some(sys_idle) = input.system_idle {
                        let since_unlock = input.now.saturating_duration_since(self.unlocked_at);
                        let effective_idle = sys_idle.min(since_unlock);
                        if effective_idle >= Duration::from_secs(u64::from(input.idle_minutes) * 60)
                        {
                            return Some(AutoLockTrigger::Idle);
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn rfc4231_hmac_sha256_test_vector_2() {
        // RFC 4231 Test Case 2: key = "Jefe", data = "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            to_hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn rfc6070_style_pbkdf2_hmac_sha256_vector() {
        // 标准 PBKDF2-HMAC-SHA256 测试向量: password="password", salt="salt", c=1
        let dk1 = pbkdf2_hmac_sha256(b"password", b"salt", 1);
        assert_eq!(
            to_hex(&dk1),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        // c=2
        let dk2 = pbkdf2_hmac_sha256(b"password", b"salt", 2);
        assert_eq!(
            to_hex(&dk2),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
        // c=4096
        let dk4096 = pbkdf2_hmac_sha256(b"password", b"salt", 4096);
        assert_eq!(
            to_hex(&dk4096),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
    }

    #[test]
    fn validate_new_passcode_checks_boundaries() {
        assert!(validate_new_passcode("").is_err());
        assert!(validate_new_passcode("    ").is_err());
        assert!(validate_new_passcode("123").is_err());
        assert!(validate_new_passcode("1234").is_ok());
        assert!(validate_new_passcode("密码锁1").is_ok());
        let too_long = "a".repeat(PASSCODE_MAX_LEN + 1);
        assert!(validate_new_passcode(&too_long).is_err());
        let exact_max = "a".repeat(PASSCODE_MAX_LEN);
        assert!(validate_new_passcode(&exact_max).is_ok());
    }

    #[test]
    fn hash_and_verify_passcode_handles_salt_uniqueness_and_malformed_inputs() {
        let h1 = hash_passcode("secret-1234").expect("hash 1");
        let h2 = hash_passcode("secret-1234").expect("hash 2");
        assert_ne!(h1, h2, "random salts must produce distinct hashes");
        assert!(verify_passcode("secret-1234", &h1));
        assert!(verify_passcode("secret-1234", &h2));
        assert!(!verify_passcode("wrong-pass", &h1));
        assert!(!verify_passcode("", &h1));

        // 畸形串与迭代次数降级应被拒绝。
        assert!(!verify_passcode("secret-1234", "invalid"));
        assert!(!verify_passcode(
            "secret-1234",
            "pbkdf2_sha256$100$c2FsdHNhbHQ=$AAAA"
        ));
        assert!(!verify_passcode(
            "secret-1234",
            "argon2id$200000$c2FsdHNhbHQ=$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        ));
    }

    #[test]
    fn unlock_cooldown_ladder_matches_policy() {
        assert_eq!(unlock_cooldown_secs(0), 0);
        assert_eq!(unlock_cooldown_secs(4), 0);
        assert_eq!(unlock_cooldown_secs(5), 5);
        assert_eq!(unlock_cooldown_secs(6), 10);
        assert_eq!(unlock_cooldown_secs(7), 30);
        assert_eq!(unlock_cooldown_secs(8), 60);
        assert_eq!(unlock_cooldown_secs(20), 60);
    }

    #[test]
    fn auto_lock_tracker_covers_foreground_background_grace_and_dialogs() {
        let t0 = Instant::now();
        let mut tracker = AutoLockTracker::new(t0);

        // 未设密码或已锁定时不触发。
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t0 + Duration::from_secs(3600),
                has_passcode: false,
                is_locked: false,
                in_foreground: true,
                file_dialog_open: false,
                system_idle: Some(Duration::from_secs(3600)),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            None
        );
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t0 + Duration::from_secs(3600),
                has_passcode: true,
                is_locked: true,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::from_secs(3600)),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Immediate,
            }),
            None
        );

        // 刚解锁即使系统空闲已经很大，也不得立刻重新锁。
        tracker.reset_after_unlock(t0 + Duration::from_secs(100));
        let t_unlock = t0 + Duration::from_secs(100);
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t_unlock + Duration::from_secs(60),
                has_passcode: true,
                is_locked: false,
                in_foreground: true,
                file_dialog_open: false,
                system_idle: Some(Duration::from_secs(7200)),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            None
        );
        // 解锁后真正达到 15 分钟才触发 Idle。
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t_unlock + Duration::from_secs(15 * 60),
                has_passcode: true,
                is_locked: false,
                in_foreground: true,
                file_dialog_open: false,
                system_idle: Some(Duration::from_secs(15 * 60)),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            Some(AutoLockTrigger::Idle)
        );

        // 后台倒计时、中途回前台清空、以及文件对话框豁免。
        tracker.reset_after_unlock(t_unlock);
        let bg_start = t_unlock + Duration::from_secs(10);
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: bg_start,
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            None
        );
        // 4 分钟回到前台会清除后台计时。
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: bg_start + Duration::from_secs(240),
                has_passcode: true,
                is_locked: false,
                in_foreground: true,
                file_dialog_open: false,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            None
        );
        // 再次切后台需重新计满 5 分钟。
        let bg2 = bg_start + Duration::from_secs(250);
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: bg2,
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            None
        );
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: bg2 + Duration::from_secs(299),
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            None
        );
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: bg2 + Duration::from_secs(300),
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Minutes(5),
            }),
            Some(AutoLockTrigger::Background)
        );

        // Immediate 防抖与 file_dialog_open 免误锁。
        tracker.reset_after_unlock(bg2 + Duration::from_secs(310));
        let t_imm = bg2 + Duration::from_secs(320);
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t_imm,
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: true,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Immediate,
            }),
            None
        );
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t_imm + Duration::from_secs(10),
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Immediate,
            }),
            None
        );
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t_imm + Duration::from_secs(12),
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::ZERO),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Immediate,
            }),
            Some(AutoLockTrigger::Background)
        );

        // Never 后台策略不因后台超时锁定，但仍尊重系统整体空闲阈值。
        tracker.reset_after_unlock(t_imm + Duration::from_secs(20));
        let t_never = t_imm + Duration::from_secs(20);
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t_never + Duration::from_secs(600),
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::from_secs(10)),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Never,
            }),
            None
        );
        assert_eq!(
            tracker.evaluate(AutoLockTickInput {
                now: t_never + Duration::from_secs(900),
                has_passcode: true,
                is_locked: false,
                in_foreground: false,
                file_dialog_open: false,
                system_idle: Some(Duration::from_secs(900)),
                idle_minutes: 15,
                background_policy: BackgroundLockPolicy::Never,
            }),
            Some(AutoLockTrigger::Idle)
        );
    }

    #[test]
    fn background_lock_policy_roundtrip() {
        assert_eq!(
            BackgroundLockPolicy::from_pref_str("immediate"),
            BackgroundLockPolicy::Immediate
        );
        assert_eq!(
            BackgroundLockPolicy::from_pref_str("never"),
            BackgroundLockPolicy::Never
        );
        assert_eq!(
            BackgroundLockPolicy::from_pref_str("10"),
            BackgroundLockPolicy::Minutes(10)
        );
        assert_eq!(
            BackgroundLockPolicy::from_pref_str("invalid"),
            BackgroundLockPolicy::Minutes(5)
        );
        assert_eq!(BackgroundLockPolicy::Immediate.as_pref_str(), "immediate");
        assert_eq!(BackgroundLockPolicy::Minutes(15).as_pref_str(), "15");
        assert_eq!(BackgroundLockPolicy::Never.as_pref_str(), "never");
    }

    #[test]
    fn persisted_lock_state_roundtrip_and_missing_is_false() {
        let dir = std::env::temp_dir().join(format!("hylki-lock-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        assert!(!is_lock_persisted_in(&dir));
        set_lock_persisted_in(&dir, false).expect("clearing absent state succeeds");
        assert!(!is_lock_persisted_in(&dir));

        set_lock_persisted_in(&dir, true).expect("persisting locked state succeeds");
        assert!(is_lock_persisted_in(&dir));

        set_lock_persisted_in(&dir, false).expect("clearing persisted state succeeds");
        assert!(!is_lock_persisted_in(&dir));

        let _ = fs::remove_dir_all(&dir);
    }
}
