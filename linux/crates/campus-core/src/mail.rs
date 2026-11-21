//! Pure Coremail mailbox parsing for inbox summaries, message details, and attachments.
use crate::{Error, Result, parse};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailAttachment {
    pub id: String,
    pub name: String,
    pub size_bytes: Option<u64>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailSummary {
    pub id: String,
    pub folder_id: i64,
    pub sender: String,
    pub subject: String,
    pub preview: String,
    pub sent_at: String,
    pub received_at: String,
    pub size_bytes: u64,
    pub is_read: bool,
    pub has_attachments: bool,
    pub recipients: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailMessage {
    pub id: String,
    pub folder_id: i64,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub sent_at: String,
    pub attachments: Vec<MailAttachment>,
}

fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
            .map(str::to_owned)
            .collect(),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                Vec::new()
            } else {
                vec![t.to_owned()]
            }
        }
        _ => Vec::new(),
    }
}

fn check_coremail_ok(body: &str) -> Result<Value> {
    let root: Value = serde_json::from_str(body).map_err(|_| Error::Malformed("mailbox JSON"))?;
    let Some(obj) = root.as_object() else {
        return Err(Error::Malformed("mailbox JSON object"));
    };
    if obj.get("code").and_then(|v| v.as_str()) != Some("S_OK") {
        return Err(Error::SessionExpired);
    }
    Ok(root)
}

/// Parses Coremail `mbox:listMessages` JSON response into inbox summaries.
pub fn parse_message_list(body: &str) -> Result<Vec<MailSummary>> {
    let root = check_coremail_ok(body)?;
    let items = root["var"]
        .as_array()
        .ok_or(Error::Malformed("mailbox list var"))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(obj) = item.as_object() else {
            return Err(Error::Malformed("mailbox list item"));
        };
        let Some(id) = obj
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let flags = obj.get("flags").and_then(|v| v.as_object());
        // Coremail omits `read` on unread messages and only sets `read: true` on read messages.
        let is_read = flags.and_then(|f| f.get("read")).and_then(|v| v.as_bool()) == Some(true);
        let has_attachments = flags
            .and_then(|f| f.get("attached"))
            .and_then(|v| v.as_bool())
            == Some(true)
            || flags
                .and_then(|f| f.get("inlineAttached"))
                .and_then(|v| v.as_bool())
                == Some(true);
        let from = obj.get("from").map(string_list).unwrap_or_default();
        let to = obj.get("to").map(string_list).unwrap_or_default();
        let party = if from.is_empty() { &to } else { &from };
        out.push(MailSummary {
            id: id.to_owned(),
            folder_id: obj.get("fid").and_then(|v| v.as_i64()).unwrap_or(0),
            sender: party.join(", "),
            subject: obj
                .get("subject")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned(),
            preview: obj
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned(),
            sent_at: obj
                .get("sentDate")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned(),
            received_at: obj
                .get("receivedDate")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned(),
            size_bytes: obj.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
            is_read,
            has_attachments,
            recipients: to,
        });
    }
    Ok(out)
}

/// Parses Coremail `readMessage.jsp` JSON response into a full message with attachments.
pub fn parse_message(body: &str, fallback_id: &str) -> Result<MailMessage> {
    let root = check_coremail_ok(body)?;
    let var = root["var"]
        .as_object()
        .ok_or(Error::Malformed("mailbox detail var"))?;
    let mail = var
        .get("mail")
        .and_then(|v| v.as_object())
        .ok_or(Error::Malformed("mailbox detail mail"))?;
    let info = var
        .get("mailInfo")
        .and_then(|v| v.as_object())
        .ok_or(Error::Malformed("mailbox detail mailInfo"))?;

    let id = info
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_id)
        .to_owned();
    let body_html = mail
        .get("mainPartData")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let body_text = parse::homework_text(body_html);

    let attachments = mail
        .get("attachments")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let id = obj
                .get("id")
                .or_else(|| obj.get("attachmentId"))
                .and_then(parse::scalar)
                .filter(|s| !s.trim().is_empty())?;
            let name = obj
                .get("filename")
                .or_else(|| obj.get("name"))
                .or_else(|| obj.get("fileName"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_owned();
            let size_bytes = obj
                .get("estimateSize")
                .or_else(|| obj.get("size"))
                .or_else(|| obj.get("sizeBytes"))
                .and_then(|v| v.as_u64())
                .or_else(|| {
                    obj.get("contentLength")
                        .and_then(|v| v.as_u64())
                        .map(|n| n / 4 * 3)
                });
            let content_type = obj
                .get("contentType")
                .or_else(|| obj.get("type"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.split(';').next())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            Some(MailAttachment {
                id,
                name,
                size_bytes,
                content_type,
            })
        })
        .collect();

    Ok(MailMessage {
        id,
        folder_id: info.get("fid").and_then(|v| v.as_i64()).unwrap_or(0),
        from: mail.get("from").map(string_list).unwrap_or_default(),
        to: mail.get("to").map(string_list).unwrap_or_default(),
        cc: mail.get("cc").map(string_list).unwrap_or_default(),
        bcc: mail.get("bcc").map(string_list).unwrap_or_default(),
        subject: mail
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned(),
        body_text,
        sent_at: info
            .get("sentDate")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned(),
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inbox_unread_and_xt5_attachments() {
        let list = r#"{
            "code":"S_OK",
            "total":2,
            "var":[
                {"id":"m1","fid":1,"from":"老师 <t@bjtu.edu.cn>","subject":"实验通知","summary":"请查收","sentDate":"2026-11-20 09:00:00","receivedDate":"2026-11-20 09:00:01","size":2048,"flags":{"attached":true}},
                {"id":"m2","fid":1,"from":["教务 <jw@bjtu.edu.cn>"],"subject":"已读通知","summary":"正文","sentDate":"2026-11-19 10:00:00","receivedDate":"2026-11-19 10:00:00","size":512,"flags":{"read":true}}
            ]
        }"#;
        let summaries = parse_message_list(list).unwrap();
        assert_eq!(summaries.len(), 2);
        assert!(!summaries[0].is_read);
        assert!(summaries[0].has_attachments);
        assert!(summaries[1].is_read);
        assert!(!summaries[1].has_attachments);

        let detail = r#"{
            "code":"S_OK",
            "var":{
                "mail":{
                    "from":["t@bjtu.edu.cn"],
                    "to":["s@bjtu.edu.cn"],
                    "subject":"实验通知",
                    "mainPartData":{"content":"<p>第一段</p><p>第二段</p>"},
                    "attachments":[
                        {"id":"3","filename":"实验模板.docx","contentType":"application/vnd.openxmlformats-officedocument.wordprocessingml.document; name=x","estimateSize":15693},
                        {"id":4,"filename":"名单.xlsx","contentLength":14502}
                    ]
                },
                "mailInfo":{"id":"m1","fid":1,"sentDate":"2026-11-20 09:00:00"}
            }
        }"#;
        let msg = parse_message(detail, "fallback").unwrap();
        assert_eq!(msg.id, "m1");
        assert!(msg.body_text.contains("第一段\n第二段"));
        assert_eq!(msg.attachments.len(), 2);
        assert_eq!(msg.attachments[0].id, "3");
        assert_eq!(msg.attachments[0].name, "实验模板.docx");
        assert_eq!(msg.attachments[1].id, "4");
        assert_eq!(msg.attachments[1].size_bytes, Some(14502 / 4 * 3));

        assert!(matches!(
            parse_message_list(r#"{"code":"FA_INVALID_SESSION"}"#),
            Err(Error::SessionExpired)
        ));
    }
}
