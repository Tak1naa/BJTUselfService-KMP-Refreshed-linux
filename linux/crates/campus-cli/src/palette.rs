#![allow(dead_code)]
//! One cool, dark-friendly palette shared by the TUI and the plain terminal renderer.
//! Colors are tuned for contrast on dark backgrounds; every meaning also carries a
//! symbol or word so output stays readable with `NO_COLOR` or `--color never`.

pub type Rgb = (u8, u8, u8);

/// Primary accent: active borders, headings, selected tab.
pub const ACCENT: Rgb = (122, 162, 247);
/// Secondary accent: keys in the legend, links, informational badges.
pub const CYAN: Rgb = (125, 207, 255);
pub const TEAL: Rgb = (115, 218, 202);
pub const PURPLE: Rgb = (187, 154, 247);
/// Status: success / free / submitted.
pub const GREEN: Rgb = (158, 206, 106);
/// Status: warning / stale / in progress.
pub const AMBER: Rgb = (224, 175, 104);
/// Status: error / busy / pending.
pub const RED: Rgb = (247, 118, 142);
/// Body text on dark backgrounds.
pub const TEXT: Rgb = (192, 202, 245);
/// Secondary metadata: timestamps, hints, column rules.
pub const MUTED: Rgb = (115, 122, 162);
/// Inactive borders and grid lines.
pub const FAINT: Rgb = (65, 72, 104);
/// Raised surfaces: grid cards and the selected-row background.
pub const SURFACE: Rgb = (36, 40, 59);
pub const SELECTION: Rgb = (51, 59, 91);
