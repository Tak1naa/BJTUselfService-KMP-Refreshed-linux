mod rooms_grid;
mod schedule_grid;

use crate::{
    args::{Args, Color, Format},
    now, open_database, palette, runtime,
};
use campus_core::{Error, Result};
use campus_db::{Database, config::Config, paths::Paths, storage};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color as Ink, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

const fn ink(c: palette::Rgb) -> Ink {
    Ink::Rgb(c.0, c.1, c.2)
}

const BLUE: Ink = ink(palette::ACCENT);
const CYAN: Ink = ink(palette::CYAN);
const GREEN: Ink = ink(palette::GREEN);
const RED: Ink = ink(palette::RED);
const MUTED: Ink = ink(palette::MUTED);
const AMBER: Ink = ink(palette::AMBER);
const FAINT: Ink = ink(palette::FAINT);
const SELECTION: Ink = ink(palette::SELECTION);
const TEXT: Ink = ink(palette::TEXT);

fn submission_color(status: &str) -> Option<Ink> {
    match status {
        "已提交" | "按时提交" => Some(GREEN),
        "未提交" | "已逾期" => Some(RED),
        "草稿 · 待最终提交" | "逾期提交" | "待提交" => Some(AMBER),
        _ => None,
    }
}

struct Page {
    dataset: &'static str,
    title: &'static str,
    fields: &'static [(&'static str, &'static str)],
}

const PAGES: &[Page] = &[
    Page {
        dataset: "courses",
        title: "课表",
        fields: &[
            ("name", "课程"),
            ("weekday", "星期"),
            ("slot", "节次"),
            ("place", "地点"),
            ("weeks_raw", "周次"),
        ],
    },
    Page {
        dataset: "homework",
        title: "作业",
        fields: &[
            ("title", "标题"),
            ("course_name", "课程"),
            ("end_time", "截止"),
            ("subStatus", "提交状态"),
            ("id", "ID"),
        ],
    },
    Page {
        dataset: "grades",
        title: "成绩",
        fields: &[
            ("course_name", "课程"),
            ("score_raw", "成绩"),
            ("credits_raw", "学分"),
            ("term", "学期"),
        ],
    },
    Page {
        dataset: "exams",
        title: "考试",
        fields: &[
            ("course_name", "课程"),
            ("time_place_raw", "时间与地点"),
            ("status", "状态"),
        ],
    },
    Page {
        dataset: "rooms",
        title: "教室",
        fields: &[("room", "教室"), ("capacity", "容量")],
    },
    Page {
        dataset: "card",
        title: "一卡通",
        fields: &[
            ("card_balance", "校园卡余额"),
            ("net_fee", "网费余额"),
            ("new_mail", "未读邮件"),
        ],
    },
    Page {
        dataset: "calendar",
        title: "学年历",
        fields: &[("term", "学期"), ("week", "周次"), ("monday", "周一日期")],
    },
    Page {
        dataset: "courseware",
        title: "课件",
        fields: &[
            ("rpName", "文件"),
            ("course_name", "课程"),
            ("download", "操作"),
        ],
    },
    Page {
        dataset: "sync",
        title: "同步",
        fields: &[
            ("dataset", "数据"),
            ("status", "状态"),
            ("last_success_at", "最近成功"),
            ("reason", "原因"),
        ],
    },
    Page {
        dataset: "phyvlab",
        title: "物理在线",
        fields: &[
            ("title", "标题"),
            ("course_name", "课程"),
            ("due_text", "截止"),
            ("status_label", "状态"),
            ("id", "ID"),
        ],
    },
];

pub fn interactive() -> bool {
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && std::env::var("TERM").as_deref() != Ok("dumb")
}

pub fn run(
    args: &Args,
    db: &Database,
    paths: &Paths,
    config: &Config,
    user: Option<&str>,
) -> Result<u8> {
    if args.json || !matches!(args.format, Format::Table) {
        return Err(Error::Usage(
            "campus tui requires the default table format".into(),
        ));
    }
    if !interactive() {
        return Err(Error::Usage(
            "campus tui requires an interactive terminal".into(),
        ));
    }

    let account = user.map(|s| db.account(s)).transpose()?;
    let mut app = App {
        page: 0,
        rows: Vec::new(),
        visible: Vec::new(),
        selection: TableState::default(),
        filter: String::new(),
        editing: false,
        detail: None,
        info: String::new(),
        status: String::new(),
        room_key: None,
        room_cells: Vec::new(),
        room_monday: None,
        first_room_day: 0,
        schedule: schedule_grid::Schedule::default(),
        download_ids: Vec::new(),
        grades: Vec::new(),
        worker: None,
        colored: !matches!(args.color, Color::Never) && std::env::var_os("NO_COLOR").is_none(),
        legacy_homework: config.get("homework.legacy_http").as_deref() != Some("off"),
        legacy_courseware: config.get("courseware.legacy_http").as_deref() != Some("off"),
    };
    app.load(db, account)?;
    let mut terminal = ratatui::try_init()
        .inspect_err(|_| ratatui::restore())
        .map_err(storage)?;
    let result = app.event_loop(&mut terminal, args, db, paths, config, user, account);
    let restoration = ratatui::try_restore().map_err(storage);
    result.and(restoration).map(|()| 0)
}

struct App {
    page: usize,
    rows: Vec<Vec<String>>,
    visible: Vec<usize>,
    selection: TableState,
    filter: String,
    editing: bool,
    detail: Option<(String, u16)>,
    info: String,
    status: String,
    room_key: Option<String>,
    room_cells: Vec<[u8; 49]>,
    room_monday: Option<jiff::civil::Date>,
    first_room_day: usize,
    schedule: schedule_grid::Schedule,
    download_ids: Vec<Option<String>>,
    grades: Vec<campus_core::Grade>,
    worker: Option<Receiver<Result<Vec<Value>>>>,
    colored: bool,
    legacy_homework: bool,
    legacy_courseware: bool,
}

impl App {
    fn page(&self) -> &'static Page {
        &PAGES[self.page]
    }

    fn load(&mut self, db: &Database, account: Option<i64>) -> Result<()> {
        let page = self.page();
        self.info.clear();
        self.detail = None;
        let id = if matches!(page.dataset, "calendar" | "sync") {
            Some(0)
        } else {
            account
        };
        let mut rows = if let Some(id) = id {
            match page.dataset {
                "sync" => db.sync_status(account.unwrap_or(0))?,
                dataset => {
                    let key = if dataset == "rooms" {
                        if self.room_key.is_none() {
                            self.room_key = Some(if let Some(key) = latest_room_key(db, id)? {
                                key
                            } else {
                                let weeks = crate::from_rows(db.rows(0, "calendar")?)?;
                                let week = campus_core::calendar::current_week(&weeks, now())
                                    .map(|(_, week)| week)
                                    .or(db
                                        .pref(&format!("courses:{id}"), "current_week")?
                                        .and_then(|s| s.parse().ok()))
                                    .filter(|week| (1..=30).contains(week))
                                    .unwrap_or(1);
                                format!("rooms:current:1:{week}")
                            });
                        }
                        self.room_key.as_deref().expect("room query initialized")
                    } else {
                        dataset
                    };
                    self.info = match db.fetched_at(id, key)? {
                        Some(time) => {
                            let suffix = db
                                .sync_failure(id, key)?
                                .map_or_else(String::new, |reason| {
                                    format!("  ·  最近更新失败：{}", sync_reason(&reason))
                                });
                            format!("本地缓存  {}{}", time_label(time), suffix)
                        }
                        None if dataset == "rooms" => {
                            "暂无教室缓存。按 b 切换楼宇，+/- 改周次，r 查询。".into()
                        }
                        None => db.sync_failure(id, key)?.map_or_else(
                            || "暂无缓存。按 r 同步当前页。".into(),
                            |reason| match (dataset, reason.as_str()) {
                                ("homework" | "courseware", "smart_legacy_http") => {
                                    "服务转到旧版 HTTP，当前配置已关闭 legacy_http 兼容。".into()
                                }
                                (
                                    "phyvlab",
                                    "account_required"
                                    | "captcha_required"
                                    | "credentials_rejected",
                                ) => {
                                    "请先在终端运行 campus phyvlab login 完成独立账号或验证码认证。"
                                        .into()
                                }
                                ("phyvlab", "phyvlab_profile_incomplete") => {
                                    "请先在浏览器打开 https://phyvlab.bjtu.edu.cn/ 完善个人资料后再同步。"
                                        .into()
                                }
                                _ => format!("暂无缓存  ·  {}", sync_reason(&reason)),
                            },
                        ),
                    };
                    db.rows(id, key)?
                }
            }
        } else {
            self.info = "请先运行 campus auth login 登录。".into();
            Vec::new()
        };
        if page.dataset == "courses" {
            let mut courses = campus_core::normalize_schedule(crate::from_rows(rows)?);
            courses.sort_by_key(|course| course.location_index);
            self.schedule.weeks = crate::from_rows(db.rows(0, "calendar")?)?;
            if self.schedule.week.is_none() {
                self.schedule.week =
                    campus_core::calendar::current_week(&self.schedule.weeks, now())
                        .map(|(_, week)| week)
                        .or(if let Some(account) = account {
                            db.pref(&format!("courses:{account}"), "current_week")?
                                .and_then(|week| week.parse().ok())
                        } else {
                            None
                        })
                        .filter(|week| (1..=30).contains(week));
            }
            self.rows = courses
                .iter()
                .map(|course| {
                    let clean = crate::output::clean;
                    let day = ["—", "周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                        [usize::from(course.weekday())];
                    vec![
                        clean(&course.name),
                        day.into(),
                        (course.slot() + 1).to_string(),
                        clean(schedule_grid::room_for_display(&course.place)),
                        clean(&course.weeks_raw),
                        format!(
                            "上课地点  {}\n教师  {}",
                            clean(&course.place),
                            clean(&course.teacher)
                        ),
                    ]
                })
                .collect();
            self.schedule.courses = courses;
            self.apply_filter();
            return Ok(());
        }
        if page.dataset == "rooms" {
            self.room_cells = rows
                .iter()
                .map(|row| {
                    let mut cells = [255; 49];
                    if let Some(values) = row["cells"].as_array() {
                        for (cell, value) in cells.iter_mut().zip(values) {
                            *cell = value
                                .as_u64()
                                .and_then(|v| u8::try_from(v).ok())
                                .unwrap_or(255);
                        }
                    }
                    cells
                })
                .collect();
            let weeks = crate::from_rows(db.rows(0, "calendar")?)?;
            self.room_monday = self
                .room_key
                .as_deref()
                .and_then(room_query)
                .and_then(|(term, _, week)| rooms_grid::monday(&weeks, term, week, now()));
        }
        self.download_ids.clear();
        if page.dataset == "courseware" {
            self.rows.clear();
            for course in &rows {
                let course_name = value_for("course_name", course);
                let course_id = campus_core::parse::scalar(&course["id"]);
                for node in course["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|node| node["kind"] == "resource")
                {
                    let permitted = campus_core::parse::scalar(&node["stu_download"])
                        .is_none_or(|v| v.trim() == "2");
                    let id = course_id
                        .as_deref()
                        .zip(campus_core::parse::scalar(&node["rpId"]))
                        .filter(|_| permitted)
                        .map(|(course, resource)| format!("{course}:{resource}"));
                    self.rows.push(vec![
                        crate::output::cell(&node["rpName"]),
                        course_name.clone(),
                        if id.is_some() {
                            "d 下载"
                        } else {
                            "不可下载"
                        }
                        .into(),
                    ]);
                    self.download_ids.push(id);
                }
            }
            self.apply_filter();
            return Ok(());
        }
        if page.dataset == "phyvlab" {
            let activities: Vec<campus_core::phyvlab::PhyVlabActivity> = crate::from_rows(rows)?;
            let current = now();
            self.rows = activities
                .iter()
                .map(|act| {
                    let status = campus_core::phyvlab::activity_status_label(act, current);
                    let mut extra = Vec::new();
                    if let Some(open) = &act.open_text {
                        extra.push(format!("开放时间  {open}"));
                    }
                    if let Some(timing) = campus_core::phyvlab::submission_timing_label(act) {
                        extra.push(format!("提交时效  {timing}"));
                    }
                    if let Some(detail) = &act.detail {
                        if !detail.submission_status.is_empty() {
                            extra.push(format!("提交状态  {}", detail.submission_status));
                        }
                        if let Some(date) = &detail.submission_date_text {
                            extra.push(format!("提交时间  {date}"));
                        }
                        if let Some(grade) = &detail.grade_text {
                            extra.push(format!("成绩  {grade}"));
                        }
                        if let Some(feedback) = &detail.feedback_text {
                            extra.push(format!("教师评语  {feedback}"));
                        }
                        if !detail.submitted_files.is_empty() {
                            extra.push(format!("已交文件  {}", detail.submitted_files.join("，")));
                        }
                        if !detail.attachments.is_empty() {
                            let names = detail
                                .attachments
                                .iter()
                                .map(|a| a.file_name.as_str())
                                .collect::<Vec<_>>()
                                .join("，");
                            extra.push(format!("老师附件  {names}（按 d 下载至 ./phyvlab）"));
                        }
                        if !detail.description.is_empty() {
                            extra.push(detail.description.clone());
                        }
                    }
                    self.download_ids.push(
                        act.detail
                            .as_ref()
                            .is_some_and(|d| !d.attachments.is_empty())
                            .then(|| act.id.to_string()),
                    );
                    vec![
                        crate::output::clean(&act.title),
                        crate::output::clean(&act.course_name),
                        act.due_text
                            .as_deref()
                            .map(crate::output::clean)
                            .unwrap_or_else(|| "—".into()),
                        status.into(),
                        act.id.to_string(),
                        extra.join("\n\n"),
                    ]
                })
                .collect();
            self.apply_filter();
            return Ok(());
        }
        if page.dataset == "grades" {
            self.grades = crate::from_rows(rows.clone())?;
        }
        let mut current_week = None;
        if page.dataset == "calendar" {
            use campus_core::calendar::{current_week as week_at, term_at};
            let weeks: Vec<campus_core::AcademicWeek> = crate::from_rows(rows.clone())?;
            let today = now();
            current_week = week_at(&weeks, today);
            let term = current_week
                .as_ref()
                .map(|(term, _)| term.as_str())
                .or_else(|| term_at(&weeks, today))
                .or_else(|| {
                    weeks
                        .iter()
                        .max_by_key(|w| &w.monday)
                        .map(|w| w.term.as_str())
                });
            rows.retain(|row| {
                row["term"].as_str() == term
                    && row["week"]
                        .as_u64()
                        .is_some_and(|week| (1..=16).contains(&week))
            });
            rows.sort_by_key(|row| std::cmp::Reverse(row["week"].as_u64()));
        }
        self.rows = rows
            .iter()
            .map(|row| {
                let mut cells: Vec<_> = page
                    .fields
                    .iter()
                    .map(|(key, _)| value_for(key, row))
                    .collect();
                if page.dataset == "homework" {
                    cells.push(campus_core::parse::homework_text(
                        row["content"].as_str().unwrap_or_default(),
                    ));
                } else if page.dataset == "calendar"
                    && current_week.as_ref().is_some_and(|(term, week)| {
                        row["term"] == term.as_str() && row["week"] == *week
                    })
                {
                    cells[1].push_str(" 本周");
                }
                cells
            })
            .collect();
        self.apply_filter();
        Ok(())
    }

    fn apply_filter(&mut self) {
        let query = self.filter.to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(idx, row)| {
                let in_week = self.page().dataset != "courses"
                    || !self.schedule.calendar
                    || self.schedule.courses[idx].in_week(self.schedule.active_week());
                (in_week
                    && (query.is_empty()
                        || row.iter().any(|cell| cell.to_lowercase().contains(&query))))
                .then_some(idx)
            })
            .collect();
        self.selection =
            TableState::default().with_selected((!self.visible.is_empty()).then_some(0));
    }

    fn move_selection(&mut self, delta: isize) {
        if let Some(index) = self.selection.selected() {
            if self.page().dataset == "courses" && self.schedule.calendar {
                self.schedule.follow_selection = true;
            }
            self.selection.select(Some(
                index
                    .saturating_add_signed(delta)
                    .min(self.visible.len() - 1),
            ));
        }
    }

    fn open_detail(&mut self) {
        if let Some(index) = self.selection.selected() {
            let page = self.page();
            let row = &self.rows[self.visible[index]];
            let mut detail = page
                .fields
                .iter()
                .zip(row)
                .map(|((_, title), value)| format!("{title}  {value}"))
                .collect::<Vec<_>>()
                .join("\n\n");
            if let Some(extra) = row.get(page.fields.len()).filter(|s| !s.is_empty()) {
                detail.push_str("\n\n");
                detail.push_str(extra);
            }
            self.detail = Some((detail, 0));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn event_loop(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        args: &Args,
        db: &Database,
        paths: &Paths,
        config: &Config,
        user: Option<&str>,
        account: Option<i64>,
    ) -> Result<()> {
        let mut redraw = true;
        loop {
            if let Some(rx) = &self.worker {
                match rx.try_recv() {
                    Ok(result) => {
                        self.worker = None;
                        self.status = summarize(result);
                        self.load(db, account)?;
                        redraw = true;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.worker = None;
                        self.status = "同步中断。".into();
                        redraw = true;
                    }
                    Err(TryRecvError::Empty) => {}
                }
            }
            if redraw {
                terminal
                    .draw(|frame| self.draw(frame, args.offline))
                    .map_err(storage)?;
                redraw = false;
            }
            if !event::poll(Duration::from_millis(120)).map_err(storage)? {
                continue;
            }
            let event = event::read().map_err(storage)?;
            redraw = true;
            let Event::Key(key) = event else { continue };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(());
            }
            if self.editing {
                match key.code {
                    KeyCode::Esc => {
                        self.editing = false;
                        self.filter.clear();
                    }
                    KeyCode::Enter => self.editing = false,
                    KeyCode::Backspace => {
                        self.filter.pop();
                    }
                    KeyCode::Char(c) if !c.is_control() => self.filter.push(c),
                    _ => {}
                }
                self.apply_filter();
                continue;
            }
            if let Some((_, scroll)) = self.detail.as_mut() {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => self.detail = None,
                    KeyCode::Down => *scroll = scroll.saturating_add(1),
                    KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::PageDown => *scroll = scroll.saturating_add(10),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                    KeyCode::Char('q') => return Ok(()),
                    _ => {}
                }
                continue;
            }
            let target = match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Tab | KeyCode::Right => Some((self.page + 1) % PAGES.len()),
                KeyCode::BackTab | KeyCode::Left => {
                    Some((self.page + PAGES.len() - 1) % PAGES.len())
                }
                KeyCode::Char(c @ '1'..='9') => {
                    let idx = (c as u8 - b'1') as usize;
                    (idx < PAGES.len()).then_some(idx)
                }
                KeyCode::Char('0') if PAGES.len() >= 10 => Some(9),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_selection(-1);
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_selection(1);
                    None
                }
                KeyCode::PageUp => {
                    self.move_selection(-12);
                    None
                }
                KeyCode::PageDown => {
                    self.move_selection(12);
                    None
                }
                KeyCode::Home => {
                    self.move_selection(isize::MIN);
                    None
                }
                KeyCode::End => {
                    self.move_selection(isize::MAX);
                    None
                }
                KeyCode::Enter => {
                    self.open_detail();
                    None
                }
                KeyCode::Char('/') => {
                    self.editing = true;
                    None
                }
                KeyCode::Char('d') if matches!(self.page().dataset, "courseware" | "phyvlab") => {
                    self.download(args, paths, config, user);
                    None
                }
                KeyCode::Char('v') if self.page().dataset == "courses" => {
                    self.schedule.calendar = !self.schedule.calendar;
                    self.schedule.follow_selection = self.schedule.calendar;
                    self.apply_filter();
                    None
                }
                KeyCode::Char(key @ ('+' | '-'))
                    if self.page().dataset == "courses" && self.schedule.calendar =>
                {
                    self.schedule.change_week(if key == '+' { 1 } else { -1 });
                    self.apply_filter();
                    None
                }
                KeyCode::Char(key @ ('[' | ']'))
                    if self.page().dataset == "courses" && self.schedule.calendar =>
                {
                    self.schedule.first_day = if key == '[' {
                        self.schedule.first_day.saturating_sub(1)
                    } else {
                        (self.schedule.first_day + 1).min(6)
                    };
                    None
                }
                KeyCode::Char('[') if self.page().dataset == "rooms" => {
                    self.first_room_day = self.first_room_day.saturating_sub(1);
                    None
                }
                KeyCode::Char(']') if self.page().dataset == "rooms" => {
                    self.first_room_day = (self.first_room_day + 1).min(6);
                    None
                }
                KeyCode::Char(key @ ('b' | 'B' | '+' | '-')) if self.page().dataset == "rooms" => {
                    self.change_room(key);
                    self.status.clear();
                    self.load(db, account)?;
                    None
                }
                KeyCode::Char('r') => {
                    self.refresh(args, paths, config, user);
                    None
                }
                _ => None,
            };
            if let Some(page) = target {
                self.page = page;
                self.filter.clear();
                self.load(db, account)?;
            }
        }
    }

    fn change_room(&mut self, key: char) {
        let Some((term, building, week)) = self.room_key.as_deref().and_then(room_query) else {
            return;
        };
        let buildings = crate::buildings();
        let index = buildings
            .iter()
            .position(|entry| entry["id"] == building)
            .unwrap_or(0);
        let next = match key {
            'b' => (index + 1) % buildings.len(),
            'B' => (index + buildings.len() - 1) % buildings.len(),
            _ => index,
        };
        let week = match key {
            '+' => week.saturating_add(1).min(53),
            '-' => week.saturating_sub(1).max(1),
            _ => week,
        };
        if let Some(building) = buildings[next]["id"].as_str() {
            self.room_key = Some(format!("rooms:{term}:{building}:{week}"));
        }
    }

    fn download(&mut self, args: &Args, paths: &Paths, config: &Config, user: Option<&str>) {
        if args.offline || self.worker.is_some() {
            return;
        }
        let dataset = self.page().dataset;
        let Some((user, id)) = user.zip(
            self.selection
                .selected()
                .and_then(|i| self.visible.get(i))
                .and_then(|i| self.download_ids.get(*i))
                .and_then(Option::as_deref),
        ) else {
            self.status = if dataset == "phyvlab" {
                "当前物理在线作业没有可下载的老师附件。".into()
            } else {
                "请选择可下载的课件。".into()
            };
            return;
        };
        let user = user.to_owned();
        let id = id.to_owned();
        let paths = paths.clone();
        let config = Config {
            values: config.values.clone(),
        };
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.status = if dataset == "phyvlab" {
            "正在下载至 ./phyvlab…".into()
        } else {
            "正在下载至 ./courseware…".into()
        };
        std::thread::spawn(move || {
            let result = (|| {
                let db = open_database(&paths, &config)?;
                if dataset == "phyvlab" {
                    let act_id = id
                        .parse::<i64>()
                        .map_err(|_| Error::Malformed("phyvlab activity ID"))?;
                    let files = crate::phyvlab::download_attachments(
                        &db,
                        &paths,
                        &config,
                        &user,
                        act_id,
                        std::path::Path::new("phyvlab"),
                    )?;
                    Ok(files.as_array().cloned().unwrap_or_else(|| vec![files]))
                } else {
                    let file = crate::transfer::courseware_download(
                        &db,
                        &paths,
                        &config,
                        &user,
                        &id,
                        std::path::Path::new("courseware"),
                    )?;
                    Ok(vec![file])
                }
            })();
            let _ = tx.send(result);
        });
    }

    fn refresh(&mut self, args: &Args, paths: &Paths, config: &Config, user: Option<&str>) {
        if args.offline || self.worker.is_some() {
            return;
        }
        let dataset = self.page().dataset;
        if user.is_none() && dataset != "calendar" {
            self.status = "请先运行 campus auth login。".into();
            return;
        }
        let mut options = BTreeMap::new();
        if dataset == "rooms" {
            let Some((term, rest)) = self
                .room_key
                .as_deref()
                .and_then(|s| s.strip_prefix("rooms:"))
                .and_then(|s| s.split_once(':'))
            else {
                self.status = "请先通过 campus rooms grid 设置楼宇和周次。".into();
                return;
            };
            let Some((building, week)) = rest.split_once(':') else {
                return;
            };
            options.extend([
                ("term".into(), term.into()),
                ("building".into(), building.into()),
                ("week".into(), week.into()),
            ]);
        }
        let datasets = if dataset == "sync" {
            [
                "courses",
                "homework",
                "grades",
                "exams",
                "card",
                "calendar",
                "grade_types",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        } else {
            vec![dataset.into()]
        };
        let (tx, rx) = mpsc::channel();
        let paths = paths.clone();
        let config = Config {
            values: config.values.clone(),
        };
        let user = user.map(str::to_owned);
        self.status = format!("正在同步 {}…", self.page().title);
        self.worker = Some(rx);
        std::thread::spawn(move || {
            let result = (|| {
                let mut db = open_database(&paths, &config)?;
                runtime()?.block_on(campus_sync::sync(
                    &mut db,
                    &paths,
                    &config,
                    user.as_deref(),
                    &datasets,
                    &options,
                    now(),
                ))
            })();
            let _ = tx.send(result);
        });
    }

    fn style(&self, color: Ink) -> Style {
        if self.colored {
            Style::default().fg(color)
        } else {
            Style::default()
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>, offline: bool) {
        let area = frame.area();
        if area.width < 28 || area.height < 9 {
            frame.render_widget(Paragraph::new("请放大终端 · q 退出"), area);
            return;
        }
        let page = self.page();
        let legacy_notice = match page.dataset {
            "homework" if self.legacy_homework => Some(" 作业兼容已开启 · HTTP 明文"),
            "courseware" if self.legacy_courseware => Some(" 课件兼容已开启 · HTTP 明文"),
            _ => None,
        };
        let [header, notice, nav, body, summary, status, controls] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(u16::from(legacy_notice.is_some())),
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(u16::from(page.dataset == "grades" && area.height >= 10)),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " ◆ 交大自由行 CLI ",
                    self.style(BLUE).add_modifier(Modifier::BOLD),
                ),
                Span::styled("│", self.style(FAINT)),
                Span::styled(
                    if offline {
                        " 离线模式 "
                    } else {
                        " 本地缓存优先 "
                    },
                    self.style(if offline { AMBER } else { GREEN }),
                ),
            ])),
            header,
        );
        if let Some(notice_text) = legacy_notice {
            frame.render_widget(Paragraph::new(notice_text).style(self.style(AMBER)), notice);
        }
        if area.width >= 88 {
            frame.render_widget(
                Tabs::new(
                    PAGES
                        .iter()
                        .enumerate()
                        .map(|(i, p)| format!(" {} {} ", (i + 1) % 10, p.title))
                        .collect::<Vec<_>>(),
                )
                .select(self.page)
                .style(self.style(MUTED))
                .highlight_style(
                    self.style(CYAN)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )
                .divider(Span::styled("·", self.style(FAINT))),
                nav,
            );
        } else {
            frame.render_widget(
                Paragraph::new(format!(
                    " ‹ {} ({}/{}) ›",
                    page.title,
                    self.page + 1,
                    PAGES.len()
                ))
                .style(self.style(BLUE).add_modifier(Modifier::BOLD)),
                nav,
            );
        }
        let title = if page.dataset == "rooms" {
            self.room_key
                .as_deref()
                .and_then(room_query)
                .map(|(_, id, week)| {
                    let name = crate::buildings()
                        .into_iter()
                        .find(|b| b["id"] == id)
                        .map(|b| crate::output::cell(&b["name"]))
                        .unwrap_or_else(|| id.to_owned());
                    format!(" 教室 · {name} · 第 {week} 周 ")
                })
                .unwrap_or_else(|| " 教室 ".into())
        } else if page.dataset == "courses" && self.schedule.calendar {
            format!(" 周历 · 第 {} 周 ", self.schedule.active_week())
        } else if page.dataset == "courses" {
            format!(" 课表列表 · {} 条 ", self.visible.len())
        } else {
            format!(" {} · {} 条 ", page.title, self.visible.len())
        };
        let frame_block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.style(if self.detail.is_some() { FAINT } else { BLUE }))
            .title(Span::styled(
                title,
                self.style(TEXT).add_modifier(Modifier::BOLD),
            ));
        if self.visible.is_empty() {
            let message = if !self.filter.is_empty() {
                "没有符合筛选的记录。"
            } else if self.info.is_empty() {
                "暂无记录。"
            } else {
                &self.info
            };
            frame.render_widget(
                Paragraph::new(message)
                    .style(self.style(MUTED))
                    .wrap(Wrap { trim: true })
                    .block(frame_block),
                body,
            );
        } else if page.dataset == "courses" && self.schedule.calendar {
            let grid = frame_block.inner(body);
            frame.render_widget(frame_block, body);
            schedule_grid::draw(frame, grid, self);
        } else if page.dataset == "rooms" {
            let grid = frame_block.inner(body);
            frame.render_widget(frame_block, body);
            rooms_grid::draw(frame, grid, self);
        } else {
            let columns: Vec<_> = if body.width >= 52 {
                (0..page.fields.len()).collect()
            } else {
                match page.dataset {
                    "homework" | "phyvlab" => vec![0, 3],
                    "calendar" => vec![1, 2],
                    _ => vec![0, 1],
                }
            };
            let colored = self.colored;
            let data = self.visible.iter().map(|&i| {
                let row = &self.rows[i];
                let current_week = page.dataset == "calendar" && row[1].ends_with(" 本周");
                Row::new(columns.iter().map(|&j| {
                    let color = if (page.dataset == "homework" && page.fields[j].0 == "subStatus")
                        || (page.dataset == "phyvlab" && page.fields[j].0 == "status_label")
                    {
                        submission_color(&row[j])
                    } else if current_week {
                        Some(GREEN)
                    } else {
                        Some(TEXT)
                    };
                    let style = color
                        .filter(|_| colored)
                        .map_or_else(Style::default, |color| Style::default().fg(color));
                    Cell::new(row[j].as_str()).style(style)
                }))
            });
            let headers = Row::new(columns.iter().map(|&i| page.fields[i].1))
                .style(self.style(CYAN).add_modifier(Modifier::BOLD));
            let layout = (0..columns.len()).map(|i| {
                Constraint::Fill(if i == 0 && page.dataset != "calendar" {
                    2
                } else {
                    1
                })
            });
            let highlight = if colored {
                Style::default()
                    .bg(SELECTION)
                    .fg(TEXT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::REVERSED)
            };
            let table = Table::new(data, layout)
                .header(headers)
                .block(frame_block)
                .row_highlight_style(highlight)
                .highlight_symbol("› ");
            frame.render_stateful_widget(table, body, &mut self.selection);
        }
        if page.dataset == "grades" && summary.height > 0 {
            let grades =
                campus_core::summarize_grades(self.visible.iter().map(|&i| &self.grades[i]));
            let text = match (grades.weighted_average, grades.gpa) {
                (Some(mean), Some(gpa)) => {
                    let label = if grades.estimated_average {
                        "估算均分"
                    } else {
                        "加权均分"
                    };
                    format!("{label} {mean:.2}  GPA {gpa:.3}/4")
                }
                _ => "均分 —  GPA —".into(),
            };
            frame.render_widget(Paragraph::new(text).style(self.style(BLUE)), summary);
        }
        let caption = if !self.status.is_empty() {
            &self.status
        } else if self.visible.is_empty() {
            ""
        } else {
            &self.info
        };
        frame.render_widget(
            Paragraph::new(caption).style(self.style(if self.worker.is_some() {
                AMBER
            } else {
                GREEN
            })),
            status,
        );
        let help = if self.editing {
            format!("/ {}  ·  Enter 确认  Esc 清除", self.filter)
        } else if self.detail.is_some() {
            "↑↓ 滚动   Esc 返回   q 退出".into()
        } else if page.dataset == "courses" {
            let primary = if self.schedule.calendar {
                "v 列表   +/- 周次   [/] 看日期"
            } else {
                "v 周历"
            };
            let sync = if offline { "" } else { "   r 同步" };
            format!("{primary}   ↑↓ 选课   Enter 详情   / 筛选{sync}   q 退出")
        } else if page.dataset == "rooms" {
            if offline {
                "b/B 楼宇   +/- 周次   [/] 看日期   ↑↓ 选教室"
            } else {
                "b/B 楼宇   +/- 周次   [/] 看日期   r 查教室   ↑↓ 选教室"
            }
            .into()
        } else if page.dataset == "courseware" && !offline {
            "↑↓ 选文件   / 筛选   d 下载至 ./courseware   r 同步   q 退出".into()
        } else if page.dataset == "phyvlab" && !offline {
            "↑↓ 选行   Enter 详情   d 下附件(./phyvlab)   / 筛选   r 同步   q 退出".into()
        } else if offline {
            "←→/Tab 切页   ↑↓ 选行   Enter 详情   / 筛选   q 退出".into()
        } else {
            "←→/Tab 切页   ↑↓ 选行   Enter 详情   / 筛选   r 同步   q 退出".into()
        };
        frame.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: false })
                .style(self.style(MUTED)),
            controls,
        );
        if let Some((content, scroll)) = &self.detail {
            let [_, box_area, _] = Layout::vertical([
                Constraint::Percentage(12),
                Constraint::Percentage(76),
                Constraint::Percentage(12),
            ])
            .areas(body);
            let [_, box_area, _] = Layout::horizontal([
                Constraint::Percentage(8),
                Constraint::Percentage(84),
                Constraint::Percentage(8),
            ])
            .areas(box_area);
            frame.render_widget(Clear, box_area);
            let text = if matches!(page.dataset, "homework" | "phyvlab") {
                const LABELS: &[&str] = &["提交状态  ", "状态  "];
                Text::from(
                    content
                        .lines()
                        .map(|line| {
                            if let Some((label, status)) = LABELS
                                .iter()
                                .find_map(|label| line.strip_prefix(label).map(|s| (*label, s)))
                                && let Some(color) = submission_color(status)
                            {
                                return Line::from(vec![
                                    Span::raw(label),
                                    Span::styled(status, self.style(color)),
                                ]);
                            }
                            Line::raw(line)
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                Text::from(content.as_str())
            };
            frame.render_widget(
                Paragraph::new(text)
                    .scroll((*scroll, 0))
                    .block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .border_style(self.style(CYAN))
                            .title(Span::styled(
                                format!(" {}详情 ", page.title),
                                self.style(TEXT).add_modifier(Modifier::BOLD),
                            )),
                    )
                    .wrap(Wrap { trim: false }),
                box_area,
            );
        }
    }
}

fn room_query(key: &str) -> Option<(&str, &str, u8)> {
    let (term, rest) = key.strip_prefix("rooms:")?.split_once(':')?;
    let (building, week) = rest.split_once(':')?;
    Some((term, building, week.parse().ok()?))
}

fn latest_room_key(db: &Database, account: i64) -> Result<Option<String>> {
    Ok(db
        .sync_status(account)?
        .iter()
        .filter(|row| {
            row["dataset"]
                .as_str()
                .is_some_and(|s| s.starts_with("rooms:"))
                && row["last_success_at"].is_number()
        })
        .max_by_key(|row| row["last_success_at"].as_i64())
        .and_then(|row| row["dataset"].as_str().map(str::to_owned)))
}

fn time_label(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds)
        .map(|t| {
            t.to_zoned(campus_core::calendar::school_zone())
                .strftime("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn value_for(key: &str, row: &Value) -> String {
    match key {
        "weekday" => ["—", "周一", "周二", "周三", "周四", "周五", "周六", "周日"]
            .get((row["location_index"].as_u64().unwrap_or(0) % 8) as usize)
            .unwrap()
            .to_string(),
        "slot" => (row["location_index"].as_u64().unwrap_or(0) / 8 + 1).to_string(),
        "subStatus" => campus_core::parse::homework_status(row).into(),
        "week" => format!("第 {} 周", crate::output::cell(&row[key])),
        "file_count" => row["nodes"].as_array().map_or(0, Vec::len).to_string(),
        "last_success_at" => row[key].as_i64().map(time_label).unwrap_or_default(),
        "reason" => row[key].as_str().map(sync_reason).unwrap_or_default(),
        "status" => match row[key].as_str() {
            Some("success") => "成功".into(),
            Some("failed") => "失败".into(),
            Some("needs_user_action") => "需处理".into(),
            _ => crate::output::cell(&row[key]),
        },
        "course_name" => ["course_name", "courseName", "name", "title"]
            .iter()
            .find_map(|k| row[k].as_str())
            .map(crate::output::clean)
            .unwrap_or_default(),
        _ => crate::output::cell(&row[key]),
    }
}

fn sync_reason(reason: &str) -> String {
    crate::output::reason_label(reason)
        .map(str::to_owned)
        .unwrap_or_else(|| crate::output::clean(reason))
}

fn summarize(result: Result<Vec<Value>>) -> String {
    match result {
        Ok(rows) => {
            if let Some(path) = rows.first().and_then(|v| v["path"].as_str()) {
                return format!("已下载 {}", crate::output::clean(path));
            }
            let failures = rows
                .iter()
                .filter(|r| r["status"] != "success")
                .collect::<Vec<_>>();
            if failures.is_empty() {
                format!("同步完成 · {} 项成功", rows.len())
            } else {
                format!(
                    "{} 项成功 · {} 项失败 · {}",
                    rows.len() - failures.len(),
                    failures.len(),
                    failures[0]["reason"]
                        .as_str()
                        .map(sync_reason)
                        .unwrap_or_default()
                )
            }
        }
        Err(Error::SessionExpired) => "会话已过期，请运行 campus auth login。".into(),
        Err(e) => crate::output::clean(&e.to_string()),
    }
}
