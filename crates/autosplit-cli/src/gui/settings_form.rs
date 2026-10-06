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
    DefaultSimilarityThreshold(f64),
    DefaultDelayTime(f64),
    DefaultPauseTime(f64),
    LoopSplits(bool),
    StartAlsoResets(bool),
    EnableAutoReset(bool),
    /// 0 for hotkeys, 1 for LiveSplit One.
    TimerConnection(i32),
    LiveSplitOnePort(f64),
}

impl SettingEdit {
    /// Whether the window shows something else after this edit. Other fields are being typed
    /// in, and refreshing them would move the cursor.
    pub fn changes_other_fields(&self) -> bool {
        matches!(self, Self::CaptureMethod(_) | Self::CaptureDevice(_) | Self::TimerConnection(_))
    }
}

/// Puts an edit into the profile. Returns `false` when the value isn't valid, like a
/// threshold above 1, which leaves the profile as it was. The spin boxes don't allow those
/// either, like AutoSplit's.
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
        SettingEdit::DefaultSimilarityThreshold(value) if (0.0..=1.0).contains(&value) => {
            // The spin box's 2 decimals, without the float noise of its steps
            profile.default_similarity_threshold = (value * 100.0).round() / 100.0;
        }
        SettingEdit::DefaultDelayTime(value) if value >= 0.0 && value.is_finite() => {
            profile.default_delay_time = value.round() as i64;
        }
        SettingEdit::DefaultPauseTime(value) if value >= 0.0 && value.is_finite() => {
            profile.default_pause_time = (value * 100.0).round() / 100.0;
        }
        SettingEdit::LoopSplits(on) => profile.loop_splits = on,
        SettingEdit::StartAlsoResets(on) => profile.start_also_resets = on,
        SettingEdit::EnableAutoReset(on) => profile.enable_auto_reset = on,
        SettingEdit::TimerConnection(index) => {
            profile.timer_connection =
                if index == 1 { TimerConnection::LiveSplitOne } else { TimerConnection::Hotkeys };
        }
        SettingEdit::LiveSplitOnePort(port) if (1.0..=65535.0).contains(&port) => {
            profile.livesplit_one_port = port.round() as u16;
        }
        SettingEdit::FpsLimit(_)
        | SettingEdit::DefaultSimilarityThreshold(_)
        | SettingEdit::DefaultDelayTime(_)
        | SettingEdit::DefaultPauseTime(_)
        | SettingEdit::LiveSplitOnePort(_) => return false,
    }
    true
}
