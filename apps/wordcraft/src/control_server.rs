//! The control server: craft-automation's loopback JSON-lines transport, started on the port the
//! user asked for.
//!
//! A connection authenticates once with the token (`docs/control-protocol.md`, Token). When no
//! token or token file is supplied, the app generates a token and writes it to
//! `<settings>/control-key.<instance>` (mode 0600) after the port is bound, and removes the file
//! when the window closes.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use craft::automation::args::ControlOptions;
use craft::automation::control::ControlRequest;
use craft::automation::server::{Listening, serve};
use craft::ui::runtime::windows::WindowHost;

/// A running control server. [`Control::shut_down`] it when the window closes.
pub struct Control {
    /// Dropping it stops accepting connections and frees the port.
    listening: Listening,
    /// The default token file this run wrote; a token file the user supplied is never removed.
    created: Option<PathBuf>,
}

impl Control {
    /// Stop accepting connections and remove the token file this run wrote, if any.
    pub fn shut_down(self) {
        let Self { listening, created } = self;
        drop(listening);
        remove(created.as_deref());
    }
}

fn remove(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

/// Bind `127.0.0.1:port`, settle the token and serve. `None` when the port cannot be bound or
/// the token cannot be set up (the control channel is off); a taken port leaves no token file.
///
/// With no token and no token file in `options`, the token is kept in the default file for
/// `port` in `settings`; a file left by a run that did not exit cleanly is replaced. Without a
/// settings folder a fresh token is generated and reported by the log.
pub fn start(options: &ControlOptions, port: u16, host: WindowHost, settings: Option<&Path>) -> Option<(Receiver<ControlRequest>, Control)> {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return None;
        }
    };
    let created = settings.filter(|_| options.token.is_none() && options.token_file.is_none()).map(|dir| wordcraft_control_key::key_path(dir, port));
    // The port was free, so no other run owns a file left here.
    remove(created.as_deref());
    let effective = ControlOptions { token_file: created.clone().or_else(|| options.token_file.clone()), ..options.clone() };
    let token = match effective.server_token() {
        Ok(t) => t,
        Err(e) => {
            remove(created.as_deref());
            log::error!("control server off: {e}");
            return None;
        }
    };
    let notice = effective.token_notice(&token);
    match serve(listener, token, host) {
        Ok((rx, listening)) => {
            log::info!("control server listening on {}, {notice}", listening.addr);
            Some((rx, Control { listening, created }))
        }
        Err(e) => {
            remove(created.as_deref());
            log::error!("control server off: cannot start the accept thread ({e})");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::automation::client::ControlClient;

    fn host() -> WindowHost {
        WindowHost::of(&egui::Context::default())
    }

    #[test]
    fn the_token_file_is_written_after_the_bind_and_removed_on_exit() {
        let dir = std::env::temp_dir().join(format!("wordcraft-control-life-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let options = ControlOptions::default();

        // The port is taken: no token file.
        let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert!(start(&options, port, host(), Some(&dir)).is_none());
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(), "no token file after a failed bind");
        drop(taken);

        // Bound: the default file holds the token that opens the port.
        let (_rx, control) = start(&options, 0, host(), Some(&dir)).unwrap();
        let path = control.created.clone().unwrap();
        assert_eq!(path, wordcraft_control_key::key_path(&dir, 0));
        let token = std::fs::read_to_string(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(ControlClient::connect(control.listening.addr, token.trim()).is_ok());
        control.shut_down();
        assert!(!path.exists());

        // A token file the user supplied stays.
        let supplied = dir.join("supplied-token");
        let options = ControlOptions { token_file: Some(supplied.clone()), ..ControlOptions::default() };
        let (_rx, control) = start(&options, 0, host(), Some(&dir)).unwrap();
        assert!(control.created.is_none());
        control.shut_down();
        assert!(supplied.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
