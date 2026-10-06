use livesplit_hotkey::KeyCode;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VkKeyScanW,
};

use crate::InputError;
use crate::keys::{self, Key};
use crate::names::Layout;

fn key_for_scan(scan: u16) -> Option<KeyCode> {
    keys::KEYS.iter().find(|key| key.scan == scan).map(|key| key.code)
}

/// Resolves names with the active keyboard layout, like the `keyboard` library does on Windows.
pub struct SystemLayout;

impl Layout for SystemLayout {
    fn key_for_char(&self, c: char) -> Option<(KeyCode, bool)> {
        let mut units = [0_u16; 2];
        let [unit] = c.encode_utf16(&mut units) else { return None };
        let scanned = unsafe { VkKeyScanW(*unit) };
        if scanned == -1 {
            return None;
        }
        let (vk, shift_state) = ((scanned & 0xFF) as u32, (scanned >> 8) & 0xFF);
        // Characters needing Ctrl or Alt (AltGr) can't be pressed as a plain hotkey key
        if shift_state & !1 != 0 {
            return None;
        }
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX) };
        let scan = if scan & 0xFF00 == 0xE000 { scan as u16 } else { (scan & 0xFF) as u16 };
        key_for_scan(scan).map(|key| (key, shift_state & 1 != 0))
    }

    fn key_for_name(&self, name: &str) -> Option<KeyCode> {
        let mut buffer = [0_u16; 64];
        for extended in [false, true] {
            for scan in 1..0x80_u16 {
                let lparam = ((scan as i32) << 16) | if extended { 1 << 24 } else { 0 };
                let length = unsafe { GetKeyNameTextW(lparam, &mut buffer) };
                if length <= 0 {
                    continue;
                }
                let key_name = String::from_utf16_lossy(&buffer[..length as usize]).to_lowercase();
                if key_name == name {
                    return key_for_scan(if extended { 0xE000 | scan } else { scan });
                }
            }
        }
        None
    }
}

fn input(key: &Key, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if key.scan & 0xFF00 == 0xE000 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(key.vk),
                wScan: key.scan & 0xFF,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Sends the virtual key with its scan code, so timers listening for either see the right key.
pub fn send(keys: &[&Key]) -> Result<(), InputError> {
    let inputs: Vec<INPUT> =
        keys.iter().map(|key| input(key, false)).chain(keys.iter().rev().map(|key| input(key, true))).collect();
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(InputError::Send(format!("Windows accepted {sent} of {} key events", inputs.len())))
    }
}
