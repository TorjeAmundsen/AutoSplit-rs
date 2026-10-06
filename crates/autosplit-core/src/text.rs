//! How numbers and the run's status are shown in the UI.

use std::time::Duration;

use crate::runner::{ImageRef, ResetReading, Showing, Status};

/// Truncates to 3 decimals, so a similarity just under a threshold never shows as reaching it.
///
/// AutoSplit truncates to 2 decimals with `int(value * 100) / 100`, which float rounding
/// sometimes pushes one step too low (0.29 shows as 0.28). Truncating the shortest decimal form
/// of the value avoids that.
pub fn decimal(value: f64) -> String {
    let text = format!("{value}");
    if !value.is_finite() {
        return text;
    }
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let fraction: String = fraction.chars().chain(std::iter::repeat('0')).take(3).collect();
    let negative_zero = integer == "-0" && fraction == "000";
    format!("{}.{fraction}", if negative_zero { "0" } else { integer })
}

/// Pause and delay countdowns, with one decimal.
pub fn seconds_remaining_text(seconds: f64) -> String {
    let shown = format!("{seconds:.1}");
    let plural = if shown == "1.0" { "" } else { "s" };
    format!("{shown} second{plural} remaining")
}

/// [`decimal`], or `-` for no value.
pub fn decimal_or_dash(value: Option<f64>) -> String {
    value.map_or("-".into(), decimal)
}

/// The one-line status of a run. Numbers come before the filename, so cutting the line short
/// only loses part of the name. `idle` is shown when there's no image to show. Without a
/// capture, the similarity shows as `-` instead of the 0 that comparing nothing gives.
pub fn status_line(status: &Status, now: Duration, idle: &str, capture: bool) -> String {
    let left = |until: Duration| format!("{:.1}s", until.saturating_sub(now).as_secs_f64());
    let value = decimal_or_dash;
    let current = if capture { status.similarity } else { None };
    let similarity = format!("{} / {} (best {})", value(current), value(status.threshold), value(status.highest));
    let mut line = match status.showing {
        Showing::Nothing => return idle.to_owned(),
        Showing::StartDelay { until } => format!("starting in {}", left(until)),
        Showing::Pause { until } => format!("comparing again in {}", left(until)),
        Showing::SplitDelay { until } => format!("splitting in {}", left(until)),
        Showing::Image(ImageRef::Start) => format!("waiting for start  {similarity}"),
        Showing::Image(ImageRef::Split(_)) => {
            let position = status.split_position.map_or(String::new(), |(at, count)| format!("split {at}/{count}  "));
            let loops = match status.loop_position {
                Some((current, total)) if total > 1 => format!("  loop {current}/{total}"),
                _ => String::new(),
            };
            format!("{position}{similarity}{loops}")
        }
    };
    if let (true, Some(ResetReading::Similarity(similarity))) = (capture, status.reset_similarity) {
        line += &format!("  reset {} / {}", decimal(similarity), value(status.reset_threshold));
    }
    if let (Showing::Image(_), Some(filename)) = (status.showing, &status.current_filename) {
        line += &format!("  {filename}");
    }
    line
}
