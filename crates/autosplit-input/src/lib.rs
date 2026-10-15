//! Global hotkeys and sending key presses, with AutoSplit's hotkey names.

mod keys;
mod names;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::collections::HashSet;
use std::sync::Arc;

use livesplit_hotkey::Hotkey;
#[cfg(not(target_os = "macos"))]
use livesplit_hotkey::{ConsumePreference, Hook};
#[cfg(target_os = "macos")]
use macos::Hook;

pub use livesplit_hotkey::{KeyCode, Modifiers};
pub use names::{HotkeyError, Layout, ParsedHotkey, UsLayout, hotkey_name, parse_hotkey, same_hotkey};

/// The layout hotkey names are read with: the system's on Windows (AutoSplit's names follow it
/// there, localized ones included), US elsewhere.
pub fn system_layout() -> &'static dyn Layout {
    #[cfg(windows)]
    return &windows::SystemLayout;
    #[allow(unreachable_code)]
    &UsLayout
}

/// What the physical `key` types on a US layout, without Shift, when it types anything.
pub fn us_character(key: KeyCode) -> Option<char> {
    keys::find(key)?.us.map(|(plain, _)| plain)
}

/// The character a hotkey recorded from the physical `key` is named by, where `typed` is what the
/// key types on the keyboard layout. Names are read back with [`system_layout`], so they have to
/// follow it: the layout's own character on Windows, and the key's US character elsewhere. There
/// the key labelled Z on a German keyboard records "y", its US name, and listens on that key.
pub fn recorded_character(key: KeyCode, typed: Option<char>) -> Option<char> {
    if cfg!(windows) { typed } else { us_character(key).or(typed) }
}

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("{0}")]
    Listen(String),
    #[error("{0}")]
    Send(String),
}

#[cfg(target_os = "linux")]
const LINUX_INPUT_GROUP_HELP: &str = "Global hotkeys need permission to read keyboard input. Add yourself to the \
     input group with `sudo usermod -a -G input $USER`, then log out and back in. (Any program you run can then \
     read your keystrokes.)";

/// A key combination no hotkey uses, to check that the hook still runs.
#[cfg(not(target_os = "macos"))]
const PROBE: Hotkey = Hotkey { key_code: KeyCode::F24, modifiers: Modifiers::all() };

/// Listens for hotkeys system wide. Keys are never swallowed, so the timer still sees them.
pub struct Listener {
    hook: Hook,
    /// Combinations with a hotkey. Tracked here because backends differ in which hotkey keeps a
    /// combination registered twice.
    taken: HashSet<Hotkey>,
    #[cfg(target_os = "linux")]
    devices: Vec<std::ffi::OsString>,
}

impl Listener {
    pub fn new() -> Result<Self, InputError> {
        #[cfg(target_os = "macos")]
        let hook = Hook::new()?;
        #[cfg(not(target_os = "macos"))]
        let hook = Hook::with_consume_preference(ConsumePreference::MustNotConsume).map_err(listen_error)?;
        Ok(Self {
            hook,
            taken: HashSet::new(),
            #[cfg(target_os = "linux")]
            devices: linux::input_devices(),
        })
    }

    /// Calls `action` whenever `hotkey` is pressed. Combinations another hotkey already has stay
    /// with it. Returns true when that takes this hotkey's own keys (for a single key, the key
    /// without modifiers), so it would never fire.
    pub fn register(&mut self, hotkey: &ParsedHotkey, action: impl Fn() + Send + Sync + 'static) -> bool {
        let action = Arc::new(action);
        let mut kept_by_another = false;
        for &combination in &hotkey.listen {
            if !self.taken.insert(combination) {
                kept_by_another |= hotkey.combo || combination.modifiers.is_empty();
                continue;
            }
            let action = action.clone();
            let _ = self.hook.register(combination, move || action());
        }
        kept_by_another
    }

    /// Stops listening for every hotkey.
    pub fn clear(&mut self) {
        for combination in self.taken.drain() {
            let _ = self.hook.unregister(combination);
        }
    }

    /// Stops listening for `hotkey`.
    pub fn unregister(&mut self, hotkey: &ParsedHotkey) {
        for combination in &hotkey.listen {
            if self.taken.remove(combination) {
                let _ = self.hook.unregister(*combination);
            }
        }
    }

    /// The hook stopped (on Linux, when any input device is unplugged), or on Linux a device was
    /// plugged in that the hook doesn't read. Create a new listener and register again.
    pub fn needs_restart(&self) -> bool {
        #[cfg(target_os = "macos")]
        let stopped = self.hook.stopped();
        #[cfg(not(target_os = "macos"))]
        let stopped = matches!(self.hook.unregister(PROBE), Err(livesplit_hotkey::Error::Platform(_)));
        #[cfg(target_os = "linux")]
        return stopped || linux::input_devices() != self.devices;
        #[allow(unreachable_code)]
        stopped
    }
}

#[cfg(not(target_os = "macos"))]
fn listen_error(error: livesplit_hotkey::Error) -> InputError {
    // Without access to the keyboards, the only Linux hook left is X11's, which swallows keys
    #[cfg(target_os = "linux")]
    if let livesplit_hotkey::Error::UnmatchedPreference = error {
        return InputError::Listen(LINUX_INPUT_GROUP_HELP.into());
    }
    #[cfg(target_os = "linux")]
    return InputError::Listen(format!("{LINUX_INPUT_GROUP_HELP} ({error})"));
    #[allow(unreachable_code)]
    InputError::Listen(error.to_string())
}

/// Sends key presses to whatever application listens for them.
pub struct Sender {
    #[cfg(target_os = "linux")]
    device: linux::Device,
}

impl Sender {
    pub fn new() -> Result<Self, InputError> {
        #[cfg(target_os = "macos")]
        macos::check_access()?;
        Ok(Self {
            #[cfg(target_os = "linux")]
            device: linux::Device::new()?,
        })
    }

    /// Presses `keys` in order, then releases them in reverse order.
    pub fn send(&mut self, keys: &[KeyCode]) -> Result<(), InputError> {
        let keys: Vec<&keys::Key> = keys
            .iter()
            .map(|&code| keys::find(code).ok_or_else(|| InputError::Send(format!("{code:?} can't be sent"))))
            .collect::<Result<_, _>>()?;
        #[cfg(windows)]
        return windows::send(&keys);
        #[cfg(target_os = "linux")]
        return self.device.send(&keys);
        #[cfg(target_os = "macos")]
        return macos::send(&keys);
        #[allow(unreachable_code)]
        {
            let _ = keys;
            Err(InputError::Send("sending keys isn't supported on this platform".into()))
        }
    }
}
