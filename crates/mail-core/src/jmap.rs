//! JMAP client (RFC 8620 / RFC 8621) over HTTPS (`ureq` + `serde_json`).

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmapSession {
    pub api_url: String,
    pub download_url: String,
    pub upload_url: String,
    pub account_id: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JmapMailbox {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub total_emails: u64,
    pub unread_emails: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JmapEmailSummary {
    pub id: String,
    pub blob_id: String,
    pub thread_id: String,
    pub subject: String,
    pub from: String,
    pub received_at: String,
    pub preview: String,
    pub unread: bool,
    pub flagged: bool,
}

pub fn discover_session(base_url: &str, bearer_or_basic_auth: &str) -> Result<JmapSession, String> {
    let url = format!("{}/.well-known/jmap", base_url.trim_end_matches('/'));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let resp = agent
        .get(&url)
        .set("Authorization", bearer_or_basic_auth)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| format!("JMAP 会话发现失败: {e}"))?;
    let body: Value = resp
        .into_json()
        .map_err(|e| format!("JMAP 会话 JSON 解析失败: {e}"))?;
    parse_session(&body)
}

pub fn parse_session(body: &Value) -> Result<JmapSession, String> {
    let api_url = body
        .get("apiUrl")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "JMAP 会话缺少 apiUrl".to_string())?
        .to_string();
    let download_url = body
        .get("downloadUrl")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let upload_url = body
        .get("uploadUrl")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let primary_mail = body
        .pointer("/primaryAccounts/urn:ietf:params:jmap:mail")
        .and_then(|v| v.as_str())
        .or_else(|| {
            body.get("accounts")
                .and_then(|a| a.as_object())
                .and_then(|m| m.keys().next().map(String::as_str))
        })
        .ok_or_else(|| "JMAP 会话缺少主邮件账号".to_string())?
        .to_string();
    let capabilities = body
        .get("capabilities")
        .and_then(|c| c.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    Ok(JmapSession {
        api_url,
        download_url,
        upload_url,
        account_id: primary_mail,
        capabilities,
    })
}

pub fn jmap_call(
    session: &JmapSession,
    auth_header: &str,
    method_calls: Vec<Value>,
) -> Result<Value, String> {
    let payload = json!({
        "using": [
            "urn:ietf:params:jmap:core",
            "urn:ietf:params:jmap:mail",
            "urn:ietf:params:jmap:submission"
        ],
        "methodCalls": method_calls,
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let resp = agent
        .post(&session.api_url)
        .set("Authorization", auth_header)
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|e| format!("JMAP RPC 请求失败: {e}"))?;
    resp.into_json()
        .map_err(|e| format!("JMAP RPC 响应解析失败: {e}"))
}

pub fn parse_mailboxes(response: &Value) -> Vec<JmapMailbox> {
    let Some(list) = response
        .pointer("/methodResponses/0/1/list")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            Some(JmapMailbox {
                id: item.get("id")?.as_str()?.to_string(),
                name: item.get("name")?.as_str()?.to_string(),
                role: item
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                total_emails: item
                    .get("totalEmails")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                unread_emails: item
                    .get("unreadEmails")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            })
        })
        .collect()
}

pub fn get_mailboxes(session: &JmapSession, auth_header: &str) -> Result<Vec<JmapMailbox>, String> {
    let resp = jmap_call(
        session,
        auth_header,
        vec![json!(["Mailbox/get", { "accountId": session.account_id }, "m0"])],
    )?;
    Ok(parse_mailboxes(&resp))
}

pub fn parse_keywords(keywords: &HashMap<String, bool>) -> (bool, bool) {
    let seen = keywords.get("$seen").copied().unwrap_or(false);
    let flagged = keywords.get("$flagged").copied().unwrap_or(false);
    (!seen, flagged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_and_mailboxes() {
        let sess_json = json!({
            "apiUrl": "https://mail.example/jmap/api/",
            "downloadUrl": "https://mail.example/jmap/download/{accountId}/{blobId}/{name}",
            "uploadUrl": "https://mail.example/jmap/upload/{accountId}/",
            "primaryAccounts": {
                "urn:ietf:params:jmap:mail": "u123"
            },
            "capabilities": {
                "urn:ietf:params:jmap:core": {},
                "urn:ietf:params:jmap:mail": {}
            }
        });
        let sess = parse_session(&sess_json).unwrap();
        assert_eq!(sess.account_id, "u123");
        assert_eq!(sess.capabilities.len(), 2);

        let mbox_resp = json!({
            "methodResponses": [[
                "Mailbox/get",
                {
                    "list": [{
                        "id": "mb-inbox",
                        "name": "Inbox",
                        "role": "inbox",
                        "totalEmails": 42,
                        "unreadEmails": 3
                    }]
                },
                "m0"
            ]]
        });
        let boxes = parse_mailboxes(&mbox_resp);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].role.as_deref(), Some("inbox"));
        assert_eq!(boxes[0].unread_emails, 3);
    }
}
