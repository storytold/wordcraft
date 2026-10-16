//! Links on Linux and the BSDs (#2349): the `Services::open_url` hook.
//!
//! egui's opener (the `webbrowser` crate) looks for the browser itself (outside Flatpak, where
//! it runs `xdg-open`): `$BROWSER`, then
//! `xdg-settings` and the browser's desktop file in the XDG data folders, then commands named
//! after the desktop it guesses from `XDG_CURRENT_DESKTOP`. A game launcher such as Steam on
//! SteamOS starts apps without those variables, so it finds nothing ("No valid browsers
//! detected") while `xdg-open` works. So links go to the system's own openers first:
//! 1. `xdg-open`, the freedesktop.org command every desktop provides (and Flatpak and distrobox
//!    forward to the host);
//! 2. the desktop portal's OpenURI, over D-Bus, where `xdg-open` is missing or fails;
//! 3. the `webbrowser` crate last, for systems with neither (a bare window manager with
//!    `$BROWSER`, WSL).
//!
//! Each runs on a worker thread, never the UI thread, and stops waiting after a few seconds.

use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

/// How long `xdg-open` may run before it counts as opened: without a desktop it runs the browser
/// itself and only returns when the browser closes. It is reaped in the background.
const XDG_OPEN_WAIT: Duration = Duration::from_secs(5);
/// How long to wait for the portal's answer, which comes once the link is handed to an app. A
/// quick "couldn't" moves on to the next way; no answer yet counts as opened (it may be asking
/// the user to choose an app).
const PORTAL_WAIT: Duration = Duration::from_secs(10);

/// The hook: opens `url` on a worker, sends the result and wakes the UI.
pub fn hook(ctx: egui::Context) -> wordcraft_ui_egui::open_url::Hook {
    Box::new(move |url: &str| {
        let (tx, rx) = channel();
        let url = url.to_string();
        let ctx = ctx.clone();
        // A worker that can't start drops `tx`, which the UI reads as a failure.
        let spawned = std::thread::Builder::new().name("open-url".into()).spawn(move || {
            let _ = tx.send(open(&url));
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            log::warn!("open link: {e}");
        }
        rx
    })
}

/// Open a web or email link with the first way that works.
fn open(url: &str) -> Result<(), String> {
    // The last line of defence behind the interface: a document's link never reaches the system
    // unless it is a web or email link (also keeps a leading `-` from reading as an option).
    if !wordcraft_doc::link::is_web_or_mail_link(url) {
        return Err("only http, https and mailto links are opened".into());
    }
    first_that_opens(
        url,
        &[
            ("xdg-open", &|u| run_opener("xdg-open", u, XDG_OPEN_WAIT)),
            ("desktop portal", &|u| portal::open_uri(u, PORTAL_WAIT)),
            ("webbrowser", &|u| webbrowser::open(u).map_err(|e| e.to_string())),
        ],
    )
}

type Way<'a> = (&'a str, &'a dyn Fn(&str) -> Result<(), String>);

/// Try each way in turn; the error lists why each one failed.
fn first_that_opens(url: &str, ways: &[Way<'_>]) -> Result<(), String> {
    let mut errors = Vec::new();
    for (name, way) in ways {
        match way(url) {
            Ok(()) => {
                if !errors.is_empty() {
                    log::info!("opened a link with {name} after: {}", errors.join("; "));
                }
                return Ok(());
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    Err(errors.join("; "))
}

/// Run `program url` and wait up to `wait` for it: an exit code of 0 is success, a non-zero one a
/// failure, and still running after `wait` counts as success (it is waited for in the background).
fn run_opener(program: &str, url: &str, wait: Duration) -> Result<(), String> {
    let mut child =
        Command::new(program).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + wait;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("failed ({status})")),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = std::thread::Builder::new().name("open-url-wait".into()).spawn(move || child.wait());
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

mod portal {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    const DESKTOP: &str = "org.freedesktop.portal.Desktop";

    /// Hand `url` to the desktop portal's OpenURI on the session bus.
    pub(super) fn open_uri(url: &str, wait: Duration) -> Result<(), String> {
        let connection = Connection::session().map_err(|e| format!("no session bus ({e})"))?;
        open_uri_on(&connection, url, wait)
    }

    /// OpenURI on `connection`, waiting up to `wait` for the portal's answer: 0 (opened) and 1
    /// (the user closed the app chooser) are fine, 2 is a failure. Once the portal has taken the
    /// request, no answer within `wait` counts as opened too: it may be showing its app chooser,
    /// and trying another way behind it would open the link twice.
    pub(super) fn open_uri_on(connection: &Connection, url: &str, wait: Duration) -> Result<(), String> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let token = format!("wordcraft{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let sender = connection.unique_name().ok_or("no name on the bus")?.trim_start_matches(':').replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let e = |e: zbus::Error| e.to_string();
        // Listen on the request before asking, so a quick answer isn't missed.
        let request = Proxy::new(connection, DESKTOP, path.as_str(), "org.freedesktop.portal.Request").map_err(e)?;
        let mut responses = request.receive_signal("Response").map_err(e)?;
        let portal = Proxy::new(connection, DESKTOP, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.OpenURI").map_err(e)?;
        let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
        let handle: OwnedObjectPath = portal.call("OpenURI", &("", url, options)).map_err(e)?;
        if handle.as_str() != path {
            // Portals before version 0.9 make up their own request path.
            log::debug!("desktop portal answers on {handle}, not {path}; not waiting for it");
            return Ok(());
        }
        // The signal iterator has no timeout: it is read on a helper, which ends with the
        // answer or the connection (after a timeout it keeps waiting for the user's choice).
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("open-url-portal".into())
            .spawn(move || {
                let code = responses.next().and_then(|m| m.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().ok()).map(|(code, _)| code);
                let _ = tx.send(code);
            })
            .map_err(|e| e.to_string())?;
        match rx.recv_timeout(wait) {
            Ok(Some(0 | 1)) => Ok(()),
            Ok(Some(code)) => Err(format!("couldn't open it (response {code})")),
            Ok(None) => Err("no answer".into()),
            Err(_) => {
                log::info!("desktop portal took the link and hasn't answered in {} s (an app chooser may be open)", wait.as_secs());
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use super::*;

    const URL: &str = "https://discord.gg/artcraft";

    /// A folder for fake programs, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("wordcraft-open-url-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        /// A shell script `name` with `body`, made executable.
        fn script(&self, name: &str, body: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let p = self.0.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(program: &Path, wait: Duration) -> Result<(), String> {
        run_opener(program.to_str().unwrap(), URL, wait)
    }

    #[test]
    fn the_opener_gets_the_link_as_one_argument() {
        let s = Scratch::new("args");
        let log = s.0.join("args");
        let fake = s.script("xdg-open", &format!("printf '%s|' \"$@\" > '{}'", log.display()));
        assert_eq!(run(&fake, Duration::from_secs(5)), Ok(()));
        assert_eq!(std::fs::read_to_string(log).unwrap(), format!("{URL}|"));
    }

    #[test]
    fn a_failing_or_missing_opener_is_an_error() {
        let s = Scratch::new("fail");
        // xdg-open's exit code when no method is available.
        let err = run(&s.script("xdg-open", "exit 3"), Duration::from_secs(5)).unwrap_err();
        assert!(err.contains('3'), "{err}");
        assert!(run(&s.0.join("not-installed"), Duration::from_secs(5)).is_err());
    }

    /// Without a desktop, xdg-open runs the browser itself until it closes: that counts as opened.
    #[test]
    fn an_opener_still_running_counts_as_opened() {
        let s = Scratch::new("slow");
        let start = Instant::now();
        assert_eq!(run(&s.script("xdg-open", "sleep 3"), Duration::from_millis(200)), Ok(()));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn ways_are_tried_in_order_until_one_opens() {
        let tried = RefCell::new(Vec::new());
        let way = |name: &'static str, works: bool| {
            let tried = &tried;
            move |_: &str| -> Result<(), String> {
                tried.borrow_mut().push(name);
                if works { Ok(()) } else { Err("nope".into()) }
            }
        };
        let (a, b, c) = (way("xdg-open", false), way("portal", true), way("webbrowser", true));
        assert_eq!(first_that_opens(URL, &[("xdg-open", &a), ("portal", &b), ("webbrowser", &c)]), Ok(()));
        assert_eq!(*tried.borrow(), ["xdg-open", "portal"], "stops at the first that opens");
        tried.borrow_mut().clear();
        let (a, b) = (way("xdg-open", false), way("portal", false));
        let err = first_that_opens(URL, &[("xdg-open", &a), ("portal", &b)]).unwrap_err();
        assert_eq!(err, "xdg-open: nope; portal: nope");
    }

    #[test]
    fn only_web_and_email_links_reach_the_system() {
        for url in ["file:///etc/passwd", "smb://host/share", "-x", "javascript:alert(1)", ""] {
            assert!(open(url).is_err(), "{url}");
        }
    }

    /// The portal half, against a stand-in portal on a private bus (never the real one, which
    /// would open a browser). Skipped where `dbus-daemon` isn't installed.
    mod portal_on_a_private_bus {
        use std::io::{BufRead, BufReader};
        use std::process::{Child, Command, Stdio};
        use std::sync::mpsc::Sender;
        use std::time::Duration;

        use zbus::blocking::{Connection, MessageIterator, connection};
        use zbus::message::Type;
        use zbus::zvariant::{ObjectPath, OwnedValue};

        use super::super::portal::open_uri_on;
        use super::{Scratch, URL};

        /// The bus daemon, its address, and the folder holding its configuration.
        struct Bus(Child, String, #[allow(dead_code, reason = "removed on drop")] Scratch);

        impl Drop for Bus {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        /// A session bus with no services to start, so nothing real can answer.
        fn private_bus(name: &str) -> Option<Bus> {
            let scratch = Scratch::new(name);
            let conf = scratch.0.join("bus.conf");
            std::fs::write(
                &conf,
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>"#,
            )
            .ok()?;
            let mut child = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", conf.display()))
                .args(["--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .spawn()
                .ok()?;
            let mut address = String::new();
            BufReader::new(child.stdout.take()?).read_line(&mut address).ok()?;
            Some(Bus(child, address.trim().to_string(), scratch))
        }

        /// A stand-in portal: answers each OpenURI with `code` after `delay` (or not at all) and
        /// reports the URL.
        fn fake_portal(bus: &Bus, code: Option<u32>, delay: Duration, seen: Sender<String>) -> Connection {
            let conn = connection::Builder::address(bus.1.as_str()).unwrap().build().unwrap();
            conn.request_name("org.freedesktop.portal.Desktop").unwrap();
            let serving = conn.clone();
            std::thread::spawn(move || {
                for msg in MessageIterator::from(&serving) {
                    let Ok(msg) = msg else { break };
                    let h = msg.header();
                    if h.message_type() != Type::MethodCall || h.member().map(|m| m.as_str()) != Some("OpenURI") {
                        continue;
                    }
                    let (_, uri, options): (String, String, std::collections::HashMap<String, OwnedValue>) = msg.body().deserialize().unwrap();
                    let token: String = options.get("handle_token").unwrap().try_clone().unwrap().try_into().unwrap();
                    let sender = h.sender().unwrap().trim_start_matches(':').replace('.', "_");
                    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
                    serving.reply(&h, &ObjectPath::try_from(path.as_str()).unwrap()).unwrap();
                    let _ = seen.send(uri);
                    if let Some(code) = code {
                        std::thread::sleep(delay);
                        let results: std::collections::HashMap<String, OwnedValue> = Default::default();
                        // The bus may be gone by then (an answer later than the test waits).
                        let _ = serving.emit_signal(
                            h.sender().map(|s| s.to_owned()),
                            path.as_str(),
                            "org.freedesktop.portal.Request",
                            "Response",
                            &(code, results),
                        );
                    }
                }
            });
            conn
        }

        /// Ask the stand-in portal (answering `code` after `delay`), waiting up to 500 ms.
        fn ask(name: &str, code: Option<u32>, delay: Duration) -> Option<(Result<(), String>, String)> {
            let Some(bus) = private_bus(name) else {
                eprintln!("skipped: no dbus-daemon");
                return None;
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let _portal = fake_portal(&bus, code, delay, tx);
            let app = connection::Builder::address(bus.1.as_str()).unwrap().build().unwrap();
            let result = open_uri_on(&app, URL, Duration::from_millis(500));
            Some((result, rx.recv_timeout(Duration::from_secs(5)).unwrap()))
        }

        #[test]
        fn the_portal_gets_the_link_and_its_answer_counts() {
            let now = Duration::ZERO;
            let Some((result, uri)) = ask("portal-0", Some(0), now) else { return };
            assert_eq!(uri, URL);
            assert_eq!(result, Ok(()));
            let Some((result, _)) = ask("portal-1", Some(1), now) else { return };
            assert_eq!(result, Ok(()), "the user closing the app chooser isn't a failure");
            let Some((result, _)) = ask("portal-2", Some(2), now) else { return };
            assert!(result.unwrap_err().contains("response 2"));
        }

        /// A portal that took the link and is still asking the user which app to use: opened, so
        /// no other way runs behind its chooser.
        #[test]
        fn a_portal_slow_to_answer_counts_as_opened() {
            let Some((result, uri)) = ask("portal-late", Some(0), Duration::from_secs(2)) else { return };
            assert_eq!(uri, URL);
            assert_eq!(result, Ok(()));
            let Some((result, _)) = ask("portal-never", None, Duration::ZERO) else { return };
            assert_eq!(result, Ok(()));
        }
    }
}
