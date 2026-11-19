use campus_core::{parse, week_mask};
use std::collections::BTreeMap;
fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name),
    )
    .unwrap()
}
#[test]
fn upstream_course_examples() {
    let teachers = parse::teachers(&fixture("teachers.html")).unwrap();
    let a = parse::courses(&fixture("courses.html"), false, &teachers).unwrap();
    let b = parse::courses(&fixture("selection.html"), true, &teachers).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].location_index, 1);
    assert_eq!(a[0].name, "离散数学");
    assert_eq!(a[0].teacher, "张老师");
    assert_eq!(a[0].weeks_mask, week_mask("第1-3,5周"));
    assert_eq!(a[0].place, "思源101");
    assert_eq!(b[0].location_index, 10);
    assert_eq!(b[0].teacher, "李老师");
    assert!(b[0].is_selection);
}
#[test]
fn numeric_and_letter_grades() {
    let g = parse::grades(&fixture("grades.html")).unwrap();
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].term, "2025-2026-1");
    assert_eq!(g[0].score_value, Some(95.));
    assert_eq!(g[0].teacher, "张老师");
    assert_eq!(g[0].detail, "平时成绩：90\n期末成绩：95");
    assert_eq!(g[1].score_value, Some(83.));
    assert_eq!(g[1].credits, Some(0.));
}
#[test]
fn profile_and_login() {
    assert_eq!(
        parse::profile(&fixture("profile.html"), "fixture-user")
            .unwrap()
            .name,
        "测试用户"
    );
    assert!(
        parse::login(&fixture("login.html"))
            .unwrap()
            .fields
            .contains_key("flowId")
    );
}
#[test]
fn room_colors_and_positions() {
    let r = parse::rooms(&fixture("rooms.html")).unwrap();
    assert_eq!(r[0].cells.len(), 49);
    assert_eq!(r[0].cells[0], 1);
    assert_eq!(r[0].cells[1], 0);
    assert_eq!(r[0].capacity, 90);
}
#[test]
fn academic_calendar() {
    let w = parse::academic_weeks(&fixture("calendar.html")).unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].term, "2026-2027-1");
    assert_eq!(w[0].week, 1);
}
#[test]
fn exams_card_headcount() {
    let e = parse::exams(&fixture("exams.html")).unwrap();
    assert!(e[0].starts_at.is_some());
    assert_eq!(e[0].ends_at.unwrap() - e[0].starts_at.unwrap(), 7200);
    assert_eq!(
        parse::card(&fixture("card.json")).unwrap().card_balance,
        "18.50"
    );
    assert_eq!(
        parse::live_rooms(&fixture("live.json")).unwrap()["data"][0][3],
        90
    );
}
#[test]
fn malformed_data_never_becomes_empty_success() {
    assert!(parse::grades("<table><tr></tr><tr><td>bad</td></tr></table>").is_err());
    assert!(parse::courses("<table></table>", false, &BTreeMap::new()).is_err());
    assert!(parse::rooms("<table><tr><td>bad</td></tr></table>").is_err());
    assert!(parse::exams("<html>login</html>").is_err());
    assert!(parse::academic_weeks("<html>bad</html>").is_err());
}
#[test]
fn sports_group_carry() {
    let types = parse::course_types(&fixture("program.html")).unwrap();
    assert_eq!(types[0]["course_type"], "pe");
    assert_eq!(types[1]["course_type"], "pe");
    assert_eq!(types[2]["course_type"], "limited");
}
