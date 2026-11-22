//! ICS calendar invitation parser (RFC 5545) and `METHOD:REPLY` generator
//! (RFC 5546) using `chrono` (pure Rust, no GLib/GTK dependency).

use chrono::{Datelike, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

const MAX_ICS: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitePerson {
    pub name: String,
    pub email: String,
    pub status: String,
    pub rsvp: bool,
    pub resource: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub method: String,
    pub uid: String,
    pub sequence: i64,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub recurrence_id: String,
    pub repeats: String,
    pub organizer: Option<InvitePerson>,
    pub attendees: Vec<InvitePerson>,
    pub cancelled: bool,
    pub ics: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteReplyAction {
    Accept,
    Tentative,
    Decline,
}

impl InviteReplyAction {
    pub fn partstat(self) -> &'static str {
        match self {
            Self::Accept => "ACCEPTED",
            Self::Tentative => "TENTATIVE",
            Self::Decline => "DECLINED",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Accept => "接受",
            Self::Tentative => "暂定",
            Self::Decline => "拒绝",
        }
    }
}

/// Detect a calendar invitation from raw RFC 822 message bytes.
pub fn detect_raw(raw: &[u8]) -> Option<Invite> {
    let parsed = mail_parser::MessageParser::default().parse(raw)?;
    detect(&parsed)
}

/// Detect a calendar invitation from a parsed message.
pub fn detect(parsed: &mail_parser::Message) -> Option<Invite> {
    use mail_parser::MimeHeaders;
    for part in parsed.parts.iter() {
        let calendar = part.content_type().is_some_and(|c| {
            c.ctype().eq_ignore_ascii_case("text")
                && c.subtype()
                    .is_some_and(|s| s.eq_ignore_ascii_case("calendar"))
        }) || part
            .attachment_name()
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".ics"));
        if !calendar || part.contents().len() > MAX_ICS {
            continue;
        }
        let text = match part.text_contents() {
            Some(t) => t.to_string(),
            None => String::from_utf8_lossy(part.contents()).to_string(),
        };
        if let Some(mut invite) = parse(&text) {
            if invite.method.is_empty() {
                invite.method = part
                    .content_type()
                    .and_then(|c| c.attribute("method"))
                    .unwrap_or("PUBLISH")
                    .to_ascii_uppercase();
            }
            return Some(invite);
        }
    }
    None
}

/// Parse an iCalendar document into an [`Invite`].
pub fn parse(ics: &str) -> Option<Invite> {
    let lines = unfold(ics);
    let mut method = String::new();
    let mut zones: Vec<TimeZoneSpec> = Vec::new();
    let mut event: Option<EventProps> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut current: Option<EventProps> = None;
    let mut zone: Option<TimeZoneSpec> = None;
    let mut observance: Option<Observance> = None;

    for line in &lines {
        let name = line.name.to_ascii_uppercase();
        match name.as_str() {
            "BEGIN" => {
                let component = line.value.to_ascii_uppercase();
                match component.as_str() {
                    "VEVENT" if event.is_none() && !stack.iter().any(|c| c == "VTIMEZONE") => {
                        current = Some(EventProps::default());
                    }
                    "VTIMEZONE" => zone = Some(TimeZoneSpec::default()),
                    "STANDARD" | "DAYLIGHT" => observance = Some(Observance::default()),
                    _ => {}
                }
                stack.push(component);
                continue;
            }
            "END" => {
                let component = line.value.to_ascii_uppercase();
                match component.as_str() {
                    "VEVENT" => {
                        if let Some(props) = current.take() {
                            event = Some(props);
                        }
                    }
                    "VTIMEZONE" => {
                        if let Some(z) = zone.take() {
                            zones.push(z);
                        }
                    }
                    "STANDARD" | "DAYLIGHT" => {
                        if let (Some(o), Some(z)) = (observance.take(), zone.as_mut()) {
                            z.observances.push(o);
                        }
                    }
                    _ => {}
                }
                stack.pop();
                continue;
            }
            "METHOD" if stack.last().is_some_and(|c| c == "VCALENDAR") => {
                method = line.value.to_ascii_uppercase();
                continue;
            }
            _ => {}
        }
        if stack.last().is_some_and(|c| c == "VALARM") {
            continue;
        }
        if let Some(o) = observance.as_mut() {
            o.take(line);
            continue;
        }
        if let Some(z) = zone.as_mut() {
            if name == "TZID" {
                z.id = line.value.clone();
            }
            continue;
        }
        if let Some(props) = current.as_mut() {
            props.take(line);
        }
    }

    let props = event?;
    let start = props.start.as_ref().and_then(|s| s.resolve(&zones));
    let end = props
        .end
        .as_ref()
        .and_then(|e| e.resolve(&zones))
        .or_else(|| Some(start? + props.duration?))
        .or(start);

    Some(Invite {
        method,
        uid: props.uid,
        sequence: props.sequence,
        summary: props.summary,
        location: props.location,
        start: start.unwrap_or_default(),
        end: end.unwrap_or_default(),
        all_day: props.start.as_ref().is_some_and(|s| s.date_only),
        recurrence_id: props.recurrence_id,
        repeats: props.rrule,
        organizer: props.organizer,
        attendees: props.attendees,
        cancelled: props.status.eq_ignore_ascii_case("CANCELLED"),
        ics: ics.to_string(),
    })
}

/// Build an RFC 5546 `METHOD:REPLY` iCalendar payload for the user's answer.
pub fn reply_ics(invite: &Invite, name: &str, email: &str, status: &str) -> String {
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&fold(&text));
        out.push_str("\r\n");
    };
    line("BEGIN:VCALENDAR".into());
    line("PRODID:-//Hyprlab//Hylki//EN".into());
    line("VERSION:2.0".into());
    line("METHOD:REPLY".into());
    line("BEGIN:VEVENT".into());
    line(format!("UID:{}", escape_ics(&invite.uid)));
    line(format!("SEQUENCE:{}", invite.sequence));
    line(format!("DTSTAMP:{}", Utc::now().format("%Y%m%dT%H%M%SZ")));
    if !invite.recurrence_id.is_empty() {
        line(format!("RECURRENCE-ID:{}", invite.recurrence_id));
    }
    if !invite.summary.is_empty() {
        line(format!("SUMMARY:{}", escape_ics(&invite.summary)));
    }
    if let Some(o) = &invite.organizer {
        line(format!("ORGANIZER{}:mailto:{}", cn(&o.name), o.email));
    }
    line(format!(
        "ATTENDEE{}PARTSTAT={status}:mailto:{email}",
        match cn(name).as_str() {
            "" => ";".to_string(),
            c => format!("{c};"),
        }
    ));
    line("END:VEVENT".into());
    line("END:VCALENDAR".into());
    out
}

fn cn(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return String::new();
    }
    format!(";CN=\"{}\"", name.replace(['"', '\\'], ""))
}

fn escape_ics(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\r', "")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

fn unescape_ics(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70);
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if width + n > 73 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out
}

#[derive(Debug, Default)]
struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
    raw: String,
}

impl Line {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn unfold(ics: &str) -> Vec<Line> {
    let mut joined: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match line.strip_prefix([' ', '\t']) {
            Some(rest) => {
                if let Some(last) = joined.last_mut() {
                    last.push_str(rest);
                    continue;
                }
                joined.push(rest.to_string());
            }
            None => joined.push(line.to_string()),
        }
    }
    joined.iter().filter_map(|l| split_line(l)).collect()
}

fn split_line(line: &str) -> Option<Line> {
    if line.trim().is_empty() {
        return None;
    }
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let (head, value) = match colon {
        Some(i) => (&line[..i], &line[i + 1..]),
        None => (line, ""),
    };
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            let val = v.trim();
            let unq = val
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(val);
            Some((k.trim().to_string(), unq.to_string()))
        })
        .collect();
    Some(Line {
        name,
        params,
        value: unescape_ics(value),
        raw: value.to_string(),
    })
}

fn split_unquoted(text: &str, sep: char) -> Vec<&str> {
    let mut quoted = false;
    let mut bounds = vec![0usize];
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c == sep && !quoted => bounds.push(i),
            _ => {}
        }
    }
    bounds.push(text.len());
    bounds
        .windows(2)
        .map(|w| {
            let piece = &text[w[0]..w[1]];
            piece.strip_prefix(sep).unwrap_or(piece)
        })
        .collect()
}

#[derive(Debug, Default)]
struct EventProps {
    uid: String,
    sequence: i64,
    summary: String,
    location: String,
    status: String,
    recurrence_id: String,
    rrule: String,
    start: Option<Stamp>,
    end: Option<Stamp>,
    duration: Option<i64>,
    organizer: Option<InvitePerson>,
    attendees: Vec<InvitePerson>,
}

impl EventProps {
    fn take(&mut self, line: &Line) {
        match line.name.to_ascii_uppercase().as_str() {
            "UID" => self.uid = line.value.clone(),
            "SEQUENCE" => self.sequence = line.value.trim().parse().unwrap_or_default(),
            "SUMMARY" => self.summary = line.value.trim().to_string(),
            "LOCATION" => self.location = line.value.trim().to_string(),
            "STATUS" => self.status = line.value.trim().to_string(),
            "RECURRENCE-ID" => self.recurrence_id = line.raw.trim().to_string(),
            "RRULE" => self.rrule = line.raw.trim().to_ascii_uppercase(),
            "DTSTART" => self.start = Stamp::read(line),
            "DTEND" => self.end = Stamp::read(line),
            "DURATION" => self.duration = duration_seconds(line.raw.trim()),
            "ORGANIZER" => self.organizer = person(line),
            "ATTENDEE" => {
                if let Some(p) = person(line) {
                    self.attendees.push(p);
                }
            }
            _ => {}
        }
    }
}

fn person(line: &Line) -> Option<InvitePerson> {
    let email = line
        .raw
        .trim()
        .strip_prefix("mailto:")
        .or_else(|| line.raw.trim().strip_prefix("MAILTO:"))
        .unwrap_or(line.raw.trim())
        .trim()
        .to_string();
    let name = line.param("CN").unwrap_or_default().trim().to_string();
    if email.is_empty() && name.is_empty() {
        return None;
    }
    Some(InvitePerson {
        name: if name.eq_ignore_ascii_case(&email) {
            String::new()
        } else {
            name
        },
        email,
        status: line
            .param("PARTSTAT")
            .unwrap_or("NEEDS-ACTION")
            .to_ascii_uppercase(),
        rsvp: line
            .param("RSVP")
            .is_some_and(|v| v.eq_ignore_ascii_case("TRUE")),
        resource: line
            .param("CUTYPE")
            .is_some_and(|v| v.eq_ignore_ascii_case("RESOURCE") || v.eq_ignore_ascii_case("ROOM")),
    })
}

#[derive(Debug)]
struct Stamp {
    y: i32,
    mo: i32,
    d: i32,
    h: i32,
    mi: i32,
    s: i32,
    utc: bool,
    date_only: bool,
    tzid: String,
}

impl Stamp {
    fn read(line: &Line) -> Option<Stamp> {
        let mut stamp = Stamp::parse(line.raw.trim())?;
        stamp.tzid = line.param("TZID").unwrap_or_default().trim().to_string();
        if line
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        {
            stamp.date_only = true;
        }
        Some(stamp)
    }

    fn parse(text: &str) -> Option<Stamp> {
        let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 8 {
            return None;
        }
        let num = |a: usize, b: usize| digits.get(a..b)?.parse::<i32>().ok();
        let mut stamp = Stamp {
            y: num(0, 4)?,
            mo: num(4, 6)?,
            d: num(6, 8)?,
            h: 0,
            mi: 0,
            s: 0,
            utc: text.ends_with('Z') || text.ends_with('z'),
            date_only: digits.len() == 8,
            tzid: String::new(),
        };
        if digits.len() >= 14 {
            stamp.h = num(8, 10)?;
            stamp.mi = num(10, 12)?;
            stamp.s = num(12, 14)?;
        }
        Some(stamp)
    }

    fn resolve(&self, zones: &[TimeZoneSpec]) -> Option<i64> {
        let date = NaiveDate::from_ymd_opt(self.y, self.mo as u32, self.d as u32)?;
        let naive = date.and_hms_opt(self.h as u32, self.mi as u32, self.s as u32)?;
        if self.utc {
            return Some(Utc.from_utc_datetime(&naive).timestamp());
        }
        if !self.tzid.is_empty() {
            if let Some(spec) = zones.iter().find(|z| z.id.eq_ignore_ascii_case(&self.tzid)) {
                if let Some(offset_secs) = spec.offset_at(self) {
                    if let Some(offset) = FixedOffset::east_opt(offset_secs) {
                        return offset
                            .from_local_datetime(&naive)
                            .earliest()
                            .map(|dt| dt.timestamp());
                    }
                }
            }
        }
        Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.timestamp())
    }
}

fn duration_seconds(text: &str) -> Option<i64> {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, text.strip_prefix('+').unwrap_or(text)),
    };
    let rest = rest.strip_prefix('P')?;
    let mut total: i64 = 0;
    let mut num = String::new();
    let mut in_time = false;
    for c in rest.chars() {
        if c == 'T' {
            in_time = true;
            continue;
        }
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: i64 = num.parse().ok()?;
        num.clear();
        let unit = match (c, in_time) {
            ('W', false) => 7 * 86400,
            ('D', false) => 86400,
            ('H', true) => 3600,
            ('M', true) => 60,
            ('S', true) => 1,
            _ => return None,
        };
        total += n * unit;
    }
    Some(sign * total)
}

#[derive(Debug, Default)]
struct TimeZoneSpec {
    id: String,
    observances: Vec<Observance>,
}

#[derive(Debug, Default)]
struct Observance {
    offset_to: i32,
    month: i32,
    day: i32,
    weekday: i32,
    ordinal: i32,
    at: i32,
}

impl Observance {
    fn take(&mut self, line: &Line) {
        match line.name.to_ascii_uppercase().as_str() {
            "TZOFFSETTO" => self.offset_to = utc_offset(line.raw.trim()).unwrap_or_default(),
            "DTSTART" => {
                if let Some(s) = Stamp::parse(line.raw.trim()) {
                    self.month = s.mo;
                    self.day = s.d;
                    self.at = s.h * 3600 + s.mi * 60 + s.s;
                }
            }
            "RRULE" => {
                for part in line.raw.split(';') {
                    let Some((k, v)) = part.split_once('=') else {
                        continue;
                    };
                    match k.to_ascii_uppercase().as_str() {
                        "BYMONTH" => self.month = v.trim().parse().unwrap_or(self.month),
                        "BYMONTHDAY" => self.day = v.trim().parse().unwrap_or(self.day),
                        "BYDAY" => {
                            let v = v.trim().to_ascii_uppercase();
                            let (num, day) = v.split_at(v.len().saturating_sub(2));
                            self.ordinal = num.parse().unwrap_or(1);
                            self.weekday = match day {
                                "SU" => 7,
                                "MO" => 1,
                                "TU" => 2,
                                "WE" => 3,
                                "TH" => 4,
                                "FR" => 5,
                                "SA" => 6,
                                _ => 0,
                            };
                            self.day = 0;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn day_in(&self, year: i32) -> Option<i32> {
        if self.day > 0 {
            return Some(self.day);
        }
        if self.weekday == 0 {
            return None;
        }
        let last = days_in_month(year, self.month);
        if self.ordinal < 0 {
            let mut found = 0;
            let mut day = last;
            while day >= 1 {
                if weekday_of(year, self.month, day) == self.weekday {
                    found -= 1;
                    if found == self.ordinal {
                        return Some(day);
                    }
                }
                day -= 1;
            }
            return None;
        }
        let mut found = 0;
        for day in 1..=last {
            if weekday_of(year, self.month, day) == self.weekday {
                found += 1;
                if found == self.ordinal.max(1) {
                    return Some(day);
                }
            }
        }
        None
    }
}

impl TimeZoneSpec {
    fn offset_at(&self, when: &Stamp) -> Option<i32> {
        if self.observances.len() == 1 {
            return Some(self.observances[0].offset_to);
        }
        let key =
            |mo: i32, d: i32, at: i32| (mo as i64) * 100_000_000 + (d as i64) * 100_000 + at as i64;
        let reading = key(when.mo, when.d, when.h * 3600 + when.mi * 60 + when.s);
        let mut changes: Vec<(i64, i32)> = self
            .observances
            .iter()
            .filter_map(|o| Some((key(o.month, o.day_in(when.y)?, o.at), o.offset_to)))
            .collect();
        if changes.is_empty() {
            return self.observances.first().map(|o| o.offset_to);
        }
        changes.sort_by_key(|(k, _)| *k);
        let in_force = changes
            .iter()
            .rev()
            .find(|(k, _)| *k <= reading)
            .or_else(|| changes.last())?;
        Some(in_force.1)
    }
}

fn utc_offset(text: &str) -> Option<i32> {
    let (sign, rest) = match text.chars().next()? {
        '-' => (-1, &text[1..]),
        '+' => (1, &text[1..]),
        _ => (1, text),
    };
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let h: i32 = digits.get(0..2)?.parse().ok()?;
    let m: i32 = digits.get(2..4)?.parse().ok()?;
    let s: i32 = digits.get(4..6).and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + s))
}

fn days_in_month(year: i32, month: i32) -> i32 {
    let (y, m) = if month >= 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(y, m as u32, 1)
        .and_then(|first| first.pred_opt())
        .map_or(30, |last| last.day() as i32)
}

fn weekday_of(year: i32, month: i32, day: i32) -> i32 {
    NaiveDate::from_ymd_opt(year, month as u32, day as u32)
        .map_or(0, |d| d.weekday().number_from_monday() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_invite_and_generates_reply() {
        let ics =
            "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:abc-123\r\nSEQUENCE:2\r\n\
            SUMMARY:Architecture Sync\r\nLOCATION:Room 4B\r\n\
            DTSTART:20261102T080000Z\r\nDTEND:20261102T090000Z\r\n\
            ORGANIZER;CN=\"Alice\":mailto:alice@example.org\r\n\
            ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:bob@example.com\r\n\
            END:VEVENT\r\nEND:VCALENDAR\r\n";
        let inv = parse(ics).unwrap();
        assert_eq!(inv.method, "REQUEST");
        assert_eq!(inv.uid, "abc-123");
        assert_eq!(inv.summary, "Architecture Sync");
        assert_eq!(inv.location, "Room 4B");
        assert_eq!(inv.end - inv.start, 3600);
        assert_eq!(inv.organizer.as_ref().unwrap().email, "alice@example.org");

        let reply = reply_ics(&inv, "Bob", "bob@example.com", "ACCEPTED");
        assert!(reply.contains("METHOD:REPLY"));
        assert!(reply.contains("PARTSTAT=ACCEPTED:mailto:bob@example.com"));
        assert!(reply.contains("SEQUENCE:2"));
    }
}
