use campus_core::{Profile, parse};
use campus_db::{Database, paths::Paths};
use serde_json::{Value, json};
use std::process::Command;
struct Fixture {
    temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            data: temp.path().join("data/campus-portal"),
            config: temp.path().join("config/campus-portal"),
            cache: temp.path().join("cache/campus-portal"),
            state: temp.path().join("state/campus-portal"),
            runtime: temp.path().join("runtime/campus-portal"),
        };
        let mut db = Database::open(&paths.database()).unwrap();
        let id = db
            .save_account(
                &Profile {
                    student_id: "fixture-user".into(),
                    name: "测试用户".into(),
                    identity: "学生".into(),
                    department: "测试学院".into(),
                },
                1,
            )
            .unwrap();
        db.set_pref("", "default_account", Some("fixture-user"))
            .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let text = |name| std::fs::read_to_string(root.join(name)).unwrap();
        let grades = parse::grades(&text("grades.html"))
            .unwrap()
            .into_iter()
            .map(|v| serde_json::to_value(v).unwrap())
            .collect::<Vec<_>>();
        db.save(id, "grades", &grades, jiff::Timestamp::now().as_second())
            .unwrap();
        let card = parse::card(&text("card.json")).unwrap();
        db.save(id, "card", &[json!(card)], 1).unwrap();
        db.save(id, "exams", &[], 1).unwrap();
        db.save(
            id,
            "grade_types",
            &[json!({"course_code":"C312009B","course_type":"required"})],
            1,
        )
        .unwrap();
        Self { temp }
    }
    fn cmd(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("campus"));
        cmd.env("HOME", self.temp.path())
            .env("NO_COLOR", "1")
            .env("CAMPUS_PORTAL_ENDPOINTS_MIS", "http://invalid.example/");
        for (env, dir) in [
            ("XDG_DATA_HOME", "data"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            cmd.env(env, self.temp.path().join(dir));
        }
        cmd
    }
    fn json(&self, args: &[&str]) -> Value {
        let out = self.cmd().arg("--json").args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}
#[test]
fn offline_json_has_contract_without_network_or_keyring() {
    let f = Fixture::new();
    let v = f.json(&["grades", "--offline"]);
    assert_eq!(v["schema"], 1);
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    assert_eq!(v["freshness"]["source"], "cache");
    assert_eq!(v["data"][0]["teacher"], "张老师");
}
#[test]
fn selection_survives_a_new_process_and_grade_type_filter_works() {
    let f = Fixture::new();
    assert_eq!(
        f.json(&["grades", "--offline", "select", "add", "1"])["data"]["selected"],
        1
    );
    assert_eq!(
        f.json(&["grades", "--offline", "gpa", "--selected"])["data"]["weighted_average"],
        95.
    );
    assert_eq!(
        f.json(&["grades", "--offline", "--type", "required"])["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn empty_success_differs_from_missing_cache() {
    let f = Fixture::new();
    assert!(
        f.json(&["exams", "--offline", "--upcoming"])["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let out = f
        .cmd()
        .args(["--json", "schedule", "--offline"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], 2);
}
#[test]
fn missing_homework_cache_preserves_last_sync_failure() {
    let f = Fixture::new();
    let db = Database::open(&f.temp.path().join("data/campus-portal/campus.db")).unwrap();
    db.failure(
        db.account("fixture-user").unwrap(),
        "homework",
        "failed",
        "secure_channel",
        1,
    )
    .unwrap();
    let out = f.cmd().args(["homework", "--offline"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let text = String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("上次同步失败") && text.contains("HTTPS"));
    assert!(text.contains("campus sync status"));
    let out = f
        .cmd()
        .args(["homework", "--offline", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["error"]["reason"], "secure_channel");
    assert!(out.stderr.is_empty());

    db.failure(
        db.account("fixture-user").unwrap(),
        "courseware",
        "failed",
        "smart_legacy_http",
        2,
    )
    .unwrap();
    let out = f.cmd().args(["courseware", "--offline"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let text = String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("campus config set courseware.legacy_http on"));
}

#[test]
fn table_color_and_machine_formats_are_independent() {
    let f = Fixture::new();
    let run = |extra: &[&str]| {
        f.cmd()
            .env_remove("NO_COLOR")
            .env("COLORTERM", "truecolor")
            .env("TERM", "xterm-256color")
            .args(["grades", "--offline", "--color", "always"])
            .args(extra)
            .output()
            .unwrap()
            .stdout
    };
    let table = String::from_utf8(run(&[])).unwrap();
    assert!(table.contains("\x1b[") && table.contains("课程") && table.contains('│'));
    let tsv = String::from_utf8(run(&["--format", "tsv"])).unwrap();
    assert!(tsv.starts_with("index\tcourse_name\t") && !tsv.contains('\x1b'));
    let json = run(&["--json"]);
    assert!(!json.contains(&27));
    assert_eq!(serde_json::from_slice::<Value>(&json).unwrap()["schema"], 1);
}

#[test]
fn ics_export_keeps_courses_in_the_cached_term() {
    let f = Fixture::new();
    let mut db = Database::open(&f.temp.path().join("data/campus-portal/campus.db")).unwrap();
    let id = db.account("fixture-user").unwrap();
    let fetched = "2026-03-04T08:00:00+08:00"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_second();
    db.save(
        0,
        "calendar",
        &[
            json!({"term":"2025-2026-1","week":1,"monday":"2025-09-01"}),
            json!({"term":"2025-2026-2","week":1,"monday":"2026-03-02"}),
            json!({"term":"2026-2027-1","week":1,"monday":"2026-09-07"}),
        ],
        fetched,
    )
    .unwrap();
    db.save(
        id,
        "courses",
        &[
            json!({"course_code":"C001","name":"操作系统","teacher":"测试","location_index":1,
            "weeks_raw":"第1周","weeks_mask":2,"place":"SY101","is_selection":false}),
        ],
        fetched,
    )
    .unwrap();
    let out = f
        .cmd()
        .args(["export", "ics", "--weeks", "1-1", "--no-exams", "--offline"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out.stderr);
    let ics = String::from_utf8(out.stdout).unwrap();
    assert_eq!(ics.matches("BEGIN:VEVENT").count(), 1);
    assert!(ics.contains("DTSTART:20260302T000000Z\r\n"));
    assert!(ics.contains("SUMMARY:操作系统\r\n"));
    let wrapped = f.json(&["export", "ics", "--weeks", "1-1", "--no-exams", "--offline"]);
    assert!(wrapped["data"].as_str().unwrap().contains("BEGIN:VEVENT"));
}
#[test]
fn help_completions_status_and_usage() {
    let f = Fixture::new();
    for args in [
        vec!["--help"],
        vec!["auth", "--help"],
        vec!["sync", "--help"],
        vec!["grades", "--help"],
        vec!["completions", "bash"],
    ] {
        assert!(f.cmd().args(args).output().unwrap().status.success());
    }
    assert!(f.json(&["sync", "status"])["data"].is_array());
    let bad = f.cmd().arg("--unknown").output().unwrap();
    assert_eq!(bad.status.code(), Some(64));
}
#[test]
fn stale_offline_cache_warns_without_refresh() {
    let f = Fixture::new();
    let v = f.json(&["card", "--offline"]);
    assert_eq!(v["freshness"]["stale"], true);
    assert!(!v["warnings"].as_array().unwrap().is_empty());
}
#[test]
fn tsv_export_and_explicit_clear() {
    let f = Fixture::new();
    let out = f
        .cmd()
        .args(["--format", "tsv", "grades", "--offline"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.contains(&b'\t'));
    assert!(!out.stdout.contains(&27));
    let out = f.cmd().args(["cache", "clear"]).output().unwrap();
    assert_eq!(out.status.code(), Some(64));
    assert_eq!(
        f.json(&["cache", "clear", "--yes"])["data"]["cleared"],
        true
    );
}

#[test]
fn phyvlab_offline_list_courses_show_and_failure_guidance() {
    let f = Fixture::new();
    let mut db = Database::open(&f.temp.path().join("data/campus-portal/campus.db")).unwrap();
    let id = db.account("fixture-user").unwrap();
    db.failure(id, "phyvlab", "needs_user_action", "captcha_required", 1)
        .unwrap();
    let out = f.cmd().args(["phyvlab", "--offline"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err_text = String::from_utf8(out.stderr).unwrap();
    assert!(err_text.contains("物理在线人机验证") && err_text.contains("campus phyvlab login"));

    db.save(
        id,
        "phyvlab_courses",
        &[json!({
            "id": 72,
            "name": "大学物理I_(2026春)",
            "category": "自然科学",
            "progress_percent": 35,
            "course_url": "https://phyvlab.bjtu.edu.cn/course/view.php?id=72"
        })],
        10,
    )
    .unwrap();
    db.save(
        id,
        "phyvlab",
        &[json!({
            "id": 3689,
            "course_id": 72,
            "course_name": "大学物理I_(2026春)",
            "title": "chap1",
            "activity_type": "作业",
            "activity_url": "https://phyvlab.bjtu.edu.cn/mod/assign/view.php?id=3689",
            "open_text": "2026年03月12日 00:00",
            "open_timestamp": 1773244800,
            "due_text": "2026年03月19日 00:00",
            "due_timestamp": 1773849600,
            "completed": true,
            "detail": {
                "description": "完成第一章作业",
                "submission_status": "已提交",
                "submission_date_text": "2026年03月18日 21:30",
                "submission_date_timestamp": 1773840600,
                "grade_text": "92.0",
                "feedback_text": "很好",
                "submitted_files": ["chap1.pdf"],
                "attachments": [{"file_name": "guide.pdf", "url": "https://phyvlab.bjtu.edu.cn/pluginfile.php/1/guide.pdf"}],
                "can_submit": true,
                "can_finalize": false,
                "is_draft": false
            }
        })],
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();

    let list = f.json(&["phyvlab", "--offline"]);
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["status_label"], "已提交");
    assert_eq!(list["data"][0]["grade_text"], "92.0");

    let courses = f.json(&["phyvlab", "--offline", "courses"]);
    assert_eq!(courses["data"][0]["name"], "大学物理I_(2026春)");

    let show = f.json(&["phyvlab", "--offline", "show", "3689"]);
    assert_eq!(show["data"]["timing"], "按时提交");
    assert_eq!(show["data"]["attachments"][0], "guide.pdf");
    assert_eq!(show["data"]["submitted_files"][0], "chap1.pdf");

    let status = f.json(&["phyvlab", "status"]);
    assert_eq!(status["data"]["configured"], false);
}

#[test]
fn homework_lists_unsubmitted_first_sorted_by_due_and_dims_overdue() {
    let f = Fixture::new();
    let mut db = Database::open(&f.temp.path().join("data/campus-portal/campus.db")).unwrap();
    let id = db.account("fixture-user").unwrap();
    db.save(
        id,
        "homework",
        &[
            json!({"id": "s2", "course_name": "算法", "title": "已交较晚", "end_time": "2099-11-20 23:59", "subStatus": "已提交"}),
            json!({"id": "u_future", "course_name": "算法", "title": "未交未逾期", "end_time": "2099-12-01 23:59", "subStatus": "未提交"}),
            json!({"id": "s1", "course_name": "算法", "title": "已交较早", "end_time": "2020-01-01 23:59", "snId": 9}),
            json!({"id": "u_past", "course_name": "算法", "title": "未交已逾期", "end_time": "2020-01-02 23:59", "subStatus": "未提交"}),
        ],
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();

    let listed = f.json(&["homework", "--offline"]);
    let ids: Vec<_> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["u_past", "u_future", "s1", "s2"]);

    let out = f
        .cmd()
        .env_remove("NO_COLOR")
        .env("COLORTERM", "truecolor")
        .env("TERM", "xterm-256color")
        .args(["homework", "--offline", "--color", "always"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("\x1b[1m\x1b[38;2;157;157;157m未提交\x1b[0m")
            || text.contains("38;2;157;157;157m未提交")
    );
    assert!(text.contains("38;2;248;81;73m未提交"));
    assert!(text.contains("38;2;181;206;168m已提交"));

    let mut db = Database::open(&f.temp.path().join("data/campus-portal/campus.db")).unwrap();
    db.save(
        id,
        "homework",
        &[json!({
            "id": "u_att",
            "course_name": "算法",
            "title": "含附件作业",
            "end_time": "2099-12-01 23:59",
            "subStatus": "未提交",
            "attachments": [{"id": "77", "file_name": "实验指导.pdf", "size_bytes": 1024, "source_path": "/guide.pdf"}],
            "submitted_attachments": [{"id": "88", "file_name": "我的报告.pdf", "source_path": "/up/r.pdf"}]
        })],
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();
    let show = f
        .cmd()
        .args(["homework", "show", "u_att", "--offline"])
        .output()
        .unwrap();
    assert!(show.status.success());
    let show_text = String::from_utf8(show.stdout).unwrap();
    assert!(show_text.contains("老师附件") && show_text.contains("实验指导.pdf"));
    assert!(show_text.contains("已交附件") && show_text.contains("我的报告.pdf"));
}

#[test]
fn update_command_rejects_offline_and_accepts_check_config() {
    let f = Fixture::new();
    let offline = f.cmd().args(["update", "--offline"]).output().unwrap();
    assert_eq!(offline.status.code(), Some(64));

    let cfg_off = f
        .cmd()
        .args(["config", "set", "update.check", "off"])
        .output()
        .unwrap();
    assert!(cfg_off.status.success());

    let cfg_get = f
        .cmd()
        .args(["config", "get", "update.check", "--json"])
        .output()
        .unwrap();
    assert!(cfg_get.status.success());
    let parsed: Value = serde_json::from_slice(&cfg_get.stdout).unwrap();
    assert_eq!(parsed["data"], "off");
}
