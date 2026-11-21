use crate::transport::SchoolTransport;
use campus_core::{Error, Result, parse};
use serde_json::Value;
use std::collections::BTreeMap;
use zeroize::Zeroizing;
pub const DATASETS: &[&str] = &[
    "courses",
    "grades",
    "grade_types",
    "exams",
    "card",
    "calendar",
    "rooms",
    "homework",
    "courseware",
    "phyvlab",
];
pub struct Fetcher<'a> {
    pub transport: &'a SchoolTransport,
}
fn values<T: serde::Serialize>(rows: Vec<T>) -> Result<Vec<Value>> {
    rows.into_iter()
        .map(|v| serde_json::to_value(v).map_err(|_| Error::Malformed("model serialization")))
        .collect()
}
fn article_session_id(
    body: &str,
    cookie: Option<Zeroizing<String>>,
    legacy_mode: bool,
) -> Result<Option<Zeroizing<String>>> {
    if cookie.is_some() && !legacy_mode {
        return Ok(cookie);
    }

    // The older courseware API can return an API session ID distinct from JSESSIONID.
    let article = match parse::json_object(body) {
        Ok(article) => article,
        Err(_) if cookie.is_some() => return Ok(cookie),
        Err(_) => return Err(Error::Malformed("smart article response JSON")),
    };
    Ok(article["sessionId"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| Zeroizing::new(id.to_owned()))
        .or(cookie))
}

impl Fetcher<'_> {
    pub async fn current_week(&self) -> Result<u8> {
        if self.smart_handshake().await.is_ok()
            && let Ok(root) = self
                .smart_json(
                    "/ve/back/coursePlatform/course.shtml",
                    &[("method", "getTimeList")],
                    None,
                )
                .await
            && let Some(week) = root
                .get("weekCode")
                .or_else(|| root.get("week_code"))
                .and_then(parse::scalar)
                .and_then(|s| s.parse::<u8>().ok())
                .filter(|w| (1..=30).contains(w))
        {
            return Ok(week);
        }
        let url = self
            .transport
            .endpoints
            .aa
            .join("classroom/timeholdresult/room_view/")
            .unwrap();
        let response = self.transport.get(url).await?;
        response.data(&self.transport.endpoints.aa)?;
        response
            .url
            .query_pairs()
            .find(|(k, _)| k == "zc")
            .and_then(|(_, s)| s.parse::<u8>().ok())
            .filter(|w| (1..=30).contains(w))
            .ok_or(Error::Malformed("current teaching week"))
    }
    async fn aa(&self, path: &str) -> Result<String> {
        let ep = &self.transport.endpoints;
        self.transport
            .get(ep.aa.join(path).map_err(|_| Error::Malformed("AA path"))?)
            .await?
            .data(&ep.aa)
    }
    pub async fn fetch(
        &self,
        dataset: &str,
        options: &BTreeMap<String, String>,
    ) -> Result<Vec<Value>> {
        match dataset {
            "courses" => {
                let teachers = parse::teachers(
                    &self
                        .aa("course_selection/courseselectabsent/absent_list/")
                        .await?,
                )?;
                let mut rows = parse::courses(
                    &self
                        .aa("course_selection/courseselect/stuschedule/")
                        .await?,
                    false,
                    &teachers,
                )?;
                rows.extend(parse::courses(
                    &self
                        .aa("course_selection/courseselecttask/schedule/")
                        .await?,
                    true,
                    &teachers,
                )?);
                values(campus_core::normalize_schedule(rows))
            }
            "grades" => {
                let mut rows = Vec::new();
                for kind in ["ln", "lr"] {
                    rows.extend(parse::grades(
                        &self
                            .aa(&format!(
                                "score/scores/stu/view/?page=1&perpage=500&ctype={kind}"
                            ))
                            .await?,
                    )?);
                }
                let mut seen = std::collections::HashSet::new();
                rows.retain(|g| {
                    seen.insert((
                        g.course_name.clone(),
                        g.score_raw.clone(),
                        g.credits_raw.clone(),
                    ))
                });
                values(rows)
            }
            "grade_types" => {
                let list = self.aa("training/training/program/").await?;
                let links = parse::program_links(&list);
                if links.is_empty() {
                    return Err(Error::Malformed("training program links"));
                }
                let mut rows = std::collections::BTreeMap::new();
                for link in links {
                    for row in parse::course_types(&self.aa(&link).await?)? {
                        rows.insert(row["course_code"].to_string(), row);
                    }
                }
                Ok(rows.into_values().collect())
            }
            "exams" => values(parse::exams(
                &self.aa("examine/examplanstudent/stulist/").await?,
            )?),
            "card" => {
                let ep = &self.transport.endpoints;
                let r = self
                    .transport
                    .get(ep.mis.join("osys_ajax_wrap/").unwrap())
                    .await?;
                values(vec![parse::card(&r.data(&ep.mis)?)?])
            }
            "calendar" => {
                let ep = &self.transport.endpoints;
                let r = self
                    .transport
                    .public(
                        ep.calendar
                            .join("Admin/SemesterTranPage.aspx?noRemark=1")
                            .unwrap(),
                    )
                    .await?;
                values(parse::academic_weeks(&r.data(&ep.calendar)?)?)
            }
            "rooms" => {
                let mut url = self
                    .transport
                    .endpoints
                    .aa
                    .join("classroom/timeholdresult/room_view/")
                    .unwrap();
                {
                    let mut q = url.query_pairs_mut();
                    q.append_pair("zc", options.get("week").map(String::as_str).unwrap_or("1"))
                        .append_pair(
                            "jxlh",
                            options.get("building").map(String::as_str).unwrap_or("1"),
                        )
                        .append_pair("page", "1")
                        .append_pair("perpage", "500");
                    if let Some(term) = options.get("term").filter(|t| *t != "current") {
                        q.append_pair("zxjxjhh", term);
                    }
                }
                let r = self.transport.get(url).await?;
                values(parse::rooms(&r.data(&self.transport.endpoints.aa)?)?)
            }
            "homework" | "courseware" => self.smart_dataset(dataset).await,
            _ => Err(Error::Usage(format!("unknown dataset: {dataset}"))),
        }
    }
    pub async fn live(&self, building: &str) -> Result<Value> {
        let mut u = self
            .transport
            .endpoints
            .headcount
            .join("api/classnum/")
            .unwrap();
        u.query_pairs_mut().append_pair("building", building);
        let r = self.transport.public(u).await?;
        if r.status != 200 {
            return Err(Error::Network);
        }
        parse::live_rooms(&r.text())
    }
    pub async fn smart_handshake(&self) -> Result<()> {
        let ep = &self.transport.endpoints;
        let r = self
            .transport
            .smart_follow(ep.mis.join("module/module/28/").unwrap())
            .await?;
        if crate::transport::same_origin(&r.url, &ep.smart) && r.status == 200 {
            return Ok(());
        }
        // Current HTTPS deployments may use the same form redirect pattern as AA.
        let action = parse::redirect(&r.text())?;
        let url = r.url.join(&action).map_err(|_| Error::SecureChannel)?;
        if crate::transport::is_legacy_smart_origin(&url) {
            return Err(Error::SmartLegacyHttp);
        }
        if !crate::transport::same_origin(&url, &ep.smart) {
            return Err(Error::SecureChannel);
        }
        let r = self.transport.smart_follow(url).await?;
        r.data(&ep.smart)?;
        Ok(())
    }
    pub async fn smart_context(&self) -> Result<SmartContext<'_>> {
        self.smart_handshake().await?;
        self.smart_session().await
    }
    async fn smart_session(&self) -> Result<SmartContext<'_>> {
        self.smart_session_on(self.transport, false).await
    }
    async fn smart_session_on<'b>(
        &self,
        transport: &'b SchoolTransport,
        courseware: bool,
    ) -> Result<SmartContext<'b>> {
        const ARTICLE: &str = "/ve/back/coursePlatform/message.shtml";
        let fetcher = Fetcher { transport };
        let cookie = || {
            let url = transport.endpoints.smart.join(ARTICLE).ok()?;
            transport
                .jar
                .lock()
                .ok()?
                .get_request_values(&url)
                .find(|(key, _)| key.eq_ignore_ascii_case("JSESSIONID"))
                .map(|(_, value)| zeroize::Zeroizing::new(value.to_owned()))
        };
        let legacy = transport.is_legacy();
        let initial = if courseware || legacy { cookie() } else { None };
        let article = fetcher
            .smart_body(
                ARTICLE,
                &[("method", "getArticleList")],
                initial.as_ref().map(|id| id.as_str()),
            )
            .await?;
        let session = article_session_id(&article, initial, legacy)?
            .or_else(|| if courseware { None } else { cookie() })
            .ok_or(Error::Malformed("smart session ID"))?;
        let semester = fetcher
            .smart_json(
                "/ve/back/rp/common/teachCalendar.shtml",
                &[("method", "queryCurrentXq")],
                Some(&session),
            )
            .await?;
        let result = smart_array(&semester, "result")?;
        let code = match result.first() {
            Some(row) => row
                .get("xqCode")
                .or_else(|| row.get("xq_code"))
                .and_then(parse::scalar)
                .filter(|code| !code.trim().is_empty())
                .ok_or(Error::Malformed("smart semester code"))?,
            None => String::new(),
        };
        let courses = if code.is_empty() {
            Vec::new()
        } else {
            let root = fetcher
                .smart_json(
                    "/ve/back/coursePlatform/course.shtml",
                    &[
                        ("method", "getCourseList"),
                        ("pagesize", "100"),
                        ("page", "1"),
                        ("xqCode", &code),
                    ],
                    Some(&session),
                )
                .await?;
            smart_array(&root, "courseList")?
        };
        Ok(SmartContext {
            fetcher,
            session,
            courses,
        })
    }
    pub async fn smart_json(
        &self,
        path: &str,
        query: &[(&str, &str)],
        session: Option<&str>,
    ) -> Result<Value> {
        let body = self.smart_body(path, query, session).await?;
        parse::json_object(&body).map_err(|error| {
            let stage = match query.iter().find(|(key, _)| *key == "method") {
                Some((_, "queryCurrentXq")) => "smart semester response JSON",
                Some((_, "getCourseList")) => "smart course list response JSON",
                Some((_, "stuQueryUploadResourceForCourseList")) => {
                    "courseware folder response JSON"
                }
                _ => return error,
            };
            Error::Malformed(stage)
        })
    }
    async fn smart_body(
        &self,
        path: &str,
        query: &[(&str, &str)],
        session: Option<&str>,
    ) -> Result<String> {
        let mut url = self
            .transport
            .endpoints
            .smart
            .join(path)
            .map_err(|_| Error::Malformed("smart path"))?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        let r = self.transport.smart_get(url, session).await?;
        r.data(&self.transport.endpoints.smart)
    }
    pub async fn courseware_context<'b>(
        &'b self,
        fallback: &'b mut Option<SchoolTransport>,
    ) -> Result<SmartContext<'b>> {
        self.scoped_smart_context(
            fallback,
            self.transport.endpoints.legacy_courseware,
            crate::transport::LegacyScope::Courseware,
            true,
        )
        .await
    }
    pub async fn homework_context<'b>(
        &'b self,
        fallback: &'b mut Option<SchoolTransport>,
    ) -> Result<SmartContext<'b>> {
        self.scoped_smart_context(
            fallback,
            self.transport.endpoints.legacy_homework,
            crate::transport::LegacyScope::HomeworkAction,
            false,
        )
        .await
    }
    async fn scoped_smart_context<'b>(
        &'b self,
        fallback: &'b mut Option<SchoolTransport>,
        allow_legacy: bool,
        scope: crate::transport::LegacyScope,
        courseware: bool,
    ) -> Result<SmartContext<'b>> {
        let secure = async {
            self.smart_handshake().await?;
            self.smart_session_on(self.transport, courseware).await
        };
        match secure.await {
            Err(error) if allow_legacy && !matches!(error, Error::SessionExpired) => {
                let transport = self.transport.new_legacy(scope)?;
                crate::legacy_homework::handshake(&transport).await?;
                let transport = fallback.insert(transport);
                self.smart_session_on(transport, courseware).await
            }
            result => result,
        }
    }
    async fn smart_dataset(&self, dataset: &str) -> Result<Vec<Value>> {
        if dataset == "courseware" {
            let mut fallback = None;
            let context = self.courseware_context(&mut fallback).await?;
            let mut out = Vec::new();
            for course in &context.courses {
                let mut nodes = context.walk(course).await?;
                if context.fetcher.transport.legacy_courseware() {
                    for node in &mut nodes {
                        if let Some(fields) = node.as_object_mut() {
                            fields.retain(|key, _| {
                                matches!(
                                    key.as_str(),
                                    "id" | "bag_name"
                                        | "resId"
                                        | "rpId"
                                        | "rpName"
                                        | "extName"
                                        | "rpSize"
                                        | "stu_download"
                                        | "parent"
                                        | "kind"
                                        | "depth"
                                )
                            });
                        }
                    }
                    let course_name = course
                        .get("course_name")
                        .or_else(|| course.get("courseName"))
                        .unwrap_or(&course["name"]);
                    out.push(serde_json::json!({"id":course["id"],"course_name":course_name,"nodes":nodes}));
                } else {
                    let mut row = course.clone();
                    row["nodes"] = serde_json::json!(nodes);
                    out.push(row);
                }
            }
            return Ok(out);
        }
        let context = match self.smart_context().await {
            Err(e)
                if dataset == "homework"
                    && self.transport.endpoints.legacy_homework
                    && !matches!(e, Error::SessionExpired) =>
            {
                let transport = self
                    .transport
                    .new_legacy(crate::transport::LegacyScope::Homework)?;
                crate::legacy_homework::handshake(&transport).await?;
                let fetcher = Fetcher {
                    transport: &transport,
                };
                let context = fetcher.smart_session().await?;
                let mut rows = Self::homework_rows(&context).await?;
                for row in &mut rows {
                    if let Some(fields) = row.as_object_mut() {
                        fields.retain(|key, _| {
                            matches!(
                                key.as_str(),
                                "id" | "course_id"
                                    | "course_name"
                                    | "title"
                                    | "end_time"
                                    | "create_date"
                                    | "open_date"
                                    | "status"
                                    | "subStatus"
                                    | "snId"
                                    | "user_id"
                                    | "content"
                                    | "stu_score"
                                    | "homework_type"
                            )
                        });
                    }
                }
                return Ok(rows);
            }
            result => result?,
        };
        Self::homework_rows(&context).await
    }
    async fn homework_rows(context: &SmartContext<'_>) -> Result<Vec<Value>> {
        use futures_util::{StreamExt, stream};
        let requests = context
            .courses
            .iter()
            .flat_map(|c| (0..3).map(move |kind| (c, kind)))
            .collect::<Vec<_>>();
        let results = stream::iter(requests.into_iter().map(|(c, kind)| async move {
            let id = c
                .get("id")
                .and_then(parse::scalar)
                .ok_or(Error::Malformed("smart course ID"))?;
            let root = context
                .fetcher
                .smart_json(
                    "/ve/back/coursePlatform/homeWork.shtml",
                    &[
                        ("method", "getHomeWorkList"),
                        ("cId", &id),
                        ("subType", &kind.to_string()),
                        ("page", "1"),
                        ("pagesize", "100"),
                    ],
                    Some(&context.session),
                )
                .await?;
            let mut rows = smart_array(&root, "courseNoteList")?;
            for row in &mut rows {
                if !row["id"].is_number() && !row["id"].is_string() {
                    return Err(Error::Malformed("homework ID"));
                }
                row["homework_type"] = kind.into();
            }
            Ok::<_, Error>(rows)
        }))
        .buffer_unordered(5)
        .collect::<Vec<_>>()
        .await;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in results {
            for item in r? {
                let key = format!(
                    "{}:{}:{}",
                    item["course_id"], item["id"], item["homework_type"]
                );
                if seen.insert(key) {
                    out.push(item);
                }
            }
        }
        Ok(out)
    }
}
pub fn smart_array(root: &Value, key: &'static str) -> Result<Vec<Value>> {
    let status = root
        .get("STATUS")
        .and_then(parse::scalar)
        .unwrap_or_default();
    if status == "2" {
        return Ok(Vec::new());
    }
    if status != "0" && !status.is_empty() {
        return Err(Error::Malformed("smart response status"));
    }
    match root.get(key) {
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Vec::new()),
        _ => Err(Error::Malformed(key)),
    }
}
pub struct SmartContext<'a> {
    pub fetcher: Fetcher<'a>,
    pub session: zeroize::Zeroizing<String>,
    pub courses: Vec<Value>,
}
impl SmartContext<'_> {
    pub async fn walk(&self, course: &Value) -> Result<Vec<Value>> {
        let mut queue = std::collections::VecDeque::from([(String::from("0"), 0)]);
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        while let Some((parent, depth)) = queue.pop_front() {
            if !seen.insert(parent.clone()) {
                continue;
            }
            for mut node in self.children(course, &parent).await? {
                node["parent"] = parent.clone().into();
                node["depth"] = depth.into();
                if node["kind"] == "folder" && depth < 6 {
                    queue.push_back((
                        node["id"]
                            .as_str()
                            .map(String::from)
                            .or_else(|| node["id"].as_i64().map(|n| n.to_string()))
                            .ok_or(Error::Malformed("folder ID"))?,
                        depth + 1,
                    ));
                }
                out.push(node);
                if out.len() > 20000 {
                    return Err(Error::Malformed("courseware node limit"));
                }
            }
        }
        Ok(out)
    }
    pub async fn homework_detail(&self, id: &str, course: &str) -> Result<Value> {
        let c = self
            .courses
            .iter()
            .find(|v| v.get("id").and_then(parse::scalar).as_deref() == Some(course))
            .ok_or(Error::Malformed("homework course"))?;
        let teacher = c
            .get("teacher_id")
            .and_then(parse::scalar)
            .ok_or(Error::Malformed("teacher ID"))?;
        self.fetcher
            .smart_json(
                "/ve/back/coursePlatform/homeWork.shtml",
                &[
                    ("method", "queryStudentCourseNote"),
                    ("id", id),
                    ("courseId", course),
                    ("teacherId", &teacher),
                ],
                Some(&self.session),
            )
            .await
    }
    pub async fn submitted_attachments(
        &self,
        id: &str,
        sn_id: &str,
        score: &str,
        user_id: &str,
    ) -> Result<Vec<parse::SubmittedAttachment>> {
        let mut url = self
            .fetcher
            .transport
            .endpoints
            .smart
            .join("/ve/back/course/courseWorkInfo.shtml")
            .unwrap();
        url.query_pairs_mut().extend_pairs([
            ("method", "piGaiDiv"),
            ("upId", id),
            ("id", sn_id),
            ("score", score),
            ("uLevel", "1"),
            ("type", "1"),
            ("username", "null"),
            ("userId", user_id),
        ]);
        let r = self
            .fetcher
            .transport
            .smart_get(url, Some(&self.session))
            .await?;
        let html = r.data_gbk(&self.fetcher.transport.endpoints.smart)?;
        parse::submitted_attachments(&html)
    }
    pub async fn submit_homework(
        &self,
        homework: &Value,
        content: &str,
        files: &[(&str, &[u8])],
    ) -> Result<()> {
        if files.is_empty() {
            return Err(Error::Usage(
                "at least one file is required to submit homework".into(),
            ));
        }
        let id = homework
            .get("id")
            .and_then(parse::scalar)
            .ok_or(Error::Malformed("homework ID"))?;
        let course_id = homework
            .get("course_id")
            .and_then(parse::scalar)
            .ok_or(Error::Malformed("homework course"))?;
        let homework_type = homework
            .get("homework_type")
            .and_then(parse::scalar)
            .unwrap_or_else(|| "0".into());

        let mut receipts = Vec::with_capacity(files.len());
        for &(file_name, bytes) in files {
            let mut url = self
                .fetcher
                .transport
                .endpoints
                .smart
                .join("/ve/back/rp/common/homeworkUpload.shtml")
                .unwrap();
            url.query_pairs_mut().append_pair("noteId", &id);
            let session = self.session.clone();
            let owned_name = file_name.to_owned();
            let owned_bytes = bytes.to_vec();
            let reply = self
                .fetcher
                .transport
                .smart_send(url, true, |client, target| {
                    let part = reqwest::multipart::Part::bytes(owned_bytes)
                        .file_name(owned_name)
                        .mime_str("application/octet-stream")
                        .expect("constant mime");
                    let form = reqwest::multipart::Form::new().part("file", part);
                    client
                        .post(target.clone())
                        .header("sessionid", session.as_str())
                        .multipart(form)
                })
                .await?;
            let body = reply.data(&self.fetcher.transport.endpoints.smart)?;
            receipts.push(parse::homework_upload_receipt(&body)?);
        }

        let mut submit_url = self
            .fetcher
            .transport
            .endpoints
            .smart
            .join("/ve/back/course/courseWorkInfo.shtml")
            .unwrap();
        submit_url
            .query_pairs_mut()
            .append_pair("method", "sendStuHomeWorks");
        let file_list = parse::upload_file_list_json(&receipts);
        let encoded_content = parse::pre_encode_form_value(content);
        let session = self.session.clone();
        let reply = self
            .fetcher
            .transport
            .smart_send(submit_url, false, |client, target| {
                client
                    .post(target.clone())
                    .header("sessionid", session.as_str())
                    .form(&[
                        ("content", encoded_content.as_str()),
                        ("groupName", ""),
                        ("groupId", ""),
                        ("courseId", course_id.as_str()),
                        ("contentType", homework_type.as_str()),
                        ("fz", "0"),
                        ("jxrl_id", ""),
                        ("fileList", file_list.as_str()),
                        ("upId", id.as_str()),
                        ("return_num", "0"),
                        ("isTeacher", "0"),
                    ])
            })
            .await?;
        let body = reply.data(&self.fetcher.transport.endpoints.smart)?;
        parse::homework_submit_receipt(&body)
    }
    pub async fn resource_ticket(&self, rp_id: &str) -> Result<url::Url> {
        let mut u = self
            .fetcher
            .transport
            .endpoints
            .smart
            .join("/ve/back/resourceSpace.shtml")
            .unwrap();
        u.query_pairs_mut()
            .append_pair("method", "rpinfoDownloadUrl")
            .append_pair("rpId", rp_id);
        let r = self.fetcher.transport.smart_post(u, &self.session).await?;
        let root = parse::json_object(&r.data(&self.fetcher.transport.endpoints.smart)?)?;
        if root["flag"] != true {
            return Err(Error::Rejected("resource download unavailable"));
        }
        let u = root["rpUrl"]
            .as_str()
            .ok_or(Error::Malformed("resource ticket"))?
            .parse::<url::Url>()
            .map_err(|_| Error::SecureChannel)?;
        if !crate::transport::same_origin(&u, &self.fetcher.transport.endpoints.smart)
            || self.fetcher.transport.legacy_courseware()
                && !self.fetcher.transport.legacy_download_allowed(&u)
        {
            return Err(Error::SecureChannel);
        }
        Ok(u)
    }
    pub async fn playback_detail(&self, course: &Value, lesson: &str) -> Result<Value> {
        let val = |k| course.get(k).and_then(parse::scalar).unwrap_or_default();
        let mut u = self
            .fetcher
            .transport
            .endpoints
            .smart
            .join("/ve/back/coursePlatform/coursePlatform.shtml")
            .unwrap();
        u.query_pairs_mut().extend_pairs([
            ("method", "toCoursePlatform"),
            ("courseToPage", "10480"),
            ("courseId", &val("course_num")),
            ("dataSource", "1"),
            ("cId", &val("id")),
            ("xkhId", &val("fz_id")),
            ("xqCode", &val("xq_code")),
            ("teacherId", &val("teacher_id")),
        ]);
        let r = self
            .fetcher
            .transport
            .smart_get(u, Some(&self.session))
            .await?;
        let html = r.data(&self.fetcher.transport.endpoints.smart)?;
        let user = campus_core::parse::platform_user_id(&html)?;
        self.fetcher
            .smart_json(
                "/ve/back/rp/common/teachCalendar.shtml",
                &[
                    ("method", "toDisplyCourseSchedDetail"),
                    ("courseSchedId", lesson),
                    ("userLevel", "1"),
                    ("userId", &user),
                ],
                Some(&self.session),
            )
            .await
    }
    pub async fn children(&self, course: &Value, parent: &str) -> Result<Vec<Value>> {
        let val = |k| course.get(k).and_then(parse::scalar).unwrap_or_default();
        let number = val("course_num");
        let group = val("fz_id");
        let semester = val("xq_code");
        let root = self
            .fetcher
            .smart_json(
                "/ve/back/coursePlatform/courseResource.shtml",
                &[
                    ("method", "stuQueryUploadResourceForCourseList"),
                    ("courseId", &number),
                    ("cId", &number),
                    ("xkhId", &group),
                    ("xqCode", &semester),
                    ("docType", "1"),
                    ("up_id", parent),
                    ("searchName", ""),
                ],
                Some(&self.session),
            )
            .await?;
        let mut folders = smart_array(&root, "bagList")?;
        for f in &mut folders {
            f["kind"] = "folder".into();
        }
        let mut resources = smart_array(&root, "resList")?;
        for f in &mut resources {
            f["kind"] = "resource".into();
        }
        folders.extend(resources);
        Ok(folders)
    }
    pub async fn lessons(&self, course: &str) -> Result<Vec<Value>> {
        let root = self
            .fetcher
            .smart_json(
                "/ve/back/rp/common/teachCalendar.shtml",
                &[("method", "toDisplyTeachCourses"), ("courseId", course)],
                Some(&self.session),
            )
            .await?;
        smart_array(&root, "courseSchedList")
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn legacy_courseware_prefers_article_session_with_cookie_fallback() {
        let cookie = || Some(Zeroizing::new("fixture-cookie".to_owned()));
        let article = r#"{"sessionId":"fixture-api-session"}"#;
        let pick = |body, legacy| {
            article_session_id(body, cookie(), legacy)
                .unwrap()
                .unwrap()
                .to_string()
        };
        assert_eq!(pick(article, true), "fixture-api-session");
        assert_eq!(pick(article, false), "fixture-cookie");
        assert_eq!(pick("<p>Notices</p>", true), "fixture-cookie");
        assert_eq!(pick(r#"{"sessionId":" "}"#, true), "fixture-cookie");
        assert!(matches!(
            article_session_id("<p>Notices</p>", None, true),
            Err(Error::Malformed("smart article response JSON"))
        ));
    }
}
