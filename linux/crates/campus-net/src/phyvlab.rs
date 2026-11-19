//! 物理在线（Moodle）会话认证、抓取、附件下载、作业上传与本地回环验证码。
use crate::transport::{Reply, SchoolTransport};
use campus_core::{
    Error, Result,
    phyvlab::{
        self, PhyVlabActivity, PhyVlabAssignmentDetail, PhyVlabAttachment, PhyVlabCaptchaProof,
        PhyVlabCourse, PhyVlabEditContext, PhyVlabFinalizationForm,
    },
};
use futures_util::{StreamExt, stream};
use reqwest::Method;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;

fn phy_debug(msg: &str) {
    if std::env::var_os("CAMPUS_PHYVLAB_DEBUG").is_some() {
        eprintln!("[phyvlab-debug] {msg}");
    }
}

pub enum SessionOutcome {
    Ready,
    AccountRequired,
    CaptchaRequired { captcha_id: String },
    CaptchaRejected,
    CredentialsRejected,
}

pub struct UploadFile<'a> {
    pub file_name: &'a str,
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

pub struct PhyVlabClient<'a> {
    pub transport: &'a SchoolTransport,
}

impl<'a> PhyVlabClient<'a> {
    pub fn origin(&self) -> String {
        self.transport
            .endpoints
            .phyvlab
            .as_str()
            .trim_end_matches('/')
            .to_owned()
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.transport
            .endpoints
            .phyvlab
            .join(path.trim_start_matches('/'))
            .map_err(|_| Error::Malformed("phyvlab url"))
    }

    fn has_session_cookie(&self) -> bool {
        let Ok(courses_url) = self.url("/my/courses.php") else {
            return false;
        };
        self.transport.has_cookie(&courses_url, "MoodleSession")
    }

    async fn get_no_redirect(&self, url: Url, referer: &str) -> Result<Reply> {
        let referer = referer.to_owned();
        let path = url.path().to_owned();
        let res = self
            .transport
            .phyvlab_send(url.clone(), |client, u| {
                client
                    .request(Method::GET, u.clone())
                    .header("Accept", "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8")
                    .header("Referer", referer)
            })
            .await;
        match &res {
            Ok(r) => phy_debug(&format!(
                "GET {path} -> {} loc={:?} bytes={}",
                r.status,
                r.location
                    .as_deref()
                    .map(|l| l.split('?').next().unwrap_or(l)),
                r.bytes.len()
            )),
            Err(e) => phy_debug(&format!("GET {path} -> err={e:?}")),
        }
        res
    }

    async fn follow(&self, mut url: Url, referer: &str) -> Result<Reply> {
        let origin = self.origin();
        let mut current_referer = referer.to_owned();
        for _ in 0..12 {
            if !self.transport.endpoints.phyvlab_allowed(&url) {
                return Err(Error::SecureChannel);
            }
            let reply = self.get_no_redirect(url.clone(), &current_referer).await?;
            if !(300..400).contains(&reply.status) {
                return Ok(reply);
            }
            let location = reply.location.as_deref().ok_or(Error::SessionExpired)?;
            let next_str =
                phyvlab::resolve_origin_url(&origin, location).ok_or(Error::SessionExpired)?;
            current_referer = reply.url.to_string();
            url = Url::parse(&next_str).map_err(|_| Error::SecureChannel)?;
        }
        Err(Error::Rejected("phyvlab redirect limit"))
    }

    async fn post_form(
        &self,
        url: Url,
        form: &BTreeMap<String, String>,
        referer: &str,
    ) -> Result<Reply> {
        let origin = self.origin();
        let referer = referer.to_owned();
        self.transport
            .phyvlab_send(url.clone(), |client, u| {
                client
                    .request(Method::POST, u.clone())
                    .header("Referer", referer)
                    .header("Origin", origin)
                    .form(form)
            })
            .await
    }

    async fn post_json(&self, url: Url, body: Vec<u8>, referer: &str) -> Result<Reply> {
        let origin = self.origin();
        let referer = referer.to_owned();
        self.transport
            .phyvlab_send(url.clone(), |client, u| {
                client
                    .request(Method::POST, u.clone())
                    .header("Referer", referer)
                    .header("Origin", origin)
                    .header("Content-Type", "application/json")
                    .body(body)
            })
            .await
    }

    async fn post_multipart(
        &self,
        url: Url,
        fields: &BTreeMap<String, String>,
        file_field: &str,
        file: &UploadFile<'_>,
        referer: &str,
    ) -> Result<Reply> {
        let origin = self.origin();
        let referer = referer.to_owned();
        let mut form = reqwest::multipart::Form::new();
        for (k, v) in fields {
            form = form.text(k.clone(), v.clone());
        }
        let part = reqwest::multipart::Part::bytes(file.bytes.to_vec())
            .file_name(file.file_name.to_owned())
            .mime_str(file.content_type)
            .map_err(|_| Error::Usage("invalid file content type".into()))?;
        form = form.part(file_field.to_owned(), part);
        self.transport
            .phyvlab_send(url.clone(), |client, u| {
                client
                    .request(Method::POST, u.clone())
                    .header("Referer", referer)
                    .header("Origin", origin)
                    .multipart(form)
            })
            .await
    }

    async fn fetch_page(&self, url: Url, referer: &str) -> Result<Reply> {
        let reply = self.follow(url, referer).await?;
        if !(200..300).contains(&reply.status) {
            return Err(Error::Network);
        }
        if !self.transport.endpoints.phyvlab_allowed(&reply.url) {
            return Err(Error::SessionExpired);
        }
        let path = reply.url.path();
        if path.contains("/login/index.php") || path.contains("/enrol/index.php") {
            return Err(Error::SessionExpired);
        }
        if path.starts_with("/user/edit") {
            return Err(Error::ActionRequired("phyvlab_profile_incomplete"));
        }
        let body = reply.text();
        if body.is_empty() {
            return Err(Error::Malformed("empty phyvlab response"));
        }
        if phyvlab::looks_like_login_page(&body) {
            return Err(Error::SessionExpired);
        }
        Ok(reply)
    }

    async fn probe_courses_session(&self) -> Result<bool> {
        let courses_url = self.url("/my/courses.php")?;
        let home_url = self.url("/")?;
        let reply = self.follow(courses_url, home_url.as_str()).await?;
        let body = reply.text();
        let auth = phyvlab::is_authenticated_page(reply.url.as_str(), &self.origin(), &body);
        let login = phyvlab::looks_like_login_page(&body);
        let course_links = body.matches("/course/view.php?id=").count();
        phy_debug(&format!(
            "probe path={} status={} auth={} login={} course_links={}",
            reply.url.path(),
            reply.status,
            auth,
            login,
            course_links
        ));
        if (200..300).contains(&reply.status) && auth && !login {
            return Ok(true);
        }
        if reply.url.path().contains("/login/index.php") || login {
            return Ok(false);
        }
        Err(Error::Network)
    }

    async fn load_login_page(&self) -> Result<Reply> {
        let login_url = self.url("/login/index.php")?;
        let home_url = self.url("/")?;
        let reply = self.follow(login_url, home_url.as_str()).await?;
        if !(200..300).contains(&reply.status) {
            return Err(Error::Network);
        }
        Ok(reply)
    }

    /// 自动恢复或建立物理在线会话。存在验证码时绝不提交密码。
    pub async fn establish_session(&self, creds: Option<(&str, &str)>) -> Result<SessionOutcome> {
        if self.has_session_cookie() {
            match self.probe_courses_session().await {
                Ok(true) => return Ok(SessionOutcome::Ready),
                Ok(false) => {
                    // 仅确认会话失效时清理已保存 cookie，临时网络故障保留原会话。
                    self.transport.clear();
                }
                Err(e) => return Err(e),
            }
        }
        let login_page = self.load_login_page().await?;
        let html = login_page.text();
        if phyvlab::is_authenticated_page(login_page.url.as_str(), &self.origin(), &html) {
            return Ok(SessionOutcome::Ready);
        }
        let Some((username, password)) =
            creds.filter(|(u, p)| !u.trim().is_empty() && !p.is_empty())
        else {
            return Ok(SessionOutcome::AccountRequired);
        };
        let form = phyvlab::parse_login_form(&html, &self.origin())?;
        if form.requires_captcha {
            let captcha_id = form
                .captcha_id
                .ok_or(Error::Malformed("phyvlab captcha id"))?;
            return Ok(SessionOutcome::CaptchaRequired { captcha_id });
        }
        self.submit_login_form(form.fields, username, password)
            .await
    }

    /// 交互式登录（支持无验证码直登与阿里云点选验证后单次提交密码）。
    pub async fn sign_in_interactive<F, Fut>(
        &self,
        username: &str,
        password: &str,
        proof_fn: F,
    ) -> Result<SessionOutcome>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<PhyVlabCaptchaProof>>,
    {
        if username.trim().is_empty() || password.is_empty() {
            return Ok(SessionOutcome::AccountRequired);
        }
        let mut login_page = self.load_login_page().await?;
        let mut html = login_page.text();
        if phyvlab::is_authenticated_page(login_page.url.as_str(), &self.origin(), &html) {
            if let Some(logout) = phyvlab::parse_logout_url(&html, &self.origin()) {
                let logout_url = Url::parse(&logout).map_err(|_| Error::SecureChannel)?;
                let _ = self.follow(logout_url, login_page.url.as_str()).await;
            }
            self.transport.clear();
            login_page = self.load_login_page().await?;
            html = login_page.text();
        }
        let form = phyvlab::parse_login_form(&html, &self.origin())?;
        if form.requires_captcha {
            let captcha_id = form
                .captcha_id
                .ok_or(Error::Malformed("phyvlab captcha id"))?;
            let proof = proof_fn(captcha_id).await?;
            let verify_url = self.url("/login/captcha_verify.php")?;
            let login_url = self.url("/login/index.php")?;
            let payload = serde_json::to_vec(&proof)
                .map_err(|_| Error::Malformed("captcha proof serialization"))?;
            let verified = self
                .post_json(verify_url, payload, login_url.as_str())
                .await?;
            if verified.status != 200 {
                return Err(Error::Network);
            }
            let ok = serde_json::from_str::<Value>(&verified.text())
                .ok()
                .and_then(|v| v.get("ok").and_then(Value::as_bool))
                == Some(true);
            if !ok {
                return Ok(SessionOutcome::CaptchaRejected);
            }
        }
        self.submit_login_form(form.fields, username, password)
            .await
    }

    async fn submit_login_form(
        &self,
        mut fields: BTreeMap<String, String>,
        username: &str,
        password: &str,
    ) -> Result<SessionOutcome> {
        let login_url = self.url("/login/index.php")?;
        fields.insert("username".into(), username.to_owned());
        fields.insert("password".into(), password.to_owned());
        let submitted = self
            .post_form(login_url.clone(), &fields, login_url.as_str())
            .await;
        use zeroize::Zeroize;
        for value in fields.values_mut() {
            value.zeroize();
        }
        let submitted = submitted?;
        let landing = if (300..400).contains(&submitted.status) {
            let loc = submitted.location.as_deref().ok_or(Error::SessionExpired)?;
            let next_str =
                phyvlab::resolve_origin_url(&self.origin(), loc).ok_or(Error::SessionExpired)?;
            let next_url = Url::parse(&next_str).map_err(|_| Error::SecureChannel)?;
            self.follow(next_url, login_url.as_str()).await?
        } else {
            submitted
        };
        let origin = self.origin();
        let landing_html = landing.text();
        if phyvlab::is_authenticated_page(landing.url.as_str(), &origin, &landing_html) {
            return Ok(SessionOutcome::Ready);
        }
        if phyvlab::looks_like_login_page(&landing_html) {
            return Ok(classify_login_rejection(&landing_html));
        }
        let courses_url = self.url("/my/courses.php")?;
        let settled = self.follow(courses_url, landing.url.as_str()).await?;
        let settled_html = settled.text();
        if phyvlab::is_authenticated_page(settled.url.as_str(), &origin, &settled_html) {
            Ok(SessionOutcome::Ready)
        } else if phyvlab::looks_like_login_page(&settled_html) {
            Ok(classify_login_rejection(&settled_html))
        } else {
            Err(Error::SessionExpired)
        }
    }

    pub async fn logout(&self) -> Result<()> {
        let home = self.url("/")?;
        let reply = self.follow(home.clone(), home.as_str()).await?;
        if let Some(logout) = phyvlab::parse_logout_url(&reply.text(), &self.origin())
            && let Ok(url) = Url::parse(&logout)
        {
            let _ = self.follow(url, reply.url.as_str()).await;
        }
        self.transport.clear();
        Ok(())
    }

    pub async fn fetch_courses(&self) -> Result<Vec<PhyVlabCourse>> {
        let courses_url = self.url("/my/courses.php")?;
        let referer = self.url("/?redirect=0")?;
        let reply = self.fetch_page(courses_url, referer.as_str()).await?;
        phyvlab::parse_courses(&reply.text(), &self.origin())
    }

    pub async fn fetch_course_activities(
        &self,
        course: &PhyVlabCourse,
    ) -> Result<Vec<PhyVlabActivity>> {
        let url = self.url(&format!("/course/view.php?id={}", course.id))?;
        let reply = self.fetch_page(url, &course.course_url).await?;
        phyvlab::parse_activities(&reply.text(), &self.origin(), course.id, &course.name)
    }

    pub async fn fetch_assignment_detail(
        &self,
        activity: &PhyVlabActivity,
    ) -> Result<PhyVlabAssignmentDetail> {
        let url = Url::parse(&activity.activity_url).map_err(|_| Error::SecureChannel)?;
        if !self.transport.endpoints.phyvlab_allowed(&url) {
            return Err(Error::SecureChannel);
        }
        let course_url = self.url(&format!("/course/view.php?id={}", activity.course_id))?;
        let reply = self.fetch_page(url, course_url.as_str()).await?;
        let mut page = phyvlab::parse_assignment_page(&reply.text(), &self.origin(), activity)?;
        if page.submission_context.is_none() {
            let edit_url_str = page.edit_submission_url.clone().unwrap_or_else(|| {
                format!(
                    "{}/mod/assign/view.php?id={}&action=editsubmission",
                    self.origin(),
                    activity.id
                )
            });
            if let Ok(edit_url) = Url::parse(&edit_url_str)
                && self.transport.endpoints.phyvlab_allowed(&edit_url)
            {
                match self.fetch_page(edit_url, &activity.activity_url).await {
                    Ok(edit_reply) => {
                        if let Ok(edit_page) = phyvlab::parse_assignment_page(
                            &edit_reply.text(),
                            &self.origin(),
                            activity,
                        ) {
                            page.detail =
                                phyvlab::merge_assignment_details(page.detail, edit_page.detail);
                        }
                    }
                    Err(Error::SessionExpired) => return Err(Error::SessionExpired),
                    Err(_) => {}
                }
            }
        }
        Ok(page.detail)
    }

    pub async fn fetch_all(
        &self,
        include_details: bool,
    ) -> Result<(Vec<PhyVlabCourse>, Vec<PhyVlabActivity>)> {
        let courses = self.fetch_courses().await?;
        let mut activities = Vec::new();
        for course in &courses {
            let list = self.fetch_course_activities(course).await?;
            activities.extend(list);
        }
        if include_details && !activities.is_empty() {
            let fetched = stream::iter(activities.into_iter().map(|mut act| async move {
                match self.fetch_assignment_detail(&act).await {
                    Ok(detail) => {
                        if let Some(done) = phyvlab::detail_submission_state(&detail) {
                            act.completed = done;
                        }
                        act.detail = Some(detail);
                        Ok(act)
                    }
                    Err(Error::SessionExpired) => Err(Error::SessionExpired),
                    Err(_) => Ok(act),
                }
            }))
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await;
            let mut merged = Vec::with_capacity(fetched.len());
            for item in fetched {
                merged.push(item?);
            }
            merged.sort_by_key(|a| (a.course_id, a.id));
            activities = merged;
        }
        Ok((courses, activities))
    }

    fn is_allowed_attachment_url(&self, url: &Url) -> bool {
        self.transport.endpoints.phyvlab_allowed(url)
            && url.path().starts_with("/pluginfile.php/")
            && url.fragment().is_none()
    }

    pub async fn download_attachment(
        &self,
        attachment: &PhyVlabAttachment,
        dest: &Path,
    ) -> Result<u64> {
        if dest.exists() {
            return Err(Error::Rejected("output file already exists"));
        }
        let mut url = Url::parse(&attachment.url).map_err(|_| Error::SecureChannel)?;
        let origin = self.origin();
        let mut reply = None;
        for hop in 0..=5 {
            if !self.is_allowed_attachment_url(&url) {
                return Err(Error::SecureChannel);
            }
            let current = self.get_no_redirect(url.clone(), &origin).await?;
            let status = current.status;
            if ![301, 302, 303, 307, 308].contains(&status) {
                reply = Some(current);
                break;
            }
            if hop == 5 {
                return Err(Error::Network);
            }
            let location = current.location.as_deref().ok_or(Error::Network)?;
            let next_str =
                phyvlab::resolve_origin_url(url.as_str(), location).ok_or(Error::SecureChannel)?;
            let next_url = Url::parse(&next_str).map_err(|_| Error::SecureChannel)?;
            if !self.is_allowed_attachment_url(&next_url) {
                return Err(Error::SecureChannel);
            }
            url = next_url;
        }
        let final_reply = reply.ok_or(Error::Network)?;
        if !self.is_allowed_attachment_url(&final_reply.url) {
            return Err(Error::SessionExpired);
        }
        if final_reply.url.path().contains("/login/index.php") {
            return Err(Error::SessionExpired);
        }
        if !(200..300).contains(&final_reply.status) {
            return Err(Error::Network);
        }
        if final_reply.bytes.is_empty() {
            return Err(Error::Malformed("empty phyvlab attachment"));
        }
        if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| Error::Storage(e.to_string()))?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(dest)
            .map_err(|e| Error::Storage(e.to_string()))?;
        file.write_all(&final_reply.bytes)
            .map_err(|e| Error::Storage(e.to_string()))?;
        file.sync_all().map_err(|e| Error::Storage(e.to_string()))?;
        Ok(final_reply.bytes.len() as u64)
    }

    async fn read_assignment_status(&self, activity_id: i64) -> Result<PhyVlabAssignmentDetail> {
        let url = self.url(&format!("/mod/assign/view.php?id={activity_id}"))?;
        let stub = PhyVlabActivity {
            id: activity_id,
            course_id: 0,
            course_name: String::new(),
            title: String::new(),
            activity_type: "作业".into(),
            activity_url: url.to_string(),
            open_text: None,
            open_timestamp: None,
            due_text: None,
            due_timestamp: None,
            completed: false,
            detail: None,
        };
        let reply = self.fetch_page(url, &self.origin()).await?;
        Ok(phyvlab::parse_assignment_page(&reply.text(), &self.origin(), &stub)?.detail)
    }

    async fn checked_write(&self, reply: Reply, referer: &str) -> Result<Reply> {
        let origin = self.origin();
        let final_reply = if [301, 302, 303].contains(&reply.status) {
            let location = reply
                .location
                .as_deref()
                .ok_or(Error::Rejected("操作跳转结果未确认"))?;
            let next_str =
                phyvlab::resolve_origin_url(&origin, location).ok_or(Error::SecureChannel)?;
            let next_url = Url::parse(&next_str).map_err(|_| Error::SecureChannel)?;
            self.fetch_page(next_url, referer).await?
        } else {
            reply
        };
        if !self.transport.endpoints.phyvlab_allowed(&final_reply.url)
            || !(200..300).contains(&final_reply.status)
        {
            return Err(Error::Rejected(
                "操作结果未确认，请刷新核对；不会自动重复提交",
            ));
        }
        let text = final_reply.text();
        if phyvlab::looks_like_login_page(&text) {
            return Err(Error::SessionExpired);
        }
        if phyvlab::has_error_banner(&text) {
            return Err(Error::Rejected("平台拒绝了操作，请刷新查看状态"));
        }
        Ok(final_reply)
    }

    async fn json_write(&self, reply: Reply, referer: &str) -> Result<()> {
        let checked = self.checked_write(reply, referer).await?;
        let obj: Value = serde_json::from_str(&checked.text())
            .map_err(|_| Error::Rejected("文件操作响应无法确认，请刷新核对"))?;
        let Some(map) = obj.as_object() else {
            return Err(Error::Rejected("文件操作响应无法确认，请刷新核对"));
        };
        if let Some(err) = map.get("error")
            && !matches!(err, Value::Null | Value::Bool(false))
            && err.as_str() != Some("")
            && err.as_i64() != Some(0)
        {
            return Err(Error::Rejected("文件操作失败，原提交未保存"));
        }
        if map.contains_key("event") {
            return Err(Error::Rejected("存在同名文件冲突，请刷新后重试"));
        }
        Ok(())
    }

    pub async fn save_files(
        &self,
        activity_id: i64,
        added: &[UploadFile<'_>],
        removed: &[String],
    ) -> Result<PhyVlabAssignmentDetail> {
        if added.is_empty() && removed.is_empty() {
            return Err(Error::Usage(
                "specify files to upload or --rm to remove".into(),
            ));
        }
        let before = self.read_assignment_status(activity_id).await?;
        if !before.can_submit {
            return Err(Error::Rejected("此作业已锁定或不允许修改"));
        }
        let view_url = self.url(&format!("/mod/assign/view.php?id={activity_id}"))?;
        let edit_url = self.url(&format!(
            "/mod/assign/view.php?id={activity_id}&action=editsubmission"
        ))?;
        let edit_reply = self.fetch_page(edit_url, view_url.as_str()).await?;
        let edit: PhyVlabEditContext =
            phyvlab::parse_moodle_edit_context(&edit_reply.text(), &self.origin(), activity_id)?;

        let mut replacement_names = BTreeSet::new();
        for f in added {
            if f.file_name.contains('/') || f.file_name.contains('\\') {
                return Err(Error::Usage("文件名包含不允许的路径字符".into()));
            }
            if !replacement_names.insert(f.file_name.to_owned()) {
                return Err(Error::Usage("新文件中存在重名，请先移除重复文件".into()));
            }
            if edit.max_bytes > 0 && (f.bytes.len() as i64) > edit.max_bytes {
                return Err(Error::Rejected("所选文件超过此作业的大小限制"));
            }
            if !edit.accepted_types.is_empty()
                && edit.accepted_types.iter().all(|t| t.starts_with('.'))
                && !edit.accepted_types.iter().any(|ext| {
                    f.file_name
                        .to_ascii_lowercase()
                        .ends_with(&ext.to_ascii_lowercase())
                })
            {
                return Err(Error::Rejected("所选文件格式不在允许范围内"));
            }
        }
        let existing_names: BTreeSet<_> = edit.files.iter().map(|f| f.name.clone()).collect();
        for r in removed {
            if !existing_names.contains(r) {
                return Err(Error::Rejected("原附件已变化，请刷新文件列表"));
            }
        }
        let removed_set: BTreeSet<&str> = removed.iter().map(String::as_str).collect();
        let deleting: Vec<_> = edit
            .files
            .iter()
            .filter(|f| {
                removed_set.contains(f.name.as_str()) || replacement_names.contains(&f.name)
            })
            .collect();
        let final_count = edit.files.len() - deleting.len() + added.len();
        if edit.max_files >= 0 && (final_count as i32) > edit.max_files {
            return Err(Error::Rejected("超出此作业允许的最大文件数量"));
        }

        let delete_url = self.url("/repository/draftfiles_ajax.php?action=delete")?;
        for f in &deleting {
            let mut fields = BTreeMap::new();
            fields.insert("sesskey".into(), edit.sesskey.clone());
            fields.insert("client_id".into(), edit.client_id.clone());
            fields.insert("itemid".into(), edit.item_id.clone());
            fields.insert("filepath".into(), f.path.clone());
            fields.insert("filename".into(), f.name.clone());
            let r = self
                .post_form(delete_url.clone(), &fields, view_url.as_str())
                .await?;
            self.json_write(r, view_url.as_str()).await?;
        }

        let upload_url = self.url("/repository/repository_ajax.php")?;
        for f in added {
            let mut fields = BTreeMap::new();
            fields.insert("action".into(), "upload".into());
            fields.insert("repo_id".into(), edit.repository_id.clone());
            fields.insert("itemid".into(), edit.item_id.clone());
            fields.insert("ctx_id".into(), edit.context_id.clone());
            fields.insert("client_id".into(), edit.client_id.clone());
            fields.insert("sesskey".into(), edit.sesskey.clone());
            fields.insert("env".into(), "filepicker".into());
            fields.insert("savepath".into(), "/".into());
            fields.insert("filepath".into(), "/".into());
            fields.insert("title".into(), f.file_name.to_owned());
            fields.insert("maxbytes".into(), edit.max_bytes.to_string());
            fields.insert("areamaxbytes".into(), "-1".into());
            fields.insert("license".into(), "unknown".into());
            fields.insert("author".into(), String::new());
            let r = self
                .post_multipart(
                    upload_url.clone(),
                    &fields,
                    "repo_upload_file",
                    f,
                    view_url.as_str(),
                )
                .await?;
            self.json_write(r, view_url.as_str()).await?;
        }

        let mut save_fields = edit.fields.clone();
        save_fields.insert(edit.manager_field.clone(), edit.item_id.clone());
        save_fields.insert("submitbutton".into(), "保存更改".into());
        let save_url = Url::parse(&edit.url).map_err(|_| Error::SecureChannel)?;
        let save_reply = self
            .post_form(save_url, &save_fields, view_url.as_str())
            .await?;
        self.checked_write(save_reply, view_url.as_str()).await?;

        let after = self.read_assignment_status(activity_id).await?;
        let mut expected: BTreeSet<String> = existing_names;
        for f in deleting {
            expected.remove(&f.name);
        }
        for name in replacement_names {
            expected.insert(name);
        }
        let actual: BTreeSet<String> = after.submitted_files.iter().cloned().collect();
        if actual != expected {
            return Err(Error::Rejected("文件保存结果未确认，请刷新核对后再操作"));
        }
        Ok(after)
    }

    pub async fn finalization_form(&self, activity_id: i64) -> Result<PhyVlabFinalizationForm> {
        let view_url = self.url(&format!("/mod/assign/view.php?id={activity_id}"))?;
        let submit_url = self.url(&format!(
            "/mod/assign/view.php?id={activity_id}&action=submit"
        ))?;
        let reply = self.fetch_page(submit_url, view_url.as_str()).await?;
        phyvlab::parse_finalization_form(&reply.text(), &self.origin(), activity_id)
    }

    pub async fn finalize(
        &self,
        activity_id: i64,
        accepted_statement: Option<&str>,
    ) -> Result<PhyVlabAssignmentDetail> {
        let before = self.read_assignment_status(activity_id).await?;
        if !before.can_finalize {
            return Err(Error::Rejected("当前作业没有待最终提交的草稿"));
        }
        let form = self.finalization_form(activity_id).await?;
        if let Some(required) = form.statement.as_deref()
            && accepted_statement != Some(required)
        {
            return Err(Error::Rejected("请先阅读并同意平台提交声明"));
        }
        let view_url = self.url(&format!("/mod/assign/view.php?id={activity_id}"))?;
        let action_url = Url::parse(&form.url).map_err(|_| Error::SecureChannel)?;
        let reply = self
            .post_form(action_url, &form.fields, view_url.as_str())
            .await?;
        self.checked_write(reply, view_url.as_str()).await?;
        let after = self.read_assignment_status(activity_id).await?;
        if !phyvlab::detail_has_submission(&after) || after.can_finalize {
            return Err(Error::Rejected("最终提交结果未确认，请刷新核对"));
        }
        Ok(after)
    }
}

fn classify_login_rejection(html: &str) -> SessionOutcome {
    let msg = phyvlab::parse_login_error_message(html);
    if msg.contains("登录无效") || msg.contains("Invalid login") {
        SessionOutcome::CredentialsRejected
    } else if phyvlab::login_requires_captcha(html) {
        match phyvlab::parse_captcha_id(html) {
            Some(captcha_id) => SessionOutcome::CaptchaRequired { captcha_id },
            None => SessionOutcome::CaptchaRejected,
        }
    } else {
        SessionOutcome::CredentialsRejected
    }
}

/// 生成本地回环验证页 HTML（只携带 captchaId，不传递任何账号或 Cookie）。
pub fn captcha_page_html(captcha_id: &str) -> Result<String> {
    if captcha_id.len() != 32 || !captcha_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Malformed("phyvlab captcha id"));
    }
    Ok(format!(
        r#"<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; base-uri 'none'; form-action 'none'; script-src 'unsafe-inline' https://phyvlab.bjtu.edu.cn/login/ct4.js https://*.alicaptcha.com; connect-src 'self' https://*.alicaptcha.com https://*.geetest.com; img-src https://*.alicaptcha.com https://*.geetest.com data: blob:; style-src 'unsafe-inline' https://*.alicaptcha.com; font-src https://*.alicaptcha.com data:">
<title>物理在线验证码</title>
<style>:root{{color-scheme:light dark}}body{{font:16px system-ui,sans-serif;margin:16px}}button{{font:inherit;padding:10px 16px}}</style>
<body>
<p id="message" role="status">正在加载学校验证码…</p>
<button id="verify" disabled>开始验证</button>
<button id="reload">重新加载</button>
<script>
const message=document.getElementById('message'),verify=document.getElementById('verify'),reload=document.getElementById('reload');
let widget,timeout;
function failed(){{clearTimeout(timeout);message.textContent='学校验证码加载失败，请重新加载。';reload.disabled=false;}}
function start(){{
  message.textContent='正在加载学校验证码…';verify.disabled=true;reload.disabled=true;
  timeout=setTimeout(failed,12000);
  const init=()=>{{
    if(typeof initAlicom4!=='function'){{failed();return;}}
    initAlicom4({{captchaId:'{captcha_id}',product:'bind',https:true}},c=>{{
      widget=c;
      const ready=()=>{{clearTimeout(timeout);message.textContent='按学校提示手动完成点选。';verify.disabled=false;reload.disabled=false;}};
      const hook=c.onReady||c.onNextReady;if(typeof hook==='function')hook.call(c,ready);else ready();
      c.onError(()=>{{message.textContent='学校验证码出现异常，请重新加载。';}});
      c.onSuccess(()=>{{
        const v=c.getValidate();
        if(!v){{message.textContent='未收到验证结果，请重试。';c.reset();return;}}
        const result=JSON.stringify({{lot_number:String(v.lot_number),captcha_output:String(v.captcha_output),pass_token:String(v.pass_token),gen_time:String(v.gen_time)}});
        verify.disabled=true;reload.disabled=true;message.textContent='正在提交验证结果…';
        fetch(location.pathname,{{method:'POST',headers:{{'Content-Type':'application/json'}},body:result}})
          .then(r=>{{if(!r.ok)throw new Error();message.textContent='已完成验证，可以返回终端。';}})
          .catch(()=>{{message.textContent='请返回终端重新打开验证码。';verify.disabled=false;reload.disabled=false;}});
      }});
    }});
  }};
  if(typeof initAlicom4==='function')init();
  else{{const s=document.createElement('script');s.src='https://phyvlab.bjtu.edu.cn/login/ct4.js';s.onload=init;s.onerror=failed;document.head.append(s);}}
}}
verify.onclick=()=>widget?.showCaptcha();
reload.onclick=()=>{{if(!widget)start();else{{widget.reset();message.textContent='按学校提示手动完成点选。';}}}};
start();
</script>
</body></html>"#
    ))
}

fn random_token() -> String {
    use sha2::Digest;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = sha2::Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let local = 0u8;
    hasher.update((&local as *const u8 as usize).to_le_bytes());
    hex::encode(&hasher.finalize()[..16])
}

fn open_browser_url(url: &str) -> bool {
    #[cfg(target_os = "linux")]
    let candidates = [("xdg-open", vec![url])];
    #[cfg(target_os = "macos")]
    let candidates = [("open", vec![url])];
    #[cfg(target_os = "windows")]
    let candidates = [("cmd", vec!["/c", "start", "", url])];
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let candidates: [(&str, Vec<&str>); 0] = [];
    for (bin, args) in candidates {
        if std::process::Command::new(bin)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return true;
        }
    }
    false
}

/// 启动一次性本机回环 HTTP 服务接收点选验证码 proof，最多等待 180 秒。
pub fn capture_captcha_proof(captcha_id: &str, open_browser: bool) -> Result<PhyVlabCaptchaProof> {
    let html = captcha_page_html(captcha_id)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| Error::Storage(e.to_string()))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::Storage(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Storage(e.to_string()))?
        .port();
    let authority = format!("127.0.0.1:{port}");
    let origin = format!("http://{authority}");
    let path = format!("/captcha/{}", random_token());
    let full_url = format!("{origin}{path}");
    let opened = open_browser && open_browser_url(&full_url);
    if opened {
        eprintln!("已在浏览器打开物理在线验证码，如未弹出请访问：{full_url}");
    } else {
        eprintln!("请在浏览器打开以下地址完成物理在线验证码：{full_url}");
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                if let Some(proof) =
                    handle_loopback_conn(&mut stream, &path, &authority, &origin, &html)
                {
                    return Ok(proof);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
    Err(Error::Rejected("物理在线验证码超时或未完成"))
}

fn handle_loopback_conn(
    stream: &mut std::net::TcpStream,
    expected_path: &str,
    expected_host: &str,
    expected_origin: &str,
    html: &str,
) -> Option<PhyVlabCaptchaProof> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let header_end;
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if buf.len() > 16384 {
            send_http(stream, 400, None);
            return None;
        }
    }
    let head = std::str::from_utf8(&buf[..header_end]).ok()?;
    let mut lines = head.lines();
    let req_line = lines.next()?;
    let mut parts = req_line.split_whitespace();
    let method = parts.next()?;
    let req_path = parts.next()?.split('?').next()?;
    let mut host = None;
    let mut origin = None;
    let mut content_type = None;
    let mut content_len: usize = 0;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim();
            if k.eq_ignore_ascii_case("host") {
                host = Some(v);
            } else if k.eq_ignore_ascii_case("origin") {
                origin = Some(v);
            } else if k.eq_ignore_ascii_case("content-type") {
                content_type = Some(v);
            } else if k.eq_ignore_ascii_case("content-length") {
                content_len = v.parse().ok()?;
            }
        }
    }
    if req_path != expected_path || host != Some(expected_host) {
        send_http(stream, 404, None);
        return None;
    }
    match method {
        "GET" => {
            send_http(stream, 200, Some(html));
            None
        }
        "POST" => {
            let ct = content_type
                .map(|s| s.split(';').next().unwrap_or("").trim())
                .unwrap_or("");
            if origin != Some(expected_origin)
                || !ct.eq_ignore_ascii_case("application/json")
                || content_len > 8192
            {
                send_http(stream, 400, None);
                return None;
            }
            while buf.len() < header_end + content_len {
                let n = stream.read(&mut chunk).ok()?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > header_end + 8192 {
                    send_http(stream, 400, None);
                    return None;
                }
            }
            let body =
                std::str::from_utf8(&buf[header_end..buf.len().min(header_end + content_len)])
                    .ok()?;
            if let Some(proof) = PhyVlabCaptchaProof::parse(body) {
                send_http(stream, 204, None);
                Some(proof)
            } else {
                send_http(stream, 400, None);
                None
            }
        }
        _ => {
            send_http(stream, 405, None);
            None
        }
    }
}

fn send_http(stream: &mut std::net::TcpStream, status: u16, body: Option<&str>) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let bytes = body.map(str::as_bytes).unwrap_or(&[]);
    let content_header = if body.is_some() {
        "Content-Type: text/html; charset=utf-8\r\n"
    } else {
        ""
    };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Connection: close\r\n\
         {content_header}\
         Content-Length: {}\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    if !bytes.is_empty() {
        let _ = stream.write_all(bytes);
    }
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Endpoints;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    fn login_html(captcha: bool, err: Option<&str>) -> String {
        let cap = if captcha {
            "<div id='aliyun-captcha-element'></div><script>initAlicom4({captchaId: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'})</script>"
        } else {
            ""
        };
        let err_block = err
            .map(|e| format!("<div id='loginerrormessage'>{e}</div>"))
            .unwrap_or_default();
        format!(
            "<html><body>{err_block}<form id='login' action='/login/index.php' method='post'>\
             <input type='hidden' name='logintoken' value='tok-1'>\
             <input type='hidden' name='anchor' value=''>\
             <input name='username'><input name='password' type='password'>{cap}</form></body></html>"
        )
    }

    fn auth_html() -> &'static str {
        "<html><body><a href='/login/logout.php?sesskey=k1'>退出</a>\
         <div data-region='course-content' data-course-id='72'>\
         <a class='aalink coursename' href='/course/view.php?id=72'><span class='multiline'>大学物理I</span></a>\
         <span class='categoryname'>物理</span><div class='progress-text'><span>20</span>%</div></div></body></html>"
    }

    #[tokio::test]
    async fn establishes_session_without_captcha_and_stops_cross_origin_redirect() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/login/index.php"))
            .and(query_param("testsession", "7"))
            .respond_with(ResponseTemplate::new(303).insert_header("Location", "/"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/login/index.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(login_html(false, None)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/login/index.php"))
            .respond_with(
                ResponseTemplate::new(303)
                    .insert_header("Location", "/login/index.php?testsession=7")
                    .insert_header("Set-Cookie", "MoodleSession=s1; Path=/; HttpOnly"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(auth_html()))
            .expect(1)
            .mount(&server)
            .await;

        let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
        let client = PhyVlabClient { transport: &t };
        let res = client
            .establish_session(Some(("lab-user", "lab-pass")))
            .await
            .unwrap();
        assert!(matches!(res, SessionOutcome::Ready));

        // Cross-origin redirect must stop cleanly.
        let s2 = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/login/index.php"))
            .respond_with(
                ResponseTemplate::new(303)
                    .insert_header("Location", "https://cas.bjtu.edu.cn/o/authorize/"),
            )
            .mount(&s2)
            .await;
        let t2 = SchoolTransport::new(Endpoints::for_test(&s2.uri()), None).unwrap();
        let c2 = PhyVlabClient { transport: &t2 };
        assert!(matches!(
            c2.establish_session(Some(("u", "p"))).await,
            Err(Error::SessionExpired)
        ));
    }

    #[tokio::test]
    async fn captcha_never_posts_password_in_background_and_verifies_interactive_flow() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/login/index.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(login_html(true, None)))
            .mount(&server)
            .await;

        let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
        let client = PhyVlabClient { transport: &t };
        let outcome = client
            .establish_session(Some(("lab-user", "lab-pass")))
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            SessionOutcome::CaptchaRequired { captcha_id } if captcha_id == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        let received = server.received_requests().await.unwrap();
        assert!(received.iter().all(|r| r.method == reqwest::Method::GET));

        // Rejected captcha must never post password.
        Mock::given(method("POST"))
            .and(path("/login/captcha_verify.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":false}"#))
            .expect(1)
            .mount(&server)
            .await;
        let proof = PhyVlabCaptchaProof {
            lot_number: "l".into(),
            captcha_output: "c".into(),
            pass_token: "p".into(),
            gen_time: "1".into(),
        };
        let res = client
            .sign_in_interactive("lab-user", "lab-pass", |_| {
                let proof = proof.clone();
                async move { Ok(proof) }
            })
            .await
            .unwrap();
        assert!(matches!(res, SessionOutcome::CaptchaRejected));
        let received = server.received_requests().await.unwrap();
        assert!(
            received
                .iter()
                .all(|r| r.url.path() != "/login/index.php" || r.method == reqwest::Method::GET)
        );
    }

    #[tokio::test]
    async fn fetches_courses_activities_and_downloads_attachment() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/my/courses.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(auth_html()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/course/view.php"))
            .and(query_param("id", "72"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<li id='module-3689' class='activity modtype_assign'>\
                 <a class='aalink' href='/mod/assign/view.php?id=3689'>\
                 <span class='instancename'>实验1 <span class='accesshide'>作业</span></span></a>\
                 <div data-region='activity-dates'><div><strong>到期日：</strong>2026年04月01日 Wednesday 23:59</div></div>\
                 </li>",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/mod/assign/view.php"))
            .and(query_param("id", "3689"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<div id='intro'><p>实验要求</p><a href='/pluginfile.php/10/mod_assign/intro/doc.pdf'>doc.pdf</a></div>\
                 <div class='submissionstatustable'><table><tr><th>提交状态</th><td>未提交</td></tr></table></div>",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/pluginfile.php/10/mod_assign/intro/doc.pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"%PDF-phyvlab".to_vec()))
            .mount(&server)
            .await;

        let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
        let client = PhyVlabClient { transport: &t };
        let (courses, activities) = client.fetch_all(true).await.unwrap();
        assert_eq!(courses.len(), 1);
        assert_eq!(activities.len(), 1);
        let detail = activities[0].detail.as_ref().unwrap();
        assert_eq!(detail.attachments.len(), 1);

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("doc.pdf");
        let n = client
            .download_attachment(&detail.attachments[0], &dest)
            .await
            .unwrap();
        assert_eq!(n, 12);
        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-phyvlab");
    }

    #[tokio::test]
    async fn saves_draft_file_and_finalizes_submission() {
        let server = MockServer::start().await;
        let editable_status = "<main><div class='submissionstatustable'><table>\
            <tr><th>提交状态</th><td>未提交</td></tr></table></div>\
            <input type='hidden' name='action' value='editsubmission'></main>";
        let edit_page = r#"<main>
            <form action="/mod/assign/view.php" method="post">
              <input type="hidden" name="id" value="3689">
              <input type="hidden" name="action" value="savesubmission">
              <input type="hidden" name="sesskey" value="sk-1">
              <input type="hidden" name="files_filemanager" value="88">
            </form>
            <script>
              M.form_filemanager.init({
                "itemid": 88,
                "context": {"id": 17},
                "client_id": "cid-1",
                "maxfiles": 3,
                "maxbytes": 1048576,
                "accepted_types": [".pdf"],
                "list": []
              });
              var repositories = {"4": {"id": 4, "type": "upload"}};
            </script>
        </main>"#;
        let draft_status = "<main><div class='submissionstatustable'><table>\
            <tr><th>提交状态</th><td>草稿（未提交）</td></tr>\
            <tr><td><a href='/pluginfile.php/10/assignsubmission_file/submission_files/report.pdf'>report.pdf</a></td></tr>\
            </table></div>\
            <input type='hidden' name='action' value='editsubmission'>\
            <input type='hidden' name='action' value='submit'></main>";
        let confirm_page = r#"<main>
            <form action="/mod/assign/view.php" method="post">
              <input type="hidden" name="id" value="3689">
              <input type="hidden" name="sesskey" value="sk-1">
              <input type="hidden" name="action" value="confirmsubmit">
              <label><input type="checkbox" name="submissionstatement" value="1">我确认这是本人作业</label>
              <input type="submit" name="submitbutton" value="继续">
            </form>
        </main>"#;
        let finalized_status = "<main><div class='submissionstatustable'><table>\
            <tr><th>提交状态</th><td>已提交待评分</td></tr>\
            <tr><td><a href='/pluginfile.php/10/assignsubmission_file/submission_files/report.pdf'>report.pdf</a></td></tr>\
            </table></div></main>";

        Mock::given(method("GET"))
            .and(path("/mod/assign/view.php"))
            .and(query_param("id", "3689"))
            .and(query_param("action", "editsubmission"))
            .respond_with(ResponseTemplate::new(200).set_body_string(edit_page))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/mod/assign/view.php"))
            .and(query_param("id", "3689"))
            .and(query_param("action", "submit"))
            .respond_with(ResponseTemplate::new(200).set_body_string(confirm_page))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repository/repository_ajax.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"url":"ok"}"#))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/mod/assign/view.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(draft_status))
            .expect(2)
            .mount(&server)
            .await;

        // First status check returns editable; subsequent GETs return draft, then finalized.
        let view_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = view_calls.clone();
        Mock::given(method("GET"))
            .and(path("/mod/assign/view.php"))
            .and(query_param("id", "3689"))
            .respond_with(move |req: &wiremock::Request| {
                if req.url.query().is_some_and(|q| q.contains("action=")) {
                    return ResponseTemplate::new(404);
                }
                let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let body = match n {
                    0 => editable_status,
                    1 | 2 => draft_status,
                    _ => finalized_status,
                };
                ResponseTemplate::new(200).set_body_string(body)
            })
            .mount(&server)
            .await;

        let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
        let client = PhyVlabClient { transport: &t };
        let saved = client
            .save_files(
                3689,
                &[UploadFile {
                    file_name: "report.pdf",
                    content_type: "application/pdf",
                    bytes: b"%PDF-1.4",
                }],
                &[],
            )
            .await
            .unwrap();
        assert!(saved.is_draft);
        assert!(saved.can_finalize);
        assert_eq!(saved.submitted_files, vec!["report.pdf"]);

        let finalized = client
            .finalize(3689, Some("我确认这是本人作业"))
            .await
            .unwrap();
        assert!(!finalized.can_finalize);
        assert!(phyvlab::detail_has_submission(&finalized));
    }

    #[tokio::test]
    async fn profile_redirect_keeps_session_ready_but_flags_action_required_on_fetch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/login/index.php"))
            .respond_with(
                ResponseTemplate::new(303)
                    .insert_header("Location", "/user/edit.php")
                    .insert_header("Set-Cookie", "MoodleSession=s1; Path=/; HttpOnly"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/my/courses.php"))
            .respond_with(ResponseTemplate::new(303).insert_header("Location", "/user/edit.php"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user/edit.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<html><body><a href='/login/logout.php?sesskey=k1'>退出</a>\
                 <a href='/course/view.php?id=1'>首页</a></body></html>",
            ))
            .mount(&server)
            .await;

        let t = SchoolTransport::new(Endpoints::for_test(&server.uri()), None).unwrap();
        let client = PhyVlabClient { transport: &t };
        assert!(matches!(
            client.establish_session(None).await.unwrap(),
            SessionOutcome::Ready
        ));
        assert!(matches!(
            client.fetch_all(false).await,
            Err(Error::ActionRequired("phyvlab_profile_incomplete"))
        ));
    }
}
