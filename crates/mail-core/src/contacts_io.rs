//! F-5 联系人导入 / 导出的文件格式解析（vCard 3.0 与 CSV）。
//!
//! 支持范围（对齐原版「import vCard or CSV」的常用子集）：
//! - vCard：多联系人文件（按 `BEGIN:VCARD` 分卡）、折行（RFC 6350 续行以
//!   空格 / Tab 开头）展开、`FN` / `N` / `EMAIL` / `NOTE` 属性；`ENCODING=
//!   QUOTED-PRINTABLE` 做基础解码（=XX 十六进制 + 软换行 `=\r\n`），其余
//!   编码参数按原样读取。导出统一生成标准 3.0 vCard。
//! - CSV：首行表头识别（`email` / `e-mail` / `email address` / `email 1`、
//!   `name` / `full name` / `display name` / `given name`+`family name`、
//!   `notes` / `note`，模糊匹配忽略大小写与分隔符）；支持带引号字段与
//!   `""` 转义，自动嗅探逗号 / 分号 / 制表符分隔符。导出固定表头
//!   `Name,Email,Notes` 并对含分隔符 / 引号的值加引号。
//!
//! 邮箱为空的卡片 / 行导入时由调用方跳过。

use crate::UserContact;

/// 解析 vCard（.vcf）文本为联系人列表；无法解析的卡片跳过。
pub fn parse_vcards(text: &str) -> Vec<UserContact> {
    let unfolded = unfold_vcard_lines(text);
    let mut contacts = Vec::new();
    let mut display_name = String::new();
    let mut names: Vec<String> = Vec::new();
    let mut emails: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut phone = String::new();
    let mut organization = String::new();
    let mut job_title = String::new();
    let mut birthday = String::new();
    let mut groups: Vec<String> = Vec::new();
    let mut in_card = false;
    for line in unfolded {
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("BEGIN:VCARD") {
            in_card = true;
            display_name.clear();
            names.clear();
            emails.clear();
            notes.clear();
            phone.clear();
            organization.clear();
            job_title.clear();
            birthday.clear();
            groups.clear();
            continue;
        }
        if upper.starts_with("END:VCARD") {
            if in_card {
                contacts.push(build_contact(
                    &display_name,
                    &names,
                    &emails,
                    &notes,
                    &phone,
                    &organization,
                    &job_title,
                    &birthday,
                    &groups,
                ));
            }
            in_card = false;
            continue;
        }
        if !in_card {
            continue;
        }
        // 属性名（如 FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE）与值以首个 ':' 分隔。
        let Some((prop, value)) = split_property(&line) else {
            continue;
        };
        let base_prop = prop
            .split(';')
            .next()
            .unwrap_or(prop)
            .trim()
            .to_ascii_uppercase();
        let params_upper = prop.to_ascii_uppercase();
        let value = if params_upper.contains("QUOTED-PRINTABLE") {
            decode_quoted_printable(value)
        } else {
            value.to_string()
        };
        match base_prop.as_str() {
            "FN" => display_name = value.trim().to_string(),
            "N" => {
                // N:姓;名;中间名;前缀;后缀 —— 常用姓 + 名拼接。
                let parts: Vec<String> = value
                    .split(';')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect();
                names = parts;
            }
            "EMAIL" => {
                let email = value.trim().to_string();
                if !email.is_empty() {
                    emails.push(email);
                }
            }
            "NOTE" => {
                if !value.trim().is_empty() {
                    notes.push(value.trim().to_string());
                }
            }
            "TEL" => {
                if phone.is_empty() && !value.trim().is_empty() {
                    phone = value.trim().to_string();
                }
            }
            "ORG" => {
                let org = value.trim().trim_end_matches(';').replace(';', " / ");
                if !org.trim().is_empty() {
                    organization = org.trim().to_string();
                }
            }
            "TITLE" => {
                if !value.trim().is_empty() {
                    job_title = value.trim().to_string();
                }
            }
            "BDAY" => {
                if !value.trim().is_empty() {
                    birthday = value.trim().to_string();
                }
            }
            "CATEGORIES" => {
                for g in value.split(',') {
                    let g = g.trim();
                    if !g.is_empty() && !groups.iter().any(|existing| existing == g) {
                        groups.push(g.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    contacts
}

/// 解析 CSV 文本为联系人列表；无表头或表头无邮箱列时返回空列表。
pub fn parse_csv_contacts(text: &str) -> Vec<UserContact> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows = parse_csv_rows(text);
    if rows.len() < 2 {
        return Vec::new();
    }
    let header = rows.remove(0);
    let norm_header: Vec<String> = header
        .iter()
        .map(|h| {
            h.split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
                .replace("e mail", "email")
        })
        .collect();
    let email_col = norm_header
        .iter()
        .position(|h| {
            matches!(
                h.as_str(),
                "email" | "email address" | "email 1" | "primary email"
            )
        })
        .or_else(|| norm_header.iter().position(|h| h.contains("email")));
    let Some(email_col) = email_col else {
        return Vec::new();
    };
    let full_name_col = norm_header.iter().position(|h| {
        matches!(
            h.as_str(),
            "name" | "full name" | "display name" | "contact name"
        )
    });
    let given_col = norm_header
        .iter()
        .position(|h| h == "given name" || h == "first name");
    let family_col = norm_header
        .iter()
        .position(|h| h == "family name" || h == "last name" || h == "surname");
    let fallback_name_col = norm_header.iter().position(|h| h.contains("name"));
    let notes_col = norm_header
        .iter()
        .position(|h| h == "notes" || h == "note" || h == "comments" || h == "备注");
    let mut contacts = Vec::new();
    for row in rows {
        let email = row
            .get(email_col)
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        if !email.contains('@') {
            continue;
        }
        // 姓名取值优先级：整名列 > 给定名 + 姓分列拼接 > 任意含 name 的列。
        let name = if let Some(col) = full_name_col {
            row.get(col)
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        } else if let (Some(g), Some(f)) = (given_col, family_col) {
            let given = row.get(g).map(|v| v.trim()).unwrap_or_default();
            let family = row.get(f).map(|v| v.trim()).unwrap_or_default();
            match (family.is_empty(), given.is_empty()) {
                (false, false) => format!("{family} {given}"),
                (false, true) => family.to_string(),
                (true, false) => given.to_string(),
                (true, true) => String::new(),
            }
        } else {
            fallback_name_col
                .and_then(|col| row.get(col))
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        };
        let notes = notes_col
            .and_then(|col| row.get(col))
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        contacts.push(UserContact {
            id: 0,
            email,
            display_name: name,
            notes,
            ..UserContact::default()
        });
    }
    contacts
}

/// 导出为 vCard 3.0 文本（每行一个属性，行尾 \r\n）。
pub fn export_vcards(contacts: &[UserContact]) -> String {
    let mut out = String::new();
    for contact in contacts {
        let name = sanitize_vcard_value(contact.display_name.trim());
        let email = sanitize_vcard_value(contact.email.trim());
        let notes = sanitize_vcard_value(contact.notes.trim());
        out.push_str("BEGIN:VCARD\r\n");
        out.push_str("VERSION:3.0\r\n");
        if !name.is_empty() {
            out.push_str(&format!("FN:{name}\r\n"));
            // N 属性（姓;名;…）：单字段显示名拆出首词作为姓，剩余作名。
            let mut parts = name.splitn(2, ' ');
            let family = parts.next().unwrap_or_default();
            let given = parts.next().unwrap_or_default();
            out.push_str(&format!("N:{family};{given};;;\r\n"));
        }
        if !email.is_empty() {
            out.push_str("EMAIL;TYPE=INTERNET:");
            out.push_str(&email);
            out.push_str("\r\n");
        }
        if !contact.phone.trim().is_empty() {
            out.push_str("TEL;TYPE=WORK,VOICE:");
            out.push_str(&sanitize_vcard_value(contact.phone.trim()));
            out.push_str("\r\n");
        }
        if !contact.organization.trim().is_empty() {
            out.push_str("ORG:");
            out.push_str(&sanitize_vcard_value(contact.organization.trim()));
            out.push_str("\r\n");
        }
        if !contact.job_title.trim().is_empty() {
            out.push_str("TITLE:");
            out.push_str(&sanitize_vcard_value(contact.job_title.trim()));
            out.push_str("\r\n");
        }
        if !contact.birthday.trim().is_empty() {
            out.push_str("BDAY:");
            out.push_str(&sanitize_vcard_value(contact.birthday.trim()));
            out.push_str("\r\n");
        }
        if !contact.groups.is_empty() {
            let joined = contact
                .groups
                .iter()
                .map(|g| sanitize_vcard_value(g.trim()))
                .filter(|g| !g.is_empty())
                .collect::<Vec<_>>()
                .join(",");
            if !joined.is_empty() {
                out.push_str("CATEGORIES:");
                out.push_str(&joined);
                out.push_str("\r\n");
            }
        }
        if !notes.is_empty() {
            out.push_str("NOTE:");
            out.push_str(&notes);
            out.push_str("\r\n");
        }
        out.push_str("END:VCARD\r\n");
    }
    out
}

/// 导出为带表头 `Name,Email,Notes` 的 CSV 文本（值含分隔符 / 引号时加引号）。
pub fn export_csv(contacts: &[UserContact]) -> String {
    let mut out = String::from("Name,Email,Notes\r\n");
    for contact in contacts {
        out.push_str(&csv_field(contact.display_name.trim()));
        out.push(',');
        out.push_str(&csv_field(contact.email.trim()));
        out.push(',');
        out.push_str(&csv_field(contact.notes.trim()));
        out.push_str("\r\n");
    }
    out
}

/// 判断导入文件是否为 vCard（按扩展名或内容特征）。
pub fn looks_like_vcard(path: &std::path::Path, text: &str) -> bool {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("vcf"))
    {
        return true;
    }
    text.to_ascii_uppercase().contains("BEGIN:VCARD")
}

/// RFC 6350 / 2426 折行展开：以空格或 Tab 开头的续行拼回上一行。
/// QUOTED-PRINTABLE 软换行的行尾 `=` 在拼接时丢弃（常见编码器把 QP 软换行
/// 与 vCard 折行混写为 `=\r\n␠`，展开后残留的 `=` 并非转义）。
fn unfold_vcard_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.trim_end_matches('\r');
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            let last = lines.last_mut().unwrap();
            if last.ends_with('=') {
                last.pop();
            }
            let cont: String = raw[1..].trim_end().to_string();
            last.push_str(&cont);
        } else {
            lines.push(raw.trim_end().to_string());
        }
    }
    lines
}

/// 按 `PROP;PARAMS:VALUE` 拆分（值内允许出现冒号，只按第一个未转义冒号切）。
fn split_property(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(':')?;
    Some((&line[..idx], &line[idx + 1..]))
}

/// 基础 quoted-printable 解码：`=XX` 十六进制 + 软换行（`=` 后跟换行或折叠空白）。
fn decode_quoted_printable(value: &str) -> String {
    // 先移除常规软换行。
    let joined = value.replace("=\r\n", "").replace("=\n", "");
    let bytes = joined.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if i + 2 < bytes.len()
                && bytes[i + 1].is_ascii_hexdigit()
                && bytes[i + 2].is_ascii_hexdigit()
            {
                out.push(u8::from_str_radix(&joined[i + 1..i + 3], 16).unwrap_or(b'='));
                i += 3;
                continue;
            }
            // 折叠残留：unfold 后软换行的 `=` 紧跟折叠空白，跳过它们。
            if bytes.get(i + 1).is_some_and(|b| b.is_ascii_whitespace()) {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 组装一张卡片的联系人：无邮箱返回占位空记录（由调用方过滤）。
fn build_contact(
    display_name: &str,
    names: &[String],
    emails: &[String],
    notes: &[String],
    phone: &str,
    organization: &str,
    job_title: &str,
    birthday: &str,
    groups: &[String],
) -> UserContact {
    // FN 优先，其次 N 的「姓 名」拼接。
    let mut name = display_name.trim().to_string();
    if name.is_empty() {
        match names.len() {
            0 => {}
            1 => name = names[0].clone(),
            _ => name = format!("{} {}", names[0], names[1]),
        }
    }
    UserContact {
        id: 0,
        email: emails.first().cloned().unwrap_or_default(),
        display_name: name,
        notes: notes.join("\n"),
        phone: phone.to_string(),
        organization: organization.to_string(),
        job_title: job_title.to_string(),
        birthday: birthday.to_string(),
        groups: groups.to_vec(),
    }
}

/// 嗅探 CSV 分隔符（首行中出现次数最多者为分隔符）。
fn detect_delimiter(text: &str) -> char {
    let first_line = text.lines().next().unwrap_or_default();
    [',', ';', '\t']
        .into_iter()
        .max_by_key(|d| first_line.matches(*d).count())
        .unwrap_or(',')
}

/// 解析 CSV 全部行（支持引号字段与 `""` 转义）。
fn parse_csv_rows(text: &str) -> Vec<Vec<String>> {
    let delimiter = detect_delimiter(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            in_quotes = true;
        } else if c == delimiter {
            row.push(field.clone());
            field.clear();
        } else if c == '\n' || (c == '\r' && chars.peek() == Some(&'\n')) {
            if c == '\r' {
                chars.next();
            }
            row.push(field.clone());
            field.clear();
            rows.push(row.clone());
            row.clear();
        } else {
            field.push(c);
        }
    }
    // 收尾：最后一行无换行符时补一行（空行忽略）。
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// CSV 字段转义：含分隔符 / 引号 / 换行时加引号并把 `"` 翻倍。
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// vCard 值转义（RFC 6350 5.8.1）：反斜杠、换行、逗号、分号、冒号。
fn sanitize_vcard_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace(':', "\\:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vcards_reads_multiple_cards_with_folded_lines() {
        let text = "BEGIN:VCARD\r\nVERSION:2.1\r\nN:陈;思远;;;\r\nFN:陈思远\r\nEMAIL; INTERNET:siy\r\n uan@example.org\r\nEND:VCARD\r\nBEGIN:VCARD\r\nFN:Group\r\nEMAIL:x@y.example\r\nNOTE:first\r\nNOTE:second\r\nEND:VCARD\r\n";
        let contacts = parse_vcards(text);
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].display_name, "陈思远");
        assert_eq!(contacts[0].email, "siyuan@example.org");
        assert_eq!(contacts[1].email, "x@y.example");
        assert_eq!(contacts[1].notes, "first\nsecond");
    }

    #[test]
    fn parse_vcards_decodes_quoted_printable_utf8() {
        let text = "BEGIN:VCARD\r\nFN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=E6=9D=8E=E6=98=8E\r\nEMAIL;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:l=69@ex=\r\n ample.com\r\nEND:VCARD\r\n";
        let contacts = parse_vcards(text);
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name, "李明");
        assert_eq!(contacts[0].email, "li@example.com");
    }

    #[test]
    fn parse_vcards_uses_n_when_fn_missing_and_skips_empty_emails() {
        let text = "BEGIN:VCARD\r\nN:王;芳;;;\r\nEND:VCARD\r\n";
        let contacts = parse_vcards(text);
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name, "王 芳");
        assert!(contacts[0].email.is_empty());
    }

    #[test]
    fn parse_csv_contacts_matches_fuzzy_headers() {
        let text = "\"Full Name\",\"E-Mail Address\",Comments\r\n\"陈思远\",\"siyuan@example.org\",\"同学\"\r\n,,\r\n";
        let contacts = parse_csv_contacts(text);
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name, "陈思远");
        assert_eq!(contacts[0].email, "siyuan@example.org");
        assert_eq!(contacts[0].notes, "同学");
    }

    #[test]
    fn parse_csv_contacts_joins_given_and_family_columns() {
        let text = "First Name;Last Name;E-mail 1\r\n思远;陈;siyuan@example.org\r\n";
        let contacts = parse_csv_contacts(text);
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name, "陈 思远");
        assert_eq!(contacts[0].email, "siyuan@example.org");
    }

    #[test]
    fn parse_csv_contacts_requires_email_column() {
        let text = "Name,Phone\r\n张三,13800000000\r\n";
        assert!(parse_csv_contacts(text).is_empty());
        assert!(parse_csv_contacts("").is_empty());
    }

    #[test]
    fn export_vcards_produces_standard_30() {
        let text = export_vcards(&[UserContact {
            id: 1,
            email: "a@b.example".to_string(),
            display_name: "A; B".to_string(),
            notes: "line1\nline2".to_string(),
            phone: "+86-13800000000".to_string(),
            organization: "KiteMail".to_string(),
            job_title: "Engineer".to_string(),
            birthday: "1995-01-02".to_string(),
            groups: vec!["工作".to_string(), "研发".to_string()],
        }]);
        assert!(text.contains("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        assert!(text.contains("FN:A\\; B\r\n"));
        assert!(text.contains("EMAIL;TYPE=INTERNET:a@b.example\r\n"));
        assert!(text.contains("TEL;TYPE=WORK,VOICE:+86-13800000000\r\n"));
        assert!(text.contains("ORG:KiteMail\r\n"));
        assert!(text.contains("TITLE:Engineer\r\n"));
        assert!(text.contains("BDAY:1995-01-02\r\n"));
        assert!(text.contains("CATEGORIES:工作,研发\r\n"));
        assert!(text.contains("NOTE:line1\\nline2\r\n"));
        assert!(text.ends_with("END:VCARD\r\n"));
        let parsed = parse_vcards(&text);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].phone, "+86-13800000000");
        assert_eq!(parsed[0].organization, "KiteMail");
        assert_eq!(parsed[0].job_title, "Engineer");
        assert_eq!(parsed[0].birthday, "1995-01-02");
        assert_eq!(parsed[0].groups, vec!["工作", "研发"]);
    }

    #[test]
    fn export_csv_quotes_fields_with_delimiters() {
        let text = export_csv(&[UserContact {
            id: 1,
            email: "a@b.example".to_string(),
            display_name: "Doe, John".to_string(),
            notes: "said \"hi\"".to_string(),
            ..UserContact::default()
        }]);
        assert!(text.starts_with("Name,Email,Notes\r\n"));
        assert!(text.contains("\"Doe, John\""));
        assert!(text.contains("\"said \"\"hi\"\"\""));
    }

    #[test]
    fn looks_like_vcard_sniffs_content_and_extension() {
        let path = std::path::Path::new("contacts.vcf");
        assert!(looks_like_vcard(path, "nothing special"));
        let path = std::path::Path::new("contacts.txt");
        assert!(looks_like_vcard(path, "BEGIN:VCARD\nFN:x\nEND:VCARD"));
        assert!(!looks_like_vcard(path, "Name,Email\nx,y"));
    }
}
