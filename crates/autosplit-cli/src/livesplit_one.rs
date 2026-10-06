//! The WebSocket server LiveSplit One connects to ("Connect to Server" with
//! `ws://localhost:<port>`). Only this computer can connect, and only LiveSplit One or pages
//! served from this computer: any web page could otherwise drive AutoSplit.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use autosplit_core::commands::{Command, TimerReport};
use autosplit_core::livesplit_one::{
    EchoFilter, Received, describe_error, describe_event, origin_allowed, parse_event, parse_message,
};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::console::Console;

pub enum ServerEvent {
    Connected(Connection),
    Message(u64, String),
    Disconnected(u64),
    /// A web page that isn't LiveSplit One tried to connect.
    Refused(String),
}

/// One LiveSplit One connection. Writing happens on a thread of its own, so a stuck socket
/// can't hold up comparisons. Dropping it closes the connection.
pub struct Connection {
    pub id: u64,
    outgoing: mpsc::Sender<&'static str>,
    stream: TcpStream,
}

impl Connection {
    /// Queues a message. False when the connection is gone.
    pub fn send(&self, text: &'static str) -> bool {
        self.outgoing.send(text).is_ok()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Ends the reader too, so LiveSplit One sees the disconnect
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// Listens on `localhost:port`, IPv4 and IPv6, since browsers try either for "localhost".
pub fn listen(port: u16, events: impl Fn(ServerEvent) + Send + Sync + 'static) -> std::io::Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let ipv6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).ok();
    let events = Arc::new(events);
    let next_id = Arc::new(AtomicU64::new(1));
    for listener in std::iter::once(listener).chain(ipv6) {
        let (events, next_id) = (events.clone(), next_id.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let events = events.clone();
                std::thread::spawn(move || serve(id, stream, &*events));
            }
        });
    }
    Ok(())
}

fn serve(id: u64, stream: TcpStream, events: &(impl Fn(ServerEvent) + ?Sized)) {
    let _ = stream.set_nodelay(true);
    let mut refused = None;
    #[allow(clippy::result_large_err, reason = "tungstenite's handshake callback has this signature")]
    let check_origin = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let origin = request.headers().get("origin").map(|origin| origin.to_str().unwrap_or("?").to_owned());
        if origin_allowed(origin.as_deref()) {
            return Ok(response);
        }
        refused = origin;
        let mut error = ErrorResponse::new(None);
        *error.status_mut() = StatusCode::FORBIDDEN;
        Err(error)
    };
    let mut reader = match tungstenite::accept_hdr(stream, check_origin) {
        Ok(reader) => reader,
        Err(_) => {
            if let Some(origin) = refused {
                events(ServerEvent::Refused(origin));
            }
            return;
        }
    };
    let Ok(writer_stream) = reader.get_ref().try_clone() else { return };
    let Ok(control_stream) = reader.get_ref().try_clone() else { return };
    let _ = writer_stream.set_write_timeout(Some(Duration::from_secs(5)));
    let (outgoing, queue) = mpsc::channel::<&'static str>();
    // A second handle on the socket writes, so a waiting read never holds up a command.
    // Commands are unmasked text frames, which need no state shared with the reader.
    std::thread::spawn(move || {
        let mut writer = WebSocket::from_raw_socket(writer_stream, Role::Server, None);
        for text in queue {
            if writer.send(Message::text(text)).is_err() {
                let _ = writer.get_ref().shutdown(Shutdown::Both);
                return;
            }
        }
    });
    events(ServerEvent::Connected(Connection { id, outgoing, stream: control_stream }));
    loop {
        match reader.read() {
            Ok(Message::Text(text)) => events(ServerEvent::Message(id, text.as_str().to_owned())),
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    events(ServerEvent::Disconnected(id));
}

/// AutoSplit's side of the connection: what LiveSplit One reports, and sending it commands.
pub struct Link {
    connection: Option<Connection>,
    echoes: EchoFilter,
    port: u16,
}

impl Link {
    pub fn new(port: u16) -> Self {
        Self { connection: None, echoes: EchoFilter::default(), port }
    }

    pub fn connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn address(&self) -> String {
        format!("ws://localhost:{}", self.port)
    }

    fn connect_hint(&self) -> String {
        format!("choose Connect to Server in LiveSplit One's menu and enter {}", self.address())
    }

    fn is_current(&self, id: u64) -> bool {
        self.connection.as_ref().is_some_and(|connection| connection.id == id)
    }

    /// Handles something from the server. Returns what the runner did in LiveSplit One.
    pub fn event(&mut self, event: ServerEvent, console: &mut Console) -> Option<TimerReport> {
        match event {
            ServerEvent::Connected(connection) => {
                let message = if self.connection.is_some() {
                    "LiveSplit One connected again (the older connection was closed)"
                } else {
                    "LiveSplit One connected"
                };
                console.log(message);
                self.connection = Some(connection);
                self.echoes.clear();
                None
            }
            ServerEvent::Message(id, text) => {
                if !self.is_current(id) {
                    return None;
                }
                match self.echoes.receive(parse_message(&text)?) {
                    Received::Nothing => None,
                    Received::Failed { command, code } => {
                        console.warn(format_args!(
                            "LiveSplit One didn't {}: {}",
                            command.name(),
                            describe_error(&code)
                        ));
                        None
                    }
                    Received::RunnerEvent(event) => {
                        let report = parse_event(&event);
                        if report.request.is_some() {
                            console.log(format_args!("LiveSplit One: {}", describe_event(&event)));
                        }
                        Some(report)
                    }
                }
            }
            ServerEvent::Disconnected(id) => {
                if self.is_current(id) {
                    self.connection = None;
                    console.warn(format_args!(
                        "LiveSplit One disconnected. To connect it again, {}.",
                        self.connect_hint()
                    ));
                }
                None
            }
            ServerEvent::Refused(origin) => {
                console.warn(format_args!("Refused a connection from {origin}: only LiveSplit One can connect"));
                None
            }
        }
    }

    /// Sends a command. `details` says what split, for the log.
    pub fn send(&mut self, command: Command, message: &'static str, details: &str, console: &mut Console) {
        let name = command.name();
        let Some(connection) = &self.connection else {
            console.warn(format_args!(
                "{name}{details} wasn't sent: LiveSplit One isn't connected. To connect it, {}, then reset the run in \
                 LiveSplit One.",
                self.connect_hint()
            ));
            return;
        };
        if connection.send(message) {
            self.echoes.sent(command);
            console.log(format_args!("{name}{details}: sent to LiveSplit One"));
        } else {
            console.error(format_args!("{name}{details} wasn't sent: the connection to LiveSplit One is gone"));
            self.connection = None;
        }
    }
}
