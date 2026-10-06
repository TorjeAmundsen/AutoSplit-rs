//! Reading a hotkey from a key press in the settings window.

use autosplit_input::{KeyCode, Modifiers};
use slint::winit_030::winit::event::KeyEvent;
use slint::winit_030::winit::keyboard::{self, Key, ModifiersState, PhysicalKey};

/// The hotkey library's key for winit's. They're both named after the web's key codes, except
/// for the Windows or Command keys.
pub fn key_code(code: keyboard::KeyCode) -> Option<KeyCode> {
    match code {
        keyboard::KeyCode::SuperLeft => Some(KeyCode::MetaLeft),
        keyboard::KeyCode::SuperRight => Some(KeyCode::MetaRight),
        code => format!("{code:?}").parse().ok(),
    }
}

/// The physical key pressed.
pub fn pressed_key(event: &KeyEvent) -> Option<KeyCode> {
    match event.physical_key {
        PhysicalKey::Code(code) => key_code(code),
        PhysicalKey::Unidentified(_) => None,
    }
}

pub fn modifiers_held(state: ModifiersState) -> Modifiers {
    let mut held = Modifiers::empty();
    for (on, flag) in [
        (state.control_key(), Modifiers::CONTROL),
        (state.alt_key(), Modifiers::ALT),
        (state.shift_key(), Modifiers::SHIFT),
        (state.super_key(), Modifiers::META),
    ] {
        if on {
            held |= flag;
        }
    }
    held
}

/// What the key types without modifiers, from the keyboard layout.
pub fn key_character(event: &KeyEvent) -> Option<char> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        use slint::winit_030::winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        if let Key::Character(text) = event.key_without_modifiers() {
            return text.chars().next();
        }
    }
    match &event.logical_key {
        Key::Character(text) => text.chars().next(),
        _ => None,
    }
}
