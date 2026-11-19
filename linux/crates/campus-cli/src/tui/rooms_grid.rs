use super::{App, BLUE, MUTED};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DAYS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
const PERIODS: usize = 7;
const LABEL: u16 = 14;

// Colors follow the school legend. Symbols are used when the terminal colors are disabled.
const STATES: [(Color, &str, &str); 7] = [
    (Color::Reset, ".", "空闲"),
    (Color::Rgb(228, 104, 104), "P", "排课"),
    (Color::Rgb(158, 104, 104), "T", "调课"),
    (Color::Rgb(57, 78, 214), "K", "考试"),
    (Color::Rgb(119, 191, 109), "L", "实验"),
    (Color::Rgb(216, 204, 86), "O", "其他"),
    (Color::Reset, "?", "未知"),
];

fn occupancy(state: u8, width: u16, colored: bool) -> Span<'static> {
    let (color, symbol, _) = STATES[usize::from(state).min(STATES.len() - 1)];
    if colored && color != Color::Reset {
        Span::styled(" ".repeat(usize::from(width)), Style::default().bg(color))
    } else {
        Span::styled(
            center(symbol, width),
            if colored {
                Style::default().fg(MUTED)
            } else {
                Style::default()
            },
        )
    }
}

fn center(text: &str, width: u16) -> String {
    let width = usize::from(width);
    let used = text.width();
    let padding = width.saturating_sub(used);
    let left = padding / 2;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(padding - left))
}

fn room_label(name: &str, selected: bool) -> String {
    let mut label = if selected {
        "› ".to_owned()
    } else {
        "  ".to_owned()
    };
    let mut width = 2;
    for ch in name.chars() {
        let char_width = ch.width().unwrap_or(0);
        if width + char_width > usize::from(LABEL) {
            break;
        }
        label.push(ch);
        width += char_width;
    }
    label.push_str(&" ".repeat(usize::from(LABEL) - width));
    label
}

fn legend(width: u16, colored: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut cells = Vec::new();
    let mut used = 0;
    for (index, (_, _, label)) in STATES.iter().enumerate() {
        let item_width = label.width() + 4;
        if used + item_width > usize::from(width) && !cells.is_empty() {
            lines.push(Line::from(std::mem::take(&mut cells)));
            used = 0;
        }
        cells.push(occupancy(index as u8, 1, colored));
        cells.push(Span::raw(format!(" {label}  ")));
        used += item_width;
    }
    if !cells.is_empty() {
        lines.push(Line::from(cells));
    }
    lines
}

pub(super) fn draw(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    if area.height < 4 || area.width < LABEL + 8 {
        frame.render_widget(Paragraph::new("请放大终端以查看占用表。"), area);
        return;
    }
    let available = area.width - LABEL;
    let count = usize::from(available / (PERIODS as u16 + 1)).clamp(1, DAYS.len());
    let day_width = (available / count as u16 - 1).min(3 * PERIODS as u16);
    let cell_width = |period: usize| {
        day_width / PERIODS as u16 + u16::from(period < usize::from(day_width) % PERIODS)
    };
    app.first_room_day = app.first_room_day.min(DAYS.len() - count);
    let start_day = app.first_room_day;
    let end_day = start_day + count;

    let mut legend_rows = legend(area.width, app.colored);
    legend_rows.truncate(usize::from(area.height - 3));
    let [day_header, period_header, rows_area, legend_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(legend_rows.len() as u16),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(legend_rows), legend_area);

    let heading = app.style(BLUE).add_modifier(Modifier::BOLD);
    let separator = app.style(MUTED);
    let mut days = vec![Span::styled("  星期        ", heading)];
    let mut periods = vec![Span::styled("  教室 (容量) ", heading)];
    for (day, label) in DAYS.iter().enumerate().take(end_day).skip(start_day) {
        let date = app
            .room_monday
            .and_then(|monday| monday.checked_add(jiff::Span::new().days(day as i64)).ok());
        let title = match date {
            Some(date) if day_width < 10 => {
                format!("{label}{:02}/{:02}", date.month(), date.day())
            }
            Some(date) => format!("周{label} {:02}/{:02}", date.month(), date.day()),
            None => format!("周{label}"),
        };
        days.push(Span::styled("│", separator));
        days.push(Span::styled(center(&title, day_width), heading));
        periods.push(Span::styled("│", separator));
        for period in 0..PERIODS {
            periods.push(Span::styled(
                center(&(period + 1).to_string(), cell_width(period)),
                heading,
            ));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(days)), day_header);
    frame.render_widget(Paragraph::new(Line::from(periods)), period_header);

    let selected = app.selection.selected().unwrap_or(0);
    let rows_fit = usize::from(rows_area.height);
    let mut offset = app
        .selection
        .offset()
        .min(app.visible.len().saturating_sub(rows_fit));
    if selected < offset {
        offset = selected;
    } else if selected >= offset + rows_fit {
        offset = selected + 1 - rows_fit;
    }
    *app.selection.offset_mut() = offset;
    let mut lines = Vec::new();
    for (index, row_id) in app
        .visible
        .iter()
        .copied()
        .enumerate()
        .skip(offset)
        .take(rows_fit)
    {
        let fields = &app.rows[row_id];
        let name = format!("{} ({})", fields[0], fields[1]);
        let mut row = vec![Span::styled(
            room_label(&name, index == selected),
            if index == selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            },
        )];
        for day in start_day..end_day {
            row.push(Span::styled("│", separator));
            for period in 0..PERIODS {
                let state = app.room_cells[row_id][day * PERIODS + period];
                row.push(occupancy(state, cell_width(period), app.colored));
            }
        }
        lines.push(Line::from(row));
    }
    frame.render_widget(Paragraph::new(lines), rows_area);
}

pub(super) fn monday(
    weeks: &[campus_core::AcademicWeek],
    term: &str,
    week: u8,
    now: i64,
) -> Option<jiff::civil::Date> {
    let active = (term == "current")
        .then(|| campus_core::calendar::current_week(weeks, now))
        .flatten();
    let term = if term == "current" {
        &active.as_ref()?.0
    } else {
        term
    };
    weeks
        .iter()
        .find(|w| w.term == term && w.week == week)?
        .monday
        .parse()
        .ok()
}
