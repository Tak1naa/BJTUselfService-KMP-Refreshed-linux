//! Coremail client for BJTU webmail inbox, message detail, and attachment download.
use crate::transport::{SchoolTransport, same_origin};
use campus_core::mail::{MailMessage, MailSummary, parse_message, parse_message_list};
use campus_core::{Error, Result};
use serde_json::json;

#[derive(Clone, Copy)]
pub struct MailClient<'a> {
    pub transport: &'a SchoolTransport,
}

impl MailClient<'_> {
    /// Bootstraps Coremail session via `https://mis.bjtu.edu.cn/module/module/26/` and returns `sid`.
    pub async fn ensure_session(&self) -> Result<String> {
        let entry = self
            .transport
            .endpoints
            .mis
            .join("/module/module/26/")
            .unwrap();
        let reply = self.transport.mail_follow_entry(entry).await?;
        if !(200..300).contains(&reply.status) {
            return Err(Error::Network);
        }
        if !same_origin(&reply.url, &self.transport.endpoints.mail) {
            return Err(Error::SessionExpired);
        }
        reply
            .url
            .query_pairs()
            .find_map(|(k, v)| (k == "sid" && !v.trim().is_empty()).then(|| v.into_owned()))
            .ok_or(Error::SessionExpired)
    }

    pub async fn list_inbox(&self, sid: &str, limit: usize) -> Result<Vec<MailSummary>> {
        let mut url = self
            .transport
            .endpoints
            .mail
            .join("/coremail/s/json")
            .unwrap();
        url.query_pairs_mut()
            .append_pair("sid", sid)
            .append_pair("func", "mbox:listMessages");
        let payload = json!({
            "start": 0,
            "limit": limit,
            "mode": "count",
            "order": "date",
            "desc": true,
            "returnTotal": true,
            "returnTag": false,
            "summaryWindowSize": limit,
            "fid": 1,
            "mboxa": "",
            "topFirst": true,
        })
        .to_string();
        let referer = self
            .transport
            .endpoints
            .mail
            .join("/coremail/XT/index.jsp")
            .unwrap()
            .to_string();
        let reply = self
            .transport
            .mail_send(url, |client, target| {
                client
                    .post(target.clone())
                    .header("Accept", "text/x-json")
                    .header("Content-Type", "text/x-json; tz=\"Asia/Shanghai\"")
                    .header("X-Requested-With", "XMLHttpRequest")
                    .header("Referer", &referer)
                    .body(payload)
            })
            .await?;
        let text = reply.data(&self.transport.endpoints.mail)?;
        parse_message_list(&text)
    }

    pub async fn read_message(&self, sid: &str, message_id: &str) -> Result<MailMessage> {
        let _ = sid;
        let url = self
            .transport
            .endpoints
            .mail
            .join("/coremail/XT/jsp/readMessage.jsp")
            .unwrap();
        let referer = self
            .transport
            .endpoints
            .mail
            .join("/coremail/XT/index.jsp")
            .unwrap()
            .to_string();
        let reply = self
            .transport
            .mail_send(url, |client, target| {
                client
                    .post(target.clone())
                    .header("Accept", "text/x-json")
                    .header("X-Requested-With", "XMLHttpRequest")
                    .header("Referer", &referer)
                    .form(&[
                        ("mid", message_id),
                        ("mboxa", ""),
                        ("part", ""),
                        ("mailCipherPassword", ""),
                    ])
            })
            .await?;
        let text = reply.data(&self.transport.endpoints.mail)?;
        parse_message(&text, message_id)
    }

    pub async fn download_attachment(
        &self,
        sid: &str,
        message_id: &str,
        part_id: &str,
    ) -> Result<Vec<u8>> {
        let mut url = self
            .transport
            .endpoints
            .mail
            .join("/coremail/mbox-data")
            .unwrap();
        url.query_pairs_mut()
            .append_pair("mode", "download")
            .append_pair("sid", sid)
            .append_pair("mid", message_id)
            .append_pair("part", part_id);
        self.transport.mail_get_attachment(url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Endpoints;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    #[tokio::test]
    async fn bootstraps_coremail_reads_message_and_downloads_attachment() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/module/module/26/"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "/coremail/XT/index.jsp?sid=sid-abc"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/coremail/XT/index.jsp"))
            .and(query_param("sid", "sid-abc"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>ok</html>"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/coremail/s/json"))
            .and(query_param("func", "mbox:listMessages"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"code":"S_OK","total":1,"var":[{"id":"m:1","fid":1,"from":"t@bjtu.edu.cn","subject":"测试邮件","summary":"摘要","sentDate":"2026-11-20 09:00:00","receivedDate":"2026-11-20 09:00:00","size":128,"flags":{"attached":true}}]}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/coremail/XT/jsp/readMessage.jsp"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"code":"S_OK","var":{"mail":{"from":["t@bjtu.edu.cn"],"to":["s@bjtu.edu.cn"],"subject":"测试邮件","mainPartData":{"content":"<p>正文内容</p>"},"attachments":[{"id":"3","filename":"附件.pdf","estimateSize":4}]},"mailInfo":{"id":"m:1","fid":1,"sentDate":"2026-11-20 09:00:00"}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/coremail/mbox-data"))
            .and(query_param("mid", "m:1"))
            .and(query_param("part", "3"))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                "/coremail/mbox-data/%E9%99%84%E4%BB%B6.pdf?mode=download&mid=m:1&part=3",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/coremail/mbox-data/%E9%99%84%E4%BB%B6.pdf"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/pdf")
                    .insert_header("Content-Disposition", "attachment; filename=\"附件.pdf\"")
                    .set_body_bytes(b"%PDF".as_slice()),
            )
            .mount(&server)
            .await;

        let transport = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
        let client = MailClient {
            transport: &transport,
        };
        let sid = client.ensure_session().await.unwrap();
        assert_eq!(sid, "sid-abc");
        let list = client.list_inbox(&sid, 20).await.unwrap();
        assert_eq!(list.len(), 1);
        let detail = client.read_message(&sid, "m:1").await.unwrap();
        assert_eq!(detail.attachments[0].name, "附件.pdf");
        let bytes = client.download_attachment(&sid, "m:1", "3").await.unwrap();
        assert_eq!(bytes, b"%PDF");

        // HTML landing outside mbox-data indicates session expiration
        Mock::given(method("GET"))
            .and(path("/coremail/mbox-data"))
            .and(query_param("mid", "expired"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "/coremail/index.jsp?cus=1"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/coremail/index.jsp"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/html")
                    .set_body_string("<html>login</html>"),
            )
            .mount(&server)
            .await;
        assert!(matches!(
            client.download_attachment(&sid, "expired", "1").await,
            Err(Error::SessionExpired)
        ));
    }
}
