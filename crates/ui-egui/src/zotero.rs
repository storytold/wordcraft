//! Zotero in the app: `ui.zotero.<command>` (`addEditCitation`, `addEditBibliography`, `addNote`,
//! `refresh`, `removeCodes`, `setDocPrefs`), `ui.zotero.status`, `ui.zotero.answer
//! {"button": n}` to answer a question Zotero is asking (agents; people click the dialog), and
//! `ui.zotero.cancel` to stop the command in flight.
//!
//! A command runs on a background thread that holds the socket to Zotero. Each call Zotero makes
//! is handed to the UI thread, answered against the session by [`wordcraft_zotero::Bridge`] in
//! [`poll`], and the answer goes back to the thread. Zotero's questions (`Document_displayAlert`)
//! become a dialog; the thread waits until the user answers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use serde_json::{Value, json};
use wordcraft_zotero::client::{MAX_METHOD_LEN, clip};
use wordcraft_zotero::{Bridge, Call, Command, Host};

use crate::WordApp;

/// A call from Zotero waiting for its answer.
struct Pending {
    call: Call,
    reply: Sender<Result<Value, String>>,
}

/// An alert Zotero is waiting on.
struct Alert {
    text: String,
    buttons: i64,
    reply: Sender<Result<Value, String>>,
}

/// The app's Zotero connection state.
#[derive(Default)]
pub struct ZoteroLink {
    /// Where Zotero listens (default `127.0.0.1:23116`).
    pub options: wordcraft_zotero::client::Options,
    bridge: Bridge,
    calls: Option<Receiver<Pending>>,
    done: Option<Receiver<Result<wordcraft_zotero::Outcome, String>>>,
    alert: Option<Alert>,
    /// The command in flight.
    pub busy: Option<Command>,
    /// The document the command in flight started in; its calls are refused once another is open.
    document: u64,
    /// How the last command ended.
    pub last: Option<Value>,
    /// Zotero's calls in the command in flight (logged when it ends): the first [`MAX_METHODS`].
    methods: Vec<String>,
    /// Calls past the first [`MAX_METHODS`].
    more_methods: usize,
    /// Set to make the connection thread hang up.
    cancel: Option<Arc<AtomicBool>>,
}

/// Calls named in the log line of one command.
const MAX_METHODS: usize = 200;
/// Longest question from Zotero shown, in bytes.
const MAX_ALERT_LEN: usize = 2000;

impl ZoteroLink {
    /// Remember a call for the log.
    fn note(&mut self, method: &str) {
        if self.methods.len() < MAX_METHODS {
            // The name comes from Zotero's side of the socket: keep a short piece of it.
            self.methods.push(clip(method, MAX_METHOD_LEN).into_owned());
        } else {
            self.more_methods = self.more_methods.saturating_add(1);
        }
    }

    /// Forget the command in flight: the thread's waits end (their senders drop) and it hangs up.
    fn clear(&mut self) -> Option<Command> {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.calls = None;
        self.done = None;
        self.alert = None;
        self.busy.take()
    }
}

/// Focus requests from the bridge.
struct AppHost<'a> {
    ctx: &'a egui::Context,
}

impl Host for AppHost<'_> {
    fn alert(&mut self, text: &str, _icon: i64, _buttons: i64) -> i64 {
        // Alerts are intercepted before the bridge (they wait for the user); this is unreachable
        // in practice and answers like Cancel.
        log::warn!("Zotero alert not shown: {text}");
        0
    }
    fn activate(&mut self) {
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}

/// Handle `ui.zotero.*`. `None` when `id` isn't one.
pub fn command(app: &mut WordApp, id: &str, params: &Value, ctx: Option<&egui::Context>) -> Option<Result<Value, String>> {
    let name = id.strip_prefix("ui.zotero.")?;
    if name == "status" {
        return Some(Ok(status(app)));
    }
    if name == "answer" {
        let Some(button) = params.get("button").and_then(Value::as_i64) else {
            return Some(Err("give `button`: Zotero's number for it (OK/Yes 1, No/Cancel 0; with Yes/No/Cancel: Yes 2, No 1, Cancel 0)".into()));
        };
        return Some(answer(app, button));
    }
    if name == "cancel" {
        return Some(cancel(app));
    }
    let Some(cmd) = Command::from_name(name) else {
        let names: Vec<&str> = Command::ALL.iter().map(|c| c.wire_name()).collect();
        return Some(Err(format!("unknown Zotero command `{name}`; use one of {}", names.join(", "))));
    };
    Some(start(app, cmd, ctx.cloned()))
}

fn status(app: &WordApp) -> Value {
    let z = &app.zotero;
    json!({"busy": z.busy.map(Command::wire_name), "waitingForAnswer": z.alert.as_ref().map(|a| a.text.clone()), "last": z.last})
}

#[cfg(target_arch = "wasm32")]
fn start(_app: &mut WordApp, _cmd: Command, _ctx: Option<egui::Context>) -> Result<Value, String> {
    Err("Zotero works with the WordCraft desktop app".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn start(app: &mut WordApp, cmd: Command, ctx: Option<egui::Context>) -> Result<Value, String> {
    use std::sync::mpsc::channel;
    if let Some(b) = app.zotero.busy {
        return Err(format!("Zotero is still working on {}; finish in Zotero's window first", b.wire_name()));
    }
    let document = app.session.document_id();
    let (call_tx, call_rx) = channel::<Pending>();
    let (done_tx, done_rx) = channel();
    let wake = ctx.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let mut opts = app.zotero.options.clone();
    opts.cancel = Some(stop.clone());
    // The whole command, dialogs included, ends with the client's session limit.
    let started = std::time::Instant::now();
    let session_limit = opts.session_timeout;
    let spawned = std::thread::Builder::new().name("zotero".into()).spawn(move || {
        let r = wordcraft_zotero::client::run_command(&opts, cmd, &mut |call| {
            let (tx, rx) = channel();
            if call_tx.send(Pending { call: call.clone(), reply: tx }).is_err() {
                return Err("WordCraft closed the document".into());
            }
            if let Some(c) = &wake {
                c.request_repaint();
            }
            match rx.recv_timeout(session_limit.saturating_sub(started.elapsed())) {
                Ok(answer) => answer,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err("the command ran out of time waiting for an answer in WordCraft".into()),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("WordCraft closed the document".into()),
            }
        });
        let _ = done_tx.send(r.map_err(|e| e.to_string()));
        if let Some(c) = &wake {
            c.request_repaint();
        }
    });
    if let Err(e) = spawned {
        return Err(format!("could not start the Zotero connection: {e}"));
    }
    let z = &mut app.zotero;
    z.calls = Some(call_rx);
    z.done = Some(done_rx);
    z.busy = Some(cmd);
    z.cancel = Some(stop);
    z.document = document;
    z.methods.clear();
    z.more_methods = 0;
    // The status bar says "Waiting for Zotero…" with a Cancel link while it runs.
    log::info!("Zotero: {} started", cmd.wire_name());
    Ok(json!({"started": cmd.wire_name()}))
}

/// Stop the command in flight.
fn cancel(app: &mut WordApp) -> Result<Value, String> {
    let Some(cmd) = app.zotero.clear() else { return Err("Zotero isn't working on anything".into()) };
    let name = cmd.wire_name();
    log::info!("Zotero: {name} cancelled in WordCraft");
    app.zotero.methods.clear();
    app.zotero.last = Some(json!({"command": name, "cancelled": true}));
    app.status(tl!("Zotero: cancelled"));
    app.canvas.want_focus = true;
    Ok(json!({"cancelled": name}))
}

/// Answer the question Zotero is waiting on with `button` (Zotero's numbering).
pub fn answer(app: &mut WordApp, button: i64) -> Result<Value, String> {
    let Some(a) = app.zotero.alert.take() else { return Err("Zotero isn't asking anything".into()) };
    let max = match a.buttons {
        3 => 2,
        _ => 1,
    };
    if !(0..=max).contains(&button) {
        let text = a.text.clone();
        app.zotero.alert = Some(a);
        return Err(format!("button must be 0–{max} for this question: {text}"));
    }
    let _ = a.reply.send(Ok(json!(button)));
    if let Some(c) = &app.ctx {
        c.request_repaint();
    }
    Ok(json!({"answered": button}))
}

/// Answer Zotero's calls (each frame).
pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    if app.zotero.busy.is_none() {
        return;
    }
    let mut changed = false;
    // One alert at a time: Zotero sends nothing else while it waits for the answer.
    while app.zotero.alert.is_none() {
        let Some(p) = app.zotero.calls.as_ref().and_then(|rx| rx.try_recv().ok()) else { break };
        if p.call.method == "Document_displayAlert" && app.session.document_id() == app.zotero.document {
            let text = clip(p.call.args.get(1).and_then(Value::as_str).unwrap_or(""), MAX_ALERT_LEN).into_owned();
            let buttons = p.call.args.get(3).and_then(Value::as_i64).unwrap_or(0);
            app.zotero.note(&p.call.method);
            app.zotero.alert = Some(Alert { text, buttons, reply: p.reply });
            continue;
        }
        app.zotero.note(&p.call.method);
        if app.session.document_id() != app.zotero.document {
            // New or Open replaced the document this command started in: its edits must not land in the new one.
            let _ = p.reply.send(Err("WordCraft replaced the document this command started in".into()));
            continue;
        }
        let rev = app.session.rev();
        let mut host = AppHost { ctx };
        let r = app.zotero.bridge.handle(&mut app.session, &mut host, &p.call);
        changed |= app.session.rev() != rev || p.call.method == "Field_select";
        let _ = p.reply.send(r);
    }
    if changed {
        app.canvas.scroll_to_caret = true;
        app.canvas.caret_visible_since = crate::now_ms();
    }
    let done = match app.zotero.done.as_ref().map(Receiver::try_recv) {
        Some(Ok(r)) => Some(r),
        // The thread ended without a word: still finish, so the buttons work again.
        Some(Err(TryRecvError::Disconnected)) | None => Some(Err("the connection to Zotero ended unexpectedly".to_string())),
        Some(Err(TryRecvError::Empty)) => None,
    };
    if let Some(r) = done {
        let cmd = app.zotero.clear();
        let name = cmd.map(Command::wire_name).unwrap_or("");
        let mut methods = std::mem::take(&mut app.zotero.methods).join(", ");
        let more = std::mem::take(&mut app.zotero.more_methods);
        if more > 0 {
            methods.push_str(&format!(" and {more} more"));
        }
        match &r {
            Ok(o) => log::info!(
                "Zotero: {name} {} after {} calls ({methods}){}{}",
                if o.completed { "completed" } else { "ended without completing" },
                o.calls,
                if o.errors.is_empty() { String::new() } else { format!("; errors: {}", o.errors.join(" | ")) },
                if o.more_errors == 0 { String::new() } else { format!(" and {} more", o.more_errors) }
            ),
            Err(e) => log::warn!("Zotero: {name} failed: {e}"),
        }
        match r {
            Ok(o) => {
                app.zotero.last =
                    Some(json!({"command": name, "completed": o.completed, "calls": o.calls, "errors": o.errors, "moreErrors": o.more_errors}));
                app.status(if o.completed { tl!("Zotero: done") } else { tl!("Zotero: cancelled") });
            }
            Err(e) => {
                app.zotero.last = Some(json!({"command": name, "error": e}));
                app.status(e);
            }
        }
        app.canvas.want_focus = true;
    }
}

/// Zotero's question, as a dialog.
pub fn show_alert(app: &mut WordApp, ctx: &egui::Context) {
    let Some(a) = app.zotero.alert.as_ref() else { return };
    // Buttons and the value Zotero expects for each.
    // macOS puts the default button last (rightmost), with Cancel just before it.
    let buttons: &[(&str, i64)] = match (a.buttons, cfg!(target_os = "macos")) {
        (1, false) => &[("OK", 1), ("Cancel", 0)],
        (1, true) => &[("Cancel", 0), ("OK", 1)],
        (2, false) => &[("Yes", 1), ("No", 0)],
        (2, true) => &[("No", 0), ("Yes", 1)],
        (3, false) => &[("Yes", 2), ("No", 1), ("Cancel", 0)],
        (3, true) => &[("No", 1), ("Cancel", 0), ("Yes", 2)],
        _ => &[("OK", 1)],
    };
    let mut picked = None;
    egui::Window::new("Zotero")
        .id(egui::Id::new("zotero-alert"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.label(a.text.trim_end());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (label, v) in buttons {
                    if ui.button(tl!(label)).clicked() {
                        picked = Some(*v);
                    }
                }
            });
        });
    if let Some(v) = picked {
        let _ = answer(app, v);
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use serde_json::{Value, json};
    use wordcraft_doc::{Document, StoryRef};
    use wordcraft_engine::Session;
    use wordcraft_zotero::wire;

    use crate::{Services, WordApp};

    /// A fake Zotero making `calls`; returns the replies.
    fn fake_zotero(calls: Vec<Value>) -> (std::net::SocketAddr, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let _command = wire::read_frame(&mut s).unwrap().unwrap();
            let mut replies = Vec::new();
            for (i, c) in calls.into_iter().enumerate() {
                wire::write_frame(&mut s, 1 + i as u32, &serde_json::to_vec(&c).unwrap()).unwrap();
                replies.push(String::from_utf8(wire::read_frame(&mut s).unwrap().unwrap().payload).unwrap());
            }
            replies
        });
        (addr, h)
    }

    /// Run frames until `done` or a few seconds pass.
    fn pump(app: &mut WordApp, ctx: &egui::Context, done: impl Fn(&WordApp) -> bool) {
        for _ in 0..500 {
            super::poll(app, ctx);
            if done(app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out");
    }

    #[test]
    fn a_zotero_session_runs_through_the_app_with_a_question() {
        let (addr, server) = fake_zotero(vec![
            json!(["Application_getActiveDocument", [3]]),
            json!(["Document_insertField", [1, "ReferenceMark", 0]]),
            json!(["Field_setCode", [1, 1, "ITEM CSL_CITATION {}"]]),
            json!(["Document_displayAlert", [1, "Keep going?", 1, 2]]),
            json!(["Field_setText", [1, 1, "{\\rtf (Doe, {\\i 2020})}", true]]),
            json!(["Document_complete", [1]]),
        ]);
        let mut app = WordApp::new(Session::new(Document::from_text("Cite: ")), Services::default());
        app.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 6));
        app.zotero.options.addr = addr;
        let ctx = egui::Context::default();
        app.ctx = Some(ctx.clone());
        assert_eq!(app.run("ui.zotero.addEditCitation", json!({})).unwrap(), json!({"started": "addEditCitation"}));
        assert!(app.run("ui.zotero.refresh", json!({})).is_err(), "one session at a time");
        pump(&mut app, &ctx, |a| a.zotero.alert.is_some());
        let st = app.run("ui.zotero.status", json!({})).unwrap();
        assert_eq!(st["waitingForAnswer"], "Keep going?");
        assert!(app.run("ui.zotero.answer", json!({"button": 7})).is_err());
        assert_eq!(app.run("ui.zotero.answer", json!({"button": 1})).unwrap(), json!({"answered": 1}));
        pump(&mut app, &ctx, |a| a.zotero.busy.is_none());
        let replies = server.join().unwrap();
        assert_eq!(replies[3], "1");
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), "Cite: (Doe, 2020)");
        assert_eq!(app.run("ui.zotero.status", json!({})).unwrap()["last"]["completed"], true);
        assert!(app.run("ui.zotero.answer", json!({"button": 1})).is_err());
        assert!(app.run("ui.zotero.nope", json!({})).is_err());
        // One undo step.
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), "Cite: ");
    }

    #[test]
    fn a_pending_citation_does_not_insert_into_a_replacement_document() {
        let (addr, server) = fake_zotero(vec![
            json!(["Application_getActiveDocument", [3]]),
            json!(["Document_insertField", [1, "ReferenceMark", 0]]),
            json!(["Field_setCode", [1, 1, "ITEM CSL_CITATION {}"]]),
            json!(["Field_setText", [1, 1, "(Sample, 2020)", false]]),
            json!(["Document_complete", [1]]),
        ]);
        let mut app = WordApp::new(Session::new(Document::new()), Services::default());
        app.zotero.options.addr = addr;
        let ctx = egui::Context::default();
        app.ctx = Some(ctx.clone());
        app.run("ui.zotero.addEditCitation", json!({})).unwrap();
        app.run("file.new", json!({"template": "blank"})).unwrap();
        pump(&mut app, &ctx, |a| a.zotero.busy.is_none());
        server.join().unwrap();
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), "");
        assert!(!app.session.can_undo(), "the old command left no undo step in the new document");
    }

    #[test]
    fn a_transport_error_clears_busy() {
        // A peer that starts a message and never finishes it.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            use std::io::Write;
            let (mut s, _) = l.accept().unwrap();
            let _command = wire::read_frame(&mut s).unwrap().unwrap();
            s.write_all(&[0, 0, 0, 1, 0, 0, 0, 64, b'[']).unwrap();
            // Hold the socket until WordCraft hangs up.
            assert!(!matches!(wire::read_frame(&mut s), Ok(Some(_))));
        });
        let mut app = WordApp::new(Session::new(Document::new()), Services::default());
        app.zotero.options.addr = addr;
        app.zotero.options.frame_timeout = std::time::Duration::from_millis(200);
        let ctx = egui::Context::default();
        app.run("ui.zotero.refresh", json!({})).unwrap();
        pump(&mut app, &ctx, |a| a.zotero.busy.is_none());
        peer.join().unwrap();
        let last = app.run("ui.zotero.status", json!({})).unwrap()["last"].clone();
        assert!(last["error"].as_str().unwrap().contains("part of a message"), "{last}");
        assert!(crate::widgets::enabled(&app, "ui.zotero.refresh"), "the buttons work again");
    }

    #[test]
    fn cancel_clears_busy_and_hangs_up() {
        // A peer that never answers.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            // The command (unless the cancel came first), then nothing until WordCraft hangs up.
            while let Ok(Some(_)) = wire::read_frame(&mut s) {}
        });
        let mut app = WordApp::new(Session::new(Document::new()), Services::default());
        app.zotero.options.addr = addr;
        assert!(app.run("ui.zotero.cancel", json!({})).is_err(), "nothing to cancel");
        app.run("ui.zotero.refresh", json!({})).unwrap();
        assert!(crate::widgets::enabled(&app, "ui.zotero.cancel"));
        assert_eq!(app.run("ui.zotero.cancel", json!({})).unwrap(), json!({"cancelled": "refresh"}));
        assert!(app.zotero.busy.is_none());
        assert_eq!(app.run("ui.zotero.status", json!({})).unwrap()["last"]["cancelled"], true);
        // The connection thread notices and hangs up.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(peer.join().is_ok()));
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(true), "the socket was closed");
    }

    #[test]
    fn a_thread_that_ends_without_reporting_still_clears_busy() {
        let mut app = WordApp::new(Session::new(Document::new()), Services::default());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        app.zotero.busy = Some(wordcraft_zotero::Command::Refresh);
        app.zotero.done = Some(done_rx);
        drop(done_tx);
        super::poll(&mut app, &egui::Context::default());
        assert!(app.zotero.busy.is_none(), "the buttons work again");
        let last = app.run("ui.zotero.status", json!({})).unwrap()["last"].clone();
        assert!(last["error"].as_str().unwrap().contains("ended unexpectedly"), "{last}");
    }

    #[test]
    fn the_call_history_is_bounded_in_count_and_bytes() {
        let mut z = super::ZoteroLink::default();
        let huge = "é".repeat(1 << 16);
        for _ in 0..250 {
            z.note(&huge);
        }
        assert_eq!(z.methods.len(), super::MAX_METHODS);
        assert_eq!(z.more_methods, 250 - super::MAX_METHODS);
        let bytes: usize = z.methods.iter().map(String::len).sum();
        assert!(bytes <= super::MAX_METHODS * (wordcraft_zotero::client::MAX_METHOD_LEN + 3), "{bytes} bytes kept");
    }

    #[test]
    fn the_status_bar_cancels_a_running_command() {
        use egui_kittest::kittest::Queryable;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            while let Ok(Some(_)) = wire::read_frame(&mut s) {}
        });
        let mut app = WordApp::new(Session::new(Document::new()), Services::default());
        app.zotero.options.addr = addr;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Waiting for Zotero…").is_none(), "nothing to cancel, no link");
        h.state_mut().run("ui.zotero.refresh", json!({})).unwrap();
        h.run_steps(4);
        assert!(h.query_by_label("Waiting for Zotero…").is_some());
        h.get_by_label("Cancel").click();
        h.run_steps(4);
        assert!(h.state().zotero.busy.is_none(), "the link cancelled the command");
        assert_eq!(h.state_mut().run("ui.zotero.status", json!({})).unwrap()["last"]["cancelled"], true);
        assert!(h.query_by_label("Cancel").is_none());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(peer.join().is_ok()));
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(true), "the socket was closed");
        // With no document open (closed mid-command), Cancel still works.
        assert!(crate::works_without_document("ui.zotero.cancel"));
    }

    #[test]
    fn no_zotero_is_a_clear_error() {
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let mut app = WordApp::new(Session::new(Document::new()), Services::default());
        app.zotero.options.addr = free;
        let ctx = egui::Context::default();
        app.run("ui.zotero.refresh", json!({})).unwrap();
        pump(&mut app, &ctx, |a| a.zotero.busy.is_none());
        let last = app.run("ui.zotero.status", json!({})).unwrap()["last"].clone();
        assert!(last["error"].as_str().unwrap().contains("Zotero isn't running"), "{last}");
    }
}
