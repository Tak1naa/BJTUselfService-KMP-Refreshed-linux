mod terminal;

use crate::args::{Args, Command, Format};
use campus_core::{Error, Result};
use serde_json::{Value, json};
pub(crate) use terminal::friendly_error;
pub use terminal::{error, help_color, help_styles, reason_label};
pub fn envelope(
    command: &str,
    account: Option<&str>,
    now: i64,
    freshness: (Option<i64>, bool, &str),
    data: Value,
    warnings: &[String],
) -> Value {
    let (fetched_at, stale, source) = freshness;
    json!({"schema":1,"command":command,"account":account,"generated_at":now,"freshness":{"fetched_at":fetched_at,"stale":stale,"source":source},"data":data,"warnings":warnings})
}
pub fn json_data<T: serde::Serialize>(
    command: &str,
    account: Option<&str>,
    now: i64,
    freshness: (Option<i64>, bool, &str),
    data: &T,
    warnings: &[String],
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Freshness<'a> {
        fetched_at: Option<i64>,
        stale: bool,
        source: &'a str,
    }
    #[derive(serde::Serialize)]
    struct Envelope<'a, T> {
        schema: u8,
        command: &'a str,
        account: Option<&'a str>,
        generated_at: i64,
        freshness: Freshness<'a>,
        data: &'a T,
        warnings: &'a [String],
    }
    let (fetched_at, stale, source) = freshness;
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    serde_json::to_writer(
        &mut out,
        &Envelope {
            schema: 1,
            command,
            account,
            generated_at: now,
            freshness: Freshness {
                fetched_at,
                stale,
                source,
            },
            data,
            warnings,
        },
    )
    .map_err(|_| Error::Storage("could not write JSON output".into()))?;
    writeln!(out).map_err(|_| Error::Storage("could not write JSON output".into()))?;
    Ok(())
}
pub(crate) fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
pub(crate) fn cell(v: &Value) -> String {
    clean(&match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        _ => v.to_string(),
    })
}
pub fn render(args: &Args, value: &Value) -> Result<()> {
    if args.json || matches!(args.format, Format::Json) {
        println!(
            "{}",
            serde_json::to_string(value).map_err(|_| Error::Malformed("output JSON"))?
        );
    } else if matches!(args.format, Format::Tsv) {
        let shown = &value["data"];
        let rows = shown
            .as_array()
            .map_or_else(|| std::slice::from_ref(shown), Vec::as_slice);
        if let Some(first) = rows.first().filter(|r| r.is_object()) {
            let keys = columns(args, first);
            println!("{}", keys.join("\t"));
            for row in rows {
                let cells: Vec<_> = keys.iter().map(|key| cell(&row[key])).collect();
                println!("{}", cells.join("\t"));
            }
        } else if rows.is_empty() {
            println!("暂无数据");
        } else {
            for row in rows {
                println!("{}", cell(row));
            }
        }
        if !args.quiet {
            for warning in warnings(value) {
                eprintln!("campus: {warning}");
            }
        }
    } else {
        terminal::render(args, value);
    }
    Ok(())
}

fn warnings(value: &Value) -> impl Iterator<Item = &str> {
    value["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn columns<'a>(args: &Args, first: &'a Value) -> Vec<&'a str> {
    let preferred: &[&str] = match &args.command {
        Command::Grades { detail: true, .. } => &[
            "index",
            "course_name",
            "term",
            "score_raw",
            "credits_raw",
            "detail",
        ],
        Command::Grades { .. } => &[
            "index",
            "course_name",
            "teacher",
            "term",
            "score_raw",
            "credits_raw",
        ],
        Command::Schedule { .. } => &["name", "teacher", "weekday", "slot", "place", "school_time"],
        Command::Exams { .. } => &["course_name", "time_place_raw", "status"],
        _ => &[],
    };
    let present: Vec<_> = preferred
        .iter()
        .copied()
        .filter(|key| first.get(*key).is_some())
        .collect();
    if present.is_empty() {
        return first
            .as_object()
            .into_iter()
            .flat_map(|r| r.keys().map(String::as_str))
            .collect();
    }
    present
}

pub fn topup_qr() -> String {
    let matrix = include_str!("../assets/topup-matrix.txt");
    let width = matrix.lines().next().map(str::len).unwrap_or(0);
    let mut rows = vec![" ".repeat((width + 6) * 2); 3];
    rows.extend(matrix.lines().map(|r| {
        format!(
            "      {}      ",
            r.chars()
                .map(|c| if c == '1' { "██" } else { "  " })
                .collect::<String>()
        )
    }));
    rows.extend(vec![" ".repeat((width + 6) * 2); 3]);
    rows.join("\n")
}
pub fn csv(rows: &[Value]) -> String {
    let keys: Vec<_> = rows
        .first()
        .and_then(Value::as_object)
        .map(|v| v.keys().cloned().collect())
        .unwrap_or_default();
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut lines = vec![keys.iter().map(|s| quote(s)).collect::<Vec<_>>().join(",")];
    for r in rows {
        lines.push(
            keys.iter()
                .map(|k| quote(&cell(&r[k])))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    lines.join("\r\n") + "\r\n"
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_escape_is_removed() {
        assert!(!clean("\x1b[31m\n").contains('\x1b'));
    }
}
