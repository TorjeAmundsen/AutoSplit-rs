//! Physical keys and their codes on each platform.

use livesplit_hotkey::KeyCode;

/// A physical key and how each platform identifies it.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code, reason = "each platform reads only its own codes")]
pub struct Key {
    pub code: KeyCode,
    /// Windows scan code set 1, with 0xE000 added for extended keys.
    pub scan: u16,
    /// Windows virtual key code.
    pub vk: u16,
    /// Linux evdev `KEY_*` code.
    pub evdev: u16,
    /// macOS virtual key code (`kVK_*`), when the key exists there.
    pub mac: Option<u16>,
    /// The characters the key types on a US layout, without and with Shift.
    pub us: Option<(char, char)>,
}

const fn key(code: KeyCode, scan: u16, vk: u16, evdev: u16, mac: Option<u16>, us: Option<(char, char)>) -> Key {
    Key { code, scan, vk, evdev, mac, us }
}

use KeyCode::*;

pub const KEYS: &[Key] = &[
    key(KeyA, 0x1E, 0x41, 30, Some(0x00), Some(('a', 'A'))),
    key(KeyB, 0x30, 0x42, 48, Some(0x0B), Some(('b', 'B'))),
    key(KeyC, 0x2E, 0x43, 46, Some(0x08), Some(('c', 'C'))),
    key(KeyD, 0x20, 0x44, 32, Some(0x02), Some(('d', 'D'))),
    key(KeyE, 0x12, 0x45, 18, Some(0x0E), Some(('e', 'E'))),
    key(KeyF, 0x21, 0x46, 33, Some(0x03), Some(('f', 'F'))),
    key(KeyG, 0x22, 0x47, 34, Some(0x05), Some(('g', 'G'))),
    key(KeyH, 0x23, 0x48, 35, Some(0x04), Some(('h', 'H'))),
    key(KeyI, 0x17, 0x49, 23, Some(0x22), Some(('i', 'I'))),
    key(KeyJ, 0x24, 0x4A, 36, Some(0x26), Some(('j', 'J'))),
    key(KeyK, 0x25, 0x4B, 37, Some(0x28), Some(('k', 'K'))),
    key(KeyL, 0x26, 0x4C, 38, Some(0x25), Some(('l', 'L'))),
    key(KeyM, 0x32, 0x4D, 50, Some(0x2E), Some(('m', 'M'))),
    key(KeyN, 0x31, 0x4E, 49, Some(0x2D), Some(('n', 'N'))),
    key(KeyO, 0x18, 0x4F, 24, Some(0x1F), Some(('o', 'O'))),
    key(KeyP, 0x19, 0x50, 25, Some(0x23), Some(('p', 'P'))),
    key(KeyQ, 0x10, 0x51, 16, Some(0x0C), Some(('q', 'Q'))),
    key(KeyR, 0x13, 0x52, 19, Some(0x0F), Some(('r', 'R'))),
    key(KeyS, 0x1F, 0x53, 31, Some(0x01), Some(('s', 'S'))),
    key(KeyT, 0x14, 0x54, 20, Some(0x11), Some(('t', 'T'))),
    key(KeyU, 0x16, 0x55, 22, Some(0x20), Some(('u', 'U'))),
    key(KeyV, 0x2F, 0x56, 47, Some(0x09), Some(('v', 'V'))),
    key(KeyW, 0x11, 0x57, 17, Some(0x0D), Some(('w', 'W'))),
    key(KeyX, 0x2D, 0x58, 45, Some(0x07), Some(('x', 'X'))),
    key(KeyY, 0x15, 0x59, 21, Some(0x10), Some(('y', 'Y'))),
    key(KeyZ, 0x2C, 0x5A, 44, Some(0x06), Some(('z', 'Z'))),
    key(Digit1, 0x02, 0x31, 2, Some(0x12), Some(('1', '!'))),
    key(Digit2, 0x03, 0x32, 3, Some(0x13), Some(('2', '@'))),
    key(Digit3, 0x04, 0x33, 4, Some(0x14), Some(('3', '#'))),
    key(Digit4, 0x05, 0x34, 5, Some(0x15), Some(('4', '$'))),
    key(Digit5, 0x06, 0x35, 6, Some(0x17), Some(('5', '%'))),
    key(Digit6, 0x07, 0x36, 7, Some(0x16), Some(('6', '^'))),
    key(Digit7, 0x08, 0x37, 8, Some(0x1A), Some(('7', '&'))),
    key(Digit8, 0x09, 0x38, 9, Some(0x1C), Some(('8', '*'))),
    key(Digit9, 0x0A, 0x39, 10, Some(0x19), Some(('9', '('))),
    key(Digit0, 0x0B, 0x30, 11, Some(0x1D), Some(('0', ')'))),
    key(Minus, 0x0C, 0xBD, 12, Some(0x1B), Some(('-', '_'))),
    key(Equal, 0x0D, 0xBB, 13, Some(0x18), Some(('=', '+'))),
    key(BracketLeft, 0x1A, 0xDB, 26, Some(0x21), Some(('[', '{'))),
    key(BracketRight, 0x1B, 0xDD, 27, Some(0x1E), Some((']', '}'))),
    key(Backslash, 0x2B, 0xDC, 43, Some(0x2A), Some(('\\', '|'))),
    key(Semicolon, 0x27, 0xBA, 39, Some(0x29), Some((';', ':'))),
    key(Quote, 0x28, 0xDE, 40, Some(0x27), Some(('\'', '"'))),
    key(Backquote, 0x29, 0xC0, 41, Some(0x32), Some(('`', '~'))),
    key(Comma, 0x33, 0xBC, 51, Some(0x2B), Some((',', '<'))),
    key(Period, 0x34, 0xBE, 52, Some(0x2F), Some(('.', '>'))),
    key(Slash, 0x35, 0xBF, 53, Some(0x2C), Some(('/', '?'))),
    key(IntlBackslash, 0x56, 0xE2, 86, Some(0x0A), None),
    key(Enter, 0x1C, 0x0D, 28, Some(0x24), None),
    key(Escape, 0x01, 0x1B, 1, Some(0x35), None),
    key(Backspace, 0x0E, 0x08, 14, Some(0x33), None),
    key(Tab, 0x0F, 0x09, 15, Some(0x30), None),
    key(Space, 0x39, 0x20, 57, Some(0x31), Some((' ', ' '))),
    key(CapsLock, 0x3A, 0x14, 58, Some(0x39), None),
    key(F1, 0x3B, 0x70, 59, Some(0x7A), None),
    key(F2, 0x3C, 0x71, 60, Some(0x78), None),
    key(F3, 0x3D, 0x72, 61, Some(0x63), None),
    key(F4, 0x3E, 0x73, 62, Some(0x76), None),
    key(F5, 0x3F, 0x74, 63, Some(0x60), None),
    key(F6, 0x40, 0x75, 64, Some(0x61), None),
    key(F7, 0x41, 0x76, 65, Some(0x62), None),
    key(F8, 0x42, 0x77, 66, Some(0x64), None),
    key(F9, 0x43, 0x78, 67, Some(0x65), None),
    key(F10, 0x44, 0x79, 68, Some(0x6D), None),
    key(F11, 0x57, 0x7A, 87, Some(0x67), None),
    key(F12, 0x58, 0x7B, 88, Some(0x6F), None),
    key(F13, 0x64, 0x7C, 183, Some(0x69), None),
    key(F14, 0x65, 0x7D, 184, Some(0x6B), None),
    key(F15, 0x66, 0x7E, 185, Some(0x71), None),
    key(F16, 0x67, 0x7F, 186, Some(0x6A), None),
    key(F17, 0x68, 0x80, 187, Some(0x40), None),
    key(F18, 0x69, 0x81, 188, Some(0x4F), None),
    key(F19, 0x6A, 0x82, 189, Some(0x50), None),
    key(F20, 0x6B, 0x83, 190, Some(0x5A), None),
    key(F21, 0x6C, 0x84, 191, None, None),
    key(F22, 0x6D, 0x85, 192, None, None),
    key(F23, 0x6E, 0x86, 193, None, None),
    key(F24, 0x76, 0x87, 194, None, None),
    key(PrintScreen, 0xE037, 0x2C, 99, None, None),
    key(ScrollLock, 0x46, 0x91, 70, None, None),
    key(Pause, 0x45, 0x13, 119, None, None),
    key(Insert, 0xE052, 0x2D, 110, Some(0x72), None),
    key(Home, 0xE047, 0x24, 102, Some(0x73), None),
    key(PageUp, 0xE049, 0x21, 104, Some(0x74), None),
    key(Delete, 0xE053, 0x2E, 111, Some(0x75), None),
    key(End, 0xE04F, 0x23, 107, Some(0x77), None),
    key(PageDown, 0xE051, 0x22, 109, Some(0x79), None),
    key(ArrowRight, 0xE04D, 0x27, 106, Some(0x7C), None),
    key(ArrowLeft, 0xE04B, 0x25, 105, Some(0x7B), None),
    key(ArrowDown, 0xE050, 0x28, 108, Some(0x7D), None),
    key(ArrowUp, 0xE048, 0x26, 103, Some(0x7E), None),
    key(NumLock, 0xE045, 0x90, 69, Some(0x47), None),
    key(NumpadDivide, 0xE035, 0x6F, 98, Some(0x4B), None),
    key(NumpadMultiply, 0x37, 0x6A, 55, Some(0x43), None),
    key(NumpadSubtract, 0x4A, 0x6D, 74, Some(0x4E), None),
    key(NumpadAdd, 0x4E, 0x6B, 78, Some(0x45), None),
    key(NumpadEnter, 0xE01C, 0x0D, 96, Some(0x4C), None),
    key(Numpad1, 0x4F, 0x61, 79, Some(0x53), None),
    key(Numpad2, 0x50, 0x62, 80, Some(0x54), None),
    key(Numpad3, 0x51, 0x63, 81, Some(0x55), None),
    key(Numpad4, 0x4B, 0x64, 75, Some(0x56), None),
    key(Numpad5, 0x4C, 0x65, 76, Some(0x57), None),
    key(Numpad6, 0x4D, 0x66, 77, Some(0x58), None),
    key(Numpad7, 0x47, 0x67, 71, Some(0x59), None),
    key(Numpad8, 0x48, 0x68, 72, Some(0x5B), None),
    key(Numpad9, 0x49, 0x69, 73, Some(0x5C), None),
    key(Numpad0, 0x52, 0x60, 82, Some(0x52), None),
    key(NumpadDecimal, 0x53, 0x6E, 83, Some(0x41), None),
    key(NumpadComma, 0x7E, 0x6C, 121, Some(0x5F), None),
    key(ContextMenu, 0xE05D, 0x5D, 127, None, None),
    key(ControlLeft, 0x1D, 0xA2, 29, Some(0x3B), None),
    key(ShiftLeft, 0x2A, 0xA0, 42, Some(0x38), None),
    key(AltLeft, 0x38, 0xA4, 56, Some(0x3A), None),
    key(MetaLeft, 0xE05B, 0x5B, 125, Some(0x37), None),
    key(ControlRight, 0xE01D, 0xA3, 97, Some(0x3E), None),
    key(ShiftRight, 0x36, 0xA1, 54, Some(0x3C), None),
    key(AltRight, 0xE038, 0xA5, 100, Some(0x3D), None),
    key(MetaRight, 0xE05C, 0x5C, 126, Some(0x36), None),
    key(AudioVolumeMute, 0xE020, 0xAD, 113, Some(0x4A), None),
    key(AudioVolumeDown, 0xE02E, 0xAE, 114, Some(0x49), None),
    key(AudioVolumeUp, 0xE030, 0xAF, 115, Some(0x48), None),
    key(MediaTrackNext, 0xE019, 0xB0, 163, None, None),
    key(MediaTrackPrevious, 0xE010, 0xB1, 165, None, None),
    key(MediaStop, 0xE024, 0xB2, 166, None, None),
    key(MediaPlayPause, 0xE022, 0xB3, 164, None, None),
    key(BrowserBack, 0xE06A, 0xA6, 158, None, None),
    key(BrowserForward, 0xE069, 0xA7, 159, None, None),
    key(BrowserRefresh, 0xE067, 0xA8, 173, None, None),
    key(BrowserStop, 0xE068, 0xA9, 128, None, None),
    key(BrowserSearch, 0xE065, 0xAA, 217, None, None),
    key(BrowserFavorites, 0xE066, 0xAB, 156, None, None),
    key(BrowserHome, 0xE032, 0xAC, 172, None, None),
    // Keypad 5 with Num Lock off. Listeners see it as keypad 5, which comes first for lookups
    // by scan code.
    key(NumpadClear, 0x4C, 0x0C, 76, Some(0x47), None),
];

pub fn find(code: KeyCode) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.code == code)
}

/// The key that types `c` on a US layout, and whether Shift is needed.
pub fn us_key_for_char(c: char) -> Option<(KeyCode, bool)> {
    KEYS.iter().find_map(|key| {
        let (plain, shifted) = key.us?;
        if c == plain {
            Some((key.code, false))
        } else if c == shifted && c != plain {
            Some((key.code, true))
        } else {
            None
        }
    })
}
