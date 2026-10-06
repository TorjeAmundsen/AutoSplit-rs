use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGPreflightPostEventAccess, CGRequestPostEventAccess,
};

use crate::InputError;
use crate::keys::Key;

fn modifier_flag(key: &Key) -> CGEventFlags {
    use livesplit_hotkey::KeyCode::*;
    match key.code {
        ControlLeft | ControlRight => CGEventFlags::MaskControl,
        ShiftLeft | ShiftRight => CGEventFlags::MaskShift,
        AltLeft | AltRight => CGEventFlags::MaskAlternate,
        MetaLeft | MetaRight => CGEventFlags::MaskCommand,
        _ => CGEventFlags::empty(),
    }
}

const ACCESSIBILITY_HELP: &str = "macOS isn't letting AutoSplit press keys, so the timer won't get them. Allow \
     your terminal app (or AutoSplit) in System Settings > Privacy & Security > Accessibility, then quit and reopen it.";

/// Without Accessibility access, macOS drops posted key events without an error.
pub fn check_access() -> Result<(), InputError> {
    if CGPreflightPostEventAccess() {
        return Ok(());
    }
    // Shows macOS's prompt the first time
    CGRequestPostEventAccess();
    Err(InputError::Send(ACCESSIBILITY_HELP.into()))
}

/// Posts the key events with the held modifiers set on each, since macOS takes modifier state
/// from the event flags.
pub fn send(keys: &[&Key]) -> Result<(), InputError> {
    let codes: Option<Vec<u16>> = keys.iter().map(|key| key.mac).collect();
    let codes = codes.ok_or_else(|| InputError::Send("a key in this hotkey doesn't exist on macOS".into()))?;
    let post = |code: u16, down: bool, flags: CGEventFlags| -> Result<(), InputError> {
        let event =
            CGEvent::new_keyboard_event(None, code, down).ok_or_else(|| InputError::Send(ACCESSIBILITY_HELP.into()))?;
        CGEvent::set_flags(Some(&event), flags);
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        Ok(())
    };
    let mut flags = CGEventFlags::empty();
    for (key, &code) in keys.iter().zip(&codes) {
        flags |= modifier_flag(key);
        post(code, true, flags)?;
    }
    for (key, &code) in keys.iter().zip(&codes).rev() {
        post(code, false, flags)?;
        flags &= !modifier_flag(key);
    }
    Ok(())
}
