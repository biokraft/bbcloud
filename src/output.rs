use crate::error::Result;
use chrono::{DateTime, Utc};
use comfy_table::presets::UTF8_BORDERS_ONLY;
use comfy_table::{Cell, ContentArrangement, Table};
use owo_colors::OwoColorize;
use serde::Serialize;
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
}

impl Format {
    pub fn from_json_flag(json: bool) -> Self {
        if json {
            Format::Json
        } else {
            Format::Human
        }
    }

    pub fn is_json(self) -> bool {
        self == Format::Json
    }
}

pub fn color_enabled() -> bool {
    color_from(
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    )
}

fn color_from(is_tty: bool, no_color: bool) -> bool {
    is_tty && !no_color
}

pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_BORDERS_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().map(Cell::new));
    for row in rows {
        table.add_row(row);
    }
    table.to_string()
}

pub fn print_table(headers: &[&str], rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        info("nothing to show");
        return;
    }
    println!("{}", table(headers, rows));
}

fn success_line(msg: &str, color: bool) -> String {
    if color {
        format!("{} {}", "✓".green(), msg)
    } else {
        format!("✓ {msg}")
    }
}

fn info_line(msg: &str, color: bool) -> String {
    if color {
        msg.dimmed().to_string()
    } else {
        msg.to_string()
    }
}

fn warn_line(msg: &str, color: bool) -> String {
    if color {
        format!("{} {}", "!".yellow(), msg)
    } else {
        format!("! {msg}")
    }
}

fn heading_line(msg: &str, color: bool) -> String {
    if color {
        msg.bold().underline().to_string()
    } else {
        msg.to_string()
    }
}

pub fn success(msg: &str) {
    println!("{}", success_line(msg, color_enabled()));
}

pub fn info(msg: &str) {
    println!("{}", info_line(msg, color_enabled()));
}

pub fn warn(msg: &str) {
    eprintln!("{}", warn_line(msg, color_enabled()));
}

pub fn heading(msg: &str) {
    println!("{}", heading_line(msg, color_enabled()));
}

/// The meaning a cell carries, so callers pick intent rather than a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Bad,
    Warn,
    Good,
    Dim,
}

fn colored_cell_with(text: &str, tone: Tone, color: bool) -> String {
    if !color {
        return text.to_string();
    }
    match tone {
        Tone::Bad => text.red().to_string(),
        Tone::Warn => text.yellow().to_string(),
        Tone::Good => text.green().to_string(),
        Tone::Dim => text.dimmed().to_string(),
    }
}

pub fn colored_cell(text: &str, tone: Tone) -> String {
    colored_cell_with(text, tone, color_enabled())
}

pub fn spinner(msg: &str) -> indicatif::ProgressBar {
    if !std::io::stderr().is_terminal() {
        return indicatif::ProgressBar::hidden();
    }
    let pb = indicatif::ProgressBar::new_spinner();
    pb.enable_steady_tick(std::time::Duration::from_millis(90));
    pb.set_message(msg.to_string());
    pb
}

/// Human-friendly timestamp. Past values within a week are relative; anything
/// older, or in the future, renders as an absolute date.
pub fn relative_time(iso: &str) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(iso) else {
        return iso.to_string();
    };
    let parsed = parsed.with_timezone(&Utc);
    let now = Utc::now();

    if parsed > now {
        return parsed.format("%b %d, %Y").to_string();
    }

    let delta = now - parsed;
    let days = delta.num_days();
    if days > 7 {
        return parsed.format("%b %d, %Y").to_string();
    }
    if days >= 1 {
        return format!("{days} day{} ago", plural(days));
    }
    let hours = delta.num_hours();
    if hours >= 1 {
        return format!("{hours} hour{} ago", plural(hours));
    }
    let minutes = delta.num_minutes();
    format!("{minutes} minute{} ago", plural(minutes))
}

fn plural(n: i64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn relative_time_renders_minutes() {
        let ts = (Utc::now() - Duration::minutes(5)).to_rfc3339();
        assert_eq!(relative_time(&ts), "5 minutes ago");
    }

    #[test]
    fn relative_time_renders_singular_hour() {
        let ts = (Utc::now() - Duration::minutes(61)).to_rfc3339();
        assert_eq!(relative_time(&ts), "1 hour ago");
    }

    #[test]
    fn relative_time_renders_days() {
        let ts = (Utc::now() - Duration::days(3)).to_rfc3339();
        assert_eq!(relative_time(&ts), "3 days ago");
    }

    #[test]
    fn relative_time_falls_back_to_absolute_beyond_a_week() {
        let ts = (Utc::now() - Duration::days(30)).to_rfc3339();
        let shown = relative_time(&ts);
        assert!(
            !shown.contains("ago"),
            "expected absolute date, got {shown}"
        );
    }

    #[test]
    fn future_timestamps_render_absolute() {
        let ts = (Utc::now() + Duration::days(2)).to_rfc3339();
        let shown = relative_time(&ts);
        assert!(
            !shown.contains("ago"),
            "future must not be relative, got {shown}"
        );
    }

    #[test]
    fn unparseable_timestamp_is_passed_through() {
        assert_eq!(relative_time("not-a-date"), "not-a-date");
    }

    #[test]
    fn table_contains_headers_and_cells() {
        let out = table(&["ID", "TITLE"], vec![vec!["7".into(), "fix thing".into()]]);
        assert!(out.contains("ID"));
        assert!(out.contains("fix thing"));
    }

    #[test]
    fn format_from_flag() {
        assert!(matches!(Format::from_json_flag(true), Format::Json));
        assert!(matches!(Format::from_json_flag(false), Format::Human));
    }

    #[test]
    fn color_from_tty_without_no_color_is_true() {
        assert!(color_from(true, false));
    }

    #[test]
    fn color_from_tty_with_no_color_is_false() {
        assert!(!color_from(true, true));
    }

    #[test]
    fn color_from_non_tty_without_no_color_is_false() {
        assert!(!color_from(false, false));
    }

    #[test]
    fn color_from_non_tty_with_no_color_is_false() {
        assert!(!color_from(false, true));
    }

    #[test]
    fn success_line_has_no_escape_when_color_disabled() {
        let line = success_line("done", false);
        assert!(!line.contains('\x1b'));
        assert!(line.contains("done"));
    }

    #[test]
    fn success_line_has_escape_when_color_enabled() {
        let line = success_line("done", true);
        assert!(line.contains('\x1b'));
        assert!(line.contains("done"));
    }

    #[test]
    fn info_line_has_no_escape_when_color_disabled() {
        let line = info_line("note", false);
        assert!(!line.contains('\x1b'));
        assert!(line.contains("note"));
    }

    #[test]
    fn info_line_has_escape_when_color_enabled() {
        let line = info_line("note", true);
        assert!(line.contains('\x1b'));
        assert!(line.contains("note"));
    }

    #[test]
    fn warn_line_has_no_escape_when_color_disabled() {
        let line = warn_line("careful", false);
        assert!(!line.contains('\x1b'));
        assert!(line.contains("careful"));
    }

    #[test]
    fn warn_line_has_escape_when_color_enabled() {
        let line = warn_line("careful", true);
        assert!(line.contains('\x1b'));
        assert!(line.contains("careful"));
    }

    #[test]
    fn heading_line_has_no_escape_when_color_disabled() {
        let line = heading_line("Title", false);
        assert!(!line.contains('\x1b'));
        assert!(line.contains("Title"));
    }

    #[test]
    fn heading_line_has_escape_when_color_enabled() {
        let line = heading_line("Title", true);
        assert!(line.contains('\x1b'));
        assert!(line.contains("Title"));
    }

    #[test]
    fn colored_cell_is_plain_without_color() {
        assert_eq!(colored_cell_with("FAILED", Tone::Bad, false), "FAILED");
        assert_eq!(colored_cell_with("-", Tone::Dim, false), "-");
    }

    #[test]
    fn colored_cell_wraps_when_color_is_on() {
        let painted = colored_cell_with("FAILED", Tone::Bad, true);
        assert!(painted.contains("FAILED"));
        assert_ne!(painted, "FAILED", "expected an ansi escape around the text");
    }
}
