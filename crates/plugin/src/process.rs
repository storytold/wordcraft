//! The connection to a plugin process: newline-delimited JSON-RPC 2.0 over its standard input
//! and output. Reading and writing happen on their own threads, so a plugin that stops reading
//! or never answers costs a timeout, never a hung app. Standard error goes to the log.

use std::path::Path;
use std::time::Duration;

use serde_json::Value as Json;

use crate::Manifest;

/// A failed request. `fatal`: the connection can't be trusted any more (the process exited, a
/// timeout left an answer in flight, the stream broke) and is restarted.
#[derive(Debug)]
pub struct RequestError {
    pub message: String,
    pub fatal: bool,
}

impl RequestError {
    fn fatal(message: impl Into<String>) -> RequestError {
        RequestError { message: message.into(), fatal: true }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Connection;

#[cfg(target_arch = "wasm32")]
pub use web::Connection;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;

    /// The browser can't start processes: plugins need the desktop app or the CLI.
    pub struct Connection;

    impl Connection {
        pub fn start(_m: &Manifest, _dir: &Path) -> Result<Connection, String> {
            Err("plugins need the desktop app or the CLI".into())
        }
        pub fn request(&mut self, _method: &str, _params: Json, _timeout: Duration) -> Result<Json, RequestError> {
            Err(RequestError::fatal("plugins need the desktop app or the CLI"))
        }
        pub fn shutdown(self) {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)] // off wasm only: the clock works here
mod native {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
    use std::time::Instant;

    use serde_json::json;

    use super::*;

    /// Longest line read from a plugin (a large array result fits).
    const MAX_LINE: u64 = 64 << 20;
    const MAX_STDERR_LINE: u64 = 64 << 10;

    pub struct Connection {
        id: String,
        child: Child,
        to_plugin: Sender<Vec<u8>>,
        from_plugin: Receiver<Result<Json, String>>,
        next_id: u64,
    }

    impl Connection {
        pub fn start(m: &Manifest, dir: &Path) -> Result<Connection, String> {
            let program =
                if m.run.command.contains('/') || m.run.command.contains('\\') { dir.join(&m.run.command) } else { m.run.command.clone().into() };
            let mut child = Command::new(&program)
                .args(&m.run.args)
                .current_dir(dir)
                .env("WORDCRAFT_PLUGIN_DIR", dir)
                .env("CRAFT_PLUGIN_DIR", dir)
                .env("CRAFT_HOST", crate::HOST_ID)
                .env("WORDCRAFT_VERSION", env!("CARGO_PKG_VERSION"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("can't start {}: {e}", program.display()))?;
            let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the plugin's standard input and output are not available".into());
            };
            let id = m.id.clone();

            let (to_plugin, outbox) = channel::<Vec<u8>>();
            let writer = std::thread::Builder::new().name(format!("plugin-{id}-in")).spawn(move || {
                let mut stdin = stdin;
                while let Ok(bytes) = outbox.recv() {
                    if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                        break;
                    }
                }
            });

            let (inbox, from_plugin) = channel::<Result<Json, String>>();
            let rid = id.clone();
            let reader = std::thread::Builder::new().name(format!("plugin-{id}-out")).spawn(move || {
                let mut out = BufReader::new(stdout);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match out.by_ref().take(MAX_LINE).read_until(b'\n', &mut line) {
                        Ok(0) => {
                            let _ = inbox.send(Err("the plugin exited".into()));
                            break;
                        }
                        Ok(_) if !line.ends_with(b"\n") && line.len() as u64 >= MAX_LINE => {
                            let _ = inbox.send(Err("the plugin sent a message larger than 64 MiB".into()));
                            break;
                        }
                        Ok(_) => match serde_json::from_slice::<Json>(&line) {
                            Ok(msg) => {
                                if inbox.send(Ok(msg)).is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                let text = String::from_utf8_lossy(&line);
                                if !text.trim().is_empty() {
                                    log::warn!("plugin {rid}: ignored a line that is not JSON: {}", text.trim());
                                }
                            }
                        },
                        Err(e) => {
                            let _ = inbox.send(Err(format!("reading from the plugin: {e}")));
                            break;
                        }
                    }
                }
            });

            if let Some(stderr) = child.stderr.take() {
                let eid = id.clone();
                let _ = std::thread::Builder::new().name(format!("plugin-{id}-err")).spawn(move || {
                    let mut err = BufReader::new(stderr);
                    let mut line = Vec::new();
                    loop {
                        line.clear();
                        match err.by_ref().take(MAX_STDERR_LINE).read_until(b'\n', &mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => log::info!("plugin {eid}: {}", String::from_utf8_lossy(&line).trim_end()),
                        }
                    }
                });
            }

            if writer.is_err() || reader.is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return Err("can't start the plugin's I/O threads".into());
            }
            Ok(Connection { id, child, to_plugin, from_plugin, next_id: 1 })
        }

        pub fn request(&mut self, method: &str, params: Json, timeout: Duration) -> Result<Json, RequestError> {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            let mut bytes = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
                .map_err(|e| RequestError::fatal(format!("encoding the request: {e}")))?;
            bytes.push(b'\n');
            self.to_plugin.send(bytes).map_err(|_| RequestError::fatal("the plugin stopped reading"))?;
            let deadline = Instant::now() + timeout;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let msg = match self.from_plugin.recv_timeout(left) {
                    Ok(Ok(msg)) => msg,
                    Ok(Err(e)) => return Err(RequestError::fatal(e)),
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(RequestError::fatal(format!("{method} took longer than {} ms", timeout.as_millis())));
                    }
                    Err(RecvTimeoutError::Disconnected) => return Err(RequestError::fatal("the plugin exited")),
                };
                if msg.get("id").and_then(Json::as_u64) != Some(id) || msg.get("method").is_some() {
                    // A notification or request from the plugin: not supported by protocol 1.
                    log::debug!("plugin {}: ignored message {msg}", self.id);
                    continue;
                }
                if let Some(err) = msg.get("error") {
                    let text = err.get("message").and_then(Json::as_str).map(str::to_string).unwrap_or_else(|| err.to_string());
                    return Err(RequestError { message: text, fatal: false });
                }
                return Ok(msg.get("result").cloned().unwrap_or(Json::Null));
            }
        }

        /// Asks the plugin to exit, then makes sure it has.
        pub fn shutdown(mut self) {
            let _ = self.to_plugin.send(b"{\"jsonrpc\":\"2.0\",\"method\":\"shutdown\"}\n".to_vec());
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    impl Drop for Connection {
        fn drop(&mut self) {
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
    }
}
