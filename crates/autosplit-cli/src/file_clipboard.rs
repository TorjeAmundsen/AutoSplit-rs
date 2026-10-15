//! Puts files on the X11 clipboard in every format Linux file managers paste from. arboard only
//! offers `text/uri-list`, which KDE's Dolphin and GTK 4 programs read, but Thunar and GNOME
//! Files read `x-special/gnome-copied-files`, and GNOME Files 3.30 to 42 (Ubuntu 22.04) only
//! reads text that starts with `x-special/nautilus-clipboard`. Under Wayland this works through
//! XWayland, whose clipboard the desktop shares with Wayland programs.
//!
//! X11 has no clipboard storage: the program that copied answers every paste itself, so a
//! thread keeps answering until another program copies something.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
    SelectionRequestEvent, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD,
        TARGETS,
        UTF8_STRING,
        URI_LIST: b"text/uri-list",
        GNOME_COPIED_FILES: b"x-special/gnome-copied-files",
        TEXT_PLAIN_UTF8: b"text/plain;charset=utf-8",
        TEXT_PLAIN: b"text/plain",
    }
}

/// `path` as a `file://` URI, with the bytes URIs can't hold percent-encoded.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// What the clipboard holds in each format, for `files`.
pub struct Contents {
    /// `text/uri-list`
    pub uri_list: Vec<u8>,
    /// `x-special/gnome-copied-files`
    pub gnome: Vec<u8>,
    /// Text, as GNOME Files 3.30 to 42 copies files
    pub text: Vec<u8>,
}

impl Contents {
    pub fn new(files: &[PathBuf]) -> Self {
        let uris: Vec<String> = files.iter().map(|file| file_uri(file)).collect();
        Self {
            // Lines end in CRLF in a URI list
            uri_list: uris.iter().map(|uri| format!("{uri}\r\n")).collect::<String>().into_bytes(),
            gnome: format!("copy\n{}", uris.join("\n")).into_bytes(),
            text: format!("x-special/nautilus-clipboard\ncopy\n{}\n", uris.join("\n")).into_bytes(),
        }
    }
}

/// Puts `files` on the clipboard, for pasting into a file manager. Fails without an X11
/// display, or XWayland under Wayland.
pub fn copy_files(files: &[PathBuf]) -> Result<(), String> {
    let contents = Contents::new(files);
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            let owner = match take_clipboard() {
                Ok(owner) => owner,
                Err(error) => return drop(sender.send(Err(error))),
            };
            let _ = sender.send(Ok(()));
            let _ = answer_pastes(&owner, &contents);
        })
        .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|_| "the clipboard thread stopped".to_owned())?
}

struct Owner {
    connection: x11rb::rust_connection::RustConnection,
    atoms: Atoms,
}

/// Connects, and makes a window of its own the clipboard's owner.
fn take_clipboard() -> Result<Owner, String> {
    let (connection, screen) = x11rb::connect(None).map_err(|error| error.to_string())?;
    let error = |error: &dyn std::fmt::Display| error.to_string();
    let atoms = Atoms::new(&connection).map_err(|e| error(&e))?.reply().map_err(|e| error(&e))?;
    let window = connection.generate_id().map_err(|e| error(&e))?;
    let root = connection.setup().roots[screen].root;
    connection
        .create_window(0, window, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())
        .map_err(|e| error(&e))?;
    connection.set_selection_owner(window, atoms.CLIPBOARD, CURRENT_TIME).map_err(|e| error(&e))?;
    let owner =
        connection.get_selection_owner(atoms.CLIPBOARD).map_err(|e| error(&e))?.reply().map_err(|e| error(&e))?;
    if owner.owner != window {
        return Err("another program kept the clipboard".into());
    }
    Ok(Owner { connection, atoms })
}

/// Answers pastes until another program copies something.
fn answer_pastes(owner: &Owner, contents: &Contents) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        match owner.connection.wait_for_event()? {
            Event::SelectionRequest(request) => answer(owner, contents, &request)?,
            Event::SelectionClear(clear) if clear.selection == owner.atoms.CLIPBOARD => return Ok(()),
            _ => {}
        }
    }
}

/// Writes what `request` asks for into the property it names on the asking window, then tells
/// it whether it's there.
fn answer(
    owner: &Owner,
    contents: &Contents,
    request: &SelectionRequestEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, atoms) = (&owner.connection, &owner.atoms);
    // Old clients name no property, and get the target's
    let property = if request.property == NONE { request.target } else { request.property };
    let data = match request.target {
        target if target == atoms.URI_LIST => Some(&contents.uri_list),
        target if target == atoms.GNOME_COPIED_FILES => Some(&contents.gnome),
        target if [atoms.UTF8_STRING, atoms.TEXT_PLAIN_UTF8, atoms.TEXT_PLAIN].contains(&target) => {
            Some(&contents.text)
        }
        _ => None,
    };
    let answered = if request.target == atoms.TARGETS {
        let targets = [
            atoms.TARGETS,
            atoms.URI_LIST,
            atoms.GNOME_COPIED_FILES,
            atoms.UTF8_STRING,
            atoms.TEXT_PLAIN_UTF8,
            atoms.TEXT_PLAIN,
        ];
        connection.change_property32(PropMode::REPLACE, request.requestor, property, AtomEnum::ATOM, &targets)?;
        property
    } else if let Some(data) = data {
        connection.change_property8(PropMode::REPLACE, request.requestor, property, request.target, data)?;
        property
    } else {
        NONE
    };
    let notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: request.time,
        requestor: request.requestor,
        selection: request.selection,
        target: request.target,
        property: answered,
    };
    connection.send_event(false, request.requestor, EventMask::NO_EVENT, notify)?;
    connection.flush()?;
    Ok(())
}
