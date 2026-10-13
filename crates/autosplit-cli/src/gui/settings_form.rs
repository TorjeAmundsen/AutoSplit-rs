//! What the settings window's fields change in the profile.

use autosplit_capture::DeviceInfo;
use autosplit_core::commands::{Command, TimerLink};
use autosplit_core::profile::{TimerConnection, UserProfile, VIDEO_CAPTURE_DEVICE, capture_method_name};

/// A field's new value, as the settings window has it.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingEdit {
    FpsLimit(i32),
    LiveCaptureRegion(bool),
    DisplayPauseTimer(bool),
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
    GroupSplitImages(bool),
    SplitImageGrid(bool),
    StartAlsoResets(bool),
    EnableAutoReset(bool),
    /// 0 for hotkeys, 1 for LiveSplit One, 2 for Molok (only listed on Linux).
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
        SettingEdit::DisplayPauseTimer(on) => profile.display_pause_timer = on,
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
        SettingEdit::GroupSplitImages(on) => profile.group_split_images = on,
        SettingEdit::SplitImageGrid(on) => profile.split_image_grid = on,
        SettingEdit::StartAlsoResets(on) => profile.start_also_resets = on,
        SettingEdit::EnableAutoReset(on) => profile.enable_auto_reset = on,
        SettingEdit::TimerConnection(index) => {
            profile.timer_connection = match index {
                1 => TimerConnection::LiveSplitOne,
                #[cfg(target_os = "linux")]
                2 => TimerConnection::Molok,
                _ => TimerConnection::Hotkeys,
            };
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

/// Gives a profile without a capture device OBS Virtual Camera when it's listed, since that's what
/// most runs use, or else the first listed device like AutoSplit's list picks when filled. The
/// device list shows it selected. Returns whether it picked one.
pub fn pick_default_device(profile: &mut UserProfile, devices: &[DeviceInfo]) -> bool {
    if devices.is_empty() || profile.has_capture_device() || profile.window_capture_method().is_some() {
        return false;
    }
    let obs = devices.iter().position(|device| device.name.to_lowercase().contains("obs virtual cam"));
    apply(profile, SettingEdit::CaptureDevice(obs.unwrap_or(0) as i32), devices)
}

/// What the settings window shows that isn't a profile value as it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub capture_methods: Vec<String>,
    pub capture_method_index: i32,
    /// Shown instead of the devices, and disabling the list, when not empty.
    pub device_placeholder: &'static str,
    /// The devices, then the profile's device when it isn't connected, which picking does
    /// nothing for.
    pub device_names: Vec<String>,
    /// -1 when the profile has no device.
    pub device_index: i32,
    pub hotkeys_note: &'static str,
    pub timer_connection: i32,
    /// The profile asks for another timer connection than the one used, which waits for the
    /// run to end.
    pub timer_connection_waiting: bool,
}

impl Shown {
    /// `scanning` while the devices are being listed. `link` is how the session tells the timer.
    pub fn new(profile: &UserProfile, devices: &[DeviceInfo], scanning: bool, link: TimerLink) -> Self {
        // The capture methods, with the profile's window capture method when it has one
        let mut capture_methods = vec!["Video Capture Device (see below)".to_owned()];
        let other_method = !profile.capture_method.is_empty() && profile.capture_method != VIDEO_CAPTURE_DEVICE;
        if other_method {
            capture_methods.push(format!("{} (not supported)", capture_method_name(&profile.capture_method)));
        }
        let device_placeholder = if other_method {
            "Select \"Video Capture Device\" above"
        } else if scanning {
            "Scanning for existing devices..."
        } else if devices.is_empty() {
            "No device found."
        } else {
            ""
        };
        let selected = autosplit_capture::pick_device(devices, &profile.capture_device_name, profile.capture_device_id)
            .and_then(|(picked, _)| devices.iter().position(|device| device == picked));
        let mut device_names: Vec<String> = devices.iter().map(|device| device.name.clone()).collect();
        // Shown as not connected instead of an empty box
        let missing = selected.is_none() && !profile.capture_device_name.is_empty();
        if missing {
            device_names.push(format!("{} (not connected)", profile.capture_device_name));
        }
        let device_index = match selected {
            Some(index) => index as i32,
            None if missing => (device_names.len() - 1) as i32,
            None => -1,
        };
        Self {
            capture_methods,
            capture_method_index: i32::from(other_method),
            device_placeholder,
            device_names,
            device_index,
            hotkeys_note: match link {
                TimerLink::Hotkeys => "",
                #[cfg(target_os = "linux")]
                TimerLink::Molok => {
                    "Molok gets signals, so these hotkeys only tell autosplit what you did to the timer."
                }
                TimerLink::AutoControlled | TimerLink::LiveSplitOne => {
                    "LiveSplit or LiveSplit One controls the timer, so only the screenshot and auto reset hotkeys are used."
                }
            },
            timer_connection: match profile.timer_connection {
                TimerConnection::Hotkeys => 0,
                TimerConnection::LiveSplitOne => 1,
                #[cfg(target_os = "linux")]
                TimerConnection::Molok => 2,
                // Not listed, and used as hotkeys
                #[cfg(not(target_os = "linux"))]
                TimerConnection::Molok => 0,
            },
            timer_connection_waiting: link != TimerLink::AutoControlled
                && TimerLink::from(profile.timer_connection) != link,
        }
    }
}
