//! Snooze & Send Later scheduler helpers (#527).

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnoozeChoice {
    LaterToday,
    Tomorrow,
    NextWeek,
    Until(i64),
    Wake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnoozeRecord {
    pub account_id: u32,
    pub message_id: u32,
    pub rfc_message_id: String,
    pub from_folder_path: String,
    pub snoozed_at_ts: i64,
    pub until_ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendLaterEntry {
    pub id: u32,
    pub account_id: u32,
    pub send_at_ts: i64,
}

pub fn snooze_presets(now_ts: i64) -> Vec<(&'static str, i64)> {
    let local_now = Local
        .timestamp_opt(now_ts, 0)
        .single()
        .unwrap_or_else(Local::now);
    let today = local_now.date_naive();

    let later_today = today
        .and_time(NaiveTime::from_hms_opt(18, 0, 0).unwrap())
        .and_local_timezone(Local)
        .earliest()
        .map(|dt| dt.timestamp())
        .filter(|&ts| ts > now_ts + 1800)
        .unwrap_or(now_ts + 3 * 3600);

    let tomorrow = (today + Duration::days(1))
        .and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap())
        .and_local_timezone(Local)
        .earliest()
        .map(|dt| dt.timestamp())
        .unwrap_or(now_ts + 24 * 3600);

    let days_until_monday = (7 - today.weekday().num_days_from_monday() as i64).max(1);
    let next_week = (today + Duration::days(days_until_monday))
        .and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap())
        .and_local_timezone(Local)
        .earliest()
        .map(|dt| dt.timestamp())
        .unwrap_or(now_ts + 7 * 24 * 3600);

    vec![
        ("今天晚些时候", later_today),
        ("明天上午 9:00", tomorrow),
        ("下周一上午 9:00", next_week),
    ]
}

pub fn woken_by_replies<'a>(
    asleep: &HashMap<&'a str, i64>,
    incoming: &[(&'a str, i64)],
) -> HashSet<String> {
    let mut woken = HashSet::new();
    for (refs, msg_ts) in incoming {
        for id in refs.split_whitespace() {
            if asleep
                .get(id)
                .is_some_and(|snoozed_at| *msg_ts > *snoozed_at)
            {
                woken.insert(id.to_string());
            }
        }
    }
    woken
}

pub fn due_snoozes(records: &[SnoozeRecord], now_ts: i64) -> Vec<SnoozeRecord> {
    records
        .iter()
        .filter(|r| r.until_ts > 0 && r.until_ts <= now_ts)
        .cloned()
        .collect()
}

pub fn due_send_later(entries: &[SendLaterEntry], now_ts: i64) -> Vec<SendLaterEntry> {
    entries
        .iter()
        .filter(|e| e.send_at_ts > 0 && e.send_at_ts <= now_ts)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_due_checks_work() {
        let now = 1_700_000_000;
        let presets = snooze_presets(now);
        assert_eq!(presets.len(), 3);
        assert!(presets.iter().all(|(_, ts)| *ts > now));

        let records = vec![
            SnoozeRecord {
                account_id: 1,
                message_id: 10,
                rfc_message_id: "<a@x>".into(),
                from_folder_path: "INBOX".into(),
                snoozed_at_ts: now - 100,
                until_ts: now - 1,
            },
            SnoozeRecord {
                account_id: 1,
                message_id: 11,
                rfc_message_id: "<b@x>".into(),
                from_folder_path: "INBOX".into(),
                snoozed_at_ts: now - 100,
                until_ts: now + 600,
            },
        ];
        let due = due_snoozes(&records, now);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message_id, 10);
    }

    #[test]
    fn wakes_snoozed_thread_on_newer_reply() {
        let mut asleep = HashMap::new();
        asleep.insert("<orig@x>", 1000);
        let woken = woken_by_replies(&asleep, &[("<orig@x> <other@x>", 1200)]);
        assert!(woken.contains("<orig@x>"));
    }
}
