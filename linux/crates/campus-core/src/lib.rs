//! Pure BJTU domain rules; no filesystem, runtime, credentials or network access.
pub mod calendar;
pub mod capture;
pub mod parse;
pub mod phyvlab;
pub mod update;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("malformed response: {0}")]
    Malformed(&'static str),
    #[error("session expired; run campus auth login")]
    SessionExpired,
    #[error("network request failed")]
    Network,
    #[error("secure channel or redirect rejected")]
    SecureChannel,
    #[error("smart platform redirected to its legacy HTTP service")]
    SmartLegacyHttp,
    #[error("request rejected: {0}")]
    Rejected(&'static str),
    #[error("user action required: {0}")]
    ActionRequired(&'static str),
    #[error("local storage: {0}")]
    Storage(String),
    #[error("no cached data")]
    NoData,
    #[error("no cached data; last sync: {0}")]
    CacheUnavailable(String),
    #[error("another sync is running")]
    Busy,
    #[error("invalid argument: {0}")]
    Usage(String),
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::NoData | Self::CacheUnavailable(_) => 2,
            Self::SessionExpired | Self::Rejected(_) | Self::ActionRequired(_) => 3,
            Self::Usage(_) => 64,
            Self::Network => 69,
            Self::Busy => 75,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Course {
    pub course_code: String,
    pub name: String,
    pub teacher: String,
    pub location_index: u8,
    pub weeks_raw: String,
    pub weeks_mask: u32,
    pub place: String,
    pub is_selection: bool,
}
/// The two school schedules can contain the same meeting. Keep the official record
/// when the course, time, location and teaching weeks all match.
pub fn normalize_schedule(rows: Vec<Course>) -> Vec<Course> {
    use std::collections::HashMap;
    use std::collections::hash_map::Entry;
    let mut positions = HashMap::new();
    let mut merged: Vec<Course> = Vec::with_capacity(rows.len());
    for mut row in rows {
        let name = row.name.trim();
        let name = ["[本]", "（本）", "(本)", "【本】"]
            .iter()
            .find_map(|suffix| name.strip_suffix(suffix))
            .unwrap_or(name)
            .trim_end();
        row.name = name.to_owned();
        let key = (
            row.course_code.trim().to_owned(),
            row.name.clone(),
            row.location_index,
            row.weeks_mask,
            if row.weeks_mask == 0 {
                row.weeks_raw.trim().to_owned()
            } else {
                String::new()
            },
            row.place.trim().to_owned(),
        );
        match positions.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(merged.len());
                merged.push(row);
            }
            Entry::Occupied(entry) => {
                let kept = &mut merged[*entry.get()];
                if kept.is_selection && !row.is_selection {
                    std::mem::swap(kept, &mut row);
                }
                if (kept.teacher.trim().is_empty() || kept.teacher == "?")
                    && !row.teacher.trim().is_empty()
                {
                    kept.teacher = row.teacher;
                }
            }
        }
    }
    merged
}

impl Course {
    pub fn weekday(&self) -> u8 {
        self.location_index % 8
    }
    pub fn slot(&self) -> u8 {
        self.location_index / 8
    }
    pub fn in_week(&self, week: u8) -> bool {
        week == 0 || week < 32 && self.weeks_mask & (1 << week) != 0
    }
    pub fn time_range(&self, staggered: bool) -> Option<(u16, u16)> {
        let (mut a, mut b) = *SLOTS.get(self.slot() as usize)?;
        if staggered && self.slot() == 1 && is_staggered(&self.place) {
            a += 20;
            b += 20;
        }
        Some((a, b))
    }
}
pub const SLOTS: [(u16, u16); 7] = [
    (480, 590),
    (610, 720),
    (730, 840),
    (850, 960),
    (980, 1090),
    (1140, 1250),
    (1260, 1310),
];
pub fn is_staggered(place: &str) -> bool {
    place.contains("逸夫教学楼")
        || place.contains("思源西楼")
        || regex::Regex::new(r"(?i)(^|[^a-z0-9])(YF|SX)\s*\d")
            .unwrap()
            .is_match(place)
}
pub fn weeks(raw: &str) -> Vec<u8> {
    let s = raw.replace(['第', '周'], "");
    let mut result = Vec::new();
    for part in s.split(',') {
        let bits: Vec<_> = part.split('-').collect();
        let Ok(a) = bits[0].parse::<u8>() else {
            return Vec::new();
        };
        if a == 0 || a > 30 || bits.len() > 2 {
            return Vec::new();
        }
        let b = if bits.len() == 2 {
            match bits[1].parse::<u8>() {
                Ok(b) if b <= 30 => b,
                _ => return Vec::new(),
            }
        } else {
            a
        };
        result.extend(a..=b);
    }
    result
}
pub fn week_mask(raw: &str) -> u32 {
    weeks(raw).iter().fold(0, |a, w| a | 1u32 << w)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Grade {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub index: usize,
    pub course_name: String,
    pub course_code: Option<String>,
    pub teacher: String,
    pub term: String,
    pub score_raw: String,
    pub score_letter: Option<String>,
    pub score_value: Option<f64>,
    pub credits_raw: String,
    pub credits: Option<f64>,
    pub detail: String,
}
fn is_zero(n: &usize) -> bool {
    *n == 0
}
// Minimum percentage, letter, grade points, and the existing representative
// percentage used for letter-only records.
const SCORE_BANDS: [(f64, &str, f64, f64); 11] = [
    (90., "A", 4.0, 95.),
    (85., "A-", 3.7, 87.),
    (81., "B+", 3.3, 83.),
    (78., "B", 3.0, 79.),
    (75., "B-", 2.7, 76.),
    (71., "C+", 2.3, 73.),
    (68., "C", 2.0, 69.),
    (65., "C-", 1.7, 66.),
    (61., "D+", 1.3, 63.),
    (60., "D", 1.0, 60.),
    (0., "F", 0.0, 30.),
];

fn score_band(raw: &str) -> Option<(&'static str, f64, f64)> {
    let raw = raw.trim();
    if let Ok(number) = raw.parse::<f64>() {
        if !(0. ..=100.).contains(&number) {
            return None;
        }
        return SCORE_BANDS
            .iter()
            .find(|(minimum, ..)| number >= *minimum)
            .map(|(_, letter, points, _)| (*letter, number, *points));
    }
    SCORE_BANDS
        .iter()
        .find(|(_, letter, ..)| raw.eq_ignore_ascii_case(letter))
        .map(|(_, letter, points, representative)| (*letter, *representative, *points))
}

pub fn score(raw: &str) -> (Option<String>, Option<f64>) {
    score_band(raw).map_or((None, None), |(letter, value, _)| {
        (Some(letter.into()), Some(value))
    })
}

#[derive(Debug, Default)]
pub struct GradeSummary {
    pub weighted_average: Option<f64>,
    pub gpa: Option<f64>,
    pub credits: f64,
    pub courses: usize,
    pub estimated_average: bool,
    pub ambiguous_failures: usize,
}

pub fn summarize_grades<'a>(grades: impl IntoIterator<Item = &'a Grade>) -> GradeSummary {
    let mut result = GradeSummary::default();
    let (mut sum, mut points_sum) = (0., 0.);
    for grade in grades {
        let raw = grade.score_raw.trim();
        let Some(credits) = grade.credits.filter(|&c| c.is_finite() && c > 0.) else {
            continue;
        };
        // A bare F is also used by the two-level P/F system. The source
        // doesn't carry a grading scheme that would distinguish these cases.
        if raw.eq_ignore_ascii_case("F") {
            result.ambiguous_failures += 1;
            continue;
        }
        let Some((_, value, points)) = score_band(raw) else {
            continue;
        };
        result.estimated_average |= raw.parse::<f64>().is_err();
        result.credits += credits;
        result.courses += 1;
        sum += value * credits;
        points_sum += points * credits;
    }
    if result.courses > 0 {
        result.weighted_average = Some(sum / result.credits);
        result.gpa = Some(points_sum / result.credits);
    }
    result
}

pub fn weighted_average(grades: &[Grade]) -> Option<f64> {
    summarize_grades(grades).weighted_average
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Exam {
    pub exam_type: String,
    pub course_name: String,
    pub time_place_raw: String,
    pub status: String,
    pub detail: String,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub location: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub student_id: String,
    pub name: String,
    pub identity: String,
    pub department: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Card {
    pub card_balance: String,
    pub net_fee: String,
    pub new_mail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcademicWeek {
    pub term: String,
    pub week: u8,
    pub monday: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Semester {
    pub id: String,
    pub label: String,
    pub is_current: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Room {
    pub room: String,
    pub capacity: u32,
    pub cells: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub kind: String,
    pub item_key: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}
/// Identity includes an occurrence number, so duplicate rows are never lost.
pub fn detect_changes(old: &[Value], new: &[Value], key: impl Fn(&Value) -> String) -> Vec<Change> {
    fn index(
        rows: &[Value],
        key: &impl Fn(&Value) -> String,
    ) -> std::collections::BTreeMap<String, Value> {
        let mut counts = std::collections::HashMap::new();
        rows.iter()
            .map(|v| {
                let k = key(v);
                let n = counts.entry(k.clone()).or_insert(0);
                *n += 1;
                (format!("{k}#{n}"), v.clone())
            })
            .collect()
    }
    let a = index(old, &key);
    let b = index(new, &key);
    let mut changes = Vec::new();
    for (k, v) in &a {
        match b.get(k) {
            None => changes.push(Change {
                kind: "deleted".into(),
                item_key: k.clone(),
                before: Some(v.clone()),
                after: None,
            }),
            Some(w) if w != v => changes.push(Change {
                kind: "modified".into(),
                item_key: k.clone(),
                before: Some(v.clone()),
                after: Some(w.clone()),
            }),
            _ => {}
        }
    }
    for (k, v) in &b {
        if !a.contains_key(k) {
            changes.push(Change {
                kind: "added".into(),
                item_key: k.clone(),
                before: None,
                after: Some(v.clone()),
            });
        }
    }
    changes
}
pub fn identity(dataset: &str, v: &Value) -> String {
    let keys: &[&str] = match dataset {
        "courses" => &["course_code", "location_index", "is_selection"],
        "grades" => &["course_name", "teacher", "term"],
        "exams" => &["course_name", "exam_type"],
        "calendar" => &["term", "week"],
        "rooms" => &["room"],
        "phyvlab" => &["course_id", "id"],
        _ => &["id"],
    };
    serde_json::to_string(
        &keys
            .iter()
            .map(|k| v.get(k).cloned().unwrap_or(Value::Null))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
    Expired,
    Missing,
}
pub fn freshness(dataset: &str, fetched_at: Option<i64>, now: i64) -> Freshness {
    let Some(t) = fetched_at else {
        return Freshness::Missing;
    };
    let (fresh, usable) = match dataset {
        "courses" => (86400, 14 * 86400),
        "grades" => (21600, 30 * 86400),
        "exams" => (21600, 7 * 86400),
        "card" => (600, 7 * 86400),
        "calendar" => (7 * 86400, 180 * 86400),
        "rooms" => (21600, 14 * 86400),
        "homework" => (1800, 7 * 86400),
        "phyvlab" | "phyvlab_courses" => (1800, 7 * 86400),
        "courseware" => (86400, 30 * 86400),
        _ => (30 * 86400, i64::MAX),
    };
    let age = now.saturating_sub(t).max(0);
    if age <= fresh {
        Freshness::Fresh
    } else if age <= usable {
        Freshness::Stale
    } else {
        Freshness::Expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_grade_points_and_exclusions() {
        for (minimum, letter, points, _) in SCORE_BANDS {
            let (_, _, got_points) = score_band(&minimum.to_string()).unwrap();
            assert_eq!(got_points, points);
            assert_eq!(score_band(letter).unwrap().2, points);
        }
        assert_eq!(score_band("59.99").unwrap().2, 0.);
        assert_eq!(score_band("89.5").unwrap().2, 3.7);
        assert!(score_band("NaN").is_none());
        assert!(score_band("105").is_none());
        let grade = |score_raw: &str, credits| Grade {
            index: 0,
            course_name: String::new(),
            course_code: None,
            teacher: String::new(),
            term: String::new(),
            score_raw: score_raw.into(),
            score_letter: None,
            score_value: None,
            credits_raw: String::new(),
            credits: Some(credits),
            detail: String::new(),
        };
        let values = [
            grade("90", 2.),
            grade("B+", 1.),
            grade("58", 1.),
            grade("P", 2.),
            grade("F", 3.),
            grade("A", 0.),
        ];
        let summary = summarize_grades(&values);
        assert_eq!(summary.courses, 3);
        assert_eq!(summary.credits, 4.);
        assert_eq!(summary.ambiguous_failures, 1);
        assert!(summary.estimated_average);
        assert_eq!(summary.weighted_average, Some(80.25));
        assert!((summary.gpa.unwrap() - 2.825).abs() < 1e-12);
        assert_eq!(summarize_grades(&values[3..]).gpa, None);
    }
    #[test]
    fn upstream_weeks() {
        assert_eq!(weeks("第1-3周,第5周"), vec![1, 2, 3, 5]);
        assert!(weeks("第1周,坏").is_empty());
        assert_eq!(week_mask("第30周"), 1 << 30);
    }
    #[test]
    fn duplicate_changes() {
        let a = vec![
            serde_json::json!({"id":1,"x":1}),
            serde_json::json!({"id":1,"x":2}),
        ];
        let mut b = a.clone();
        b[1]["x"] = 3.into();
        let c = detect_changes(&a, &b, |v| v["id"].to_string());
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, "modified");
    }
    #[test]
    fn freshness_boundaries() {
        for d in [
            "courses",
            "grades",
            "exams",
            "card",
            "calendar",
            "rooms",
            "homework",
            "phyvlab",
            "phyvlab_courses",
            "courseware",
        ] {
            assert_eq!(freshness(d, None, 0), Freshness::Missing);
            assert_eq!(freshness(d, Some(0), 0), Freshness::Fresh);
            assert_eq!(freshness(d, Some(0), 1000 * 86400), Freshness::Expired);
        }
        assert_eq!(freshness("card", Some(0), 601), Freshness::Stale);
    }
    proptest::proptest! { #[test] fn weeks_never_panic(s in ".*") {let w=weeks(&s);proptest::prop_assert!(w.iter().all(|w| (1..=30).contains(w)));} }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn merges_same_meeting_from_both_sources_without_losing_distinct_meetings() {
        let official = Course {
            course_code: "C100 01".into(),
            name: "合成课程 [本]".into(),
            teacher: "测试教师".into(),
            location_index: 1,
            weeks_raw: "第01-02周".into(),
            weeks_mask: week_mask("第01-02周"),
            place: "测试楼,101".into(),
            is_selection: false,
        };
        let mut selection = official.clone();
        selection.name = "合成课程".into();
        selection.weeks_raw = "第1-2周".into();
        selection.is_selection = true;
        let mut different_weeks = selection.clone();
        different_weeks.weeks_mask = week_mask("第3-4周");
        different_weeks.weeks_raw = "第3-4周".into();
        let mut different_place = selection.clone();
        different_place.place = "测试楼,102".into();
        let mut different_slot = selection.clone();
        different_slot.location_index = 9;
        let mut different_section = selection.clone();
        different_section.course_code = "C100 02".into();
        let merged = normalize_schedule(vec![
            selection,
            official,
            different_weeks,
            different_place,
            different_slot,
            different_section,
        ]);
        assert_eq!(merged.len(), 5);
        assert_eq!(merged[0].name, "合成课程");
        assert_eq!(merged[0].weeks_raw, "第01-02周");
        assert!(!merged[0].is_selection);
    }
}
