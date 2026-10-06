//! AutoSplit's hotkey strings (Python `keyboard` library names, like `num 1` or
//! `ctrl+shift+p`): which keys to listen for and which keys to send.
//!
//! Ambiguous names keep AutoSplit's behavior, so existing AutoSplit and LiveSplit setups keep
//! working: `-` listens to both minus keys and sends keypad minus, `plus` sends keypad plus on
//! its own and the main row plus in combos, and so on. Combos match the exact key, though
//! (AutoSplit ignored the keypad there), and send every modifier.

use livesplit_hotkey::{Hotkey, KeyCode, Modifiers};

use crate::keys;

/// Where layout dependent names come from.
pub trait Layout {
    /// The key typing `c`, and whether Shift is needed.
    fn key_for_char(&self, c: char) -> Option<(KeyCode, bool)>;

    /// The key with this localized name (lowercase), like German "entf" for Delete.
    fn key_for_name(&self, _name: &str) -> Option<KeyCode> {
        None
    }
}

/// The US layout, which is what AutoSplit's names follow on Linux (they come from the console
/// keymap) and a reasonable default elsewhere.
pub struct UsLayout;

impl Layout for UsLayout {
    fn key_for_char(&self, c: char) -> Option<(KeyCode, bool)> {
        keys::us_key_for_char(c)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedHotkey {
    /// Key combinations that trigger the hotkey.
    pub listen: Vec<Hotkey>,
    /// Keys to press in order and release in reverse to send the hotkey.
    pub send: Vec<KeyCode>,
    /// Modifiers and a key. A single key fires whatever modifiers are held.
    pub combo: bool,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum HotkeyError {
    #[error("the hotkey is empty")]
    Empty,
    #[error("unknown key '{0}'")]
    UnknownKey(String),
    #[error("'{0}' is used like a modifier but isn't one")]
    NotAModifier(String),
    #[error("a hotkey needs a key besides modifiers")]
    OnlyModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    Control,
    Shift,
    Alt,
    Meta,
    AltGr,
}

impl Modifier {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "ctrl" | "left ctrl" | "right ctrl" | "control" => Self::Control,
            "shift" | "left shift" | "right shift" => Self::Shift,
            "alt" | "left alt" | "right alt" | "option" => Self::Alt,
            "windows" | "left windows" | "right windows" | "command" | "cmd" => Self::Meta,
            "alt gr" => Self::AltGr,
            _ => return None,
        })
    }

    fn from_key(key: KeyCode) -> Option<Self> {
        Some(match key {
            KeyCode::ControlLeft | KeyCode::ControlRight => Self::Control,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => Self::Shift,
            KeyCode::AltLeft => Self::Alt,
            KeyCode::AltRight => Self::AltGr,
            KeyCode::MetaLeft | KeyCode::MetaRight => Self::Meta,
            _ => return None,
        })
    }

    fn flags(self) -> Modifiers {
        match self {
            Self::Control => Modifiers::CONTROL,
            Self::Shift => Modifiers::SHIFT,
            Self::Alt => Modifiers::ALT,
            Self::Meta => Modifiers::META,
            // Windows reports AltGr as Ctrl + Alt
            Self::AltGr if cfg!(windows) => Modifiers::CONTROL | Modifiers::ALT,
            Self::AltGr => Modifiers::ALT,
        }
    }

    fn key(self) -> KeyCode {
        match self {
            Self::Control => KeyCode::ControlLeft,
            Self::Shift => KeyCode::ShiftLeft,
            Self::Alt => KeyCode::AltLeft,
            Self::Meta => KeyCode::MetaLeft,
            Self::AltGr => KeyCode::AltRight,
        }
    }
}

/// A key name resolved to the keys it can mean.
struct Resolved {
    /// Keys that trigger it, each with whether Shift has to be held.
    listen: Vec<(KeyCode, bool)>,
    send: (KeyCode, bool),
}

impl Resolved {
    fn single(key: KeyCode) -> Self {
        Self { listen: vec![(key, false)], send: (key, false) }
    }
}

fn named_key(name: &str) -> Option<KeyCode> {
    use KeyCode::*;
    Some(match name {
        "space" => Space,
        "tab" => Tab,
        "backspace" => Backspace,
        "esc" | "escape" => Escape,
        "caps lock" => CapsLock,
        "num lock" => NumLock,
        "scroll lock" => ScrollLock,
        "insert" => Insert,
        "delete" | "del" => Delete,
        "home" => Home,
        "end" => End,
        "page up" => PageUp,
        "page down" => PageDown,
        "up" => ArrowUp,
        "down" => ArrowDown,
        "left" => ArrowLeft,
        "right" => ArrowRight,
        "print screen" => PrintScreen,
        "pause" | "break" => Pause,
        "menu" => ContextMenu,
        "decimal" => NumpadDecimal,
        "separator" => NumpadComma,
        "volume mute" => AudioVolumeMute,
        "volume down" => AudioVolumeDown,
        "volume up" => AudioVolumeUp,
        "next track" => MediaTrackNext,
        "previous track" => MediaTrackPrevious,
        "stop media" => MediaStop,
        "play/pause media" => MediaPlayPause,
        "browser back" => BrowserBack,
        "browser forward" => BrowserForward,
        "browser refresh" => BrowserRefresh,
        "browser stop" => BrowserStop,
        "browser search key" => BrowserSearch,
        "browser favorites" => BrowserFavorites,
        "browser start and home" => BrowserHome,
        // Linux console keymap names for keypad operators
        "×" => NumpadMultiply,
        "÷" => NumpadDivide,
        "−" => Minus,
        _ => {
            if let Some(digit) = name.strip_prefix("num ").and_then(|digit| digit.parse::<u8>().ok()) {
                return [Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9]
                    .get(digit as usize)
                    .copied();
            }
            if let Some(number) = name.strip_prefix('f').and_then(|number| number.parse::<u8>().ok()) {
                return [
                    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21,
                    F22, F23, F24,
                ]
                .get(usize::from(number).checked_sub(1)?)
                .copied();
            }
            return None;
        }
    })
}

/// The keys a name means, with AutoSplit's choices for ambiguous names.
fn resolve(name: &str, in_combo: bool, layout: &dyn Layout) -> Option<Resolved> {
    use KeyCode::*;
    let typed = |c: char| layout.key_for_char(c);
    Some(match name {
        "-" => Resolved { listen: vec![(Minus, false), (NumpadSubtract, false)], send: (NumpadSubtract, false) },
        "plus" => {
            let main_row = typed('+').unwrap_or((Equal, true));
            let send = if in_combo { main_row } else { (NumpadAdd, false) };
            Resolved { listen: vec![(NumpadAdd, false), main_row], send }
        }
        "*" => {
            let main_row = typed('*').unwrap_or((Digit8, true));
            Resolved { listen: vec![(NumpadMultiply, false), main_row], send: (NumpadMultiply, false) }
        }
        "/" => {
            let main_row = typed('/').unwrap_or((Slash, false));
            Resolved { listen: vec![main_row, (NumpadDivide, false)], send: main_row }
        }
        "enter" => Resolved { listen: vec![(Enter, false), (NumpadEnter, false)], send: (Enter, false) },
        // Keypad 5 with Num Lock off, which AutoSplit sends as VK_CLEAR
        "clear" => Resolved { listen: vec![(Numpad5, false)], send: (NumpadClear, false) },
        // On Linux the keypad's decimal key is also called "."
        "." if cfg!(target_os = "linux") => {
            Resolved { listen: vec![(Period, false), (NumpadDecimal, false)], send: (Period, false) }
        }
        _ => {
            if let Some(key) = named_key(name) {
                return Some(Resolved::single(key));
            }
            let mut chars = name.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                // Letters are the same key whatever their case
                let (key, shift) = if c.is_alphabetic() {
                    typed(c.to_lowercase().next()?).map(|(key, _)| (key, c.is_uppercase()))?
                } else {
                    typed(c)?
                };
                return Some(Resolved { listen: vec![(key, shift)], send: (key, shift) });
            }
            Resolved::single(layout.key_for_name(name)?)
        }
    })
}

fn modifier(name: &str, layout: &dyn Layout) -> Option<Modifier> {
    Modifier::from_name(name).or_else(|| layout.key_for_name(name).and_then(Modifier::from_key))
}

/// All 16 combinations of the 4 modifiers.
fn every_modifier_combination() -> impl Iterator<Item = Modifiers> {
    (0..16_u8).map(|bits| {
        let mut modifiers = Modifiers::empty();
        for (bit, flag) in
            [Modifiers::SHIFT, Modifiers::CONTROL, Modifiers::ALT, Modifiers::META].into_iter().enumerate()
        {
            if bits & (1 << bit) != 0 {
                modifiers |= flag;
            }
        }
        modifiers
    })
}

/// Reads a hotkey string from an AutoSplit profile.
///
/// A single key fires whatever modifiers are held, like AutoSplit (and LiveSplit). A combo fires
/// for exactly its keys.
pub fn parse_hotkey(text: &str, layout: &dyn Layout) -> Result<ParsedHotkey, HotkeyError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(HotkeyError::Empty);
    }
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (main, modifier_names) = parts.split_last().ok_or(HotkeyError::Empty)?;
    let main = lowercase_if_word(main);

    if modifier_names.is_empty() {
        if modifier(&main, layout).is_some() {
            return Err(HotkeyError::OnlyModifiers);
        }
        let resolved = resolve(&main, false, layout).ok_or_else(|| HotkeyError::UnknownKey(main.clone()))?;
        let listen = resolved
            .listen
            .iter()
            .flat_map(|&(key_code, _)| {
                every_modifier_combination().map(move |modifiers| Hotkey { key_code, modifiers })
            })
            .collect();
        // The `keyboard` library sends a single key without the Shift it may need to type it
        let (key, _) = resolved.send;
        return Ok(ParsedHotkey { listen, send: vec![key], combo: false });
    }

    let mut modifiers = Vec::new();
    for name in modifier_names {
        let name = lowercase_if_word(name);
        let found = modifier(&name, layout).ok_or_else(|| HotkeyError::NotAModifier(name.clone()))?;
        if !modifiers.contains(&found) {
            modifiers.push(found);
        }
    }
    if modifier(&main, layout).is_some() {
        return Err(HotkeyError::OnlyModifiers);
    }
    let resolved = resolve(&main, true, layout).ok_or_else(|| HotkeyError::UnknownKey(main.clone()))?;
    let flags = modifiers.iter().fold(Modifiers::empty(), |flags, modifier| flags | modifier.flags());
    let listen = resolved
        .listen
        .iter()
        .map(|&(key_code, shift)| Hotkey { key_code, modifiers: if shift { flags | Modifiers::SHIFT } else { flags } })
        .collect();
    let (key, shift) = resolved.send;
    let mut send: Vec<KeyCode> = modifiers.iter().map(|modifier| modifier.key()).collect();
    if shift && !modifiers.contains(&Modifier::Shift) {
        send.push(KeyCode::ShiftLeft);
    }
    send.push(key);
    Ok(ParsedHotkey { listen, send, combo: true })
}

/// The `keyboard` library lowercases names longer than one character; single characters keep
/// their case.
fn lowercase_if_word(name: &str) -> String {
    if name.chars().count() > 1 { name.to_lowercase() } else { name.to_owned() }
}
