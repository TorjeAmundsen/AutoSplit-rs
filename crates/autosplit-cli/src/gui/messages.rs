//! Message boxes, one at a time, like AutoSplit's modal ones.

use std::collections::VecDeque;

/// Which buttons a message box has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Buttons {
    /// An error, titled "Error" like AutoSplit's.
    Ok,
    /// Something to know that doesn't stop anything, like keys in the wrong place in a profile.
    Warning,
    /// A question, titled "AutoSplit".
    YesNoCancel,
    /// A question with four buttons: Yes, No, an extra choice and Cancel. Always made with
    /// [`Message::choice4`], which gives the labels, so the ones in [`Buttons::labels`] are only
    /// placeholders.
    Choice4,
}

impl Buttons {
    pub fn title(self) -> &'static str {
        match self {
            Self::Ok => "Error",
            Self::Warning => "Warning",
            Self::YesNoCancel | Self::Choice4 => "AutoSplit",
        }
    }

    /// The buttons' texts, an empty one not shown.
    pub fn labels(self) -> [&'static str; 4] {
        match self {
            Self::Ok | Self::Warning => ["OK", "", "", ""],
            Self::YesNoCancel => ["Yes", "No", "Cancel", ""],
            Self::Choice4 => ["Yes", "No", "Other", "Cancel"],
        }
    }

    /// The answer of a button, by its position in [`Buttons::labels`].
    pub fn answer(self, button: usize) -> Answer {
        match (self, button) {
            (Self::Ok | Self::Warning, _) | (Self::YesNoCancel | Self::Choice4, 0) => Answer::Yes,
            (Self::YesNoCancel | Self::Choice4, 1) => Answer::No,
            (Self::Choice4, 2) => Answer::Other,
            _ => Answer::Cancel,
        }
    }

    /// The answer when the box is closed instead: OK for an error, Cancel for a question.
    pub fn dismissed(self) -> Answer {
        match self {
            Self::Ok | Self::Warning => Answer::Yes,
            Self::YesNoCancel | Self::Choice4 => Answer::Cancel,
        }
    }
}

/// How a message box was answered: Yes, which is also OK, No, Cancel, or the extra choice of a
/// question with four buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Cancel,
    /// The extra choice of a question with four buttons, between No and Cancel, like Delete after
    /// Back Up and Keep. Questions with three buttons never answer it.
    Other,
}

pub type Answered = Box<dyn FnOnce(Answer)>;

/// A message box to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    pub buttons: Buttons,
    /// The buttons' texts, by their place in [`Buttons::labels`]. An empty one isn't shown.
    pub labels: [&'static str; 4],
    /// The button Enter picks.
    pub default: usize,
}

pub struct Message {
    shown: Shown,
    answered: Option<Answered>,
}

impl Message {
    fn new(text: impl Into<String>, buttons: Buttons, answered: Option<Answered>) -> Self {
        let shown = Shown { text: text.into(), buttons, labels: buttons.labels(), default: 0 };
        Self { shown, answered }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(text, Buttons::Ok, None)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(text, Buttons::Warning, None)
    }

    pub fn question(text: impl Into<String>, answered: impl FnOnce(Answer) + 'static) -> Self {
        Self::new(text, Buttons::YesNoCancel, Some(Box::new(answered)))
    }

    /// A question whose buttons say what they do instead of Yes, No and Cancel. They answer by
    /// their place: Yes, No, Cancel. `default` is the one Enter picks.
    pub fn choice(
        text: impl Into<String>,
        labels: [&'static str; 3],
        default: usize,
        answered: impl FnOnce(Answer) + 'static,
    ) -> Self {
        let [yes, no, cancel] = labels;
        let mut message = Self::question(text, answered);
        (message.shown.labels, message.shown.default) = ([yes, no, cancel, ""], default);
        message
    }

    /// A question with four buttons that say what they do. They answer by their place: Yes, No,
    /// [`Answer::Other`], Cancel. `default` is the one Enter picks.
    pub fn choice4(
        text: impl Into<String>,
        labels: [&'static str; 4],
        default: usize,
        answered: impl FnOnce(Answer) + 'static,
    ) -> Self {
        let mut message = Self::new(text, Buttons::Choice4, Some(Box::new(answered)));
        (message.shown.labels, message.shown.default) = (labels, default);
        message
    }
}

/// The message box shown, and the ones waiting for it to be answered.
#[derive(Default)]
pub struct MessageQueue {
    waiting: VecDeque<Message>,
    /// The buttons of the box shown and what to do with the answer.
    showing: Option<(Buttons, Option<Answered>)>,
}

impl MessageQueue {
    pub fn is_showing(&self) -> bool {
        self.showing.is_some()
    }

    /// Queues a message. Returns its text and buttons when it's to be shown now.
    pub fn push(&mut self, message: Message) -> Option<Shown> {
        self.waiting.push_back(message);
        self.next()
    }

    /// The shown box was answered with a button, or closed when `button` is `None`. Returns what
    /// to do with the answer, to call once nothing is borrowed, and the next box to show.
    pub fn answer(&mut self, button: Option<usize>) -> (Option<(Answered, Answer)>, Option<Shown>) {
        let answered = self.showing.take().and_then(|(buttons, answered)| {
            let answer = button.map_or_else(|| buttons.dismissed(), |button| buttons.answer(button));
            answered.map(|answered| (answered, answer))
        });
        (answered, self.next())
    }

    fn next(&mut self) -> Option<Shown> {
        if self.showing.is_some() {
            return None;
        }
        let message = self.waiting.pop_front()?;
        self.showing = Some((message.shown.buttons, message.answered));
        Some(message.shown)
    }
}
