//! What the settings window's fields change in the profile.

use autosplit_capture::DeviceInfo;
use autosplit_core::commands::Command;
use autosplit_core::profile::{TimerConnection, UserProfile, VIDEO_CAPTURE_DEVICE};

/// A field's new value, as the settings window has it.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingEdit {
    FpsLimit(i32),
    LiveCaptureRegion(bool),
    /// The capture method list's index: 0 is Video Capture Device, 1 the profile's own
    /// unsupported method when it has one.
    CaptureMethod(i32),
    /// The device list's index.
    CaptureDevice(i32),
    OpenScreenshot(bool),
    /// Screenshot on start, split, reset, skip, undo and pause.
    ScreenshotOn {
        start: bool,
        split: bool,
        reset: bool,
        skip: bool,
        undo: bool,
        pause: bool,
    },
    DefaultComparisonMethod(i32),
    DefaultSimilarityThreshold(String),
    DefaultDelayTime(String),
    DefaultPauseTime(String),
    LoopSplits(bool),
    StartAlsoResets(bool),
    EnableAutoReset(bool),
    /// 0 for hotkeys, 1 for LiveSplit One.
    TimerConnection(i32),
    LiveSplitOnePort(String),
}

impl SettingEdit {
    /// Whether the window shows something else after this edit. Other fields are being typed
    /// in, and refreshing them would move the cursor.
    pub fn changes_other_fields(&self) -> bool {
        matches!(self, Self::CaptureMethod(_) | Self::CaptureDevice(_) | Self::TimerConnection(_))
    }
}

/// Puts an edit into the profile. Returns `false` when the value isn't valid, like a
/// threshold above 1, which leaves the profile as it was; AutoSplit's spin boxes don't allow
/// those.
pub fn apply(profile: &mut UserProfile, edit: SettingEdit, devices: &[DeviceInfo]) -> bool {
    match edit {
        SettingEdit::FpsLimit(fps) if (20..=240).contains(&fps) => profile.fps_limit = fps.into(),
        SettingEdit::LiveCaptureRegion(on) => profile.live_capture_region = on,
        SettingEdit::CaptureMethod(0) => profile.capture_method = VIDEO_CAPTURE_DEVICE.into(),
        SettingEdit::CaptureMethod(_) => {}
        SettingEdit::CaptureDevice(index) => {
            let Some(device) = usize::try_from(index).ok().and_then(|index| devices.get(index)) else { return false };
            profile.capture_device_name = device.name.clone();
            profile.capture_device_id = device.index as i64;
            profile.capture_method = VIDEO_CAPTURE_DEVICE.into();
        }
        SettingEdit::OpenScreenshot(on) => profile.open_screenshot = on,
        SettingEdit::ScreenshotOn { start, split, reset, skip, undo, pause } => {
            // In AutoSplit's order
            profile.screenshot_on = [
                (split, Command::Split),
                (start, Command::Start),
                (pause, Command::Pause),
                (reset, Command::Reset),
                (skip, Command::Skip),
                (undo, Command::Undo),
            ]
            .into_iter()
            .filter_map(|(on, command)| on.then_some(command))
            .collect();
        }
        SettingEdit::DefaultComparisonMethod(method) => profile.default_comparison_method = method.into(),
        SettingEdit::DefaultSimilarityThreshold(text) => match text.trim().parse::<f64>() {
            Ok(value) if (0.0..=1.0).contains(&value) => profile.default_similarity_threshold = value,
            _ => return false,
        },
        SettingEdit::DefaultDelayTime(text) => match text.trim().parse::<i64>() {
            Ok(value) if value >= 0 => profile.default_delay_time = value,
            _ => return false,
        },
        SettingEdit::DefaultPauseTime(text) => match text.trim().parse::<f64>() {
            Ok(value) if value >= 0.0 && value.is_finite() => profile.default_pause_time = value,
            _ => return false,
        },
        SettingEdit::LoopSplits(on) => profile.loop_splits = on,
        SettingEdit::StartAlsoResets(on) => profile.start_also_resets = on,
        SettingEdit::EnableAutoReset(on) => profile.enable_auto_reset = on,
        SettingEdit::TimerConnection(index) => {
            profile.timer_connection =
                if index == 1 { TimerConnection::LiveSplitOne } else { TimerConnection::Hotkeys };
        }
        SettingEdit::LiveSplitOnePort(text) => match text.trim().parse::<u16>() {
            Ok(port) if port > 0 => profile.livesplit_one_port = port,
            _ => return false,
        },
        SettingEdit::FpsLimit(_) => return false,
    }
    true
}
