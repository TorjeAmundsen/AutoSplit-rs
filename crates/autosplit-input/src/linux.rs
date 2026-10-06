//! Sends keys through a uinput virtual keyboard. Timers reading input devices directly (like
//! LiveSplit One with evdev) only see keys from real or uinput devices, which is why AutoSplit
//! uses uinput too.

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode as EvdevKey};

use crate::InputError;
use crate::keys::{self, Key};

const UINPUT_HELP: &str = "Sending key presses needs write access to /dev/uinput. Allow it with \
     `echo 'KERNEL==\"uinput\", TAG+=\"uaccess\"' | sudo tee /etc/udev/rules.d/50-uinput.rules`, then restart your computer.";

const UINPUT_MISSING_HELP: &str = "/dev/uinput doesn't exist, so key presses can't be sent. Load it with \
     `sudo modprobe uinput`, and keep it loaded after a restart with \
     `echo uinput | sudo tee /etc/modules-load.d/uinput.conf`. Under WSL, key presses can't be sent.";

/// The input devices the hotkey hook reads, to notice devices being plugged in.
pub fn input_devices() -> Vec<std::ffi::OsString> {
    let mut devices: Vec<_> = std::fs::read_dir("/dev/input")
        .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default();
    devices.sort();
    devices
}

pub struct Device(VirtualDevice);

impl Device {
    /// Created once and kept, because desktops can miss the first keys of a brand new device.
    pub fn new() -> Result<Self, InputError> {
        let error = |error: std::io::Error| match error.kind() {
            std::io::ErrorKind::NotFound => InputError::Send(UINPUT_MISSING_HELP.into()),
            _ => InputError::Send(format!("{UINPUT_HELP} ({error})")),
        };
        let mut supported = AttributeSet::<EvdevKey>::new();
        for key in keys::KEYS {
            supported.insert(EvdevKey::new(key.evdev));
        }
        let device =
            VirtualDevice::builder().map_err(error)?.name("AutoSplit").with_keys(&supported).map_err(error)?.build();
        device.map(Self).map_err(error)
    }

    pub fn send(&mut self, keys: &[&Key]) -> Result<(), InputError> {
        let event = |key: &Key, value: i32| InputEvent::new(EventType::KEY.0, key.evdev, value);
        let events: Vec<InputEvent> =
            keys.iter().map(|key| event(key, 1)).chain(keys.iter().rev().map(|key| event(key, 0))).collect();
        self.0.emit(&events).map_err(|error| InputError::Send(error.to_string()))
    }
}
