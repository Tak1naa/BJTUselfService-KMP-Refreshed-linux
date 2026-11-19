//! 物理在线（Moodle）领域模型、HTML 解析与作业状态判定。
use crate::{Error, Result, calendar::school_zone, parse};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhyVlabCourse {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub progress_percent: u8,
    pub course_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhyVlabAttachment {
    pub file_name: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhyVlabAssignmentDetail {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub submission_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_date_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_date_timestamp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grading_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_text: Option<String>,
    #[serde(default)]
    pub submitted_files: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<PhyVlabAttachment>,
    #[serde(default)]
    pub can_submit: bool,
    #[serde(default)]
    pub can_finalize: bool,
    #[serde(default)]
    pub is_draft: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhyVlabActivity {
    pub id: i64,
    pub course_id: i64,
    pub course_name: String,
    pub title: String,
    pub activity_type: String,
    pub activity_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_timestamp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_timestamp: Option<i64>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<PhyVlabAssignmentDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhyVlabEvent {
    pub id: String,
    pub title: String,
    pub date_text: String,
    pub day_timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_url: Option<String>,
    #[serde(default)]
    pub submitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhyVlabLoginForm {
    pub fields: BTreeMap<String, String>,
    pub requires_captcha: bool,
    pub captcha_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhyVlabCaptchaProof {
    pub lot_number: String,
    pub captcha_output: String,
    pub pass_token: String,
    pub gen_time: String,
}

impl PhyVlabCaptchaProof {
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.len() > 8192 {
            return None;
        }
        let value: Value = serde_json::from_str(raw).ok()?;
        let obj = value.as_object()?;
        if obj.len() != 4 {
            return None;
        }
        let field = |name: &str| {
            obj.get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            lot_number: field("lot_number")?,
            captcha_output: field("captcha_output")?,
            pass_token: field("pass_token")?,
            gen_time: field("gen_time")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhyVlabSubmissionContext {
    pub form_url: String,
    pub form_fields: BTreeMap<String, String>,
    pub sesskey: String,
    pub draft_item_id: Option<String>,
    pub context_id: Option<String>,
    pub client_id: Option<String>,
    pub repository_id: Option<String>,
}

impl PhyVlabSubmissionContext {
    pub fn is_upload_ready(&self) -> bool {
        self.draft_item_id.as_deref().is_some_and(|s| !s.is_empty())
            && self.context_id.as_deref().is_some_and(|s| !s.is_empty())
            && self.client_id.as_deref().is_some_and(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhyVlabAssignmentPage {
    pub detail: PhyVlabAssignmentDetail,
    pub submission_context: Option<PhyVlabSubmissionContext>,
    pub edit_submission_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhyVlabAssignmentFile {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhyVlabEditContext {
    pub url: String,
    pub fields: BTreeMap<String, String>,
    pub sesskey: String,
    pub item_id: String,
    pub context_id: String,
    pub client_id: String,
    pub repository_id: String,
    pub manager_field: String,
    pub files: Vec<PhyVlabAssignmentFile>,
    pub max_files: i32,
    pub max_bytes: i64,
    pub accepted_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhyVlabFinalizationForm {
    pub url: String,
    pub fields: BTreeMap<String, String>,
    pub statement: Option<String>,
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("constant CSS selector")
}

fn clean_text(raw: &str) -> String {
    raw.replace('\u{00a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn elem_text(e: ElementRef<'_>) -> String {
    clean_text(&e.text().collect::<Vec<_>>().join(" "))
}

fn date_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日(?:\s+[^\d\s]+)?\s+(\d{1,2}):(\d{2})").unwrap()
    })
}

fn weekday_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\b").unwrap()
    })
}

pub fn resolve_origin_url(base: &str, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) || trimmed.contains('#') {
        return None;
    }
    let origin = Url::parse(base).ok()?;
    let mut cleaned_base = origin.clone();
    cleaned_base.set_query(None);
    cleaned_base.set_fragment(None);
    let target = cleaned_base.join(trimmed).ok()?;
    let same = target.scheme() == origin.scheme()
        && target.host_str() == origin.host_str()
        && target.port_or_known_default() == origin.port_or_known_default();
    let secure = target.scheme() == "https" && target.port_or_known_default() == Some(443)
        || (origin.scheme() == "http" && origin.host_str() == Some("127.0.0.1"));
    (same && secure).then(|| target.to_string())
}

pub fn looks_like_login_page(html: &str) -> bool {
    let doc = Html::parse_document(html);
    if doc
        .select(&sel(
            "a[href*='/auth/oauth2/login.php'], form[action*='/login/index.php']",
        ))
        .next()
        .is_some()
    {
        return true;
    }
    let text = elem_text(doc.root_element());
    text.contains("用户名或邮箱") && text.contains("密码")
}

pub fn is_authenticated_page(url: &str, origin: &str, html: &str) -> bool {
    let Ok(base) = Url::parse(origin) else {
        return false;
    };
    let Ok(current) = Url::parse(url) else {
        return false;
    };
    if current.scheme() != base.scheme()
        || current.host_str() != base.host_str()
        || current.port_or_known_default() != base.port_or_known_default()
    {
        return false;
    }
    let doc = Html::parse_document(html);
    if doc
        .select(&sel(
            "a[href*='/login/logout.php'], [data-region='usermenu'], a[href='/user/profile.php']",
        ))
        .next()
        .is_some()
    {
        return true;
    }
    let text = elem_text(doc.root_element());
    text.contains("您已经以") && text.contains("身份登录")
}

pub fn login_requires_captcha(html: &str) -> bool {
    let doc = Html::parse_document(html);
    doc.select(&sel("form#login #aliyun-captcha-element"))
        .next()
        .is_some()
        || html.contains("/login/captcha_verify.php")
}

pub fn parse_captcha_id(html: &str) -> Option<String> {
    if !login_requires_captcha(html) {
        return None;
    }
    let re =
        Regex::new(r#"initAlicom4\s*\(\s*\{\s*captchaId\s*:\s*['"]([a-fA-F0-9]{32})['"]"#).unwrap();
    re.captures(html).map(|c| c[1].to_string())
}

pub fn parse_login_form(html: &str, origin: &str) -> Result<PhyVlabLoginForm> {
    let doc = Html::parse_document(html);
    let form = doc
        .select(&sel("form#login"))
        .next()
        .ok_or(Error::Malformed("phyvlab login form"))?;
    let login_url = format!("{}/login/index.php", origin.trim_end_matches('/'));
    if let Some(action) = form.value().attr("action").filter(|s| !s.trim().is_empty()) {
        let resolved = resolve_origin_url(&login_url, action).ok_or(Error::SecureChannel)?;
        if resolved.split('?').next() != Some(login_url.as_str()) {
            return Err(Error::SecureChannel);
        }
    }
    if form.select(&sel("input[name=username]")).next().is_none()
        || form.select(&sel("input[name=password]")).next().is_none()
    {
        return Err(Error::Malformed("phyvlab login inputs"));
    }
    let mut fields = BTreeMap::new();
    for input in form.select(&sel("input[type=hidden][name]")) {
        if let Some(name) = input
            .value()
            .attr("name")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            fields.insert(
                name.to_owned(),
                input.value().attr("value").unwrap_or_default().to_owned(),
            );
        }
    }
    if fields.get("logintoken").is_none_or(|v| v.trim().is_empty()) {
        return Err(Error::Malformed("phyvlab logintoken"));
    }
    let requires_captcha = login_requires_captcha(html);
    let captcha_id = parse_captcha_id(html);
    if requires_captcha && captcha_id.is_none() {
        return Err(Error::Malformed("phyvlab captcha id"));
    }
    Ok(PhyVlabLoginForm {
        fields,
        requires_captcha,
        captcha_id,
    })
}

pub fn parse_login_error_message(html: &str) -> String {
    let doc = Html::parse_document(html);
    doc.select(&sel("[role=alert], #loginerrormessage"))
        .map(elem_text)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn has_error_banner(html: &str) -> bool {
    let doc = Html::parse_document(html);
    doc.select(&sel(
        ".error, .errormessage, .alert-danger, .invalid-feedback",
    ))
    .any(|el| !elem_text(el).is_empty())
}

pub fn parse_logout_url(html: &str, origin: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let login_url = format!("{}/login/index.php", origin.trim_end_matches('/'));
    for link in doc.select(&sel("a[href*='/login/logout.php']")) {
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let Some(resolved) = resolve_origin_url(&login_url, href) else {
            continue;
        };
        if let Ok(u) = Url::parse(&resolved)
            && u.query_pairs()
                .any(|(k, v)| k == "sesskey" && !v.trim().is_empty())
        {
            return Some(resolved);
        }
    }
    None
}

pub fn parse_courses(html: &str, origin: &str) -> Result<Vec<PhyVlabCourse>> {
    let doc = Html::parse_document(html);
    let origin_clean = origin.trim_end_matches('/');
    let cards: Vec<_> = doc.select(&sel("[data-region='course-content']")).collect();
    if cards.is_empty() {
        return Ok(parse_legacy_courses(&doc, origin_clean));
    }
    let mut courses = Vec::new();
    for card in cards {
        let Some(id) = card
            .value()
            .attr("data-course-id")
            .and_then(|s| s.trim().parse::<i64>().ok())
        else {
            continue;
        };
        let link = card.select(&sel("a.aalink.coursename")).next().or_else(|| {
            card.select(&sel("[data-region='course-content'] > a"))
                .next()
        });
        let Some(link) = link else {
            continue;
        };
        let name = link
            .select(&sel(".multiline"))
            .next()
            .map(elem_text)
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let category = card
            .select(&sel(".categoryname"))
            .next()
            .map(elem_text)
            .unwrap_or_default();
        let progress = card
            .select(&sel(".progress-text"))
            .next()
            .and_then(|node| {
                node.select(&sel("span"))
                    .next()
                    .and_then(|s| elem_text(s).parse::<u8>().ok())
                    .or_else(|| {
                        let digits: String = elem_text(node)
                            .chars()
                            .filter(char::is_ascii_digit)
                            .collect();
                        digits.parse::<u8>().ok()
                    })
            })
            .unwrap_or(0)
            .min(100);
        let href = link.value().attr("href").unwrap_or_default();
        let Some(course_url) = resolve_origin_url(origin_clean, href) else {
            continue;
        };
        courses.push(PhyVlabCourse {
            id,
            name,
            category,
            progress_percent: progress,
            course_url,
        });
    }
    if courses.is_empty() {
        return Err(Error::Malformed("phyvlab courses"));
    }
    courses.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(courses)
}

fn parse_legacy_courses(doc: &Html, origin: &str) -> Vec<PhyVlabCourse> {
    let generic = ["课程图片", "课程名称"];
    let markers = ["收藏此课程", "取消收藏", "在视图中恢复", "归档本课程"];
    let id_re = Regex::new(r"[?&]id=(\d+)").unwrap();
    let mut by_id: HashMap<i64, Vec<(String, String)>> = HashMap::new();
    for link in doc.select(&sel("a[href*='/course/view.php?id=']")) {
        let href = link.value().attr("href").unwrap_or_default().trim();
        let Some(id) = id_re.captures(href).and_then(|c| c[1].parse::<i64>().ok()) else {
            continue;
        };
        let visible = clean_text(&link.text().collect::<Vec<_>>().join(" "));
        let title = clean_text(link.value().attr("title").unwrap_or_default());
        let name = if visible.is_empty()
            || generic.contains(&visible.as_str())
            || markers.iter().any(|m| visible.contains(m))
        {
            title
        } else {
            visible
        };
        if name.is_empty()
            || generic.contains(&name.as_str())
            || markers.iter().any(|m| name.contains(m))
        {
            continue;
        }
        let Some(course_url) = resolve_origin_url(origin, href) else {
            continue;
        };
        by_id.entry(id).or_default().push((name, course_url));
    }
    let mut courses: Vec<_> = by_id
        .into_iter()
        .filter_map(|(id, mut candidates)| {
            candidates.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
            candidates
                .into_iter()
                .next()
                .map(|(name, course_url)| PhyVlabCourse {
                    id,
                    name,
                    category: String::new(),
                    progress_percent: 0,
                    course_url,
                })
        })
        .collect();
    courses.sort_by(|a, b| a.name.cmp(&b.name));
    courses
}

pub fn parse_activities(
    html: &str,
    origin: &str,
    course_id: i64,
    course_name: &str,
) -> Result<Vec<PhyVlabActivity>> {
    let doc = Html::parse_document(html);
    let origin_clean = origin.trim_end_matches('/');
    let anchors: Vec<_> = doc.select(&sel("li.activity.modtype_assign")).collect();
    if anchors.is_empty() {
        return Ok(Vec::new());
    }
    let mut parsed = Vec::new();
    for activity in anchors {
        let id = activity
            .value()
            .attr("id")
            .and_then(|s| s.strip_prefix("module-"))
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| {
                activity
                    .value()
                    .attr("data-id")
                    .and_then(|s| s.parse::<i64>().ok())
            });
        let Some(id) = id else {
            continue;
        };
        let Some(link) = activity
            .select(&sel("a.aalink[href*='/mod/assign/view.php']"))
            .next()
        else {
            continue;
        };
        let raw_title = link
            .select(&sel(".instancename"))
            .next()
            .map(elem_text)
            .unwrap_or_default();
        let title = raw_title
            .strip_suffix("作业")
            .unwrap_or(&raw_title)
            .trim()
            .to_owned();
        if title.is_empty() {
            continue;
        }
        let Some(activity_url) = link
            .value()
            .attr("href")
            .and_then(|h| resolve_origin_url(origin_clean, h))
        else {
            continue;
        };
        let completion = activity
            .select(&sel("button[data-action='toggle-manual-completion']"))
            .next();
        let completion_type = completion
            .and_then(|b| b.value().attr("data-toggletype"))
            .unwrap_or_default();
        let completed = completion_type.contains("undo")
            || (completion.is_some()
                && completion_type.starts_with("manual:")
                && !completion_type.contains("mark-done"));
        let date_region = activity
            .select(&sel("[data-region='activity-dates']"))
            .next();
        let open_text = date_region.and_then(|r| activity_date_text(r, "打开"));
        let due_text = date_region.and_then(|r| activity_date_text(r, "到期日"));
        let open_timestamp = open_text.as_deref().and_then(parse_date_timestamp);
        let due_timestamp = due_text.as_deref().and_then(parse_date_timestamp);
        parsed.push(PhyVlabActivity {
            id,
            course_id,
            course_name: course_name.to_owned(),
            title,
            activity_type: "作业".into(),
            activity_url,
            open_text,
            open_timestamp,
            due_text,
            due_timestamp,
            completed,
            detail: None,
        });
    }
    if parsed.is_empty() {
        return Err(Error::Malformed("phyvlab activities"));
    }
    parsed.sort_by_key(|a| a.id);
    Ok(parsed)
}

fn activity_date_text(region: ElementRef<'_>, label: &str) -> Option<String> {
    for strong in region.select(&sel("strong")) {
        let heading = elem_text(strong);
        let normalized = heading.trim_end_matches([':', '：']).trim();
        if normalized != label {
            continue;
        }
        let parent = ElementRef::wrap(strong.parent()?)?;
        let text = elem_text(parent);
        let stripped = text
            .strip_prefix(&format!("{label}："))
            .or_else(|| text.strip_prefix(&format!("{label}:")))
            .unwrap_or(&text)
            .trim();
        if !stripped.is_empty() {
            return Some(normalize_date_text(stripped));
        }
    }
    None
}

pub fn normalize_date_text(value: &str) -> String {
    let Some(caps) = date_regex().captures(value) else {
        return value.to_owned();
    };
    format!(
        "{}年{:0>2}月{:0>2}日 {:0>2}:{}",
        &caps[1], &caps[2], &caps[3], &caps[4], &caps[5]
    )
}

fn normalize_dates_in_text(value: &str) -> String {
    date_regex()
        .replace_all(value, |caps: &regex::Captures<'_>| {
            format!(
                "{}年{:0>2}月{:0>2}日 {:0>2}:{}",
                &caps[1], &caps[2], &caps[3], &caps[4], &caps[5]
            )
        })
        .into_owned()
}

pub fn parse_date_timestamp(value: &str) -> Option<i64> {
    let caps = date_regex().captures(value)?;
    let dt = jiff::civil::DateTime::new(
        caps[1].parse().ok()?,
        caps[2].parse().ok()?,
        caps[3].parse().ok()?,
        caps[4].parse().ok()?,
        caps[5].parse().ok()?,
        0,
        0,
    )
    .ok()?;
    Some(dt.to_zoned(school_zone()).ok()?.timestamp().as_second())
}

pub fn parse_assignment_page(
    html: &str,
    origin: &str,
    activity: &PhyVlabActivity,
) -> Result<PhyVlabAssignmentPage> {
    let doc = Html::parse_document(html);
    let origin_clean = origin.trim_end_matches('/');
    let row_sel = sel(
        "div.submissionstatustable tr, table.submissionstatustable tr, \
         div.gradingsummarytable tr, table.gradingsummarytable tr, \
         div.feedbacktable tr, table.feedbacktable tr",
    );
    let cell_sel = sel("th, td");
    let mut labeled_values = Vec::new();
    for row in doc.select(&row_sel) {
        let cells: Vec<_> = row.select(&cell_sel).map(elem_text).collect();
        if cells.len() >= 2 {
            labeled_values.push((cells[0].clone(), cells[1..].join(" ").trim().to_owned()));
        }
    }
    let row_value = |labels: &[&str]| -> Option<String> {
        labeled_values
            .iter()
            .find(|(label, value)| {
                let lower = label.to_lowercase();
                labels.iter().any(|w| lower.contains(&w.to_lowercase())) && !value.is_empty()
            })
            .map(|(_, value)| value.clone())
    };
    let exact_row_value = |labels: &[&str]| -> Option<String> {
        labeled_values
            .iter()
            .find(|(label, value)| {
                let norm = label.trim_end_matches([':', '：']).trim();
                labels.iter().any(|w| norm.eq_ignore_ascii_case(w)) && !value.is_empty()
            })
            .map(|(_, value)| value.clone())
    };

    let description_el = [
        "#intro",
        ".activity-description",
        ".mod_introbox",
        "[data-region='activity-description']",
        ".box.generalbox",
    ]
    .iter()
    .filter_map(|s| doc.select(&sel(s)).next())
    .find(|e| !elem_text(*e).is_empty());

    let description = description_el
        .map(|el| parse::homework_text(&el.inner_html()))
        .unwrap_or_default();
    let attachments = description_el
        .map(|el| parse_description_attachments(el, origin_clean))
        .unwrap_or_default();

    let body_text = doc
        .select(&sel("body"))
        .next()
        .map(elem_text)
        .unwrap_or_else(|| elem_text(doc.root_element()));

    let submission_status = row_value(&[
        "提交状态",
        "作业状态",
        "Submission status",
        "Assignment status",
    ])
    .or_else(|| {
        doc.select(&sel(".submissionstatus, [data-region='submission-status']"))
            .next()
            .map(elem_text)
            .filter(|s| !s.is_empty())
    })
    .or_else(|| {
        if body_text.contains("未提交") {
            Some("未提交".into())
        } else if body_text.contains("已提交") {
            Some("已提交".into())
        } else {
            None
        }
    })
    .unwrap_or_default();

    let submission_date_candidate =
        row_value(&["最后修改", "Last modified", "提交时间", "Submitted"]);
    let numeric_grade_re =
        Regex::new(r"(?i)成绩\s*([0-9]+(?:\.[0-9]+)?\s*/\s*[0-9]+(?:\.[0-9]+)?)").unwrap();
    let numeric_grade = labeled_values
        .iter()
        .find_map(|(_, v)| numeric_grade_re.captures(v).map(|c| c[1].to_string()))
        .or_else(|| {
            numeric_grade_re
                .captures(&body_text)
                .map(|c| c[1].to_string())
        });
    let grading_status = row_value(&["批改状态", "Grading status", "评分状态"])
        .or_else(|| row_value(&["批改成绩", "Graded"]).filter(|_| numeric_grade.is_none()));
    let grade = numeric_grade
        .or_else(|| {
            row_value(&["成绩", "Grade", "评分"]).filter(|v| v.chars().any(|c| c.is_ascii_digit()))
        })
        .or_else(|| first_detail_text(&doc, ".gradefordisplay, [data-region='grade']"));

    let feedback = exact_row_value(&[
        "教师评语",
        "教师反馈",
        "Feedback comments",
        "评语",
        "反馈",
        "Comments",
        "Comment",
    ])
    .filter(|v| !numeric_grade_re.is_match(v))
    .or_else(|| {
        first_detail_text(
            &doc,
            ".assignfeedback_comments, .feedbackcomments, .feedback-comments, \
             [data-region='feedback-comments'], .feedback .comments, .feedback .feedbacktext",
        )
        .filter(|v| !numeric_grade_re.is_match(v))
    });

    let mut seen_files = BTreeSet::new();
    let mut submitted_files = Vec::new();
    for link in doc.select(&sel(
        ".submissionstatussubmitted .files a, .assignsubmission_file .files a, \
         .submissionstatustable a[href*='/pluginfile.php']",
    )) {
        let name = elem_text(link);
        if !name.is_empty() && seen_files.insert(name.clone()) {
            submitted_files.push(name);
        }
    }

    let submission_date = submission_date_candidate.filter(|_| {
        activity.completed
            || !submitted_files.is_empty()
            || status_indicates_submission(&submission_status)
    });

    let form = doc.select(&sel("form")).find(|form| {
        let action = form.value().attr("action").unwrap_or_default();
        let id = form.value().attr("id").unwrap_or_default();
        action.contains("/mod/assign/editsubmission.php")
            || id.to_ascii_lowercase().contains("submission")
            || (action.contains("/mod/assign/view.php")
                && form
                    .select(&sel("input[name*='filemanager']"))
                    .next()
                    .is_some())
    });
    let mut form_fields = BTreeMap::new();
    if let Some(form) = form {
        for input in form.select(&sel("input[type='hidden'][name]")) {
            if let Some(name) = input
                .value()
                .attr("name")
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                form_fields.insert(
                    name.to_owned(),
                    input.value().attr("value").unwrap_or_default().to_owned(),
                );
            }
        }
    }
    let file_manager = form
        .and_then(|f| f.select(&sel("input[name*='filemanager']")).next())
        .or_else(|| doc.select(&sel("input[name*='filemanager']")).next());
    let file_manager_node = form
        .and_then(|f| f.select(&sel(".filemanager")).next())
        .or_else(|| doc.select(&sel(".filemanager")).next());
    let field_name = file_manager
        .and_then(|n| n.value().attr("name"))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (meta_item, meta_ctx, meta_client, meta_repo) =
        parse_filemanager_metadata(&doc, field_name);
    let sesskey = form_fields
        .get("sesskey")
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .or_else(|| parse_sesskey_from_html(html));
    let fallback_edit_url = format!(
        "{origin_clean}/mod/assign/view.php?id={}&action=editsubmission",
        activity.id
    );
    let form_url = form
        .and_then(|f| f.value().attr("action"))
        .and_then(|a| resolve_origin_url(origin_clean, a))
        .unwrap_or(fallback_edit_url);
    let submission_context = match sesskey {
        Some(sesskey) if file_manager.is_some() || file_manager_node.is_some() => {
            Some(PhyVlabSubmissionContext {
                form_url,
                form_fields,
                sesskey,
                draft_item_id: file_manager
                    .and_then(|n| n.value().attr("value"))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| {
                        file_manager_node
                            .and_then(|n| n.value().attr("data-itemid"))
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                    })
                    .or(meta_item),
                context_id: file_manager_node
                    .and_then(|n| n.value().attr("data-contextid"))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or(meta_ctx),
                client_id: file_manager_node
                    .and_then(|n| n.value().attr("data-clientid"))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or(meta_client),
                repository_id: file_manager_node
                    .and_then(|n| n.value().attr("data-repositoryid"))
                    .or_else(|| file_manager_node.and_then(|n| n.value().attr("data-repo-id")))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or(meta_repo),
            })
        }
        _ => None,
    };

    let edit_submission_url = doc
        .select(&sel("a[href]"))
        .filter_map(|a| a.value().attr("href"))
        .filter_map(|href| resolve_origin_url(origin_clean, href))
        .find(|u| {
            u.contains("/mod/assign/editsubmission.php") || u.contains("action=editsubmission")
        });

    let can_submit = submission_context
        .as_ref()
        .is_some_and(PhyVlabSubmissionContext::is_upload_ready)
        || doc
            .select(&sel(
                "input[name=action][value=editsubmission], a[href*='action=editsubmission']",
            ))
            .next()
            .is_some();
    let can_finalize = doc
        .select(&sel(
            "input[name=action][value=submit], a[href*='action=submit']",
        ))
        .next()
        .is_some();
    let normalized_status = normalize_dates_in_text(&submission_status);
    let is_draft = normalized_status.to_ascii_lowercase().contains("draft")
        || normalized_status.contains("草稿");
    let detail = PhyVlabAssignmentDetail {
        description: normalize_dates_in_text(&description),
        submission_status: normalized_status,
        submission_date_text: submission_date.as_deref().map(normalize_date_text),
        submission_date_timestamp: submission_date.as_deref().and_then(parse_date_timestamp),
        grading_status: grading_status.as_deref().map(normalize_dates_in_text),
        grade_text: grade.as_deref().map(normalize_dates_in_text),
        feedback_text: feedback.as_deref().map(normalize_dates_in_text),
        submitted_files,
        attachments,
        can_submit,
        can_finalize,
        is_draft,
    };
    if detail.description.is_empty()
        && detail.submission_status.is_empty()
        && detail.grade_text.is_none()
        && submission_context.is_none()
        && body_text.is_empty()
    {
        return Err(Error::Malformed("phyvlab assignment detail"));
    }
    Ok(PhyVlabAssignmentPage {
        detail,
        submission_context,
        edit_submission_url,
    })
}

pub fn merge_assignment_details(
    original: PhyVlabAssignmentDetail,
    secondary: PhyVlabAssignmentDetail,
) -> PhyVlabAssignmentDetail {
    PhyVlabAssignmentDetail {
        description: if secondary.description.is_empty() {
            original.description
        } else {
            secondary.description
        },
        submission_status: if secondary.submission_status.is_empty() {
            original.submission_status
        } else {
            secondary.submission_status
        },
        submission_date_text: secondary
            .submission_date_text
            .or(original.submission_date_text),
        submission_date_timestamp: secondary
            .submission_date_timestamp
            .or(original.submission_date_timestamp),
        grading_status: secondary.grading_status.or(original.grading_status),
        grade_text: secondary.grade_text.or(original.grade_text),
        feedback_text: secondary.feedback_text.or(original.feedback_text),
        submitted_files: if secondary.submitted_files.is_empty() {
            original.submitted_files
        } else {
            secondary.submitted_files
        },
        attachments: if secondary.attachments.is_empty() {
            original.attachments
        } else {
            secondary.attachments
        },
        can_submit: secondary.can_submit || original.can_submit,
        can_finalize: secondary.can_finalize || original.can_finalize,
        is_draft: secondary.is_draft || original.is_draft,
    }
}

fn parse_description_attachments(el: ElementRef<'_>, origin: &str) -> Vec<PhyVlabAttachment> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let a_sel = sel("a[href*='/pluginfile.php/']");
    let img_sel = sel("img[src*='/pluginfile.php/']");
    let candidates = el
        .select(&a_sel)
        .filter_map(|a| Some((a.value().attr("href")?, elem_text(a))))
        .chain(el.select(&img_sel).filter_map(|img| {
            Some((
                img.value().attr("src")?,
                clean_text(img.value().attr("alt").unwrap_or_default()),
            ))
        }));
    let prefix = format!("{origin}/pluginfile.php/");
    for (raw, label) in candidates {
        let stripped = raw.split('#').next().unwrap_or(raw).trim();
        let url = if stripped.starts_with('/') {
            format!("{origin}{stripped}")
        } else {
            stripped.to_owned()
        };
        if !url.starts_with(&prefix) || url.contains("assignsubmission") {
            continue;
        }
        if !seen.insert(url.clone()) {
            continue;
        }
        let leaf = url
            .split('?')
            .next()
            .unwrap_or(&url)
            .rsplit('/')
            .next()
            .unwrap_or("");
        let decoded = percent_decode(leaf);
        let file_name = if !decoded.is_empty() {
            decoded
        } else if !label.is_empty() {
            label
        } else {
            "附件".into()
        };
        out.push(PhyVlabAttachment { file_name, url });
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_owned())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn first_detail_text(doc: &Html, selector: &str) -> Option<String> {
    doc.select(&sel(selector))
        .map(elem_text)
        .find(|s| !s.is_empty())
}

fn parse_sesskey_from_html(html: &str) -> Option<String> {
    let re = Regex::new(r#"["']sesskey["']\s*[:=]\s*["']([^"']+)["']"#).unwrap();
    re.captures(html)
        .map(|c| c[1].trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn parse_filemanager_metadata(
    doc: &Html,
    field_name: Option<&str>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let scripts: Vec<_> = doc
        .select(&sel("script"))
        .map(|s| s.inner_html())
        .filter(|s| {
            field_name.is_none_or(|name| s.to_lowercase().contains(&name.to_lowercase()))
                || s.to_lowercase().contains("filemanager")
        })
        .collect();
    if scripts.is_empty() {
        return (None, None, None, None);
    }
    let joined = scripts.join("\n");
    (
        parse_js_scalar(&joined, &["itemid", "itemId"]),
        parse_js_scalar(
            &joined,
            &["contextid", "context_id", "contextId", "context"],
        ),
        parse_js_scalar(&joined, &["client_id", "clientId"]),
        parse_repository_id(&joined),
    )
}

fn parse_js_scalar(text: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        let pat = format!(
            r#"(?i)(?:["']{}["']|{})\s*[:=]\s*(?:["']([^"']*)["']|([A-Za-z0-9_.:/-]+))"#,
            regex::escape(key),
            regex::escape(key)
        );
        let re = Regex::new(&pat).ok()?;
        if let Some(caps) = re.captures(text) {
            let val = caps
                .get(1)
                .map(|m| m.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| caps.get(2).map(|m| m.as_str()))
                .unwrap_or("")
                .trim();
            if !val.is_empty() {
                return Some(val.to_owned());
            }
        }
    }
    None
}

fn parse_repository_id(text: &str) -> Option<String> {
    if let Some(id) = parse_js_scalar(text, &["repositoryid", "repository_id", "repo_id"]) {
        return Some(id);
    }
    let start = Regex::new(r#"(?i)["']?repositories["']?\s*[:=]"#)
        .unwrap()
        .find(text)?
        .end();
    let snippet: String = text[start..].chars().take(12000).collect();
    let re1 = Regex::new(
        r#"(?is)\{[^{}]{0,900}?(?:["']?id["']?\s*[:=]\s*["']?(\d+)["']?)[^{}]{0,900}?(?:["']?type["']?\s*[:=]\s*["']?upload["']?)"#,
    )
    .unwrap();
    if let Some(c) = re1.captures(&snippet) {
        return Some(c[1].to_string());
    }
    let re2 = Regex::new(
        r#"(?is)\{[^{}]{0,900}?(?:["']?type["']?\s*[:=]\s*["']?upload["']?)[^{}]{0,900}?(?:["']?id["']?\s*[:=]\s*["']?(\d+)["']?)"#,
    )
    .unwrap();
    if let Some(c) = re2.captures(&snippet) {
        return Some(c[1].to_string());
    }
    let re3 = Regex::new(
        r#"(?is)(?:["'](\d+)["']|\b(\d+))\s*[:=]\s*\{[^{}]{0,900}?(?:["']?type["']?\s*[:=]\s*["']?upload["']?)"#,
    )
    .unwrap();
    re3.captures(&snippet)
        .and_then(|c| c.get(1).or_else(|| c.get(2)).map(|m| m.as_str().to_owned()))
}

fn extract_moodle_json(text: &str, start: usize) -> Option<Value> {
    let rel_begin = text[start..].find('{')?;
    let begin = start + rel_begin;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, ch) in text[begin..].char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
        } else {
            match ch {
                '"' => quoted = true,
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let slice = &text[begin..=begin + offset];
                        return serde_json::from_str(slice).ok();
                    }
                }
                _ => {}
            }
        }
    }
    None
}

pub fn parse_moodle_edit_context(
    html: &str,
    origin: &str,
    activity_id: i64,
) -> Result<PhyVlabEditContext> {
    let doc = Html::parse_document(html);
    let origin_clean = origin.trim_end_matches('/');
    let assign_url = format!("{origin_clean}/mod/assign/view.php?id={activity_id}");
    let form = doc
        .select(&sel("form"))
        .find(|f| {
            f.select(&sel("input[name*='filemanager']"))
                .next()
                .is_some()
        })
        .ok_or(Error::Rejected("此作业已锁定或暂不允许上传"))?;
    let mut fields = BTreeMap::new();
    for input in form.select(&sel("input[type=hidden][name]")) {
        if let Some(name) = input.value().attr("name") {
            fields.insert(
                name.to_owned(),
                input.value().attr("value").unwrap_or_default().to_owned(),
            );
        }
    }
    if fields.get("id").map(String::as_str) != Some(&activity_id.to_string())
        || fields.get("action").map(String::as_str) != Some("savesubmission")
    {
        return Err(Error::Malformed("assignment submission context"));
    }
    let manager = form
        .select(&sel("input[name*='filemanager']"))
        .next()
        .ok_or(Error::Malformed("filemanager input missing"))?;
    let manager_field = manager
        .value()
        .attr("name")
        .filter(|s| !s.is_empty())
        .ok_or(Error::Malformed("filemanager field name"))?
        .to_owned();
    let item_id = manager
        .value()
        .attr("value")
        .filter(|s| !s.is_empty())
        .ok_or(Error::Malformed("draft item id"))?
        .to_owned();
    let scripts: Vec<_> = doc.select(&sel("script")).map(|s| s.inner_html()).collect();
    let init_script = scripts
        .iter()
        .find(|s| s.contains("M.form_filemanager.init"))
        .ok_or(Error::Malformed("filemanager init script missing"))?;
    let start = init_script
        .find("M.form_filemanager.init")
        .ok_or(Error::Malformed("filemanager init offset"))?;
    let config = extract_moodle_json(init_script, start)
        .ok_or(Error::Malformed("filemanager config JSON"))?;
    let joined_scripts = scripts.join("\n");
    let repository_id = parse_repository_id(&joined_scripts)
        .ok_or(Error::Malformed("upload repository missing"))?;
    let context_id = config["context"]["id"]
        .as_str()
        .or_else(|| config["context"]["id"].as_i64().map(|_| ""))
        .and_then(|_| parse::scalar(&config["context"]["id"]))
        .ok_or(Error::Malformed("filemanager context id"))?;
    let client_id = parse::scalar(&config["client_id"])
        .filter(|s| !s.is_empty())
        .ok_or(Error::Malformed("filemanager client id"))?;
    let files = config["list"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            let name = raw["fullname"]
                .as_str()
                .or_else(|| raw["filename"].as_str())?
                .to_owned();
            let path = raw["filepath"].as_str().unwrap_or("/").to_owned();
            Some(PhyVlabAssignmentFile { name, path })
        })
        .collect();
    let max_files = parse::scalar(&config["maxfiles"])
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(-1);
    let max_bytes = parse::scalar(&config["maxbytes"])
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(-1);
    let accepted_types = config["accepted_types"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let url = form
        .value()
        .attr("action")
        .and_then(|a| resolve_origin_url(&assign_url, a))
        .unwrap_or(assign_url);
    let sesskey = fields
        .get("sesskey")
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .ok_or(Error::Malformed("assignment sesskey"))?;
    Ok(PhyVlabEditContext {
        url,
        fields,
        sesskey,
        item_id,
        context_id,
        client_id,
        repository_id,
        manager_field,
        files,
        max_files,
        max_bytes,
        accepted_types,
    })
}

pub fn parse_finalization_form(
    html: &str,
    origin: &str,
    activity_id: i64,
) -> Result<PhyVlabFinalizationForm> {
    let doc = Html::parse_document(html);
    let origin_clean = origin.trim_end_matches('/');
    let base_url = format!("{origin_clean}/mod/assign/view.php?id={activity_id}");
    if doc
        .select(&sel("input[name=action][value=confirmsubmit]"))
        .next()
        .is_none()
    {
        return Err(Error::Malformed("finalization confirmation page missing"));
    }
    let form = doc
        .select(&sel("form"))
        .find(|f| {
            f.select(&sel("input[name=action][value=confirmsubmit]"))
                .next()
                .is_some()
        })
        .ok_or(Error::Malformed("finalization form missing"))?;
    let mut fields = BTreeMap::new();
    for input in form.select(&sel("input[type=hidden][name]")) {
        if let Some(name) = input.value().attr("name") {
            fields.insert(
                name.to_owned(),
                input.value().attr("value").unwrap_or_default().to_owned(),
            );
        }
    }
    if fields.get("id").map(String::as_str) != Some(&activity_id.to_string())
        || fields.get("sesskey").is_none_or(|s| s.trim().is_empty())
    {
        return Err(Error::Malformed("finalization context mismatch"));
    }
    let statement = form
        .select(&sel("input[type=checkbox][name]"))
        .next()
        .and_then(|checkbox| {
            let name = checkbox.value().attr("name")?;
            if name != "submissionstatement" {
                return Some(Err(Error::Malformed(
                    "unexpected checkbox in finalization form",
                )));
            }
            let val = checkbox.value().attr("value").unwrap_or("1");
            fields.insert(
                name.to_owned(),
                if val.trim().is_empty() { "1" } else { val }.to_owned(),
            );
            let label = checkbox
                .parent()
                .and_then(ElementRef::wrap)
                .map(elem_text)
                .filter(|s| !s.is_empty());
            Some(label.ok_or(Error::Malformed("submission statement missing")))
        })
        .transpose()?;
    if let Some(submit) = form
        .select(&sel("input[type=submit][name]:not([name=cancel])"))
        .next()
        && let Some(name) = submit.value().attr("name")
    {
        fields.insert(
            name.to_owned(),
            submit.value().attr("value").unwrap_or_default().to_owned(),
        );
    }
    let url = form
        .value()
        .attr("action")
        .and_then(|a| resolve_origin_url(&base_url, a))
        .unwrap_or(base_url);
    Ok(PhyVlabFinalizationForm {
        url,
        fields,
        statement,
    })
}

pub fn parse_events(html: &str, origin: &str) -> Vec<PhyVlabEvent> {
    let doc = Html::parse_document(html);
    let origin_clean = origin.trim_end_matches('/');
    let mut seen = BTreeSet::new();
    let mut events = Vec::new();
    for cell in doc.select(&sel("td.day.hasevent")) {
        let Some(day_ts) = cell
            .value()
            .attr("data-day-timestamp")
            .and_then(|s| s.parse::<i64>().ok())
        else {
            continue;
        };
        let raw_title = cell
            .value()
            .attr("data-title")
            .unwrap_or_default()
            .split(" 事件")
            .next()
            .unwrap_or_default();
        let date_text = clean_text(&weekday_regex().replace_all(raw_title, ""));
        for (idx, item) in cell.select(&sel("[data-region='event-item']")).enumerate() {
            let Some(link) = item.select(&sel("a[data-action='view-event']")).next() else {
                continue;
            };
            let title = link
                .select(&sel(".eventname"))
                .next()
                .map(elem_text)
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let id = link
                .value()
                .attr("data-event-id")
                .filter(|s| !s.is_empty())
                .or_else(|| item.value().attr("data-event-id").filter(|s| !s.is_empty()))
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{day_ts}-{idx}"));
            if !seen.insert(id.clone()) {
                continue;
            }
            let event_url = link
                .value()
                .attr("href")
                .and_then(|h| resolve_origin_url(origin_clean, h));
            events.push(PhyVlabEvent {
                id,
                title,
                date_text: date_text.clone(),
                day_timestamp: day_ts,
                event_url,
                submitted: false,
            });
        }
    }
    events.sort_by(|a, b| {
        a.day_timestamp
            .cmp(&b.day_timestamp)
            .then_with(|| a.title.cmp(&b.title))
    });
    events
}

fn status_indicates_submission(value: &str) -> bool {
    let status = value.trim().to_ascii_lowercase();
    status.contains("已提交") || (status.contains("submitted") && !status.contains("not submitted"))
}

pub fn detail_has_submission(detail: &PhyVlabAssignmentDetail) -> bool {
    let status = detail.submission_status.trim().to_ascii_lowercase();
    if detail.is_draft || detail.can_finalize || status.contains("draft") || status.contains("草稿")
    {
        return false;
    }
    !detail.submitted_files.is_empty() || status_indicates_submission(&detail.submission_status)
}

pub fn detail_submission_state(detail: &PhyVlabAssignmentDetail) -> Option<bool> {
    let status = detail.submission_status.trim().to_ascii_lowercase();
    if detail.is_draft || detail.can_finalize || status.contains("draft") || status.contains("草稿")
    {
        return Some(false);
    }
    if detail_has_submission(detail) {
        return Some(true);
    }
    if status.contains("未提交") || status.contains("not submitted") || status.contains("没有提交")
    {
        return Some(false);
    }
    None
}

pub fn submission_status_label(detail: &PhyVlabAssignmentDetail) -> &str {
    if detail.is_draft || detail.can_finalize {
        return "草稿 · 待最终提交";
    }
    if !detail_has_submission(detail) {
        return "未提交";
    }
    let status = detail.submission_status.trim();
    if status_indicates_submission(status) {
        status
    } else {
        "已提交"
    }
}

pub fn submission_timing_label(activity: &PhyVlabActivity) -> Option<&'static str> {
    let detail = activity.detail.as_ref()?;
    if !detail_has_submission(detail) {
        return None;
    }
    let submitted_at = detail.submission_date_timestamp?;
    let due_at = activity.due_timestamp?;
    Some(if submitted_at > due_at {
        "逾期提交"
    } else {
        "按时提交"
    })
}

pub fn activity_status_label(activity: &PhyVlabActivity, now: i64) -> &'static str {
    if let Some(detail) = &activity.detail {
        if detail.is_draft || detail.can_finalize {
            return "草稿 · 待最终提交";
        }
        if detail_has_submission(detail) {
            return match (activity.due_timestamp, detail.submission_date_timestamp) {
                (Some(due), Some(sub)) if sub > due => "逾期提交",
                _ => "已提交",
            };
        }
    }
    if activity.completed {
        return "已提交";
    }
    match activity.due_timestamp {
        None => "未知",
        Some(due) if now < due => "待提交",
        Some(_) => "已逾期",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://phyvlab.bjtu.edu.cn";

    #[test]
    fn parses_courses_cards_and_legacy_fallback() {
        let cards_html = r#"
        <div data-region="course-content" data-course-id="72">
          <a class="aalink coursename" href="https://phyvlab.bjtu.edu.cn/course/view.php?id=72">
            <span class="multiline">大学物理I_(2026春)</span>
          </a>
          <span class="categoryname">自然科学</span>
          <div class="progress-text"><span>8</span>%</div>
        </div>
        <div data-region="course-content" data-course-id="74">
          <a class="aalink coursename" href="/course/view.php?id=74">
            <span class="multiline">物理实验I_(2026春)</span>
          </a>
          <span class="categoryname">物理实验</span>
          <div class="progress-text">已完成 30%</div>
        </div>
        "#;
        let courses = parse_courses(cards_html, ORIGIN).unwrap();
        assert_eq!(courses.len(), 2);
        assert_eq!(courses[0].id, 72);
        assert_eq!(courses[0].progress_percent, 8);
        assert_eq!(
            courses[1].course_url,
            "https://phyvlab.bjtu.edu.cn/course/view.php?id=74"
        );
        assert_eq!(courses[1].progress_percent, 30);

        assert!(
            parse_courses("<html><body>尚未选课</body></html>", ORIGIN)
                .unwrap()
                .is_empty()
        );

        let legacy_html = r#"
        <nav>
          <a class="list-group-item" href="https://phyvlab.bjtu.edu.cn/course/view.php?id=72">大学物理I_(2026春)</a>
          <a class="dropdown-item" title="大学物理I_(2026春)" href="https://phyvlab.bjtu.edu.cn/course/view.php?id=72">收藏此课程</a>
        </nav>
        "#;
        let legacy = parse_courses(legacy_html, ORIGIN).unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].name, "大学物理I_(2026春)");
    }

    #[test]
    fn parses_activities_and_normalizes_english_weekdays() {
        let html = r#"
        <li id="module-3689" class="activity modtype_assign">
          <a class="aalink" href="/mod/assign/view.php?id=3689">
            <span class="instancename">chap1 <span class="accesshide">作业</span></span>
          </a>
          <div data-region="activity-dates">
            <div><strong>打开：</strong>2026年03月12日 Thursday 00:00</div>
            <div><strong>到期日：</strong>2026年03月19日 Thursday 00:00</div>
          </div>
          <button data-action="toggle-manual-completion" data-toggletype="manual:undo"></button>
        </li>
        "#;
        let list = parse_activities(html, ORIGIN, 72, "大学物理I").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "chap1");
        assert_eq!(list[0].open_text.as_deref(), Some("2026年03月12日 00:00"));
        assert_eq!(list[0].due_text.as_deref(), Some("2026年03月19日 00:00"));
        assert!(list[0].due_timestamp.is_some());
        assert!(list[0].completed);
    }

    #[test]
    fn parses_assignment_detail_attachments_and_filemanager_context() {
        let activity = PhyVlabActivity {
            id: 3700,
            course_id: 72,
            course_name: "大学物理I".into(),
            title: "Chap-2-3".into(),
            activity_type: "作业".into(),
            activity_url: "https://phyvlab.bjtu.edu.cn/mod/assign/view.php?id=3700".into(),
            open_text: None,
            open_timestamp: None,
            due_text: None,
            due_timestamp: None,
            completed: false,
            detail: None,
        };
        let html = r#"
        <main>
          <div id="intro">
            <p>第一行<br><strong>重点</strong></p>
            <a href="/pluginfile.php/100/mod_assign/intro/guide%201.pdf">说明.pdf</a>
            <a href="/pluginfile.php/101/assignsubmission_file/submission_files/mine.pdf">mine.pdf</a>
            <script>bad()</script>
          </div>
          <div data-region="activity-dates">
            <div><strong>打开：</strong>2026年09月16日 Wednesday 00:00</div>
          </div>
          <div class="submissionstatustable"><table>
            <tr><th>作业状态</th><td>尚未批改</td></tr>
          </table></div>
          <form action="/mod/assign/view.php?id=3700&amp;action=editsubmission" method="post">
            <input type="hidden" name="sesskey" value="abc123">
            <input type="hidden" name="id" value="3700">
            <input type="hidden" name="files_filemanager" value="42">
            <div class="filemanager"></div>
          </form>
          <script>
            M.form_filemanager.init({
              "id": "id_files_filemanager",
              "itemid": 42,
              "contextid": 17,
              "client_id": "client-3700",
              "repositories": {
                "1": {"type": "recent"},
                "4": {"type": "upload"}
              }
            });
          </script>
        </main>
        "#;
        let parsed = parse_assignment_page(html, ORIGIN, &activity).unwrap();
        assert_eq!(
            parsed.detail.description,
            "第一行\n重点\n说明.pdf\nmine.pdf"
        );
        assert!(!parsed.detail.description.contains("bad()"));
        assert_eq!(parsed.detail.attachments.len(), 1);
        assert_eq!(parsed.detail.attachments[0].file_name, "guide 1.pdf");
        assert!(parsed.detail.submission_date_text.is_none());
        assert!(parsed.detail.can_submit);
        let ctx = parsed.submission_context.unwrap();
        assert_eq!(ctx.draft_item_id.as_deref(), Some("42"));
        assert_eq!(ctx.context_id.as_deref(), Some("17"));
        assert_eq!(ctx.client_id.as_deref(), Some("client-3700"));
        assert_eq!(ctx.repository_id.as_deref(), Some("4"));
    }

    #[test]
    fn login_form_and_captcha_detection_rejects_foreign_actions() {
        let html = r#"
        <form id="login" action="/login/index.php" method="post">
          <input type="hidden" name="logintoken" value="tok-1">
          <input name="username">
          <input name="password" type="password">
          <div id="aliyun-captcha-element"></div>
        </form>
        <script>initAlicom4({captchaId: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'})</script>
        "#;
        let form = parse_login_form(html, ORIGIN).unwrap();
        assert!(form.requires_captcha);
        assert_eq!(
            form.captcha_id.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        let foreign = html.replace("/login/index.php", "https://evil.example/login");
        assert!(matches!(
            parse_login_form(&foreign, ORIGIN),
            Err(Error::SecureChannel)
        ));
    }
}
