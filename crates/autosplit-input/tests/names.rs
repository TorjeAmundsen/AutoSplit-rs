//! AutoSplit hotkey strings: what they listen for and what they send.

use autosplit_input::{HotkeyError, UsLayout, parse_hotkey, same_hotkey};
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

#[test_case(KeyCode::Numpad1, None, Modifiers::empty() => Some("num 1".to_owned()); "keypad digit")]
#[test_case(KeyCode::Digit1, Some('1'), Modifiers::empty() => Some("1".to_owned()); "main row digit")]
#[test_case(KeyCode::KeyP, Some('p'), Modifiers::CONTROL | Modifiers::SHIFT => Some("ctrl+shift+p".to_owned()); "combo in keyboard's order")]
#[test_case(KeyCode::KeyA, Some('A'), Modifiers::empty() => Some("a".to_owned()); "letters are lowercase")]
#[test_case(KeyCode::F12, None, Modifiers::ALT => Some("alt+f12".to_owned()); "function key")]
#[test_case(KeyCode::NumpadAdd, None, Modifiers::empty() => Some("plus".to_owned()); "keypad plus")]
#[test_case(KeyCode::Equal, Some('+'), Modifiers::empty() => Some("plus".to_owned()); "plus character")]
#[test_case(KeyCode::NumpadEnter, None, Modifiers::empty() => Some("enter".to_owned()); "keypad enter")]
#[test_case(KeyCode::PageUp, None, Modifiers::empty() => Some("page up".to_owned()); "named key")]
#[test_case(KeyCode::ShiftLeft, None, Modifiers::SHIFT => None; "modifier alone")]
#[test_case(KeyCode::NumpadDivide, None, Modifiers::empty() => Some("/".to_owned()); "keypad divide")]
#[test_case(KeyCode::NumpadMultiply, None, Modifiers::empty() => Some("*".to_owned()); "keypad multiply")]
#[test_case(KeyCode::NumpadSubtract, None, Modifiers::empty() => Some("-".to_owned()); "keypad subtract")]
#[test_case(KeyCode::NumpadDecimal, None, Modifiers::empty() => Some("decimal".to_owned()); "keypad decimal")]
fn names_pressed_keys(key: KeyCode, character: Option<char>, modifiers: Modifiers) -> Option<String> {
    // Act, with the expected value checked by test_case
    autosplit_input::hotkey_name(key, character, modifiers)
}

#[test_case(KeyCode::Numpad1, None, Modifiers::empty(); "keypad digit")]
#[test_case(KeyCode::KeyP, Some('p'), Modifiers::CONTROL | Modifiers::SHIFT; "combo")]
#[test_case(KeyCode::PageUp, None, Modifiers::empty(); "named key")]
#[test_case(KeyCode::F12, None, Modifiers::ALT; "function key")]
#[test_case(KeyCode::KeyX, Some('x'), Modifiers::META; "windows or command key")]
#[test_case(KeyCode::NumpadDecimal, None, Modifiers::empty(); "keypad decimal")]
#[test_case(KeyCode::NumpadAdd, None, Modifiers::empty(); "keypad plus")]
fn recorded_names_listen_for_the_key(key: KeyCode, character: Option<char>, modifiers: Modifiers) {
    // Arrange
    let name = autosplit_input::hotkey_name(key, character, modifiers).unwrap();

    // Act
    let parsed = parse_hotkey(&name, &UsLayout).unwrap();

    // Assert
    assert!(parsed.listen.contains(&Hotkey { key_code: key, modifiers }), "{name}");
}

#[test]
fn keys_named_by_their_us_character_listen_for_themselves() {
    // Every key the US layout types something on, like the key labelled Z on a German keyboard,
    // named the way macOS and Linux record it
    let letters = ('A'..='Z').map(|letter| format!("Key{letter}"));
    let digits = (0..=9).map(|digit| format!("Digit{digit}"));
    let symbols = ["Backquote", "Minus", "Equal", "BracketLeft", "BracketRight", "Backslash", "Semicolon", "Quote"]
        .into_iter()
        .chain(["Comma", "Period", "Slash", "Space"])
        .map(String::from);
    let keys: Vec<KeyCode> = letters.chain(digits).chain(symbols).map(|name| name.parse().unwrap()).collect();
    let typing: Vec<(KeyCode, char)> =
        keys.into_iter().filter_map(|key| Some((key, autosplit_input::us_character(key)?))).collect();
    assert_eq!(typing.len(), 48, "every one types a character");
    for (key, character) in typing {
        // Arrange
        let name = autosplit_input::hotkey_name(key, Some(character), Modifiers::CONTROL).unwrap();

        // Act
        let parsed = parse_hotkey(&name, &UsLayout).unwrap();

        // Assert
        let hotkey = Hotkey { key_code: key, modifiers: Modifiers::CONTROL };
        assert!(parsed.listen.contains(&hotkey), "{key:?} named {name}");
    }
}

#[test]
fn keys_are_recorded_by_their_us_character_off_windows() {
    // Act: the German layout's Z, on the key US layouts have Y on
    let character = autosplit_input::recorded_character(KeyCode::KeyY, Some('z'));

    // Assert
    assert_eq!(character, Some(if cfg!(windows) { 'z' } else { 'y' }));
}

#[test_case("f1", "F1" => true; "case of a key name")]
#[test_case("shift+ctrl+a", "ctrl+shift+a" => true; "modifier order")]
#[test_case(" num 1", "num 1" => true; "spaces")]
#[test_case("num 1", "1" => false; "keypad and main row digits")]
#[test_case("ctrl+a", "a" => false; "combo and single key")]
#[test_case("hyperdrive", "hyperdrive" => true; "same unknown name")]
fn same_hotkeys(first: &str, second: &str) -> bool {
    // Act, with the expected value checked by test_case
    same_hotkey(first, second)
}
