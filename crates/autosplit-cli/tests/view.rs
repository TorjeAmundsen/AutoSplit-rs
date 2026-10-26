//! What the window's view hands the window: only changes, merged while the window is busy.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autosplit::gui::view::{GuiView, NO_DEVICE, NO_DEVICE_IN_TOOL, Update};
use autosplit::session::{Snapshot, View};
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{Runner, RunnerSettings};
use autosplit_core::split_folder::SplitSet;

/// What the view delivered, taken from its slot like the window does.
#[derive(Clone, Default)]
struct Delivered(Arc<Mutex<Vec<Update>>>);

impl Delivered {
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    fn last(&self) -> Update {
        self.0.lock().unwrap().pop().unwrap()
    }
}

fn view() -> (GuiView, Delivered) {
    let delivered = Delivered::default();
    let into = delivered.clone();
    let deliver = Box::new(move |slot: Arc<Mutex<Option<Update>>>| {
        // The window takes it right away
        let update = slot.lock().unwrap().take().unwrap();
        into.0.lock().unwrap().push(update);
    });
    let visible = Arc::new(AtomicBool::new(true));
    let hidden = || Arc::new(AtomicBool::new(false));
    (GuiView::new(visible, hidden(), hidden(), deliver, |_| {}), delivered)
}

/// Shows an idle session with `profile` and `device_problem`.
fn show(view: &mut GuiView, profile: &UserProfile, device_problem: Option<&str>, tester_request: u64) {
    let runner = Runner::new(RunnerSettings::from_profile(profile), false);
    let images = SplitSet::default();
    view.update(Snapshot {
        status: runner.status(),
        profile,
        now: Duration::ZERO,
        idle: "ready".into(),
        device_name: None,
        receiving: false,
        device_problem,
        frame: None,
        images: &images,
        images_loaded: 0,
        tester: None,
        tester_loaded: 0,
        tester_request,
    });
}

fn with_device() -> UserProfile {
    UserProfile { capture_device_name: "OBS Virtual Camera".into(), ..UserProfile::default() }
}

#[test]
fn nothing_changed_delivers_nothing() {
    // Arrange
    let (mut view, delivered) = view();
    show(&mut view, &with_device(), None, 0);

    // Act
    show(&mut view, &with_device(), None, 0);

    // Assert
    assert_eq!(delivered.count(), 1);
}

#[test]
fn closed_tester_is_said_once_per_request() {
    // Arrange
    let (mut view, delivered) = view();
    show(&mut view, &with_device(), None, 1);

    // Act
    show(&mut view, &with_device(), None, 1);
    show(&mut view, &with_device(), None, 2);

    // Assert
    let gone: Vec<(bool, u64)> =
        delivered.0.lock().unwrap().iter().map(|update| (update.tester.gone, update.tester.request)).collect();
    assert_eq!(gone, [(true, 1), (true, 2)]);
}

#[test]
fn preview_without_a_device_says_to_pick_one() {
    // Arrange
    let (mut view, delivered) = view();

    // Act
    show(&mut view, &UserProfile::default(), None, 0);

    // Assert
    assert_eq!(delivered.last().texts.unwrap().live_message, NO_DEVICE);
}

#[test]
fn maker_without_a_device_says_to_pick_one_in_settings() {
    // Arrange
    let delivered = Delivered::default();
    let into = delivered.clone();
    let deliver = Box::new(move |slot: Arc<Mutex<Option<Update>>>| {
        into.0.lock().unwrap().push(slot.lock().unwrap().take().unwrap())
    });
    let visible = || Arc::new(AtomicBool::new(true));
    let mut view = GuiView::new(visible(), visible(), visible(), deliver, |_| {});

    // Act
    show(&mut view, &UserProfile::default(), None, 0);

    // Assert: the main window's menu can't be used while the maker is open
    assert_eq!(delivered.last().maker.message.as_deref(), Some(NO_DEVICE_IN_TOOL));
}

#[test]
fn preview_says_why_the_device_sends_nothing() {
    // Arrange
    let (mut view, delivered) = view();
    let problem = "'OBS Virtual Camera' is connected but not started. In OBS, click Start Virtual Camera.";

    // Act
    show(&mut view, &with_device(), Some(problem), 0);

    // Assert
    assert_eq!(delivered.last().texts.unwrap().live_message, problem);
}

#[test]
fn preview_waits_for_a_device_without_a_known_problem() {
    // Arrange
    let (mut view, delivered) = view();

    // Act
    show(&mut view, &with_device(), None, 0);

    // Assert
    assert_eq!(delivered.last().texts.unwrap().live_message, "Waiting for capture device...");
}
