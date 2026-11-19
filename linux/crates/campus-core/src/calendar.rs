use crate::*;
use jiff::{
    civil::Date,
    tz::{Offset, TimeZone},
};
pub fn school_zone() -> TimeZone {
    TimeZone::fixed(Offset::from_hours(8).unwrap())
}
pub fn parse_exam(value: &str) -> Option<(i64, i64, String)> {
    let date_re = regex::Regex::new(
        r"([0-9]{4})\s*(?:[-/.]|年)\s*([0-9]{1,2})\s*(?:[-/.]|月)\s*([0-9]{1,2})\s*日?",
    )
    .unwrap();
    let clock = regex::Regex::new(r"([0-9]{1,2})\s*[:：]\s*([0-9]{2})").unwrap();
    let found = date_re.captures(value)?;
    let date = Date::new(
        found[1].parse().ok()?,
        found[2].parse().ok()?,
        found[3].parse().ok()?,
    )
    .ok()?;
    let tail = &value[found.get(0)?.end()..];
    let times: Vec<_> = clock
        .captures_iter(tail)
        .take(2)
        .filter_map(|m| {
            let h = m[1].parse::<i8>().ok()?;
            let min = m[2].parse::<i8>().ok()?;
            ((0..24).contains(&h) && (0..60).contains(&min)).then_some((h, min))
        })
        .collect();
    let &(hour, minute) = times.first()?;
    let start = date
        .at(hour, minute, 0, 0)
        .to_zoned(school_zone())
        .ok()?
        .timestamp()
        .as_second();
    let end = if let Some((h, m)) = times.get(1) {
        let t = date
            .at(*h, *m, 0, 0)
            .to_zoned(school_zone())
            .ok()?
            .timestamp()
            .as_second();
        if t <= start { t + 86400 } else { t }
    } else {
        start + 7200
    };
    let location = clock.replace_all(tail, " ");
    let joiner = regex::Regex::new(r"\s*(?:-|—|~|～|至)\s*").unwrap();
    let location = joiner.replace_all(&location, " ");
    let weekday = regex::Regex::new(r"[（(]\s*(?:星期|周)?[一二三四五六日天]\s*[)）]").unwrap();
    let location = weekday.replace_all(&location, " ");
    let location = location
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches([' ', '-', '—', '~', '～', '至', '，', ','])
        .to_string();
    Some((start, end, location))
}
pub fn current_week(weeks: &[AcademicWeek], now: i64) -> Option<(String, u8)> {
    let today = jiff::Timestamp::from_second(now)
        .ok()?
        .to_zoned(school_zone())
        .date();
    weeks
        .iter()
        .filter_map(|w| {
            let start = w.monday.parse::<Date>().ok()?;
            let end = start.checked_add(jiff::Span::new().days(7)).ok()?;
            (today >= start && today < end).then(|| (w.term.clone(), w.week))
        })
        .next()
}
/// Anchor the calendar to the cached course snapshot, including between teaching weeks.
pub fn term_at(weeks: &[AcademicWeek], fetched_at: i64) -> Option<&str> {
    let date = jiff::Timestamp::from_second(fetched_at)
        .ok()?
        .to_zoned(school_zone())
        .date();
    weeks
        .iter()
        .filter_map(|w| Some((w.monday.parse::<Date>().ok()?, w.term.as_str())))
        .filter(|(monday, _)| *monday <= date)
        .max_by_key(|(monday, _)| *monday)
        .map(|(_, term)| term)
}
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
        .replace('\r', "")
}
pub fn fold(s: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for c in s.chars() {
        if count + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            count = 1;
        }
        out.push(c);
        count += c.len_utf8();
    }
    out.push_str("\r\n");
    out
}
/// Emit each actual occurrence; explicit dates preserve holiday / shifted week mapping.
pub fn ics(
    courses: &[Course],
    exams: &[Exam],
    weeks: &[AcademicWeek],
    range: std::ops::RangeInclusive<u8>,
    now: i64,
) -> Result<String> {
    let mut lines = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//BJTU campus//Linux CLI//ZH".into(),
        "CALSCALE:GREGORIAN".into(),
    ];
    let stamp = jiff::Timestamp::from_second(now)
        .map_err(|_| Error::Malformed("timestamp"))?
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string();
    for (i, c) in courses.iter().enumerate() {
        for w in weeks
            .iter()
            .filter(|w| range.contains(&w.week) && c.in_week(w.week))
        {
            let monday = w
                .monday
                .parse::<Date>()
                .map_err(|_| Error::Malformed("week date"))?;
            let date = monday
                .checked_add(jiff::Span::new().days(c.weekday() as i64 - 1))
                .map_err(|_| Error::Malformed("course date"))?;
            let (a, b) = c.time_range(true).ok_or(Error::Malformed("course slot"))?;
            let ts = |minute: u16| -> Result<String> {
                let z = date
                    .at((minute / 60) as i8, (minute % 60) as i8, 0, 0)
                    .to_zoned(school_zone())
                    .map_err(|_| Error::Malformed("course time"))?;
                Ok(z.timestamp().strftime("%Y%m%dT%H%M%SZ").to_string())
            };
            lines.extend([
                "BEGIN:VEVENT".into(),
                format!("UID:c{i}-{}-{}@campus.bjtu", w.term, w.week),
                format!("DTSTAMP:{stamp}"),
                format!("DTSTART:{}", ts(a)?),
                format!("DTEND:{}", ts(b)?),
                format!("SUMMARY:{}", escape(&c.name)),
                format!("LOCATION:{}", escape(&c.place)),
                format!("DESCRIPTION:{}", escape(&c.teacher)),
                "END:VEVENT".into(),
            ]);
        }
    }
    for (i, e) in exams.iter().enumerate() {
        if let Some(start) = e.starts_at {
            let ts = |t| {
                jiff::Timestamp::from_second(t)
                    .map(|v| v.strftime("%Y%m%dT%H%M%SZ").to_string())
                    .map_err(|_| Error::Malformed("exam timestamp"))
            };
            lines.extend([
                "BEGIN:VEVENT".into(),
                format!("UID:exam{i}-{start}@campus.bjtu"),
                format!("DTSTAMP:{stamp}"),
                format!("DTSTART:{}", ts(start)?),
                format!("DTEND:{}", ts(e.ends_at.unwrap_or(start + 7200))?),
                format!("SUMMARY:{}", escape(&format!("考试：{}", e.course_name))),
                format!("LOCATION:{}", escape(e.location.as_deref().unwrap_or(""))),
                "END:VEVENT".into(),
            ]);
        }
    }
    lines.push("END:VCALENDAR".into());
    Ok(lines.iter().map(|s| fold(s)).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    proptest::proptest! {#[test]fn folded_utf8_valid(s in ".*"){let v=fold(&s.replace(['\n','\r'],""));for l in v.split("\r\n"){proptest::prop_assert!(l.len()<=75);} }}
    #[test]
    fn escaped() {
        assert_eq!(escape("a,b;c\n"), "a\\,b\\;c\\n");
    }
}
