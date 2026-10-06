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
}

impl Buttons {
    pub fn title(self) -> &'static str {
        match self {
            Self::Ok => "Error",
            Self::Warning => "Warning",
            Self::YesNoCancel => "AutoSplit",
        }
    }

    pub fn labels(self) -> [&'static str; 3] {
        match self {
            Self::Ok | Self::Warning => ["OK", "", ""],
            Self::YesNoCancel => ["Yes", "No", "Cancel"],
        }
    }

    /// The answer of a button, by its position in [`Buttons::labels`].
    pub fn answer(self, button: usize) -> Answer {
        match (self, button) {
            (Self::Ok | Self::Warning, _) | (Self::YesNoCancel, 0) => Answer::Yes,
            (Self::YesNoCancel, 1) => Answer::No,
            _ => Answer::Cancel,
        }
    }

    /// The answer when the box is closed instead: OK for an error, Cancel for a question.
    pub fn dismissed(self) -> Answer {
        match self {
            Self::Ok | Self::Warning => Answer::Yes,
            Self::YesNoCancel => Answer::Cancel,
        }
    }
}

/// Yes, or OK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Cancel,
}

pub type Answered = Box<dyn FnOnce(Answer)>;

/// A message box to show: its text and buttons.
pub type Shown = (String, Buttons);

pub struct Message {
    text: String,
    buttons: Buttons,
    answered: Option<Answered>,
}

impl Message {
    pub fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), buttons: Buttons::Ok, answered: None }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self { text: text.into(), buttons: Buttons::Warning, answered: None }
    }

    pub fn question(text: impl Into<String>, answered: impl FnOnce(Answer) + 'static) -> Self {
        Self { text: text.into(), buttons: Buttons::YesNoCancel, answered: Some(Box::new(answered)) }
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
        self.showing = Some((message.buttons, message.answered));
        Some((message.text, message.buttons))
    }
}
