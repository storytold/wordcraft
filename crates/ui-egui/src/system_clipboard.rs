//! The system clipboard for the Paste commands (#584, #687, #833).
//!
//! egui's paste event only carries text, so the host reads the rest through [`crate::Services`]:
//! another program's formatted copy as HTML or RTF (Paste keeps its formatting, Merge Formatting
//! its emphasis), a picture for Paste Special, and the names of the formats there so a paste that
//! finds nothing usable says what the clipboard holds instead of "empty". Everything read here
//! comes from other programs: sizes are capped before it reaches the engine.

use serde_json::Value;
use wordcraft_engine::cmd::paste::{MAX_PASTE_BYTES, MAX_PICTURE_BYTES};

use crate::{WordApp, paste_picture};

/// The commands that paste the clipboard.
pub(crate) fn is_paste(id: &str) -> bool {
    matches!(id, "edit.paste" | "edit.pasteText" | "edit.pasteMerge" | "edit.pasteSpecial")
}

/// Most format names an error lists.
const MAX_NAMES: usize = 6;
/// Longest format name shown.
const MAX_NAME_CHARS: usize = 60;

/// Add what the system clipboard holds to a paste command's params: its text (the ribbon and
/// context-menu Paste send none), its HTML and RTF, and for Paste Special its picture. Params a
/// caller already gave are kept. `Some` when the paste was done here instead (copied picture
/// files, which reach us as their paths).
pub(crate) fn fill(app: &mut WordApp, id: &str, params: &mut Value) -> Option<Result<Value, String>> {
    if !is_paste(id) {
        return None;
    }
    let o = params.as_object_mut()?;
    if !o.contains_key("text")
        && let Some(text) = app.services.clipboard_text.as_ref().and_then(|f| f()).filter(|t| !t.is_empty())
    {
        // Picture files copied in a file manager reach us as their paths: paste the pictures,
        // as Ctrl+V does.
        if id == "edit.paste"
            && app.session.math.is_none()
            && paste_picture::names_picture_files(&text)
            && let Some(r) = paste_picture::paste(app)
        {
            return Some(r);
        }
        o.insert("text".into(), Value::String(text));
    } else if !o.contains_key("text")
        && let Some(r) = defer(app, id, params)
    {
        return Some(r);
    }
    let o = params.as_object_mut()?;
    // Keep Text Only needs nothing more; a caller that passed formats of its own (an agent, Paste
    // Special's OK) has them already.
    if id == "edit.pasteText" || ["html", "rtf", "picture"].iter().any(|k| o.contains_key(*k)) {
        return None;
    }
    let fits = |t: &String| !t.trim().is_empty() && t.len() <= MAX_PASTE_BYTES;
    if let Some(html) = app.services.clipboard_html.as_ref().and_then(|f| f()).filter(fits) {
        o.insert("html".into(), Value::String(html));
    }
    if let Some(rtf) = app.services.clipboard_rtf.as_ref().and_then(|f| f()).filter(fits) {
        o.insert("rtf".into(), Value::String(rtf));
    }
    if id == "edit.pasteSpecial"
        && let Some(png) = paste_picture::clipboard_png(app)
    {
        let data = wordcraft_engine::cmd::insert::base64_encode(&png);
        if data.len() <= MAX_PICTURE_BYTES {
            o.insert("picture".into(), Value::String(data));
        }
    }
    None
}

/// How long a deferred Paste waits for egui's text before saying there is none.
const PASTE_WAIT_SECS: f64 = 1.0;
/// Frames a deferred Paste lets go by before asking egui: the menu it came from closes first.
/// egui-winit drops a paste whose keyboard focus changed between the request and the text, and
/// a closing context menu changes it.
const PASTE_SETTLE_FRAMES: u8 = 2;

/// A menu or ribbon Paste waiting for the clipboard's text from egui (see [`defer`]).
pub(crate) struct PendingPaste {
    id: String,
    params: Value,
    /// egui's input time when it was asked, once it has been.
    since: Option<f64>,
    /// Frames left before egui is asked.
    settle: u8,
    /// The text egui read, claimed from the raw input ([`claim`]), pasted next frame.
    text: Option<String>,
}

/// When the host can't open the system clipboard at all (GNOME's Wayland has no data-control
/// protocol, and a Flatpak without X11 has no fallback, #1368), the menu and ribbon Paste found
/// no text and failed while Ctrl+V worked: Ctrl+V's text comes through egui's own Wayland
/// clipboard. Ask egui for a paste the same way and run the command when its text arrives
/// ([`deliver`]), once the menu it came from has closed ([`tick`]). Paste Special keeps its
/// dialog's own path.
fn defer(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "edit.pasteSpecial" || !app.services.clipboard_unreachable.as_ref().is_some_and(|f| f()) {
        return None;
    }
    let ctx = app.ctx.clone()?;
    log::debug!("{id}: the system clipboard can't be opened; reading the text through egui");
    app.pending_paste = Some(PendingPaste { id: id.to_string(), params: params.clone(), since: None, settle: PASTE_SETTLE_FRAMES, text: None });
    ctx.request_repaint();
    Some(Ok(serde_json::json!({"pending": "paste"})))
}

/// Take the paste event egui sent for the waiting Paste out of the frame's raw input, before the
/// page's own handling: the page only reads typing while it has the keyboard focus, which it
/// doesn't have yet as the menu closes, and with it the event would paste a second time. Only
/// a Paste that has asked egui claims one; any other paste event is Ctrl+V's own.
pub(crate) fn claim(app: &mut WordApp, events: &mut Vec<egui::Event>) {
    let Some(p) = app.pending_paste.as_mut().filter(|p| p.since.is_some() && p.text.is_none()) else {
        return;
    };
    if let Some(i) = events.iter().position(|e| matches!(e, egui::Event::Paste(_)))
        && let egui::Event::Paste(t) = events.remove(i)
    {
        p.text = Some(t);
    }
}

/// Run the waiting Paste with the text egui read. `false` when nothing was waiting.
fn deliver(app: &mut WordApp, text: &str) -> bool {
    let Some(PendingPaste { id, mut params, .. }) = app.pending_paste.take() else {
        return false;
    };
    log::debug!("{id}: egui read {} bytes of text", text.len());
    if let Some(o) = params.as_object_mut() {
        o.insert("text".into(), Value::String(text.to_string()));
    }
    let _ = app.run(&id, params);
    true
}

/// Each frame: paste the text egui read, else ask egui for it once the menu has closed, and if no
/// text came in time (the clipboard holds none, or only a picture, which egui's paste doesn't
/// carry), say so instead of waiting for ever.
pub(crate) fn tick(app: &mut WordApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    if let Some(text) = app.pending_paste.as_mut().and_then(|p| p.text.take()) {
        deliver(app, &text);
        return;
    }
    let Some(p) = app.pending_paste.as_mut() else { return };
    match p.since {
        None if p.settle > 0 => {
            p.settle -= 1;
            ctx.request_repaint();
        }
        None => {
            p.since = Some(now);
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(PASTE_WAIT_SECS + 0.1));
        }
        Some(since) if now - since > PASTE_WAIT_SECS => {
            log::debug!("{}: no text came from egui", p.id);
            app.pending_paste = None;
            app.status("the clipboard has no text to paste".to_string());
        }
        Some(_) => {}
    }
}

/// Put a copy on the system clipboard: its HTML beside the text (the formatting, #1523, and
/// equations as MathML, #816; a lone equation in the MathML formats too, #1297) when the host can
/// set it, else the text through egui. egui sets its text at the end of the frame, which would
/// replace the HTML, so only one of them runs.
pub(crate) fn put(app: &WordApp, ctx: &egui::Context, text: &str, html: &str, mathml: &str) {
    if text.is_empty() {
        // A copied picture or shape has no text, only HTML (#2276): empty the clipboard's text,
        // else Paste would find the text copied before it and paste that instead of our copy.
        if !html.is_empty() {
            ctx.copy_text(String::new());
        }
        return;
    }
    if !html.is_empty() && app.services.set_clipboard_html.as_ref().is_some_and(|f| f(html, text, mathml)) {
        return;
    }
    ctx.copy_text(text.to_string());
}

/// Put what Copy or Cut returned on the system clipboard (see [`put`]).
pub(crate) fn put_result(app: &WordApp, ctx: &egui::Context, r: &Value) {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    put(app, ctx, s("text"), s("html"), s("mathml"));
}

/// Format names worth showing: not the clipboard protocol's own bookkeeping targets.
fn shown(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && !matches!(n, "TARGETS" | "TIMESTAMP" | "MULTIPLE" | "SAVE_TARGETS" | "DELETE" | "INCR")
        && !n.starts_with("chromium/")
        && !n.starts_with("application/x-qt-")
}

/// What to say when a paste found nothing it can use: the formats on the clipboard when the
/// host lists them (#833), else that it is empty.
pub(crate) fn nothing_to_paste(app: &WordApp) -> String {
    let names: Vec<String> = app.services.clipboard_formats.as_ref().map(|f| f()).unwrap_or_default();
    let mut shown_names: Vec<String> = Vec::new();
    for n in names.iter().filter(|n| shown(n)) {
        let n: String = n.trim().chars().take(MAX_NAME_CHARS).collect();
        if !shown_names.contains(&n) {
            shown_names.push(n);
        }
    }
    if shown_names.is_empty() {
        return "the clipboard is empty".into();
    }
    let more = shown_names.len().saturating_sub(MAX_NAMES);
    shown_names.truncate(MAX_NAMES);
    let mut list = shown_names.join(", ");
    if more > 0 {
        list.push_str(&format!(" and {more} more"));
    }
    format!("nothing on the clipboard can be pasted here (it holds {list})")
}

/// A paste that found the clipboard empty, explained (#833): the status bar says what is there.
pub(crate) fn explain(app: &mut WordApp, id: &str, r: Result<Value, String>) -> Result<Value, String> {
    match r {
        Err(e) if is_paste(id) && e == "the clipboard is empty" => {
            let e = nothing_to_paste(app);
            app.status(e.clone());
            Err(e)
        }
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use crate::paste_picture::ClipboardPicture;
    use serde_json::json;
    use wordcraft_engine::Session;

    fn app(services: Services) -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), services)
    }

    fn first_para(a: &WordApp) -> String {
        let focus = a.session.sel.focus.clone();
        a.session.doc.para_at(&focus).map(|p| p.text.clone()).unwrap_or_default()
    }

    fn bold_at(a: &WordApp, off: usize) -> bool {
        let focus = a.session.sel.focus.clone();
        a.session.doc.para_at(&focus).and_then(|p| p.props_of_char(off).bold).unwrap_or(false)
    }

    /// Ctrl+V (a paste event with the text) and the ribbon's Paste keep another program's
    /// formatting from the clipboard's HTML (#687); Keep Text Only doesn't.
    #[test]
    fn paste_keeps_formatting_from_the_system_clipboard_html() {
        let services = Services {
            clipboard_text: Some(Box::new(|| Some("plain strong".into()))),
            clipboard_html: Some(Box::new(|| Some("<p>plain <b>strong</b></p>".into()))),
            ..Default::default()
        };
        let mut a = app(services);
        // Ctrl+V: the paste event carries the text.
        a.run("edit.paste", json!({"text": "plain strong"})).unwrap();
        assert_eq!(first_para(&a), "plain strong");
        assert!(!bold_at(&a, 0) && bold_at(&a, 7));
        a.run("edit.undo", json!({})).unwrap();
        // The ribbon's Paste sends nothing.
        a.run("edit.paste", json!({})).unwrap();
        assert!(bold_at(&a, 7));
        a.run("edit.undo", json!({})).unwrap();
        // Merge Formatting inserts it, keeping the bold.
        a.run("edit.pasteMerge", json!({})).unwrap();
        assert_eq!(first_para(&a), "plain strong");
        assert!(bold_at(&a, 7));
        a.run("edit.undo", json!({})).unwrap();
        a.run("edit.pasteText", json!({})).unwrap();
        assert!(!bold_at(&a, 7));
        // Paste Special lists what is on the system clipboard.
        let r = a.run("edit.pasteSpecial", json!({})).unwrap();
        let ids: Vec<&str> = r["formats"].as_array().unwrap().iter().filter_map(|f| f["id"].as_str()).collect();
        assert_eq!(ids, ["html", "text"]);
    }

    /// RTF is used where the clipboard has no HTML; oversized payloads are dropped, not pasted.
    #[test]
    fn rtf_and_oversized_clipboard_payloads() {
        let services = Services {
            clipboard_text: Some(Box::new(|| Some("bold".into()))),
            clipboard_html: Some(Box::new(|| Some(format!("<b>{}</b>", "x".repeat(MAX_PASTE_BYTES))))),
            clipboard_rtf: Some(Box::new(|| Some(r"{\rtf1\ansi {\b bold}}".into()))),
            ..Default::default()
        };
        let mut a = app(services);
        a.run("edit.paste", json!({})).unwrap();
        assert_eq!(first_para(&a), "bold");
        assert!(bold_at(&a, 0), "the RTF's bold, not the oversized HTML");
    }

    /// Paste Special lists a picture on the system clipboard and pastes it.
    #[test]
    fn paste_special_offers_the_clipboard_picture() {
        let services = Services {
            clipboard_picture: Some(Box::new(|| Some(ClipboardPicture::Pixels { width: 4, height: 4, rgba: vec![200; 64] }))),
            ..Default::default()
        };
        let mut a = app(services);
        let r = a.run("edit.pasteSpecial", json!({})).unwrap();
        let ids: Vec<&str> = r["formats"].as_array().unwrap().iter().filter_map(|f| f["id"].as_str()).collect();
        assert_eq!(ids, ["picture"]);
        // Choosing it in the dialog (or naming it) pastes the picture.
        let r = a.run("edit.pasteSpecial", json!({"as": "picture"})).unwrap();
        assert_eq!(r["pastedAs"], "picture");
        assert_eq!(a.session.doc.media.len(), 1);
    }

    /// A clipboard holding nothing WordCraft can paste says what it holds (#833); one that can't
    /// be read says why; a truly empty one says it is empty.
    #[test]
    fn nothing_to_paste_says_what_the_clipboard_holds() {
        let services = Services {
            clipboard_formats: Some(Box::new(|| vec!["TARGETS".into(), "image/x-unknown".into(), "text/x-moz-url".into(), "image/x-unknown".into()])),
            clipboard_picture: Some(Box::new(|| None)),
            ..Default::default()
        };
        let mut a = app(services);
        let e = a.run("edit.paste", json!({})).unwrap_err();
        assert_eq!(e, "nothing on the clipboard can be pasted here (it holds image/x-unknown, text/x-moz-url)");
        assert_eq!(a.run("edit.pasteMerge", json!({})).unwrap_err(), e);
        let many = Services { clipboard_formats: Some(Box::new(|| (0..20).map(|i| format!("f{i}")).collect())), ..Default::default() };
        assert!(nothing_to_paste(&app(many)).ends_with("f5 and 14 more)"));
        let mut a = app(Services::default());
        assert_eq!(a.run("edit.paste", json!({})).unwrap_err(), "the clipboard is empty");
        let unreadable = Services {
            clipboard_picture: Some(Box::new(|| Some(ClipboardPicture::Unreadable("its format isn't supported".into())))),
            ..Default::default()
        };
        let e = app(unreadable).run("edit.paste", json!({})).unwrap_err();
        assert!(e.contains("couldn't be read") && e.contains("isn't supported"), "{e}");
    }

    #[test]
    fn copying_an_equation_puts_its_html_on_the_system_clipboard() {
        // #816: the host gets HTML with the equation as MathML beside the text.
        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(String, String, String)>::new()));
        let sink = got.clone();
        let mut a = app(Services {
            set_clipboard_html: Some(Box::new(move |h, t, m| {
                sink.borrow_mut().push((h.to_string(), t.to_string(), m.to_string()));
                true
            })),
            ..Default::default()
        });
        a.run("document.setText", json!({"text": "Speed "})).unwrap();
        a.run("caret.docEnd", json!({})).unwrap();
        a.run("insert.equation", json!({"linear": "y=2x_s", "edit": false})).unwrap();
        a.run("select.all", json!({})).unwrap();
        let ctx = egui::Context::default();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&a, &ctx, &r);
        let got_now = got.borrow().clone();
        assert_eq!(got_now.len(), 1);
        assert!(got_now[0].0.contains("<math") && got_now[0].1.contains("y=2x_s"), "{got_now:?}");
        // With text around it, no MathML formats: Word would paste the equation alone (#1297).
        assert!(got_now[0].2.is_empty(), "{got_now:?}");
        // Text without an equation gets HTML too, so other programs keep its formatting (#1523).
        a.run("select.range", json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&a, &ctx, &r);
        let got_now = got.borrow().clone();
        assert_eq!(got_now.len(), 2);
        assert!(got_now[1].0.contains("Speed") && !got_now[1].0.contains("<math") && got_now[1].1 == "Speed", "{got_now:?}");
        assert!(got_now[1].2.is_empty(), "{got_now:?}");
        // The equation alone: its MathML for the platform's MathML formats too (#1297).
        a.run("select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 9}})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&a, &ctx, &r);
        let got_now = got.borrow().clone();
        assert_eq!(got_now.len(), 3);
        assert!(got_now[2].2.starts_with("<math") && got_now[2].2.contains("<msub>") && got_now[2].1 == "y=2x_s", "{got_now:?}");
    }

    /// A desktop whose system clipboard can't be opened (GNOME's Wayland in a Flatpak without
    /// X11, #1368): the host reads no text, as an empty clipboard would.
    fn unreachable() -> Services {
        Services { clipboard_text: Some(Box::new(|| None)), clipboard_unreachable: Some(Box::new(|| true)), ..Default::default() }
    }

    /// Whether a pass's output asks egui for a paste.
    fn asks_paste(out: &egui::FullOutput) -> bool {
        out.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste)))
    }

    /// Runs `id` (no text, as the context menu and ribbon send) inside an egui pass at `time`, as
    /// the app does: its result and whether egui was asked for a paste in that pass.
    fn menu_paste(a: &mut WordApp, ctx: &egui::Context, id: &str) -> (Result<Value, String>, bool) {
        let mut r = None;
        let out = ctx.run_pass(egui::RawInput { time: Some(0.0), ..Default::default() }, |ctx| {
            a.ctx = Some(ctx.clone());
            r = Some(a.run(id, json!({})));
        });
        let asked = asks_paste(&out);
        out.drop_without_applying_deltas();
        (r.unwrap(), asked)
    }

    /// One app frame at `time` with `events`, as `WordApp` runs it: the raw input hook claims, then
    /// the frame's logic ticks. Whether egui was asked for a paste, and the events left for the
    /// page.
    fn frame(a: &mut WordApp, ctx: &egui::Context, time: f64, mut events: Vec<egui::Event>) -> (bool, Vec<egui::Event>) {
        claim(a, &mut events);
        let left = events.clone();
        let out = ctx.run_pass(egui::RawInput { time: Some(time), events, ..Default::default() }, |ctx| tick(a, ctx));
        let asked = asks_paste(&out);
        out.drop_without_applying_deltas();
        (asked, left)
    }

    fn paste(t: &str) -> egui::Event {
        egui::Event::Paste(t.into())
    }

    /// The context menu's Paste with the system clipboard out of reach waits for the menu to
    /// close, asks egui for the text as Ctrl+V gets it, and pastes it once: the event is taken
    /// before the page sees it (the page isn't focused yet, and would paste it again). A Ctrl+V
    /// before egui is asked, and any paste event after, are left to the page.
    #[test]
    fn menu_paste_without_the_system_clipboard_reads_the_text_through_egui() {
        let mut a = app(unreachable());
        let ctx = egui::Context::default();
        let (r, asked) = menu_paste(&mut a, &ctx, "edit.paste");
        assert_eq!(r.unwrap(), json!({"pending": "paste"}));
        assert!(!asked, "not while the menu is closing");
        let (asked, left) = frame(&mut a, &ctx, 0.01, vec![paste("ctrl+v")]);
        assert!(!asked && left.len() == 1, "a paste before egui is asked is Ctrl+V's");
        assert!(!frame(&mut a, &ctx, 0.02, vec![]).0);
        assert!(frame(&mut a, &ctx, 0.03, vec![]).0, "asked once the menu has closed");
        assert_eq!(first_para(&a), "", "nothing is pasted before the text comes");
        let (_, left) = frame(&mut a, &ctx, 0.1, vec![paste("from another program")]);
        assert!(left.is_empty(), "the page never sees the claimed text");
        assert_eq!(first_para(&a), "from another program");
        assert!(a.pending_paste.is_none());
        let (_, left) = frame(&mut a, &ctx, 0.2, vec![paste("again")]);
        assert_eq!(left.len(), 1, "a later paste event is Ctrl+V's own");
        assert_eq!(first_para(&a), "from another program");
    }

    /// Keep Text Only and Merge Formatting wait the same way and run as themselves.
    #[test]
    fn the_waiting_paste_keeps_its_own_command() {
        for id in ["edit.pasteText", "edit.pasteMerge"] {
            let mut a = app(unreachable());
            let ctx = egui::Context::default();
            menu_paste(&mut a, &ctx, id).0.unwrap();
            for t in [0.01, 0.02, 0.03] {
                frame(&mut a, &ctx, t, vec![]);
            }
            frame(&mut a, &ctx, 0.1, vec![paste("plain")]);
            assert_eq!(first_para(&a), "plain", "{id}");
            assert_eq!(a.session.last_command.as_ref().map(|(c, _)| c.as_str()), Some(id), "{id} ran, not Paste");
        }
    }

    /// No text within a second of asking (the clipboard is empty, or holds only a picture): the
    /// wait ends with a message, and a paste event after that is left to the page.
    #[test]
    fn a_paste_that_gets_no_text_gives_up_with_a_message() {
        let mut a = app(unreachable());
        let ctx = egui::Context::default();
        menu_paste(&mut a, &ctx, "edit.paste").0.unwrap();
        for t in [0.01, 0.02, 0.03] {
            frame(&mut a, &ctx, t, vec![]);
        }
        frame(&mut a, &ctx, 0.5, vec![]);
        assert!(a.pending_paste.is_some(), "still waiting");
        frame(&mut a, &ctx, 0.03 + PASTE_WAIT_SECS + 0.1, vec![]);
        assert!(a.pending_paste.is_none());
        assert_eq!(a.status_msg.as_ref().map(|(m, _)| m.as_str()), Some("the clipboard has no text to paste"));
        let (_, left) = frame(&mut a, &ctx, 2.0, vec![paste("late")]);
        assert_eq!(left.len(), 1);
        assert_eq!(first_para(&a), "");
    }

    /// Only an unreachable clipboard waits: one that opens but is empty says so at once, a paste
    /// that brings its text (Ctrl+V, an agent) runs as before, Paste Special keeps its dialog's
    /// path, and with no window yet (the CLI, MCP) nothing waits.
    #[test]
    fn only_a_menu_paste_with_the_clipboard_out_of_reach_waits() {
        let ctx = egui::Context::default();
        let empty = Services { clipboard_text: Some(Box::new(|| None)), clipboard_unreachable: Some(Box::new(|| false)), ..Default::default() };
        let mut a = app(empty);
        assert!(menu_paste(&mut a, &ctx, "edit.paste").0.is_err());
        assert!(a.pending_paste.is_none());

        let mut a = app(unreachable());
        a.ctx = Some(ctx.clone());
        a.run("edit.paste", json!({"text": "typed"})).unwrap();
        assert_eq!(first_para(&a), "typed");
        assert!(a.pending_paste.is_none());
        let _ = menu_paste(&mut a, &ctx, "edit.pasteSpecial");
        assert!(a.pending_paste.is_none());

        let mut a = app(unreachable());
        assert!(a.run("edit.paste", json!({})).is_err());
        assert!(a.pending_paste.is_none());
    }
}
