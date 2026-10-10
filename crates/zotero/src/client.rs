//! Running one integration command against Zotero over its socket.
//!
//! [`run_command`] connects, sends the command, then answers each of Zotero's calls through a
//! callback until `Document_complete` (or until Zotero closes the connection, which it does when
//! the user cancels). The callback form lets the app answer on its UI thread;
//! [`run_on_session`] answers directly against a session (CLI, tests).

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

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
}

impl Default for Options {
    fn default() -> Options {
        Options {
            addr: SocketAddr::from(([127, 0, 0, 1], wire::PORT)),
            connect_timeout: Duration::from_secs(3),
            idle_timeout: Duration::from_secs(60 * 60),
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
    pub errors: Vec<String>,
}

/// Run `command`, answering every call with `answer`.
pub fn run_command(opts: &Options, command: Command, answer: &mut dyn FnMut(&Call) -> Result<Value, String>) -> Result<Outcome, ZoteroError> {
    let mut stream = TcpStream::connect_timeout(&opts.addr, opts.connect_timeout).map_err(|e| ZoteroError::NotRunning(e.to_string()))?;
    let _ = stream.set_nodelay(true);
    stream.set_read_timeout(Some(opts.idle_timeout)).map_err(|e| ZoteroError::Io(e.to_string()))?;
    wire::write_frame(&mut stream, 0, &wire::command_payload(command.wire_name()))?;
    serve(&mut stream, answer)
}

/// Answer calls on an open connection until the session ends.
pub fn serve<S: std::io::Read + std::io::Write>(
    stream: &mut S,
    answer: &mut dyn FnMut(&Call) -> Result<Value, String>,
) -> Result<Outcome, ZoteroError> {
    let mut out = Outcome::default();
    while let Some(frame) = wire::read_frame(stream)? {
        if frame.txid == 0 {
            // Not a call; nothing to answer.
            continue;
        }
        out.calls += 1;
        let (reply, done) = match Call::parse(&frame.payload) {
            Ok(call) => {
                log::debug!("Zotero → {}", call.method);
                let r = answer(&call);
                (r, call.method == "Document_complete")
            }
            Err(e) => (Err(e), false),
        };
        if let Err(e) = &reply {
            log::warn!("Zotero call failed: {e}");
            out.errors.push(e.clone());
        }
        wire::write_frame(stream, frame.txid, &wire::encode_reply(&reply))?;
        if done {
            out.completed = true;
            break;
        }
    }
    Ok(out)
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
