//! Running one integration command against Zotero over its socket.
//!
//! [`run_command`] connects, sends the command, then answers each of Zotero's calls through a
//! callback until `Document_complete` (or until Zotero closes the connection, which it does when
//! the user cancels). The callback form lets the app answer on its UI thread;
//! [`run_on_session`] answers directly against a session (CLI, tests).

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use wordcraft_engine::Session;

use crate::wire::{self, Call};
use crate::{Bridge, Host, ZoteroError};

/// Zotero's integration commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Add a citation, or edit the one at the caret.
    AddEditCitation,
    /// Add a bibliography, or edit the one at the caret.
    AddEditBibliography,
    /// Insert a Zotero note.
    AddNote,
    /// Update every citation and the bibliography.
    Refresh,
    /// Unlink citations: keep their text, drop the codes.
    RemoveCodes,
    /// Document preferences: citation style, language, note type.
    SetDocPrefs,
}

impl Command {
    pub const ALL: [Command; 6] =
        [Command::AddEditCitation, Command::AddEditBibliography, Command::AddNote, Command::Refresh, Command::RemoveCodes, Command::SetDocPrefs];

    /// The name on the wire.
    pub fn wire_name(self) -> &'static str {
        match self {
            Command::AddEditCitation => "addEditCitation",
            Command::AddEditBibliography => "addEditBibliography",
            Command::AddNote => "addNote",
            Command::Refresh => "refresh",
            Command::RemoveCodes => "removeCodes",
            Command::SetDocPrefs => "setDocPrefs",
        }
    }

    /// By wire name (any case).
    pub fn from_name(name: &str) -> Option<Command> {
        Command::ALL.into_iter().find(|c| c.wire_name().eq_ignore_ascii_case(name))
    }
}

/// Connection settings.
#[derive(Clone, Debug)]
pub struct Options {
    pub addr: SocketAddr,
    /// How long to wait for Zotero to accept the connection.
    pub connect_timeout: Duration,
    /// How long to wait for Zotero's next call. Zotero's dialogs wait for the user, so this is
    /// long; it only ends a session Zotero abandoned without closing the connection.
    pub idle_timeout: Duration,
    /// How long one message from Zotero may take to arrive once it has started (a peer that
    /// drip-feeds bytes).
    pub frame_timeout: Duration,
    /// How long one reply may take to send (a peer that stops reading).
    pub write_timeout: Duration,
    /// The longest a whole command may run, dialogs included.
    pub session_timeout: Duration,
    /// The most messages Zotero may send in one command.
    pub max_calls: usize,
    /// Set to end the command early; checked several times a second.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            addr: SocketAddr::from(([127, 0, 0, 1], wire::PORT)),
            connect_timeout: Duration::from_secs(3),
            idle_timeout: Duration::from_secs(60 * 60),
            frame_timeout: Duration::from_secs(60),
            write_timeout: Duration::from_secs(60),
            session_timeout: Duration::from_secs(4 * 60 * 60),
            max_calls: 200_000,
            cancel: None,
        }
    }
}

/// How a session ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Calls Zotero made.
    pub calls: usize,
    /// Zotero finished with `Document_complete` (false: it closed the connection, e.g. the
    /// user cancelled its dialog).
    pub completed: bool,
    /// Calls we answered with an error.
    /// Calls we answered with an error: the first [`MAX_ERRORS`], each clipped to
    /// [`MAX_ERROR_LEN`] bytes.
    pub errors: Vec<String>,
    /// Failed calls past the first [`MAX_ERRORS`].
    pub more_errors: usize,
}

/// Failed calls kept in [`Outcome::errors`].
pub const MAX_ERRORS: usize = 20;
/// Longest error text kept, in bytes.
pub const MAX_ERROR_LEN: usize = 500;
/// Longest method name logged, in bytes.
pub const MAX_METHOD_LEN: usize = 128;
/// How often a blocked read or write wakes to check the limits and [`Options::cancel`].
const TICK: Duration = Duration::from_millis(100);

/// Run `command`, answering every call with `answer`.
pub fn run_command(opts: &Options, command: Command, answer: &mut dyn FnMut(&Call) -> Result<Value, String>) -> Result<Outcome, ZoteroError> {
    crate::peer::check_addr(&opts.addr)?;
    let stream = TcpStream::connect_timeout(&opts.addr, opts.connect_timeout).map_err(|e| ZoteroError::NotRunning(e.to_string()))?;
    // Before Zotero sees anything: the command, and then the document, go only to this computer
    // and, on Linux only, not to a listener the socket table shows belongs to another user.
    crate::peer::check(&stream)?;
    let _ = stream.set_nodelay(true);
    let mut link = Link::new(&stream, opts);
    link.start_write();
    wire::write_frame(&mut link, 0, &wire::command_payload(command.wire_name()))?;
    let r = serve(&mut link, opts, answer);
    // Hang up at once, also on an error, so the peer sees the end.
    let _ = stream.shutdown(std::net::Shutdown::Both);
    r
}

/// Answer calls on an open connection until the session ends.
fn serve(link: &mut Link<'_>, opts: &Options, answer: &mut dyn FnMut(&Call) -> Result<Value, String>) -> Result<Outcome, ZoteroError> {
    let mut out = Outcome::default();
    let mut frames = 0usize;
    loop {
        link.start_read();
        let Some(frame) = wire::read_frame(link)? else { break };
        frames = frames.saturating_add(1);
        if frames > opts.max_calls {
            return Err(ZoteroError::Protocol(format!("Zotero sent more than {} messages in one command", opts.max_calls)));
        }
        if frame.txid == 0 {
            // Not a call; nothing to answer.
            continue;
        }
        out.calls = out.calls.saturating_add(1);
        let (reply, done) = match Call::parse(&frame.payload) {
            Ok(call) => {
                log::debug!("Zotero → {}", clip(&call.method, MAX_METHOD_LEN));
                let r = answer(&call);
                (r, call.method == "Document_complete")
            }
            Err(e) => (Err(e), false),
        };
        if let Err(e) = &reply {
            let e = clip(e, MAX_ERROR_LEN);
            // The first few go to the log; the command's summary counts the rest.
            if out.errors.len() < MAX_ERRORS {
                log::warn!("Zotero call failed: {e}");
                out.errors.push(e.to_string());
            } else {
                out.more_errors = out.more_errors.saturating_add(1);
            }
        }
        let reply = wire::encode_reply(&reply);
        // The send deadline starts once the reply is ready: it measures Zotero, not us.
        link.start_write();
        wire::write_frame(link, frame.txid, &reply)?;
        if done {
            out.completed = true;
            break;
        }
    }
    Ok(out)
}

/// `text` cut to at most `max` bytes, at a character boundary (with "…" when cut). For keeping
/// or logging text that came from Zotero.
pub fn clip(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if text.len() <= max {
        return text.into();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}…", text.get(..end).unwrap_or("")).into()
}

/// The socket with the session's limits: every blocking read or write wakes each [`TICK`] to
/// check [`Options::cancel`] and the deadlines, and fails once one has passed.
struct Link<'a> {
    stream: &'a TcpStream,
    opts: &'a Options,
    /// The whole session's deadline.
    end: Option<Instant>,
    /// When we started waiting for Zotero's next message.
    read_since: Instant,
    /// When the message being read began to arrive.
    frame_since: Option<Instant>,
    /// When we started sending the current reply.
    write_since: Instant,
}

impl<'a> Link<'a> {
    fn new(stream: &'a TcpStream, opts: &'a Options) -> Link<'a> {
        let now = Instant::now();
        Link { stream, opts, end: now.checked_add(opts.session_timeout), read_since: now, frame_since: None, write_since: now }
    }

    fn start_read(&mut self) {
        self.read_since = Instant::now();
        self.frame_since = None;
    }

    fn start_write(&mut self) {
        self.write_since = Instant::now();
    }

    /// How long the next blocking call may wait, or the error that ends the session.
    fn wait(&self, deadline: Option<Instant>, why: impl FnOnce() -> String) -> io::Result<Duration> {
        if self.opts.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(io::Error::other("cancelled in WordCraft"));
        }
        let now = Instant::now();
        if self.end.is_some_and(|end| now >= end) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, format!("the command ran longer than {}", span(self.opts.session_timeout))));
        }
        if deadline.is_some_and(|d| now >= d) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, why()));
        }
        let left = [deadline, self.end].into_iter().flatten().map(|d| d.saturating_duration_since(now)).fold(TICK, Duration::min);
        // A zero timeout is an error for the socket.
        Ok(left.max(Duration::from_millis(1)))
    }
}

/// A duration for messages: "90 ms", "30 s", "60 min".
fn span(d: Duration) -> String {
    match d.as_secs() {
        0 => format!("{} ms", d.as_millis()),
        s if s < 120 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

fn timed_out(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

impl Read for Link<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let opts = self.opts;
            let wait = match self.frame_since {
                Some(since) => self.wait(since.checked_add(opts.frame_timeout), || {
                    format!("Zotero sent part of a message and didn't finish it within {}", span(opts.frame_timeout))
                })?,
                None => {
                    self.wait(self.read_since.checked_add(opts.idle_timeout), || format!("Zotero sent nothing for {}", span(opts.idle_timeout)))?
                }
            };
            self.stream.set_read_timeout(Some(wait))?;
            match (&mut &*self.stream).read(buf) {
                Ok(n) => {
                    if n > 0 && self.frame_since.is_none() {
                        self.frame_since = Some(Instant::now());
                    }
                    return Ok(n);
                }
                Err(e) if timed_out(&e) => {}
                Err(e) => return Err(e),
            }
        }
    }
}

impl Write for Link<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            let opts = self.opts;
            let wait = self.wait(self.write_since.checked_add(opts.write_timeout), || {
                format!("Zotero stopped reading WordCraft's reply for {}", span(opts.write_timeout))
            })?;
            self.stream.set_write_timeout(Some(wait))?;
            match (&mut &*self.stream).write(buf) {
                Err(e) if timed_out(&e) => {}
                r => return r,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        (&mut &*self.stream).flush()
    }
}

/// Run `command` against `session` directly.
pub fn run_on_session(
    opts: &Options,
    command: Command,
    bridge: &mut Bridge,
    session: &mut Session,
    host: &mut dyn Host,
) -> Result<Outcome, ZoteroError> {
    run_command(opts, command, &mut |call| bridge.handle(session, host, call))
}
