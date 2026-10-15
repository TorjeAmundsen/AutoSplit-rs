//! How numbers and the run's status are shown in the UI.

use std::time::Duration;

use crate::commands::Command;
use crate::runner::{ImageRef, ResetReading, Showing, StartStatus, Status};
use crate::split_folder::MAX_GROUP_SIZE;
use crate::split_image::ImageType;
use crate::tester::{Phase, Tester, Trigger, TriggerKind};

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
    /// The current image's file name, `-` when none is shown. A split with several images
    /// says how many instead.
    pub current_file: String,
    /// Shown instead of the current image: a countdown, or `idle`.
    pub current_message: String,
    /// The similarity viewer: live, highest and threshold, for the current image then the
    /// reset image.
    pub table: [String; 6],
    /// The similarity viewer's current image rows when split images are grouped: live,
    /// highest and threshold of each image in the group. The first row is the start image
    /// while it's shown.
    pub group_rows: [[String; 3]; MAX_GROUP_SIZE],
    pub image_loop: String,
    pub start_status: String,
}

/// AutoSplit's main window texts. `idle` is shown when there's no image to show. Without a
/// capture, the live similarities show as `-` instead of the 0 that comparing nothing gives.
/// Without `pause_timer` (the profile's `display_pause_timer`), pauses and split delays don't
/// show the seconds left, like AutoSplit.
pub fn window_texts(status: &Status, now: Duration, idle: &str, capture: bool, pause_timer: bool) -> WindowTexts {
    let left = |until: Duration| seconds_remaining_text(until.saturating_sub(now).as_secs_f64());
    let with_timer = |message: &str, until: Duration| {
        if pause_timer { format!("{message} {}", left(until)) } else { message.to_owned() }
    };
    let current_message = match status.showing {
        Showing::StartDelay { until } => format!("Delayed Before Starting:\n {}", left(until)),
        Showing::Pause { until } => with_timer("None (Paused).", until),
        Showing::SplitDelay { until } => with_timer("Delayed Split:", until),
        Showing::Image(_) => String::new(),
        Showing::Nothing => idle.to_owned(),
    };
    let live = |similarity: Option<f64>| decimal_or_dash(if capture { similarity } else { None });
    let current = [live(status.similarity), decimal_or_dash(status.highest), decimal_or_dash(status.threshold)];
    let group_rows = std::array::from_fn(|slot| match status.group.get(slot) {
        Some(reading) => [live(reading.similarity), decimal_or_dash(reading.highest), decimal(reading.threshold)],
        None if slot == 0 && status.group.is_empty() => current.clone(),
        None => ["-".into(), "-".into(), "-".into()],
    });
    let [current_live, current_highest, current_threshold] = current;
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
        current_file: match &status.current_filename {
            Some(_) if status.group.len() > 1 => format!("Comparing against {} images", status.group.len()),
            Some(filename) => filename.clone(),
            None => "-".into(),
        },
        current_message,
        table: [current_live, current_highest, current_threshold, reset_live, reset_highest, reset_threshold],
        group_rows,
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

/// The name of the cell that says what the image does, which has the most to say.
pub const ACTS_CELL: &str = "ACTS";

/// One cell of the Live Tester's information about its image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfoCell {
    pub name: &'static str,
    pub value: String,
    /// Where the value comes from, when that's the profile, or why it doesn't apply.
    pub note: String,
}

/// What the Live Tester's image does and how it's compared, one cell per value. The values its name
/// leaves out come from the profile.
pub fn tester_info(tester: &Tester) -> Vec<InfoCell> {
    let image = tester.image();
    let defaults = tester.defaults();
    let cell = |name, value: String, note: &str| InfoCell { name, value, note: note.to_owned() };
    let from_profile = |named: bool| if named { "" } else { "from profile" };
    let reset = image.image_type == ImageType::Reset;
    let action = match action(tester) {
        action if tester.below() => format!("{action} once it drops back below"),
        action => action.to_owned(),
    };
    let delay = match tester.delay() {
        _ if reset => cell("DELAY", "none".into(), "reset images don't wait"),
        _ if image.is_dummy() => cell("DELAY", "none".into(), "dummies don't wait"),
        0 => cell("DELAY", "none".into(), from_profile(image.names_delay())),
        delay => cell("DELAY", format!("{delay} ms"), from_profile(image.names_delay())),
    };
    vec![
        cell(ACTS_CELL, action, if image.is_dummy() { "dummy" } else { "" }),
        match image.comparison_method(defaults) {
            Some(method) => cell("METHOD", method.name().into(), from_profile(image.names_comparison_method())),
            None => cell("METHOD", "unknown".into(), "always compares as 0"),
        },
        cell("THRESHOLD", decimal(tester.threshold()), from_profile(image.names_threshold())),
        delay,
    ]
}

/// The values in the Live Tester's image's name that are ignored, or nothing.
pub fn tester_ignored(tester: &Tester) -> String {
    let warnings: Vec<String> = tester.image().warnings.iter().map(ToString::to_string).collect();
    if warnings.is_empty() { String::new() } else { format!("Ignored in the name: {}", warnings.join("; ")) }
}

/// What the Live Tester's image does when it triggers, as in "it {action}".
fn action(tester: &Tester) -> &'static str {
    let image = tester.image();
    match image.image_type {
        ImageType::Start if image.is_dummy() => "starts the run without the start hotkey",
        ImageType::Start => "starts the run",
        ImageType::Reset => "resets the run",
        ImageType::Split if image.is_dummy() => "moves to the next image without splitting",
        ImageType::Split if image.is_pause() => "pauses",
        ImageType::Split => "splits",
    }
}

/// The caption of a frame the Live Tester kept.
pub fn trigger_caption(tester: &Tester, trigger: &Trigger) -> String {
    let image = tester.image();
    let similarity = decimal(trigger.similarity);
    let acted = match image.image_type {
        ImageType::Start => "Start",
        ImageType::Reset => "Reset",
        ImageType::Split if image.is_dummy() => "Next image",
        ImageType::Split if image.is_pause() => "Pause",
        ImageType::Split => "Split",
    };
    match trigger.kind {
        TriggerKind::Reached => format!("Reached: {similarity}"),
        TriggerKind::DroppedBelow => format!("Dropped below: {similarity}"),
        TriggerKind::Acted { delay } if delay > 0 => format!("{acted} +{delay} ms: {similarity}"),
        TriggerKind::Acted { .. } => format!("{acted}: {similarity}"),
    }
}

/// What the Live Tester is waiting for.
pub fn tester_status(tester: &Tester) -> String {
    match tester.phase() {
        Phase::Waiting => "Waiting for the similarity to reach the threshold.".into(),
        Phase::Armed => "Reached the threshold. Waiting for it to drop back below.".into(),
        Phase::Delaying { delay, .. } => format!("Waiting for the {delay} ms delay."),
        Phase::Done => {
            format!("Done: this is where it {}. Press Start Over to watch for the next match.", action(tester))
        }
    }
}
