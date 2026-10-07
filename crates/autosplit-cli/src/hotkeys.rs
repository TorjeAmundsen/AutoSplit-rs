//! The profile's hotkeys: listening, pressing them for the timer, and ignoring AutoSplit's own
//! presses.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use autosplit_core::commands::TimerLink;
use autosplit_core::hotkeys::{HotkeyAction, OwnPresses};
use autosplit_core::profile::UserProfile;
use autosplit_input::{HotkeyError, KeyCode, Listener, ParsedHotkey, Sender, parse_hotkey, system_layout};

use crate::console::Console;

/// How often the hook is checked, since on Linux it stops when any input device is unplugged.
/// On Windows and macOS, livesplit-hotkey can't tell that its hook stopped.
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(3);

/// A profile hotkey that was read.
struct Profiled {
    action: HotkeyAction,
    text: String,
    hotkey: ParsedHotkey,
}

pub struct Hotkeys {
    link: TimerLink,
    on_press: Arc<dyn Fn(HotkeyAction) + Send + Sync>,
    listener: Option<Listener>,
    /// Listening failed, so hotkeys can't be pressed by the user.
    listening_failed: bool,
    profiled: Vec<Profiled>,
    sender: Option<Sender>,
    own: OwnPresses,
    next_health_check: Option<Instant>,
}

/// Why a hotkey text isn't one, in words that say what would work.
fn hotkey_problem(text: &str, error: &HotkeyError) -> String {
    match error {
        HotkeyError::UnknownKey(_) => {
            format!("'{text}' won't work: {error}. Use AutoSplit's names, like num 1, f1, space or ctrl+shift+p.")
        }
        _ => format!("'{text}' won't work: {error}"),
    }
}

impl Hotkeys {
    /// Hotkeys for this timer link. `on_press` runs on the hook's thread.
    pub fn new(link: TimerLink, on_press: impl Fn(HotkeyAction) + Send + Sync + 'static) -> Self {
        Self {
            link,
            on_press: Arc::new(on_press),
            listener: None,
            listening_failed: false,
            profiled: Vec::new(),
            sender: None,
            own: OwnPresses::default(),
            next_health_check: None,
        }
    }

    /// Prepares pressing hotkeys for the timer. Without it, only the log says what would have
    /// been pressed.
    pub fn start_sending(&mut self, console: &mut Console) {
        match Sender::new() {
            Ok(sender) => self.sender = Some(sender),
            Err(error) => console.warn(format_args!(
                "AutoSplit can't press the split, reset, skip, undo and pause hotkeys, so the timer won't get them. \
                 {error}"
            )),
        }
    }

    /// Switches to another timer link: hotkeys are pressed for the timer, or only the
    /// screenshot and auto reset ones are listened for when the timer reports its splits.
    pub fn set_link(&mut self, link: TimerLink, profile: &UserProfile, console: &mut Console) {
        self.link = link;
        if link.needs_hotkeys() && self.sender.is_none() {
            self.start_sending(console);
        }
        self.set_profile(profile, console);
    }

    pub fn listening_failed(&self) -> bool {
        self.listening_failed
    }

    /// Reads the profile's hotkeys, once, and listens for the ones used with this timer link.
    pub fn set_profile(&mut self, profile: &UserProfile, console: &mut Console) {
        self.profiled.clear();
        for action in HotkeyAction::ALL {
            let text = action.hotkey(profile);
            let used =
                action.listened(self.link) || (!self.link.timer_synced() && HotkeyAction::SENT.contains(&action));
            if text.is_empty() || !used {
                continue;
            }
            match parse_hotkey(text, system_layout()) {
                Ok(hotkey) => self.profiled.push(Profiled { action, text: text.to_owned(), hotkey }),
                Err(error) => {
                    console.warn(format_args!("The {} hotkey {}", action.name(), hotkey_problem(text, &error)))
                }
            }
        }
        self.listen(console);
    }

    fn listen(&mut self, console: &mut Console) {
        let link = self.link;
        let listened: Vec<&Profiled> = self.profiled.iter().filter(|profiled| profiled.action.listened(link)).collect();
        if let Some(listener) = &mut self.listener {
            listener.clear();
        } else if !listened.is_empty() && !self.listening_failed {
            match Listener::new() {
                Ok(listener) => self.listener = Some(listener),
                Err(error) => {
                    console.warn(format_args!(
                        "Hotkeys won't work. {error} Or use LiveSplit One (see TUTORIAL.md), which needs no permissions."
                    ));
                    self.listening_failed = true;
                }
            }
        }
        let Some(listener) = &mut self.listener else { return };
        let mut listened = listened;
        // Combos first: a single key also fires with modifiers held, unless a combo has them
        listened.sort_by_key(|profiled| !profiled.hotkey.combo);
        for Profiled { action, text, hotkey } in listened {
            let (action, on_press) = (*action, self.on_press.clone());
            if listener.register(hotkey, move || on_press(action)) {
                console.warn(format_args!(
                    "The {} hotkey '{text}' is the same as another hotkey, which keeps it",
                    action.name()
                ));
            }
        }
        self.next_health_check = Some(Instant::now() + HEALTH_CHECK_INTERVAL);
    }

    /// Starts listening again when the hook stopped or, on Linux, input devices changed.
    pub fn check_health(&mut self, now: Instant, console: &mut Console) {
        if self.next_health_check.is_none_or(|next| now < next) {
            return;
        }
        let Some(listener) = &self.listener else {
            self.next_health_check = None;
            return;
        };
        self.next_health_check = Some(now + HEALTH_CHECK_INTERVAL);
        if !listener.needs_restart() {
            return;
        }
        self.listener = None;
        console.log("An input device was plugged in or removed, so hotkeys were set up again");
        self.listen(console);
    }

    pub fn next_health_check(&self) -> Option<Instant> {
        self.next_health_check
    }

    /// Presses `hotkey` for the timer. Returns what happened, for the log.
    pub fn press(&mut self, hotkey: &str, now: Duration) -> Result<String, String> {
        let keys: &[KeyCode] = self
            .profiled
            .iter()
            .find(|profiled| profiled.text == hotkey)
            .map(|profiled| profiled.hotkey.send.as_slice())
            .ok_or_else(|| format!("'{hotkey}' isn't a hotkey AutoSplit can press"))?;
        let Some(sender) = &mut self.sender else {
            return Err(format!("couldn't press {hotkey} (see the warning at the start)"));
        };
        sender.send(keys).map_err(|error| format!("couldn't press {hotkey}: {error}"))?;
        self.own.pressed(hotkey, now);
        Ok(format!("pressed {hotkey}"))
    }

    /// Whether a press of `hotkey` is AutoSplit's own, which its listener hears too.
    pub fn is_own(&mut self, hotkey: &str, now: Duration) -> bool {
        self.own.is_own(hotkey, now)
    }
}

/// Problems with a profile's hotkeys, for `autosplit check`: names that aren't keys, and
/// hotkeys that are the same as another.
pub fn hotkey_problems(profile: &UserProfile, link: TimerLink) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: HashMap<String, HotkeyAction> = HashMap::new();
    for action in HotkeyAction::ALL {
        let text = action.hotkey(profile);
        if text.is_empty() || !(action.listened(link) || (!link.timer_synced() && HotkeyAction::SENT.contains(&action)))
        {
            continue;
        }
        if let Err(error) = parse_hotkey(text, system_layout()) {
            problems.push(format!("The {} hotkey {}", action.name(), hotkey_problem(text, &error)));
        } else if let Some(other) = seen.insert(text.to_lowercase(), action) {
            problems.push(format!("The {} and {} hotkeys are both '{text}'", other.name(), action.name()));
        }
    }
    problems
}
