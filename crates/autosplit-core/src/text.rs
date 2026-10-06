//! How numbers and the run's status are shown in the UI.

use std::time::Duration;

use crate::commands::Command;
use crate::runner::{ImageRef, ResetReading, Showing, StartStatus, Status};

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

/// The texts of AutoSplit's main window for a run's status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowTexts {
    /// The current image's file name, `-` when none is shown.
    pub current_file: String,
    /// Shown instead of the current image: a countdown, or `idle`.
    pub current_message: String,
    /// The similarity viewer: live, highest and threshold, for the current image then the
    /// reset image.
    pub table: [String; 6],
    pub image_loop: String,
    pub start_status: String,
}

/// AutoSplit's main window texts. `idle` is shown when there's no image to show. Without a
/// capture, the live similarities show as `-` instead of the 0 that comparing nothing gives.
pub fn window_texts(status: &Status, now: Duration, idle: &str, capture: bool) -> WindowTexts {
    let left = |until: Duration| seconds_remaining_text(until.saturating_sub(now).as_secs_f64());
    let current_message = match status.showing {
        Showing::StartDelay { until } => format!("Delayed Before Starting:\n {}", left(until)),
        Showing::Pause { until } => format!("None (Paused). {}", left(until)),
        Showing::SplitDelay { until } => format!("Delayed Split: {}", left(until)),
        Showing::Image(_) => String::new(),
        Showing::Nothing => idle.to_owned(),
    };
    let current_live = decimal_or_dash(if capture { status.similarity } else { None });
    let reset = |live: String| [live, decimal_or_dash(status.reset_highest), decimal_or_dash(status.reset_threshold)];
    let [reset_live, reset_highest, reset_threshold] = match status.reset_similarity {
        // Like AutoSplit: dashes until the run starts, N/A during a run without a reset image
        None => ["-".into(), "-".into(), "-".into()],
        Some(ResetReading::NotApplicable) => ["N/A".into(), "N/A".into(), "N/A".into()],
        Some(ResetReading::Disabled) => reset("disabled".into()),
        Some(ResetReading::Paused) => reset("paused".into()),
        Some(ResetReading::Similarity(similarity)) => reset(if capture { decimal(similarity) } else { "-".into() }),
    };
    WindowTexts {
        current_file: status.current_filename.clone().unwrap_or_else(|| "-".into()),
        current_message,
        table: [
            current_live,
            decimal_or_dash(status.highest),
            decimal_or_dash(status.threshold),
            reset_live,
            reset_highest,
            reset_threshold,
        ],
        image_loop: status.loop_position.map_or("N/A".into(), |(at, of)| format!("{at}/{of}")),
        start_status: match status.start_status {
            StartStatus::NotFound => "not found",
            StartStatus::Ready => "ready",
            StartStatus::DelayingStart => "delaying start...",
            StartStatus::Started => "started",
            StartStatus::NotReady => "not ready",
        }
        .into(),
    }
}

/// The image compared before a tick, to say in the log what split.
#[derive(Clone, Debug, PartialEq)]
pub struct Compared {
    pub filename: String,
    pub threshold: Option<f64>,
    pub below: bool,
}

impl Compared {
    /// Follows the image the status shows. It's kept through a split delay, which shows no
    /// image, so the delayed split still says which image split.
    pub fn follow(compared: &mut Option<Compared>, status: &Status) {
        if let Some(filename) = &status.current_filename
            && compared.as_ref().is_none_or(|compared| compared.filename != *filename)
        {
            *compared = Some(Compared { filename: filename.clone(), threshold: status.threshold, below: status.below });
        }
    }

    /// What split, for the log: the image and the similarity that matched. Only splits and
    /// pauses come from an image.
    pub fn details(compared: Option<&Compared>, command: Command, similarity: Option<f64>) -> String {
        let Some(Compared { filename, threshold, below }) = compared else { return String::new() };
        if !matches!(command, Command::Split | Command::Pause) {
            return String::new();
        }
        let (similarity, threshold) = (decimal_or_dash(similarity), decimal_or_dash(*threshold));
        if *below {
            format!(" ({filename} dropped to {similarity}, below {threshold})")
        } else {
            format!(" ({filename} at {similarity} / {threshold})")
        }
    }
}
