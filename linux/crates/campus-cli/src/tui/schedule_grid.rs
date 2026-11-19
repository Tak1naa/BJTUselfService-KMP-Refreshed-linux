use super::{App, BLUE, MUTED};
use campus_core::{AcademicWeek, Course, SLOTS};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

#[derive(Default)]
pub(super) struct Schedule {
    pub calendar: bool,
    pub week: Option<u8>,
    pub courses: Vec<Course>,
    pub weeks: Vec<AcademicWeek>,
    pub first_day: usize,
    pub follow_selection: bool,
}

impl Schedule {
    pub fn active_week(&self) -> u8 {
        self.week.unwrap_or(1)
    }
    pub fn change_week(&mut self, delta: i8) {
        self.week = Some(self.active_week().saturating_add_signed(delta).clamp(1, 30));
    }
}

fn fit(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    let mut used = 0;
    let mut text = String::new();
    for ch in value.chars() {
        let size = ch.width().unwrap_or(0);
        if used + size >= width {
            break;
        }
        text.push(ch);
        used += size;
    }
    text.push('…');
    text
}

pub(super) fn room_for_display(place: &str) -> &str {
    place
        .rsplit([',', '，'])
        .map(str::trim)
        .find(|part| !part.is_empty())
        .unwrap_or(place)
}

pub(super) fn draw(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    if area.width < 23 || area.height < 3 {
        frame.render_widget(Paragraph::new("请放大终端查看周历。"), area);
        return;
    }
    let label_width = if area.width >= 75 { 12u16 } else { 8 };
    let available = area.width - label_width;
    let count = usize::from(available / 13).clamp(1, 7);
    let width = available / count as u16 - 1;
    let week = app.schedule.active_week();
    let selected = app
        .selection
        .selected()
        .and_then(|i| app.visible.get(i))
        .copied();
    let focus = selected.map(|i| &app.schedule.courses[i]);
    let mut start = app.schedule.first_day.min(7 - count);
    if app.schedule.follow_selection {
        if let Some(day) = focus.and_then(|c| usize::from(c.weekday()).checked_sub(1)) {
            if day < start {
                start = day;
            }
            if day >= start + count {
                start = day + 1 - count;
            }
        }
        app.schedule.follow_selection = false;
    }
    app.schedule.first_day = start.min(7 - count);
    let days = app.schedule.first_day..app.schedule.first_day + count;
    let date = super::rooms_grid::monday(&app.schedule.weeks, "current", week, crate::now());
    let mut heading = vec!["节次".to_owned()];
    for day in days.clone() {
        let label = DAYS[day];
        let title = date
            .and_then(|monday| monday.checked_add(jiff::Span::new().days(day as i64)).ok())
            .map_or_else(
                || label.to_owned(),
                |d| format!("{label} {:02}/{:02}", d.month(), d.day()),
            );
        heading.push(title);
    }

    let row_height = ((area.height - 1) / 7).clamp(1, 3);
    let slot_count = usize::from((area.height - 1) / row_height).clamp(1, SLOTS.len());
    let focus_slot = focus.map_or(0, |c| usize::from(c.slot()));
    let first_slot = focus_slot
        .saturating_add(1)
        .saturating_sub(slot_count)
        .min(SLOTS.len() - slot_count);
    let grid = || std::array::from_fn(|_| Vec::new());
    let mut grid: [[Vec<usize>; 7]; SLOTS.len()] = std::array::from_fn(|_| grid());
    for &idx in &app.visible {
        let c = &app.schedule.courses[idx];
        if let Some(day) = usize::from(c.weekday()).checked_sub(1)
            && let Some(cell) = grid
                .get_mut(usize::from(c.slot()))
                .and_then(|row| row.get_mut(day))
        {
            cell.push(idx);
        }
    }
    let rows = (first_slot..first_slot + slot_count).map(|slot| {
        let mut cells = vec![Cell::from(format!("第 {} 节", slot + 1)).style(app.style(MUTED))];
        for day in days.clone() {
            let items = &grid[slot][day];
            let idx = selected
                .filter(|i| items.contains(i))
                .or_else(|| items.first().copied());
            let Some(idx) = idx else {
                cells.push(Cell::from("·").style(app.style(MUTED)));
                continue;
            };
            let course = &app.schedule.courses[idx];
            let is_selected = Some(idx) == selected;
            let style = if is_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else if app.colored {
                Style::default().bg(Color::Rgb(31, 44, 58))
            } else {
                Style::default()
            };
            let suffix = if items.len() > 1 {
                format!(" +{}", items.len() - 1)
            } else {
                String::new()
            };
            let title = fit(
                &crate::output::clean(&course.name),
                usize::from(width).saturating_sub(suffix.width()),
            );
            let mut lines = vec![Line::from(Span::styled(
                format!("{title}{suffix}"),
                Style::default().add_modifier(Modifier::BOLD),
            ))];
            if row_height >= 2 {
                lines.push(Line::from(fit(
                    &crate::output::clean(room_for_display(&course.place)),
                    width.into(),
                )));
            }
            if row_height >= 3
                && let Some((start, end)) = course.time_range(true)
            {
                lines.push(Line::from(format!(
                    "{:02}:{:02}–{:02}:{:02}",
                    start / 60,
                    start % 60,
                    end / 60,
                    end % 60
                )));
            }
            cells.push(Cell::from(lines).style(style));
        }
        Row::new(cells).height(row_height)
    });
    let constraints = std::iter::once(Constraint::Length(label_width))
        .chain(std::iter::repeat_n(Constraint::Length(width), count));
    let table = Table::new(rows, constraints)
        .column_spacing(1)
        .header(Row::new(heading).style(app.style(BLUE).add_modifier(Modifier::BOLD)));
    frame.render_widget(table, area);
}
