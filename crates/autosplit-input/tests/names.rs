//! AutoSplit hotkey strings: what they listen for and what they send.

use autosplit_input::{HotkeyError, UsLayout, parse_hotkey};
use livesplit_hotkey::{Hotkey, KeyCode, Modifiers};
use test_case::test_case;

fn sent(text: &str) -> Result<Vec<KeyCode>, HotkeyError> {
    parse_hotkey(text, &UsLayout).map(|hotkey| hotkey.send)
}

fn listens(text: &str, key_code: KeyCode, modifiers: Modifiers) -> bool {
    parse_hotkey(text, &UsLayout).unwrap().listen.contains(&Hotkey { key_code, modifiers })
}

#[test_case("num 1" => Ok(vec![KeyCode::Numpad1]); "keypad digit")]
#[test_case("1" => Ok(vec![KeyCode::Digit1]); "main row digit")]
#[test_case("f12" => Ok(vec![KeyCode::F12]); "function key")]
#[test_case("page up" => Ok(vec![KeyCode::PageUp]); "name with a space")]
#[test_case("a" => Ok(vec![KeyCode::KeyA]); "letter")]
#[test_case("A" => Ok(vec![KeyCode::KeyA]); "upper case letter sends the bare key like AutoSplit")]
#[test_case("!" => Ok(vec![KeyCode::Digit1]); "shifted character alone sends the bare key like AutoSplit")]
#[test_case("clear" => Ok(vec![KeyCode::NumpadClear]); "clear sends the clear key")]
#[test_case("browser back" => Ok(vec![KeyCode::BrowserBack]); "browser key")]
#[test_case("num lock" => Ok(vec![KeyCode::NumLock]); "num lock")]
#[test_case("-" => Ok(vec![KeyCode::NumpadSubtract]); "minus sends keypad minus like AutoSplit")]
#[test_case("plus" => Ok(vec![KeyCode::NumpadAdd]); "plus alone sends keypad plus like AutoSplit")]
#[test_case("ctrl+plus" => Ok(vec![KeyCode::ControlLeft, KeyCode::ShiftLeft, KeyCode::Equal]); "plus in a combo sends the main row plus")]
#[test_case("*" => Ok(vec![KeyCode::NumpadMultiply]); "asterisk sends keypad multiply")]
#[test_case("/" => Ok(vec![KeyCode::Slash]); "slash sends the main row slash")]
#[test_case("enter" => Ok(vec![KeyCode::Enter]); "enter sends the main enter")]
#[test_case("decimal" => Ok(vec![KeyCode::NumpadDecimal]); "keypad decimal")]
#[test_case("ctrl+shift+p" => Ok(vec![KeyCode::ControlLeft, KeyCode::ShiftLeft, KeyCode::KeyP]); "combo")]
#[test_case("windows+x" => Ok(vec![KeyCode::MetaLeft, KeyCode::KeyX]); "windows modifier is sent")]
#[test_case("ctrl" => Err(HotkeyError::OnlyModifiers); "modifier alone")]
#[test_case("ctrl+shift" => Err(HotkeyError::OnlyModifiers); "modifiers only")]
#[test_case("hyperdrive" => Err(HotkeyError::UnknownKey("hyperdrive".into())); "unknown name")]
#[test_case("x+p" => Err(HotkeyError::NotAModifier("x".into())); "non modifier before the key")]
#[test_case("" => Err(HotkeyError::Empty); "empty")]
fn send_keys(text: &str) -> Result<Vec<KeyCode>, HotkeyError> {
    // Act, with the expected value checked by test_case
    sent(text)
}

#[test_case("num 1", KeyCode::Numpad1, Modifiers::empty() => true; "keypad digit")]
#[test_case("num 1", KeyCode::Digit1, Modifiers::empty() => false; "keypad digit isn't the main row digit")]
#[test_case("num 1", KeyCode::Numpad1, Modifiers::SHIFT => true; "single keys fire whatever modifiers are held")]
#[test_case("-", KeyCode::Minus, Modifiers::empty() => true; "minus listens to the main row minus")]
#[test_case("-", KeyCode::NumpadSubtract, Modifiers::empty() => true; "minus listens to keypad minus")]
#[test_case("enter", KeyCode::NumpadEnter, Modifiers::empty() => true; "enter listens to keypad enter")]
#[test_case("ctrl+shift+p", KeyCode::KeyP, Modifiers::CONTROL | Modifiers::SHIFT => true; "combo")]
#[test_case("ctrl+shift+p", KeyCode::KeyP, Modifiers::CONTROL => false; "combo needs every modifier")]
#[test_case("ctrl+num 7", KeyCode::Numpad7, Modifiers::CONTROL => true; "keypad combo")]
#[test_case("ctrl+num 7", KeyCode::Digit7, Modifiers::CONTROL => false; "keypad combo isn't the main row digit")]
#[test_case("ctrl+num 7", KeyCode::Home, Modifiers::CONTROL => false; "keypad combo isn't home")]
#[test_case("shift+!", KeyCode::Digit1, Modifiers::SHIFT => true; "shifted character in a combo")]
#[test_case("clear", KeyCode::Numpad5, Modifiers::empty() => true; "clear listens to keypad 5")]
fn listen_keys(text: &str, key_code: KeyCode, modifiers: Modifiers) -> bool {
    // Act, with the expected value checked by test_case
    listens(text, key_code, modifiers)
}
