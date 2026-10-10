//! The chat of a WordCraft window: members, one-time invites, keys, messages, the rules
//! (mentions, reserved names, the single-agent rule, the brake, one-line agent text) and the log
//! format. No UI and no network, and no clock or OS random source either: the app supplies both
//! through [`Env`], so this crate builds for every target, wasm included.
//!
//! What agents may do with the document is decided by the chat gate in `wordcraft-ui-egui`: a
//! policy for cooperating agents, not a sandbox.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod hub;
pub mod keys;
pub mod log;
pub mod rules;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use hub::{ChatError, Hub, Member, Message, Principal, Role};

/// What the hub needs from its host.
pub trait Env: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
    /// Fill `buf` with random bytes from the operating system.
    fn random(&self, buf: &mut [u8]) -> Result<(), String>;
}

/// The window's control port as the chat sees it (the desktop app implements it).
pub trait Port: Send + Sync {
    /// Open the port unless it is open (by `--control`, or by an earlier start); its address,
    /// `127.0.0.1:PORT`.
    fn open(&self) -> Result<String, String>;
    /// Close the port [`Port::open`] opened. A port opened with `--control` stays open.
    fn close(&self);
    /// The address while the port is open.
    fn address(&self) -> Option<String>;
}

/// The answer to an invite before Start chat.
pub const NOT_STARTED: &str = "the chat is not started: Review › Chat › Start Chat (command chat.start)";

/// The command an agent runs to use the chat, at the start of every invite line (the app can
/// set another one with [`Chat::set_client_command`]).
pub const DEFAULT_CLIENT_COMMAND: &str = "wordcraft-cli chat";

/// The environment variable that sets the command agents run to use the chat, for packagers
/// whose agents reach the app through another command (for example a Flatpak wrapper). The app
/// reads it for its invite lines; the client reads it for the briefing. The client's usage text
/// does not change: it always names `wordcraft-cli chat`.
pub const CLIENT_ENV: &str = "WORDCRAFT_CHAT_CLIENT";

/// The line the owner gives an agent: `{client} join {addr} {code} --as {handle}`.
pub fn invite_line(client: &str, addr: &str, code: &str, handle: &str) -> String {
    format!("{client} join {addr} {code} --as {handle}")
}

/// An invite waiting for its agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub handle: String,
    pub code: String,
    pub line: String,
}

/// A window's chat: the hub and the port agents reach it on.
pub struct Chat {
    hub: Arc<Hub>,
    port: Arc<dyn Port>,
    client: Mutex<String>,
    logs: Option<PathBuf>,
}

impl Chat {
    pub fn new(hub: Arc<Hub>, port: Arc<dyn Port>) -> Chat {
        Chat { hub, port, client: Mutex::new(DEFAULT_CLIENT_COMMAND.to_string()), logs: None }
    }
    /// Keep logs in `dir` (`<settings>/chats`); without it the chat stays in memory.
    pub fn with_logs(mut self, dir: PathBuf) -> Chat {
        self.logs = Some(dir);
        self
    }
    /// The log of the document at `canonical_doc`.
    pub fn log_for(&self, canonical_doc: &Path) -> Option<PathBuf> {
        self.logs.as_ref().map(|d| d.join(log::file_name(canonical_doc)))
    }
    pub fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }
    pub fn address(&self) -> Option<String> {
        self.port.address()
    }
    /// Started, and agents can reach it.
    pub fn running(&self) -> bool {
        self.hub.is_open() && self.port.address().is_some()
    }
    /// Start chat: open the port (unless `--control` opened it) and take invites.
    pub fn start(&self) -> Result<String, String> {
        let addr = self.port.open()?;
        self.hub.set_open(true);
        Ok(addr)
    }
    /// Stop chat: every member is disconnected (keys and invites revoked); the port Start
    /// opened closes.
    pub fn stop(&self) {
        self.hub.set_open(false);
        self.port.close();
    }
    /// The command at the start of invite lines (default [`DEFAULT_CLIENT_COMMAND`]). Empty or
    /// blank input is ignored.
    pub fn set_client_command(&self, cmd: String) {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            *self.client.lock().unwrap_or_else(|e| e.into_inner()) = cmd.to_string();
        }
    }
    /// Invite an agent by name (`Claude` or `@claude`).
    pub fn invite(&self, name: &str) -> Result<Invite, String> {
        let addr = self.address().filter(|_| self.hub.is_open()).ok_or_else(|| NOT_STARTED.to_string())?;
        let (handle, code) = self.hub.invite(name).map_err(|e| e.to_string())?;
        let client = self.client.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Ok(Invite { line: invite_line(&client, &addr, &code, &handle), handle, code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakePort, TestEnv};

    #[test]
    fn start_invite_stop() {
        let chat = Chat::new(Hub::new(TestEnv::at(1_000)), FakePort::closed());
        assert!(!chat.running());
        assert_eq!(chat.invite("@claude").unwrap_err(), NOT_STARTED);
        assert_eq!(chat.start().unwrap(), "127.0.0.1:7981");
        assert!(chat.running());
        let inv = chat.invite("Claude").unwrap();
        assert_eq!(inv.handle, "@claude");
        assert!(keys::valid_code(&inv.code), "{}", inv.code);
        assert_eq!(inv.line, format!("wordcraft-cli chat join 127.0.0.1:7981 {} --as @claude", inv.code));
        let (_, key) = chat.hub().join(&inv.code).unwrap();
        assert_eq!(chat.hub().member_for_key(&key).as_deref(), Some("@claude"));
        chat.stop();
        assert!(!chat.running());
        assert!(chat.hub().member_for_key(&key).is_none(), "Stop chat revokes every member");
        assert!(chat.address().is_none(), "the port Start opened is closed");
    }

    #[test]
    fn invite_line_uses_the_client_command_the_app_sets() {
        assert_eq!(invite_line(DEFAULT_CLIENT_COMMAND, "127.0.0.1:1", "K", "@pi"), "wordcraft-cli chat join 127.0.0.1:1 K --as @pi");
        let chat = Chat::new(Hub::new(TestEnv::at(1_000)), FakePort::closed());
        assert!(chat.start().is_ok());
        chat.set_client_command(String::new());
        chat.set_client_command(" \t ".into());
        let inv = chat.invite("@pi").unwrap();
        assert_eq!(inv.line, format!("wordcraft-cli chat join 127.0.0.1:7981 {} --as @pi", inv.code), "empty input is ignored");
        chat.set_client_command("  other-client chat ".into());
        let inv = chat.invite("@agent-2_x").unwrap();
        assert_eq!(inv.line, format!("other-client chat join 127.0.0.1:7981 {} --as @agent-2_x", inv.code));
    }

    #[test]
    fn stop_leaves_a_control_flag_port_open() {
        let chat = Chat::new(Hub::new(TestEnv::at(1)), FakePort::flag("127.0.0.1:7990"));
        assert!(!chat.running(), "a --control port alone does not start the chat");
        assert_eq!(chat.start().unwrap(), "127.0.0.1:7990");
        chat.stop();
        assert_eq!(chat.address().as_deref(), Some("127.0.0.1:7990"));
        assert!(!chat.running());
    }

    #[test]
    fn invite_needs_random_bytes() {
        let env = TestEnv::at(1_000);
        let chat = Chat::new(Hub::new(env.clone()), FakePort::closed());
        assert!(chat.start().is_ok());
        env.fail_random(true);
        let e = chat.invite("@claude").unwrap_err();
        assert!(e.starts_with("no random bytes"), "{e}");
    }

    /// wasm: the app supplies time and random bytes (point 4 of the review).
    #[test]
    fn no_clock_and_no_os_random_in_this_crate() {
        let sources = [
            ("Cargo.toml", include_str!("../Cargo.toml")),
            ("lib.rs", include_str!("lib.rs")),
            ("hub.rs", include_str!("hub.rs")),
            ("keys.rs", include_str!("keys.rs")),
            ("log.rs", include_str!("log.rs")),
            ("rules.rs", include_str!("rules.rs")),
        ];
        for (name, src) in sources {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            for banned in [concat!("SystemTime", "::now"), concat!("Instant", "::now"), concat!("get", "random")] {
                assert!(!code.contains(banned), "{name} uses {banned}");
            }
        }
    }
}
