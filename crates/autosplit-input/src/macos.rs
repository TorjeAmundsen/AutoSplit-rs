use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, mpsc};

use livesplit_hotkey::{Hotkey, KeyCode, Modifiers};
use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopDefaultMode};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy,
    CGEventType, CGPreflightListenEventAccess, CGPreflightPostEventAccess, CGRequestListenEventAccess,
    CGRequestPostEventAccess,
};

use crate::InputError;
use crate::keys::{KEYS, Key};

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

const INPUT_MONITORING_HELP: &str = "macOS isn't letting AutoSplit read the keyboard. Allow your terminal app \
     (or AutoSplit) in System Settings > Privacy & Security > Input Monitoring, then quit and reopen it.";

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

/// A Core Foundation object used from another thread, which these allow: the run loop is only
/// stopped from there, and the tap only enabled.
struct Shared<T>(T);
// SAFETY: see above, Core Foundation's functions are thread-safe
unsafe impl<T> Send for Shared<T> {}
// SAFETY: as for Send
unsafe impl<T> Sync for Shared<T> {}

type Action = Box<dyn FnMut() + Send + 'static>;

struct State {
    hotkeys: Mutex<HashMap<Hotkey, Action>>,
    /// The tap, to enable again when macOS disables it.
    tap: OnceLock<Shared<CFRetained<CFMachPort>>>,
}

/// Listens for hotkeys with a listen-only event tap, which only needs Input Monitoring.
/// livesplit-hotkey's hook makes the same tap, but first asks for Accessibility, which only
/// pressing keys needs, so with a timer that gets no key presses, like LiveSplit One, a
/// screenshot hotkey would ask for more than it uses.
pub struct Hook {
    run_loop: Shared<CFRetained<CFRunLoop>>,
    state: Arc<State>,
    stopped: Arc<AtomicBool>,
}

impl Hook {
    pub fn new() -> Result<Self, InputError> {
        if !CGPreflightListenEventAccess() {
            // Shows macOS's prompt the first time. Like any permission, it applies once
            // AutoSplit (or the terminal app) is started again.
            CGRequestListenEventAccess();
            return Err(InputError::Listen(INPUT_MONITORING_HELP.into()));
        }
        let state = Arc::new(State { hotkeys: Mutex::new(HashMap::new()), tap: OnceLock::new() });
        let stopped = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (thread_state, thread_stopped) = (state.clone(), stopped.clone());
        let spawned = std::thread::Builder::new().name("hotkeys".into()).spawn(move || {
            let user_info = Arc::as_ptr(&thread_state).cast_mut().cast::<c_void>();
            let mask = 1 << CGEventType::KeyDown.0;
            // SAFETY: the callback has the tap callback's signature, and `user_info` points at the
            // State that `thread_state` keeps alive for as long as the tap runs on this thread
            let tap = unsafe {
                CGEvent::tap_create(
                    CGEventTapLocation::SessionEventTap,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    mask,
                    Some(callback),
                    user_info,
                )
            };
            let Some(tap) = tap else { return };
            let source = CFMachPort::new_run_loop_source(None, Some(&tap), 0);
            let (Some(source), Some(run_loop)) = (source, CFRunLoop::current()) else { return };
            // SAFETY: a constant Core Foundation defines
            run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
            let tap = &thread_state.tap.get_or_init(|| Shared(tap)).0;
            if sender.send(Shared(run_loop)).is_ok() {
                CFRunLoop::run();
            }
            CGEvent::tap_enable(tap, false);
            tap.invalidate();
            thread_stopped.store(true, Ordering::Relaxed);
        });
        let failed = |error: String| InputError::Listen(format!("{INPUT_MONITORING_HELP} ({error})"));
        spawned.map_err(|error| failed(error.to_string()))?;
        let run_loop = receiver.recv().map_err(|_| failed("couldn't create the event tap".into()))?;
        Ok(Self { run_loop, state, stopped })
    }

    pub fn register(
        &self,
        hotkey: Hotkey,
        action: impl FnMut() + Send + 'static,
    ) -> Result<(), livesplit_hotkey::Error> {
        let mut hotkeys = self.state.hotkeys.lock().unwrap_or_else(PoisonError::into_inner);
        if hotkeys.contains_key(&hotkey) {
            return Err(livesplit_hotkey::Error::AlreadyRegistered);
        }
        hotkeys.insert(hotkey, Box::new(action));
        Ok(())
    }

    pub fn unregister(&self, hotkey: Hotkey) -> Result<(), livesplit_hotkey::Error> {
        let mut hotkeys = self.state.hotkeys.lock().unwrap_or_else(PoisonError::into_inner);
        hotkeys.remove(&hotkey).map(drop).ok_or(livesplit_hotkey::Error::NotRegistered)
    }

    /// Whether the tap's thread ended, so no hotkey fires anymore.
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        self.run_loop.0.stop();
    }
}

/// The modifiers `flags` hold, apart from the one `key` is, like livesplit-hotkey reads them.
fn modifiers(flags: CGEventFlags, key: KeyCode) -> Modifiers {
    use livesplit_hotkey::KeyCode::*;
    let mut modifiers = Modifiers::empty();
    let held = [
        (CGEventFlags::MaskShift, Modifiers::SHIFT, [ShiftLeft, ShiftRight]),
        (CGEventFlags::MaskControl, Modifiers::CONTROL, [ControlLeft, ControlRight]),
        (CGEventFlags::MaskAlternate, Modifiers::ALT, [AltLeft, AltRight]),
        (CGEventFlags::MaskCommand, Modifiers::META, [MetaLeft, MetaRight]),
    ];
    for (flag, modifier, own) in held {
        if flags.contains(flag) && !own.contains(&key) {
            modifiers |= modifier;
        }
    }
    modifiers
}

/// Runs the action of the hotkey pressed, if any. A listen-only tap passes every event on.
unsafe extern "C-unwind" fn callback(
    _: CGEventTapProxy,
    kind: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: the State the tap was made with, alive while the tap runs
    let state = unsafe { &*user_info.cast::<State>() };
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        if let Some(tap) = state.tap.get() {
            CGEvent::tap_enable(&tap.0, true);
        }
        return event.as_ptr();
    }
    // SAFETY: macOS passes a valid event, for the length of the call
    let event_ref = Some(unsafe { event.as_ref() });
    if CGEvent::integer_value_field(event_ref, CGEventField::KeyboardEventAutorepeat) != 0 {
        return event.as_ptr();
    }
    let code = CGEvent::integer_value_field(event_ref, CGEventField::KeyboardEventKeycode);
    let flags = CGEvent::flags(event_ref);
    let mut hotkeys = state.hotkeys.lock().unwrap_or_else(PoisonError::into_inner);
    // A code can be more than one key, like Clear, which is also Num Lock
    for key in KEYS.iter().filter(|key| key.mac.is_some_and(|mac| i64::from(mac) == code)) {
        let hotkey = Hotkey { key_code: key.code, modifiers: modifiers(flags, key.code) };
        if let Some(action) = hotkeys.get_mut(&hotkey) {
            action();
            break;
        }
    }
    event.as_ptr()
}
