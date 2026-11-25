//! Mail merge (#447): CSV / contact-group table parsing and `{{Field}}`
//! placeholder substitution to generate personalized messages.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Letter {
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub reply_to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    To,
    Cc,
    Bcc,
    ReplyTo,
    Subject,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub name: String,
    pub places: Vec<Place>,
}

fn field_key(raw: &str) -> String {
    field_text(raw)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn field_text(raw: &str) -> String {
    let mut plain = String::with_capacity(raw.len());
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    plain
        .split(|c: char| c.is_whitespace() || c == '\u{a0}')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn placeholders(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(mut open) = text[from..].find("{{").map(|i| from + i) {
        while text[open + 2..].starts_with('{') {
            open += 1;
        }
        let inner = open + 2;
        let Some(close) = text[inner..].find("}}").map(|i| inner + i) else {
            break;
        };
        let raw = &text[inner..close];
        if let Some(again) = raw.rfind("{{") {
            from = inner + again;
            continue;
        }
        let key = field_key(raw);
        if !key.is_empty() && raw.len() <= 200 && !raw.contains('\n') {
            found.push((open..close + 2, key));
        }
        from = close + 2;
    }
    found
}

fn is_address(s: &str) -> bool {
    let s = s.trim();
    if let Some((local, domain)) = s.rsplit_once('@') {
        !local.is_empty() && domain.contains('.') && !s.contains(char::is_whitespace)
    } else {
        false
    }
}

impl Table {
    /// Parse a CSV/TSV file's bytes into a [`Table`] (UTF-8 BOM, UTF-16 LE/BE, or UTF-8).
    pub fn from_csv_bytes(bytes: &[u8]) -> Result<Self, String> {
        parse_csv(bytes)
    }

    pub fn column(&self, key: &str) -> Option<usize> {
        let key = field_key(key);
        self.columns.iter().position(|c| field_key(c) == key)
    }

    pub fn address_column(&self) -> Option<usize> {
        let counts =
            (0..self.columns.len()).map(|c| self.rows.iter().filter(|r| is_address(&r[c])).count());
        let (best, n) =
            counts.enumerate().fold(
                (None, 0),
                |(bi, bn), (i, n)| {
                    if n > bn {
                        (Some(i), n)
                    } else {
                        (bi, bn)
                    }
                },
            );
        best.filter(|_| n * 2 >= self.rows.len().max(1))
    }

    pub fn missing_fields(&self, letter: &Letter) -> Vec<Missing> {
        let mut missing: Vec<Missing> = Vec::new();
        for (place, part) in letter.parts() {
            for (range, key) in placeholders(part) {
                if self.column(&key).is_some() {
                    continue;
                }
                match missing.iter_mut().find(|m| field_key(&m.name) == key) {
                    Some(m) if !m.places.contains(&place) => m.places.push(place),
                    Some(_) => {}
                    None => missing.push(Missing {
                        name: field_text(&part[range.start + 2..range.end - 2]),
                        places: vec![place],
                    }),
                }
            }
        }
        missing
    }

    fn fill(&self, text: &str, row: usize, blank_missing: bool) -> String {
        let Some(values) = self.rows.get(row) else {
            return text.to_string();
        };
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (range, key) in placeholders(text) {
            let value = match self.column(&key) {
                Some(col) => values[col].trim(),
                None if blank_missing => "",
                None => continue,
            };
            out.push_str(&text[last..range.start]);
            out.push_str(value);
            last = range.end;
        }
        out.push_str(&text[last..]);
        out
    }

    pub fn letter_for(&self, letter: &Letter, row: usize, blank_missing: bool) -> Letter {
        let fill = |text: &str| self.fill(text, row, blank_missing);
        let mut to = fill(&letter.to);
        if to.trim().is_empty() {
            if let Some(col) = self.address_column() {
                if let Some(row_vals) = self.rows.get(row) {
                    to = row_vals[col].trim().to_string();
                }
            }
        }
        Letter {
            to,
            cc: fill(&letter.cc),
            bcc: fill(&letter.bcc),
            reply_to: fill(&letter.reply_to),
            subject: fill(&letter.subject),
            body: fill(&letter.body),
        }
    }

    /// Generate personalized letters for all rows in the table.
    pub fn generate_all(&self, template: &Letter, blank_missing: bool) -> Vec<Letter> {
        (0..self.rows.len())
            .map(|i| self.letter_for(template, i, blank_missing))
            .collect()
    }
}

impl Letter {
    fn parts(&self) -> [(Place, &str); 6] {
        [
            (Place::To, &self.to),
            (Place::Cc, &self.cc),
            (Place::Bcc, &self.bcc),
            (Place::ReplyTo, &self.reply_to),
            (Place::Subject, &self.subject),
            (Place::Body, &self.body),
        ]
    }
}

fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn delimiter(text: &str) -> char {
    let mut counts = [(',', 0), (';', 0), ('\t', 0)];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' | '\r' if !quoted => break,
            _ if !quoted => {
                if let Some(entry) = counts.iter_mut().find(|(d, _)| *d == c) {
                    entry.1 += 1;
                }
            }
            _ => {}
        }
    }
    counts
        .iter()
        .fold(
            (',', 0),
            |best, &(d, n)| if n > best.1 { (d, n) } else { best },
        )
        .0
}

fn records(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.trim().is_empty() => {
                field.clear();
                quoted = true;
            }
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

fn column_names(header: &[String]) -> Vec<String> {
    header
        .iter()
        .enumerate()
        .map(|(i, name)| match name.trim() {
            "" => format!("Column {}", i + 1),
            name => name.to_string(),
        })
        .collect()
}

pub fn parse_csv(bytes: &[u8]) -> Result<Table, String> {
    let text = decode(bytes);
    let mut rows = records(&text, delimiter(&text)).into_iter();
    let Some(header) = rows.next() else {
        return Err("CSV 文件为空。".to_string());
    };
    let columns = column_names(&header);
    let rows: Vec<Vec<String>> = rows
        .map(|mut r| {
            r.resize(columns.len(), String::new());
            r
        })
        .collect();
    if rows.is_empty() {
        return Err("CSV 文件除表头外没有数据行。".to_string());
    }
    Ok(Table { columns, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csv_and_generates_personalized_letters() {
        let csv = b"\xEF\xBB\xBFName,Email,Plan\n\"Alice, M.\",alice@example.com,Pro\nBob,bob@example.com,Free\n";
        let table = parse_csv(csv).unwrap();
        assert_eq!(table.columns, vec!["Name", "Email", "Plan"]);
        assert_eq!(table.address_column(), Some(1));

        let tpl = Letter {
            to: "{{email}}".into(),
            subject: "Welcome {{name}}".into(),
            body: "Hi {{name}}, your plan is {{plan}}.".into(),
            ..Default::default()
        };
        assert!(table.missing_fields(&tpl).is_empty());
        let out = table.generate_all(&tpl, false);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].to, "alice@example.com");
        assert_eq!(out[0].subject, "Welcome Alice, M.");
        assert_eq!(out[1].body, "Hi Bob, your plan is Free.");
    }
}
