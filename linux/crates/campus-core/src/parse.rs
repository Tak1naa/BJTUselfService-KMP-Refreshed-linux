//! Selectors ported from the original (unsanitized) Kotlin parsers.
use crate::*;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use std::collections::BTreeMap;
fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("constant CSS selector")
}
fn text(e: ElementRef<'_>) -> String {
    e.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn compact(e: ElementRef<'_>) -> String {
    text(e).chars().filter(|c| !c.is_whitespace()).collect()
}
fn required<'a>(html: &'a Html, s: &str) -> Result<ElementRef<'a>> {
    html.select(&sel(s))
        .next()
        .ok_or(Error::Malformed("required element missing"))
}
pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
pub fn homework_status(row: &Value) -> &'static str {
    if row["subStatus"] == "已提交"
        || scalar(&row["snId"]).is_some_and(|id| {
            let id = id.trim();
            !matches!(id, "" | "0" | "null")
        })
    {
        "已提交"
    } else if row["subStatus"] == "未提交" {
        "未提交"
    } else {
        "未知"
    }
}

pub fn homework_text(raw: &str) -> String {
    let source = raw.replace(r"\<", "<").replace(r"\>", ">");
    let html = Html::parse_fragment(&source);
    let mut text = String::new();
    for node in html.root_element().descendants() {
        if let Some(element) = node.value().as_element() {
            if matches!(
                element.name(),
                "br" | "p" | "div" | "li" | "tr" | "blockquote" | "pre"
            ) {
                text.push('\n');
            }
        } else if let Some(value) = node.value().as_text() {
            if node.ancestors().any(|ancestor| {
                ancestor
                    .value()
                    .as_element()
                    .is_some_and(|e| matches!(e.name(), "script" | "style"))
            }) {
                continue;
            }
            text.extend(value.chars().filter(|&c| c == '\n' || !c.is_control()));
        }
    }
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn platform_user_id(html: &str) -> Result<String> {
    Regex::new(r#"var\s+uId\s*=\s*["'](\d{1,18})["']"#)
        .unwrap()
        .captures(html)
        .map(|m| m[1].to_string())
        .ok_or(Error::Malformed("platform user ID"))
}

pub fn program_links(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let re = Regex::new(r"stuview/(\d+)/?").unwrap();
    let mut links = Vec::new();
    for e in doc.select(&sel("a[href*=stuview]")) {
        if let Some(m) = e.value().attr("href").and_then(|v| re.captures(v)) {
            let path = format!("training/training/program/stuview/{}/", &m[1]);
            if !links.contains(&path) {
                links.push(path);
            }
        }
    }
    links
}

pub fn course_types(html: &str) -> Result<Vec<Value>> {
    let doc = Html::parse_document(html);
    let table = doc
        .select(&sel("table"))
        .find(|t| t.select(&sel("th,td")).any(|e| compact(e) == "课程性质"))
        .ok_or(Error::Malformed("program course table"))?;
    let code = Regex::new(r"^[A-Z]\d{3}[A-Z0-9]{4,5}$").unwrap();
    let mut carry: BTreeMap<usize, (String, usize)> = BTreeMap::new();
    let mut result = BTreeMap::new();
    for row in table.select(&sel("tr")) {
        carry.retain(|_, (_, n)| *n > 0);
        let mut labels: Vec<_> = carry
            .iter()
            .map(|(c, (label, _))| (*c, label.clone()))
            .collect();
        for (_, n) in carry.values_mut() {
            *n -= 1;
        }
        let cells: Vec<_> = row.select(&sel("td")).collect();
        let mut column = 0;
        for cell in &cells {
            while carry.contains_key(&column) {
                column += 1;
            }
            let text = compact(*cell);
            let rows = cell
                .value()
                .attr("rowspan")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1);
            if rows > 1 && text.contains('【') {
                carry.insert(column, (text.clone(), rows - 1));
                labels.push((column, text));
            }
            column += cell
                .value()
                .attr("colspan")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
        }
        let Some(i) = cells.iter().position(|e| code.is_match(&compact(*e))) else {
            continue;
        };
        let Some(kind) = cells.get(i + 1).map(|e| compact(*e)) else {
            continue;
        };
        labels.sort_by_key(|v| v.0);
        let kind = if labels.last().is_some_and(|(_, s)| s.contains("体育")) {
            "pe"
        } else {
            match kind.as_str() {
                "必修" => "required",
                "限选" => "limited",
                "任选" => "elective",
                "体育" => "pe",
                _ => continue,
            }
        };
        result.insert(compact(cells[i]), kind);
    }
    Ok(result
        .into_iter()
        .map(|(code, kind)| serde_json::json!({"course_code":code,"course_type":kind}))
        .collect())
}
#[derive(Clone)]
pub struct LoginPage {
    pub fields: BTreeMap<String, String>,
    pub sm2_public_key: String,
    pub error_code: Option<u32>,
    pub requires_captcha: bool,
}
// No Debug implementation: the page contains sensitive hidden fields.
pub fn login(html: &str) -> Result<LoginPage> {
    let doc = Html::parse_document(html);
    let form = required(&doc, "form#loginForm")?;
    let fields: BTreeMap<_, _> = form
        .select(&sel("input[name]"))
        .filter(|e| e.value().attr("type") != Some("checkbox"))
        .map(|e| {
            (
                e.value().attr("name").unwrap().into(),
                e.value().attr("value").unwrap_or("").into(),
            )
        })
        .collect();
    if fields.get("flowId").is_none_or(|v: &String| v.is_empty())
        || !fields.contains_key("username")
        || !fields.contains_key("password")
    {
        return Err(Error::Malformed("login form"));
    }
    let sm2 = Regex::new(r#""sm2"\s*:\s*(\{[^{}]*\})"#)
        .unwrap()
        .captures(html)
        .and_then(|c| serde_json::from_str::<Value>(&c[1]).ok());
    let enabled = sm2
        .as_ref()
        .and_then(|v| v["enabled"].as_bool())
        .unwrap_or(false);
    let key = if enabled {
        sm2.and_then(|v| v["publicKey"].as_str().map(String::from))
            .filter(|s| !s.is_empty())
            .ok_or(Error::Malformed("sm2 public key"))?
    } else {
        String::new()
    };
    // Read a balanced JSON object, rather than stopping at nested captcha data.
    let err = html
        .find("var error")
        .and_then(|p| html[p..].find('=').map(|n| p + n + 1))
        .and_then(|p| {
            serde_json::Deserializer::from_str(html[p..].trim_start())
                .into_iter::<Value>()
                .next()
        })
        .and_then(|v| v.ok());
    let code = err
        .as_ref()
        .and_then(|e| scalar(&e["code"]))
        .and_then(|s| s.parse().ok());
    let captcha = code.is_some_and(|c| (41100..41200).contains(&c))
        || err
            .as_ref()
            .and_then(|e| e["data"]["captchaType"].as_str())
            .is_some_and(|s| !s.is_empty());
    Ok(LoginPage {
        fields,
        sm2_public_key: key,
        error_code: code,
        requires_captcha: captcha,
    })
}
pub fn profile(html: &str, student_id: &str) -> Result<Profile> {
    let doc = Html::parse_document(html);
    let name = text(required(&doc, ".name_right > h3 > a")?)
        .split('，')
        .next()
        .unwrap_or("")
        .to_string();
    let spans: Vec<_> = doc
        .select(&sel(".name_right .nr_con span"))
        .map(text)
        .collect();
    let field = |prefix: &str| -> Result<String> {
        spans
            .iter()
            .find(|s| s.contains(prefix))
            .map(|s| {
                s.trim_start_matches(prefix)
                    .trim_start_matches(['：', ':'])
                    .to_owned()
            })
            .filter(|s| !s.is_empty())
            .ok_or(Error::Malformed("profile field"))
    };
    if name.is_empty() {
        return Err(Error::Malformed("profile name"));
    }
    Ok(Profile {
        student_id: student_id.into(),
        name,
        identity: field("身份")?,
        department: field("部门")?,
    })
}
pub fn redirect(html: &str) -> Result<String> {
    let doc = Html::parse_document(html);
    required(&doc, "form#redirect")?
        .value()
        .attr("action")
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or(Error::Malformed("academic redirect"))
}
pub fn teachers(html: &str) -> Result<BTreeMap<String, String>> {
    let doc = Html::parse_document(html);
    let t = required(&doc, "table")?;
    let rows: Vec<_> = t.select(&sel("tr")).collect();
    if rows.len() < 2 {
        return Err(Error::Malformed("teacher grid"));
    }
    Ok(rows
        .iter()
        .skip(2)
        .filter_map(|r| {
            let c: Vec<_> = r.select(&sel("td")).collect();
            if c.len() < 2 {
                return None;
            }
            let raw = text(c[0]);
            let parts: Vec<_> = raw.split_whitespace().collect();
            let key = if parts.len() == 3 {
                parts[..2].join(" ")
            } else {
                raw
            };
            let value = text(c[1]);
            (!key.is_empty() && !value.is_empty()).then_some((key, value))
        })
        .collect())
}
pub fn courses(
    html: &str,
    selection: bool,
    teachers: &BTreeMap<String, String>,
) -> Result<Vec<Course>> {
    let doc = Html::parse_document(html);
    let table = required(&doc, "table")?;
    let rows: Vec<_> = table.select(&sel("tr")).skip(1).collect();
    if rows.len() < 7 {
        return Err(Error::Malformed("schedule rows"));
    }
    let br = Regex::new(r"(?i)<br\s*/?>").unwrap();
    let mut out = Vec::new();
    for (slot, r) in rows.iter().take(7).enumerate() {
        let cells: Vec<_> = r.select(&sel("td")).collect();
        if cells.len() < 8 {
            return Err(Error::Malformed("schedule columns"));
        }
        for (day, cell) in cells.iter().skip(1).take(7).enumerate() {
            for child in cell.children().filter_map(ElementRef::wrap) {
                let raw = if selection {
                    child
                        .select(&sel("span"))
                        .next()
                        .map(|e| e.inner_html())
                        .unwrap_or_default()
                } else {
                    child.inner_html()
                };
                let pieces: Vec<_> = br.splitn(&raw, 2).collect();
                if pieces.len() < 2 {
                    continue;
                }
                let code = Html::parse_fragment(pieces[0])
                    .root_element()
                    .text()
                    .collect::<String>()
                    .trim()
                    .to_string();
                let fragment = Html::parse_fragment(pieces[1]);
                let name = if selection {
                    text(fragment.root_element())
                } else {
                    fragment
                        .select(&sel("span"))
                        .next()
                        .map(text)
                        .unwrap_or_default()
                };
                if code.is_empty() || name.is_empty() {
                    continue;
                }
                let info = child.select(&sel("div[style^=max-width]")).next();
                let info_text = info.map(text).unwrap_or_default();
                let weeks_raw = info_text
                    .find('周')
                    .map(|p| {
                        info_text[..p + 3]
                            .chars()
                            .filter(|c| !c.is_whitespace())
                            .collect::<String>()
                    })
                    .unwrap_or_else(|| "未知".into());
                let embedded = info
                    .and_then(|e| e.select(&sel("i")).next())
                    .map(text)
                    .unwrap_or_default();
                let section = code
                    .split_whitespace()
                    .nth(1)
                    .map(|s| s.chars().skip(1).take(2).collect::<String>())
                    .unwrap_or_default();
                let key = format!(
                    "{}{}{}",
                    name.split(' ').next().unwrap_or(""),
                    if section.is_empty() { "" } else { " " },
                    section
                );
                let teacher = if embedded.is_empty() {
                    teachers.get(&key).cloned().unwrap_or_else(|| "?".into())
                } else {
                    embedded
                };
                let place = child
                    .select(&sel("span.text-muted"))
                    .next()
                    .map(compact)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "未知".into());
                out.push(Course {
                    course_code: code,
                    name,
                    teacher,
                    location_index: (slot * 8 + day + 1) as u8,
                    weeks_mask: week_mask(&weeks_raw),
                    weeks_raw,
                    place,
                    is_selection: selection,
                });
            }
        }
    }
    Ok(out)
}
pub fn grades(html: &str) -> Result<Vec<Grade>> {
    let doc = Html::parse_document(html);
    let table = required(&doc, "table")?;
    let mut out = Vec::new();
    let breaks_re = Regex::new(r"(?i)<br\s*/?>|</(?:p|div|li|tr)\s*>").unwrap();
    let code_re = Regex::new(r"^[A-Z]\d{3}[A-Z0-9]{4,5}").unwrap();
    for row in table.select(&sel("tr")).skip(1) {
        let c: Vec<_> = row.select(&sel("td")).collect();
        if c.len() < 8 {
            return Err(Error::Malformed("grade row"));
        }
        let term = compact(c[1]);
        let name = compact(c[2]);
        if term.is_empty() || name.is_empty() {
            return Err(Error::Malformed("grade identity"));
        }
        let raw = compact(c[4]);
        let (letter, value) = score(&raw);
        let credits_raw = compact(c[3]);
        let credits_raw = if credits_raw.is_empty() {
            "0.0".into()
        } else {
            credits_raw
        };
        let detail = c[7]
            .select(&sel("span[data-content]"))
            .next()
            .and_then(|e| e.value().attr("data-content"))
            .unwrap_or("");
        let breaks = breaks_re.replace_all(detail, "\n").into_owned();
        let detail = breaks
            .lines()
            .map(|s| {
                Html::parse_fragment(s)
                    .root_element()
                    .text()
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let code = code_re.find(&name).map(|m| m.as_str().into());
        out.push(Grade {
            index: 0,
            course_name: name,
            course_code: code,
            teacher: compact(c[6]),
            term,
            score_raw: raw,
            score_letter: letter,
            score_value: value,
            credits: credits_raw.parse().ok(),
            credits_raw,
            detail,
        });
    }
    Ok(out)
}
pub fn exams(html: &str) -> Result<Vec<Exam>> {
    let doc = Html::parse_document(html);
    let body = required(&doc, "tbody")?;
    let mut out = Vec::new();
    for r in body.select(&sel("tr")) {
        let c: Vec<_> = r.select(&sel("td")).collect();
        if c.len() < 6 {
            return Err(Error::Malformed("exam row"));
        }
        let raw = text(c[3]);
        let times = crate::calendar::parse_exam(&raw);
        out.push(Exam {
            exam_type: text(c[1]),
            course_name: text(c[2]),
            time_place_raw: raw,
            status: text(c[4]),
            detail: text(c[5]),
            starts_at: times.as_ref().map(|v| v.0),
            ends_at: times.as_ref().map(|v| v.1),
            location: times.map(|v| v.2),
        });
    }
    Ok(out)
}
pub fn card(body: &str) -> Result<Card> {
    let v: Value = serde_json::from_str(body).map_err(|_| Error::Malformed("card JSON"))?;
    let get = |k| {
        scalar(&v[k])
            .filter(|s| !s.is_empty())
            .ok_or(Error::Malformed("card field"))
    };
    Ok(Card {
        card_balance: get("ecard_yuer")?,
        net_fee: get("net_fee")?,
        new_mail: get("newmail_count")?,
    })
}
pub fn rooms(html: &str) -> Result<Vec<Room>> {
    let doc = Html::parse_document(html);
    let table = required(&doc, "table")?;
    let mut out = Vec::new();
    let cap = Regex::new(r"\((\d+)\)").unwrap();
    let color = Regex::new(r"background-color:\s*(#[0-9a-fA-F]{3,6})").unwrap();
    let pos = Regex::new(r"星期([一二三四五六日天1-7])\s*第(\d+)节").unwrap();
    for r in table.select(&sel("tr")) {
        if r.select(&sel("th")).next().is_some() {
            continue;
        }
        let c: Vec<_> = r.select(&sel("td")).collect();
        if c.is_empty() {
            continue;
        }
        if c.len() < 50 {
            return Err(Error::Malformed("room row"));
        }
        let header = text(c[0]);
        let room = header.split([' ', '(']).next().unwrap_or("").into();
        let capacity = cap
            .captures(&header)
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(0);
        let mut cells = vec![255; 49];
        for (i, cell) in c.iter().skip(1).take(49).enumerate() {
            let at = pos
                .captures(cell.value().attr("title").unwrap_or(""))
                .and_then(|m| {
                    let day = match &m[1] {
                        "一" | "1" => 1,
                        "二" | "2" => 2,
                        "三" | "3" => 3,
                        "四" | "4" => 4,
                        "五" | "5" => 5,
                        "六" | "6" => 6,
                        _ => 7,
                    };
                    let period = m[2].parse::<usize>().ok()?;
                    (1..=7)
                        .contains(&period)
                        .then_some((day - 1) * 7 + period - 1)
                })
                .unwrap_or(i);
            let style = cell.value().attr("style").unwrap_or("");
            let kind = match color
                .captures(style)
                .map(|m| m[1].to_lowercase())
                .as_deref()
            {
                None | Some("#fff" | "#ffffff") => 0,
                Some("#e46868") => 1,
                Some("#9e6868") => 2,
                Some("#394ed6") => 3,
                Some("#77bf6d") => 4,
                Some("#d8cc56") => 5,
                _ => 255,
            };
            cells[at] = kind;
        }
        out.push(Room {
            room,
            capacity,
            cells,
        });
    }
    Ok(out)
}
pub fn semesters(html: &str) -> Result<Vec<Semester>> {
    let doc = Html::parse_document(html);
    let list = required(&doc, "select[name=zxjxjhh]")?;
    let mut all: Vec<Semester> = list
        .select(&sel("option"))
        .filter_map(|e| {
            Some(Semester {
                id: e.value().attr("value")?.into(),
                label: text(e),
                is_current: e.value().attr("selected").is_some(),
            })
        })
        .filter(|s| !s.id.is_empty() && !s.label.is_empty())
        .collect();
    if !all.iter().any(|s| s.is_current) {
        let re = Regex::new(r#"\[name\s*=\s*zxjxjhh\][^)]*\)\.val\(\s*["']([^"']+)["']"#).unwrap();
        if let Some(m) = re.captures(html)
            && let Some(s) = all.iter_mut().find(|s| s.id == m[1])
        {
            s.is_current = true;
        }
    }
    Ok(all)
}
pub fn academic_weeks(html: &str) -> Result<Vec<AcademicWeek>> {
    let doc = Html::parse_document(html);
    let inputs: BTreeMap<_, _> = doc
        .select(&sel("input"))
        .filter_map(|e| {
            Some((
                e.value()
                    .attr("name")
                    .or_else(|| e.value().attr("id"))?
                    .to_string(),
                e.value().attr("value")?.to_string(),
            ))
        })
        .collect();
    let payload = inputs
        .get("hidJson")
        .ok_or(Error::Malformed("calendar hidJson"))?;
    let decoded = url::form_urlencoded::parse(format!("x={payload}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .ok_or(Error::Malformed("calendar encoding"))?;
    let v: Vec<Value> =
        serde_json::from_str(&decoded).map_err(|_| Error::Malformed("calendar JSON"))?;
    let title = Regex::new(r"第(一|二)学期\s*[（(]\s*(\d{4})-(\d{4})\s*学年\s*[)）]").unwrap();
    let week = Regex::new(r"第(\d+)(教学)?周").unwrap();
    let date = Regex::new(r"/Date\((\d+)[^)]*\)/").unwrap();
    let mut out = Vec::new();
    for item in v {
        let Some(t) = scalar(&item["Id"]).and_then(|id| inputs.get(&format!("hidTitle_{id}")))
        else {
            continue;
        };
        let Some(t) = title.captures(t) else {
            continue;
        };
        let spring = &t[1] == "二";
        let term = format!("{}-{}-{}", &t[2], &t[3], if spring { 2 } else { 1 });
        for row in item["Json"].as_array().into_iter().flatten() {
            let Some(w) = row["Week"].as_str().and_then(|s| week.captures(s)) else {
                continue;
            };
            if w.get(2).is_none() && !spring {
                continue;
            }
            let Some(n) = w[1]
                .parse::<u8>()
                .ok()
                .and_then(|n| n.checked_add(if w.get(2).is_none() { 18 } else { 0 }))
                .filter(|n| (1..=30).contains(n))
            else {
                continue;
            };
            let Some(ms) = row["DT"]
                .as_str()
                .and_then(|s| date.captures(s))
                .and_then(|m| m[1].parse::<i64>().ok())
            else {
                continue;
            };
            let ts = jiff::Timestamp::from_millisecond(ms)
                .map_err(|_| Error::Malformed("calendar date"))?;
            let monday = ts
                .to_zoned(jiff::tz::TimeZone::fixed(
                    jiff::tz::Offset::from_hours(8).unwrap(),
                ))
                .date()
                .to_string();
            out.push(AcademicWeek {
                term: term.clone(),
                week: n,
                monday,
            });
        }
    }
    Ok(out)
}
pub fn live_rooms(body: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(body).map_err(|_| Error::Malformed("headcount JSON"))?;
    if !v["data"].is_array() || !v["time"].is_array() {
        return Err(Error::Malformed("headcount fields"));
    }
    Ok(v)
}
pub fn json_object(body: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(body).map_err(|_| Error::Malformed("JSON"))?;
    if !v.is_object() {
        return Err(Error::Malformed("JSON object"));
    }
    Ok(v)
}
pub fn session_expired(status: u16, url: &str, html: &str) -> bool {
    if status == 401 || status == 403 {
        return true;
    }
    if url::Url::parse(url).is_ok_and(|u| {
        [
            "/sso/login",
            "/o/authorize/",
            "/auth/login",
            "/accounts/login",
        ]
        .iter()
        .any(|p| u.path().contains(p))
    }) {
        return true;
    }
    if [
        "会话已过期",
        "登录已过期",
        "请重新登录",
        "登录超时",
        "Session expired",
    ]
    .iter()
    .any(|k| html.contains(k))
    {
        return true;
    }
    let doc = Html::parse_document(html);
    doc.select(&sel(
        "form input[type=password], form input[name=loginname], form input[name=captcha]",
    ))
    .next()
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn homework_details_decode_markup_and_keep_lines() {
        let content = r"\<p>题目 1：A &lt; B\</p><p>题目 2<br/>下一行</p><p>&#x20;&nbsp;<br/></p><script>ignored</script>";
        assert_eq!(homework_text(content), "题目 1：A < B\n题目 2\n下一行");
        assert_eq!(
            homework_status(&serde_json::json!({"subStatus":"未提交"})),
            "未提交"
        );
        assert_eq!(homework_status(&serde_json::json!({"snId": 42})), "已提交");
        assert_eq!(homework_status(&serde_json::json!({"snId": 0})), "未知");
    }
    #[test]
    fn rejects_login_page_as_grade() {
        assert!(grades("<form><input type=password></form>").is_err());
        assert!(session_expired(
            200,
            "https://aa.example/data",
            "<form><input type=password></form>"
        ));
    }
    #[test]
    fn card_number_or_string() {
        assert_eq!(
            card(r#"{"ecard_yuer":"12.3","net_fee":4,"newmail_count":0}"#)
                .unwrap()
                .new_mail,
            "0"
        );
    }
    #[test]
    fn nested_login_error() {
        let p=login("<form id=loginForm><input name=flowId value=abc><input name=username><input name=password></form><script>var error = {\"code\":41100,\"data\":{\"captchaType\":\"image\"}};\n</script>").unwrap();
        assert!(p.requires_captcha);
    }
}
