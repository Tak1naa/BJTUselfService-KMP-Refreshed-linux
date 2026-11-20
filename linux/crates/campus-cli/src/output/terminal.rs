use super::{cell, clean, columns, warnings};
use crate::args::{Args, Card, Color, Command, Format, Rooms};
use anstyle::{AnsiColor, RgbColor, Style};
use campus_core::Error;
use serde_json::{Value, json};
use std::{ffi::OsString, io::IsTerminal};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy)]
struct Accent(RgbColor, AnsiColor);

const BLUE: Accent = Accent(RgbColor(86, 156, 214), AnsiColor::Blue);
const CYAN: Accent = Accent(RgbColor(156, 220, 254), AnsiColor::Cyan);
const GREEN: Accent = Accent(RgbColor(181, 206, 168), AnsiColor::Green);
const PURPLE: Accent = Accent(RgbColor(197, 134, 192), AnsiColor::Magenta);
const AMBER: Accent = Accent(RgbColor(226, 192, 141), AnsiColor::Yellow);
const RED: Accent = Accent(RgbColor(248, 81, 73), AnsiColor::Red);
const MUTED: Accent = Accent(RgbColor(157, 157, 157), AnsiColor::BrightBlack);

impl Accent {
    fn style(self, bold: bool) -> Style {
        let rgb = matches!(
            std::env::var("COLORTERM").as_deref(),
            Ok("truecolor" | "24bit")
        );
        let style = Style::new().fg_color(Some(if rgb { self.0.into() } else { self.1.into() }));
        if bold { style.bold() } else { style }
    }
}

fn color_available() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::env::var("TERM").as_deref() != Ok("dumb")
}

fn color_enabled(args: &Args, terminal: bool) -> bool {
    color_available()
        && match args.color {
            Color::Never => false,
            Color::Always => true,
            Color::Auto => terminal,
        }
}

pub fn help_color(raw: &[OsString]) -> clap::ColorChoice {
    use clap::ColorChoice;
    if !color_available() {
        return ColorChoice::Never;
    }
    let mut options = raw.iter().skip(1);
    let mut color = ColorChoice::Auto;
    while let Some(arg) = options.next() {
        if arg == "--" {
            break;
        }
        let value = if arg == "--color" {
            options.next().and_then(|arg| arg.to_str())
        } else {
            arg.to_str().and_then(|arg| arg.strip_prefix("--color="))
        };
        match value {
            Some("never") => color = ColorChoice::Never,
            Some("always") => color = ColorChoice::Always,
            Some("auto") => color = ColorChoice::Auto,
            _ => {}
        }
    }
    color
}

pub fn help_styles() -> clap::builder::Styles {
    clap::builder::Styles::styled()
        .header(BLUE.style(true))
        .usage(BLUE.style(true))
        .literal(CYAN.style(true))
        .placeholder(MUTED.style(false))
        .error(RED.style(true))
        .valid(GREEN.style(false))
        .invalid(AMBER.style(false))
}

fn identity(command: &Command) -> (&'static str, &'static str, Accent) {
    match command {
        Command::Auth { .. } => ("auth", "账号", PURPLE),
        Command::Schedule { .. } => ("schedule", "课表", BLUE),
        Command::Grades { .. } => ("grades", "成绩", GREEN),
        Command::Exams { .. } => ("exams", "考试", AMBER),
        Command::Rooms { .. } => ("rooms", "教室", CYAN),
        Command::Card { .. } => ("card", "一卡通", GREEN),
        Command::Calendar { .. } => ("calendar", "学年历", BLUE),
        Command::Sync { .. } => ("sync", "同步", CYAN),
        Command::Export { .. } => ("export", "导出", PURPLE),
        Command::Changes { .. } => ("changes", "数据变动", AMBER),
        Command::Courseware { .. } => ("courseware", "课件", CYAN),
        Command::Homework { .. } => ("homework", "作业", PURPLE),
        Command::Phyvlab { .. } => ("phyvlab", "物理在线", CYAN),
        Command::Config { .. } => ("config", "设置", BLUE),
        Command::Cache { .. } => ("cache", "本地缓存", GREEN),
        Command::Completions { .. } => ("completions", "Shell 补全", CYAN),
        Command::Tui => ("tui", "交大自由行 CLI", BLUE),
        #[cfg(feature = "developer")]
        Command::Debug { .. } => ("debug", "本地调试", AMBER),
    }
}

fn paint(enabled: bool, style: Style, s: &str) -> String {
    if enabled && !s.is_empty() {
        format!("{style}{s}{style:#}")
    } else {
        s.to_owned()
    }
}

pub fn reason_label(reason: &str) -> Option<&'static str> {
    match reason {
        "secure_channel" => Some("学校服务未建立有效的 HTTPS 会话，跳转已拒绝"),
        "smart_legacy_http" => Some("智慧教学返回旧版 HTTP 地址，未发送会话请求"),
        "session_expired" | "login_required" => Some("需要重新登录"),
        "account_required" => Some("尚未配置物理在线独立账号"),
        "captcha_required" => Some("需要完成物理在线人机验证"),
        "credentials_rejected" => Some("物理在线账号或密码无效"),
        "phyvlab_profile_incomplete" => Some("物理在线要求先在网页完善个人资料"),
        "network" => Some("网络请求失败"),
        "malformed_response" => Some("学校服务返回的内容无法解析"),
        _ => None,
    }
}

pub fn error(args: &Args, error: &Error) {
    if args.json || matches!(args.format, Format::Json) {
        let mut body = json!({"code":error.exit_code(),"message":error.to_string()});
        if let Error::CacheUnavailable(reason) = error {
            body["reason"] = json!(reason);
        }
        println!("{}", json!({"schema":1,"error":body}));
        return;
    }
    if matches!(args.format, Format::Tsv) {
        eprintln!("campus: {error}");
        return;
    }
    let (name, title, _) = identity(&args.command);
    let colored = color_enabled(args, std::io::stderr().is_terminal());
    let label = paint(colored, RED.style(true), "错误");
    let message = match error {
        Error::CacheUnavailable(reason) => format!(
            "没有可用缓存。上次同步失败：{}。",
            reason_label(reason).unwrap_or("原因未知")
        ),
        Error::NoData => "没有可用缓存或找不到指定记录。".into(),
        Error::SessionExpired => "没有有效会话，请运行 campus auth login。".into(),
        Error::SecureChannel => "学校服务未建立有效的 HTTPS 会话，跳转已拒绝。".into(),
        Error::SmartLegacyHttp => "智慧教学返回旧版 HTTP 地址，未发送会话请求。".into(),
        Error::ActionRequired(reason) => reason_label(reason)
            .unwrap_or("需要人工完成验证或配置")
            .into(),
        other => clean(&other.to_string()),
    };
    eprintln!("{label}  {title}  │  {message}");
    if args.quiet {
        return;
    }
    let next = match (error, name) {
        (Error::CacheUnavailable(reason), "homework" | "courseware")
            if reason == "smart_legacy_http" =>
        {
            format!(
                "如已手动关闭兼容，可运行 campus config set {name}.legacy_http on 恢复；否则请通过学校智慧教学平台查看。"
            )
        }
        (Error::SmartLegacyHttp, "homework" | "courseware") => {
            format!("如已手动关闭兼容，可运行 campus config set {name}.legacy_http on 恢复。")
        }
        (Error::SmartLegacyHttp, _) => {
            "如已手动关闭兼容，可运行 campus config set homework.legacy_http on / courseware.legacy_http on 恢复。".into()
        }
        (Error::CacheUnavailable(reason), "homework" | "courseware")
            if reason == "secure_channel" =>
        {
            "请通过学校智慧教学平台查看。同步记录可用 campus sync status 查看。".into()
        }
        (Error::ActionRequired(reason), _) if *reason == "phyvlab_profile_incomplete" => {
            "请在浏览器打开 https://phyvlab.bjtu.edu.cn/ 完善并保存个人资料后再同步".into()
        }
        (Error::CacheUnavailable(reason), "phyvlab") if reason == "phyvlab_profile_incomplete" => {
            "请在浏览器打开 https://phyvlab.bjtu.edu.cn/ 完善并保存个人资料后再同步".into()
        }
        (Error::ActionRequired(_), _) => "配置或完成物理在线验证：campus phyvlab login".into(),
        (Error::CacheUnavailable(reason), "phyvlab")
            if matches!(
                reason.as_str(),
                "account_required" | "captcha_required" | "credentials_rejected"
            ) =>
        {
            "配置或完成物理在线验证：campus phyvlab login".into()
        }
        (Error::CacheUnavailable(reason), _)
            if reason == "session_expired" || reason == "login_required" =>
        {
            "重新登录后再试：campus auth login".into()
        }
        (
            Error::NoData | Error::CacheUnavailable(_),
            "homework" | "courseware" | "phyvlab" | "schedule" | "grades" | "exams" | "card"
            | "calendar",
        ) => {
            format!("获取最新数据：campus {name} --refresh")
        }
        _ => return,
    };
    eprintln!("{}  {next}", paint(colored, CYAN.style(false), "下一步"));
}

pub fn render(args: &Args, value: &Value) {
    let width = terminal_size::terminal_size_of(std::io::stdout())
        .map(|(w, _)| usize::from(w.0))
        .filter(|w| *w > 0)
        .unwrap_or(100);
    let colored = color_enabled(args, std::io::stdout().is_terminal());
    let (name, title, accent) = identity(&args.command);
    let table = Table {
        width,
        colored,
        accent,
    };
    let status = if value["freshness"]["source"] == "network" {
        "  │  已联网更新"
    } else if value["freshness"]["stale"] == true {
        "  │  缓存可能过期"
    } else if value["freshness"]["fetched_at"].is_number() {
        "  │  本地缓存"
    } else {
        ""
    };
    let heading = paint(colored, accent.style(true), title);
    let command = value["command"]
        .as_str()
        .filter(|s| s.starts_with(name))
        .unwrap_or(name);
    println!(
        "{heading}  {}",
        paint(
            colored,
            MUTED.style(false),
            &format!("campus {command}{status}")
        )
    );

    let data = &value["data"];
    let records = data
        .as_array()
        .map_or_else(|| std::slice::from_ref(data), Vec::as_slice);
    if records.is_empty() || data.is_null() {
        println!(
            "{}",
            if name == "homework" {
                "没有作业记录。"
            } else {
                "暂无匹配记录。"
            }
        );
    } else if matches!(
        args.command,
        Command::Card {
            action: Some(Card::Topup),
            ..
        }
    ) {
        println!(
            "{}\n{}\n{}",
            cell(&data["link"]),
            data["qr_ascii"].as_str().unwrap_or(""),
            cell(&data["notice"])
        );
    } else if matches!(args.command, Command::Schedule { grid: true, .. }) && width >= 96 {
        schedule_grid(&table, records);
    } else if matches!(
        args.command,
        Command::Rooms {
            action: Rooms::Grid { .. }
        }
    ) {
        room_grid(&table, records);
    } else if matches!(
        args.command,
        Command::Rooms {
            action: Rooms::Live { .. }
        }
    ) {
        room_live(&table, data);
    } else if let Some(object) = data.as_object() {
        let rows: Vec<_> = object
            .iter()
            .map(|(key, value)| (field_label(key).to_owned(), present(key, value)))
            .map(|(key, value)| vec![key, value])
            .collect();
        table.print(&["字段", "内容"], &rows);
    } else if records[0].is_object() {
        let preferred: &[&str] = match name {
            "homework" => &["id", "course_name", "title", "end_time", "subStatus"],
            "phyvlab" => &[
                "id",
                "course_name",
                "name",
                "category",
                "progress_percent",
                "title",
                "due_text",
                "status_label",
                "grade_text",
                "path",
                "bytes",
            ],
            "sync" => &[
                "dataset",
                "status",
                "count",
                "changes",
                "last_success_at",
                "reason",
            ],
            "courseware" => &[
                "id",
                "resId",
                "course_name",
                "courseName",
                "rpName",
                "name",
                "extName",
                "path",
                "bytes",
                "segments",
            ],
            "rooms" => &["id", "name", "room", "capacity"],
            _ => &[],
        };
        let mut keys: Vec<_> = preferred
            .iter()
            .copied()
            .filter(|key| records.iter().any(|r| r.get(*key).is_some()))
            .collect();
        if keys.is_empty() {
            keys = columns(args, &records[0]);
        }
        let headers: Vec<_> = keys.iter().map(|key| field_label(key)).collect();
        let current = if name == "calendar"
            && matches!(
                &args.command, Command::Calendar { view, .. } if view == "weeks"
            ) {
            let weeks: Vec<campus_core::AcademicWeek> = records
                .iter()
                .filter_map(|row| serde_json::from_value(row.clone()).ok())
                .collect();
            campus_core::calendar::current_week(&weeks, crate::now())
        } else {
            None
        };
        let rows: Vec<Vec<String>> = records
            .iter()
            .map(|r| {
                keys.iter()
                    .map(|key| match *key {
                        "subStatus" => campus_core::parse::homework_status(r).into(),
                        "week"
                            if current.as_ref().is_some_and(|(term, week)| {
                                r["term"] == term.as_str() && r["week"] == *week
                            }) =>
                        {
                            format!("{} 本周", present(key, &r[*key]))
                        }
                        _ => present(key, &r[*key]),
                    })
                    .collect()
            })
            .collect();
        table.print(&headers, &rows);
    } else {
        for record in records {
            println!("{}", cell(record));
        }
    }
    if !args.quiet {
        let colored = color_enabled(args, std::io::stderr().is_terminal());
        let label = paint(colored, AMBER.style(true), "提示");
        for warning in warnings(value) {
            eprintln!("{label}  {}", clean(warning));
        }
    }
}

fn field_label(key: &str) -> &str {
    match key {
        "index" => "序号",
        "id" | "resId" => "ID",
        "name" => "名称",
        "course_name" | "courseName" => "课程",
        "teacher" => "教师",
        "weekday" => "星期",
        "slot" => "节次",
        "place" => "地点",
        "school_time" => "上课时间",
        "time_place_raw" => "时间与地点",
        "term" => "学期",
        "week" => "周次",
        "monday" => "周一日期",
        "score_raw" => "成绩",
        "weighted_average" => "加权均分",
        "gpa" => "学分绩点",
        "gpa_scale" => "绩点满分",
        "graded_courses" => "计入课程数",
        "estimated_average" => "均分含字母折算",
        "ambiguous_failures" => "待确认的 F 课程",
        "credits_raw" => "学分",
        "detail" => "详情",
        "title" => "标题",
        "end_time" | "due_text" => "截止时间",
        "open_text" => "开放时间",
        "content" | "description" => "内容",
        "subStatus" | "submission_status" => "提交状态",
        "status_label" => "状态",
        "submission_date_text" => "提交时间",
        "timing" => "提交时间状态",
        "grading_status" => "批改状态",
        "grade_text" => "成绩",
        "feedback_text" => "教师评语",
        "submitted_files" => "已交文件",
        "attachments" => "老师附件",
        "can_submit" => "可上传草稿",
        "can_finalize" => "可最终提交",
        "is_draft" => "草稿状态",
        "category" => "分类",
        "progress_percent" => "完成进度(%)",
        "configured" => "已配置独立账号",
        "phyvlab_user" => "物理在线账号",
        "saved" => "草稿已保存",
        "finalized" => "已最终提交",
        "password_saved" => "密码已安全保存",
        "password_forgotten" => "已清除保存密码",
        "room" => "教室",
        "capacity" => "容量",
        "card_balance" => "一卡通余额",
        "net_fee" => "网费余额",
        "new_mail" => "未读邮件",
        "dataset" => "数据",
        "status" => "状态",
        "count" => "记录",
        "changes" => "变动",
        "reason" => "原因",
        "last_success_at" => "最近成功（北京时间）",
        "path" => "路径",
        "bytes" => "字节",
        "segments" => "分片",
        "rpName" => "文件",
        "extName" => "格式",
        "session_saved" => "已保存会话",
        "verified" => "已在线验证",
        "note" => "说明",
        _ => key,
    }
}

fn present(key: &str, value: &Value) -> String {
    const DAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
    match (key, value) {
        ("weekday", Value::Number(n)) => n
            .as_u64()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| DAYS.get(n as usize))
            .map_or_else(|| cell(value), ToString::to_string),
        ("status", Value::String(s)) => match s.as_str() {
            "success" => "成功",
            "failed" => "失败",
            "needs_user_action" => "需处理",
            _ => return clean(s),
        }
        .into(),
        ("content", Value::String(s)) => campus_core::parse::homework_text(s),
        ("weighted_average" | "gpa", Value::Number(n)) => n
            .as_f64()
            .map_or_else(|| cell(value), |v| format!("{v:.3}")),
        ("reason", Value::String(s)) => {
            reason_label(s).map_or_else(|| clean(s), ToString::to_string)
        }
        ("last_success_at", Value::Number(n)) => n
            .as_i64()
            .and_then(|t| jiff::Timestamp::from_second(t).ok())
            .map(|t| {
                t.to_zoned(campus_core::calendar::school_zone())
                    .strftime("%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| cell(value)),
        (_, Value::Bool(b)) => if *b { "是" } else { "否" }.into(),
        (_, Value::Array(items)) if items.iter().all(Value::is_string) => {
            if items.is_empty() {
                "—".into()
            } else {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(clean)
                    .collect::<Vec<_>>()
                    .join("，")
            }
        }
        (_, Value::Null) => "—".into(),
        _ => cell(value),
    }
}

struct Table {
    width: usize,
    colored: bool,
    accent: Accent,
}

impl Table {
    fn print(&self, headers: &[&str], rows: &[Vec<String>]) {
        if headers.is_empty() {
            return;
        }
        let minimum: Vec<_> = headers.iter().map(|h| h.width().max(2)).collect();
        let available = self.width.saturating_sub((headers.len() - 1) * 3);
        if minimum.iter().sum::<usize>() > available || available < headers.len() * 4 {
            self.stacked(headers, rows);
            return;
        }
        let targets: Vec<_> = minimum
            .iter()
            .enumerate()
            .map(|(i, min)| {
                rows.iter()
                    .map(|r| r[i].width())
                    .max()
                    .unwrap_or(0)
                    .max(*min)
            })
            .collect();
        let mut widths = minimum;
        let mut remaining = available - widths.iter().sum::<usize>();
        while remaining != 0 {
            let prior = remaining;
            for (width, target) in widths.iter_mut().zip(&targets) {
                if *width < *target && remaining > 0 {
                    *width += 1;
                    remaining -= 1;
                }
            }
            if remaining == prior {
                break;
            }
        }
        let separator = paint(self.colored, MUTED.style(false), " │ ");
        let border = widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┼─");
        let headings = headers
            .iter()
            .zip(&widths)
            .map(|(h, w)| paint(self.colored, self.accent.style(true), &pad(h, *w, false)))
            .collect::<Vec<_>>()
            .join(&separator);
        println!(
            "{headings}\n{}",
            paint(self.colored, MUTED.style(false), &border)
        );
        for row in rows {
            let lines: Vec<_> = row.iter().zip(&widths).map(|(s, w)| wrap(s, *w)).collect();
            let height = lines.iter().map(Vec::len).max().unwrap_or(1);
            for i in 0..height {
                let line = lines
                    .iter()
                    .zip(&widths)
                    .zip(row)
                    .map(|((lines, w), s)| {
                        let text = lines.get(i).map(String::as_str).unwrap_or("");
                        let padded = pad(text, *w, s.parse::<f64>().is_ok());
                        self.paint_cell(s, &padded)
                    })
                    .collect::<Vec<_>>()
                    .join(&separator);
                println!("{line}");
            }
        }
    }

    fn stacked(&self, headers: &[&str], rows: &[Vec<String>]) {
        for (i, row) in rows.iter().enumerate() {
            if i != 0 {
                println!();
            }
            for (key, value) in headers.iter().zip(row) {
                println!("{}", paint(self.colored, self.accent.style(true), key));
                for line in wrap(value, self.width.saturating_sub(2).max(2)) {
                    println!("  {}", self.paint_cell(value, &line));
                }
            }
        }
    }

    fn paint_cell(&self, cell: &str, padded: &str) -> String {
        let accent = match cell {
            "成功" | "已提交" | "按时提交" => GREEN,
            "失败" | "未提交" | "已逾期" => RED,
            value if value.ends_with(" 本周") => GREEN,
            "需处理" | "草稿 · 待最终提交" | "逾期提交" | "待提交" => AMBER,
            _ => return padded.into(),
        };
        paint(self.colored, accent.style(true), padded)
    }
}

fn pad(s: &str, width: usize, right: bool) -> String {
    let spaces = " ".repeat(width.saturating_sub(s.width()));
    if right {
        format!("{spaces}{s}")
    } else {
        format!("{s}{spaces}")
    }
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for c in s.chars() {
        let n = c.width().unwrap_or(0);
        if used + n > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(c);
        used += n;
    }
    lines.push(line);
    lines
}

fn schedule_grid(table: &Table, records: &[Value]) {
    let rows: Vec<_> = (1..=7)
        .map(|slot| {
            let mut row = vec![slot.to_string()];
            for day in 1..=7 {
                let names = records
                    .iter()
                    .filter(|r| r["slot"] == slot && r["weekday"] == day)
                    .map(|r| format!("{} {}", cell(&r["name"]), cell(&r["place"])))
                    .collect::<Vec<_>>()
                    .join(" / ");
                row.push(names);
            }
            row
        })
        .collect();
    table.print(
        &[
            "节次", "周一", "周二", "周三", "周四", "周五", "周六", "周日",
        ],
        &rows,
    );
}

fn room_grid(table: &Table, records: &[Value]) {
    let rows: Vec<_> = records
        .iter()
        .map(|r| {
            let mut row = vec![cell(&r["room"]), cell(&r["capacity"])];
            for day in 0..7 {
                row.push(
                    (0..7)
                        .map(|slot| match r["cells"][day * 7 + slot].as_u64() {
                            Some(0) => '.',
                            Some(1..=5) => '#',
                            _ => '?',
                        })
                        .collect(),
                );
            }
            row
        })
        .collect();
    table.print(
        &[
            "教室", "容量", "周一", "周二", "周三", "周四", "周五", "周六", "周日",
        ],
        &rows,
    );
    println!(". 空闲   # 占用   ? 未知  │  每天从第 1 节到第 7 节");
}

fn room_live(table: &Table, data: &Value) {
    let rows: Vec<_> = data["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            let percentage = row[1]
                .as_f64()
                .map_or_else(|| "—".into(), |n| format!("{n:.1}%"));
            vec![cell(&row[0]), percentage, cell(&row[2]), cell(&row[3])]
        })
        .collect();
    if rows.is_empty() {
        println!("暂无匹配教室。");
    } else {
        table.print(&["教室", "使用率", "人数", "容量"], &rows);
        if let (Some(start), Some(end)) = (data["time"][0].as_str(), data["time"][1].as_str()) {
            println!("统计时段  {} – {}", clean(start), clean(end));
        }
    }
}
