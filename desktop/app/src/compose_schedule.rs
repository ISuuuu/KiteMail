//! WP-10-5 定时发送（Send Later）的时间预设与手输时间解析。
//!
//! 移植自原版 `src/ui/later.rs` 的 `preset_time_from` / `days_until` /
//! `snooze_presets_at`：原版 Send Later 与 Snooze 共用同一份时间表，所以这里
//! 的预设与 `later.rs` 完全一致——今晚 18:00（还来得及的话）、明天上午 8:00、
//! 明天下午 13:00、周六上午 8:00、周一上午 8:00。
//!
//! 只做「算出时间戳」这件事，不碰 UI 与状态；调用方把时间戳交给 worker
//! 入队（`MailRequest::SendDraft { send_at: Some(剩余秒数) }`）。

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, TimeZone, Timelike};

/// 一个预设：菜单里显示的文字 + 对应的本地时间戳（秒）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaterPreset {
    pub(crate) label: String,
    pub(crate) at: i64,
}

/// `hour`:00 的显示文字（`18:00`）。
fn clock_label(hour: u32) -> String {
    format!("{hour:02}:00")
}

/// `days` 天后 `hour`:00 的本地时间戳（对齐原版 `preset_time_from`）。
fn preset_time_from<Tz: TimeZone>(now: &DateTime<Tz>, days: i64, hour: u32) -> i64 {
    let day = now.date_naive() + ChronoDuration::days(days);
    let ndt = match day.and_hms_opt(hour, 0, 0) {
        Some(ndt) => ndt,
        None => return now.timestamp(),
    };
    now.timezone()
        .from_local_datetime(&ndt)
        .single()
        .map(|t| t.timestamp())
        .unwrap_or_else(|| now.timestamp())
}

/// 距离最近的 `weekday`（0 = 周一）还有几天；今天就是则算一周后。
fn days_until(today: u32, weekday: u32) -> i64 {
    let ahead = (7 + weekday as i64 - today as i64) % 7;
    if ahead == 0 {
        7
    } else {
        ahead
    }
}

/// 当前时刻的定时发送预设（就近在前）。
pub(crate) fn later_presets() -> Vec<LaterPreset> {
    later_presets_at(&Local::now())
}

/// 以给定时刻为基准算预设，便于测试固定「今天」。
fn later_presets_at<Tz: TimeZone>(now: &DateTime<Tz>) -> Vec<LaterPreset> {
    let today = now.weekday().num_days_from_monday();
    let mut presets = Vec::new();

    // 今天 18:00——只在 17:00 之前有意义，否则「今晚」已经过去了。
    if now.hour() < 17 {
        presets.push(LaterPreset {
            label: format!("今天 {}", clock_label(18)),
            at: preset_time_from(now, 0, 18),
        });
    }
    presets.push(LaterPreset {
        label: format!("明天上午 {}", clock_label(8)),
        at: preset_time_from(now, 1, 8),
    });
    presets.push(LaterPreset {
        label: format!("明天下午 {}", clock_label(13)),
        at: preset_time_from(now, 1, 13),
    });
    // 周五的「明天」已经是周六，这条就没必要了。
    if today < 4 {
        presets.push(LaterPreset {
            label: format!("周六上午 {}", clock_label(8)),
            at: preset_time_from(now, days_until(today, 5), 8),
        });
    }
    // 周日的「明天上午」就是周一，避免出现两条一样的。
    if today != 6 {
        presets.push(LaterPreset {
            label: format!("周一上午 {}", clock_label(8)),
            at: preset_time_from(now, days_until(today, 0), 8),
        });
    }
    presets
}

/// 解析手输的定时时间：`YYYY-MM-DD HH:MM`（日期也接受 `/` 分隔）。
///
/// 与旧版 GTK 日历对话框的语义一致：**过去的时间直接拒绝**，而不是悄悄理解
/// 成「现在」。
pub(crate) fn parse_schedule_input(text: &str, now: DateTime<Local>) -> Result<i64, String> {
    const FORMAT_HINT: &str = "时间格式应为 2026-11-23 08:00";
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("请填写定时发送时间".to_string());
    }
    let normalized = trimmed.replace('/', "-").replace('：', ":");
    let mut parts = normalized.split_whitespace();
    let (Some(date), Some(clock), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(FORMAT_HINT.to_string());
    };

    let mut ymd = date.split('-');
    let (Some(y), Some(m), Some(d), None) = (ymd.next(), ymd.next(), ymd.next(), ymd.next()) else {
        return Err(FORMAT_HINT.to_string());
    };
    let (Ok(year), Ok(month), Ok(day)) = (y.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>())
    else {
        return Err(FORMAT_HINT.to_string());
    };

    let mut hm = clock.split(':');
    let (Some(h), Some(min), None) = (hm.next(), hm.next(), hm.next()) else {
        return Err(FORMAT_HINT.to_string());
    };
    let (Ok(hour), Ok(minute)) = (h.parse::<u32>(), min.parse::<u32>()) else {
        return Err(FORMAT_HINT.to_string());
    };

    let ndt = chrono::NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .ok_or_else(|| "该日期不存在，请检查月日与时间".to_string())?;
    let at = now
        .timezone()
        .from_local_datetime(&ndt)
        .single()
        .ok_or_else(|| "该时间在夏令时切换处不存在，请换一个时间".to_string())?;
    if at.timestamp() <= now.timestamp() {
        return Err("定时发送时间必须晚于当前时间".to_string());
    }
    Ok(at.timestamp())
}

/// 把时间戳格式化成写信栏里的提示文字（`11月23日 08:00`，跨年才带年份）。
pub(crate) fn schedule_label(at: i64) -> String {
    let Some(dt) = Local.timestamp_opt(at, 0).single() else {
        return String::new();
    };
    let now = Local::now();
    let hm = format!("{:02}:{:02}", dt.hour(), dt.minute());
    if dt.year() == now.year() {
        format!("{}月{}日 {}", dt.month(), dt.day(), hm)
    } else {
        format!("{}年{}月{}日 {}", dt.year(), dt.month(), dt.day(), hm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 2026-11-23（周一）08:30 本地时间。
    fn monday_morning() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 11, 23, 8, 30, 0)
            .single()
            .expect("valid local time")
    }

    fn labels(presets: &[LaterPreset]) -> Vec<String> {
        presets.iter().map(|p| p.label.clone()).collect()
    }

    #[test]
    fn presets_skip_tonight_after_five_pm() {
        // 08:30：今天 18:00 还有意义，应该出现。
        let morning = later_presets_at(&monday_morning());
        assert_eq!(labels(&morning)[0], "今天 18:00");

        // 18:30：今晚已经过去，不该再提供。
        let evening = Local
            .with_ymd_and_hms(2026, 11, 23, 18, 30, 0)
            .single()
            .unwrap();
        assert!(!labels(&later_presets_at(&evening))
            .iter()
            .any(|l| l.starts_with("今天")));
    }

    #[test]
    fn presets_are_strictly_increasing() {
        // 每个预设都比前一个晚，且都晚于现在（否则选中会立刻发送）。
        let now = monday_morning();
        let presets = later_presets_at(&now);
        assert!(!presets.is_empty());
        let mut prev = now.timestamp();
        for preset in &presets {
            assert!(preset.at > prev, "预设时间未递增: {preset:?}");
            prev = preset.at;
        }
    }

    #[test]
    fn monday_preset_moves_to_next_week_on_monday() {
        // 今天就是周一：「周一上午」应指下周一而不是今天。
        let now = monday_morning();
        let presets = later_presets_at(&now);
        let next_monday = presets
            .iter()
            .find(|p| p.label.starts_with("周一上午"))
            .expect("应该有周一预设");
        let days = (next_monday.at - now.timestamp()) / 86_400;
        assert!((6..=7).contains(&days), "应指向下周一的上午: {days} 天");
    }

    #[test]
    fn sunday_has_no_duplicate_monday_preset() {
        // 周日：「明天上午」已经是周一，不再额外给一条「周一上午」。
        let sunday = Local
            .with_ymd_and_hms(2026, 11, 22, 9, 0, 0)
            .single()
            .unwrap();
        let labels = labels(&later_presets_at(&sunday));
        assert!(!labels.iter().any(|l| l.starts_with("周一上午")));
        assert!(labels.iter().any(|l| l.starts_with("明天上午")));
    }

    #[test]
    fn friday_has_no_saturday_next_week_preset() {
        // 周五的「明天」就是周六，不该再给一条「周六上午」。
        let friday = Local
            .with_ymd_and_hms(2026, 11, 27, 9, 0, 0)
            .single()
            .unwrap();
        let labels = labels(&later_presets_at(&friday));
        assert!(!labels.iter().any(|l| l.starts_with("周六上午")));
    }

    #[test]
    fn parse_accepts_documented_format() {
        let now = monday_morning();
        let at = parse_schedule_input("2026-11-24 09:15", now).expect("应能解析");
        let dt = Local.timestamp_opt(at, 0).single().unwrap();
        assert_eq!(
            (dt.month(), dt.day(), dt.hour(), dt.minute()),
            (11, 24, 9, 15)
        );
        // 斜杠与全角冒号也接受（用户从别处粘贴的格式）。
        assert!(parse_schedule_input("2026/11/24 09：15", now).is_ok());
    }

    #[test]
    fn parse_rejects_past_and_garbage() {
        let now = monday_morning();
        assert!(parse_schedule_input("2026-11-22 09:00", now).is_err());
        assert!(parse_schedule_input("明天早上", now).is_err());
        assert!(parse_schedule_input("2026-02-30 08:00", now).is_err());
        assert!(parse_schedule_input("", now).is_err());
    }

    #[test]
    fn schedule_label_omits_year_within_same_year() {
        let now = Local::now();
        let at = now.timestamp() + 3_600;
        let label = schedule_label(at);
        assert!(label.contains(':'), "应包含时分: {label}");
        if now.year() == Local.timestamp_opt(at, 0).single().unwrap().year() {
            assert!(!label.contains('年'), "同年不该带年份: {label}");
        }
    }
}
