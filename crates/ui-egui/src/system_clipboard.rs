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
    }
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

/// Put a copy on the system clipboard: its HTML beside the text (the formatting, #1523, and
/// equations as MathML, #816; a lone equation in the MathML formats too, #1297) when the host can
/// set it, else the text through egui. egui sets its text at the end of the frame, which would
/// replace the HTML, so only one of them runs.
///
/// The text also goes on the primary selection where the platform has one (Linux), and only with
/// "Use the middle mouse button to paste instead of autoscroll" on (#1934): a middle click then
/// pastes a copy from here, as it can from a browser. Without the option WordCraft never reads it,
/// so it leaves the mouse selection other programs keep to them. It is plain text either way — the
/// primary selection carries no formatting — and the ordinary clipboard is set whether or not this
/// succeeds.
pub(crate) fn put(app: &WordApp, ctx: &egui::Context, text: &str, html: &str, mathml: &str) {
    if text.is_empty() {
        // A copied picture or shape has no text, only HTML (#2276): empty the clipboard's text,
        // else Paste would find the text copied before it and paste that instead of our copy.
        if !html.is_empty() {
            ctx.copy_text(String::new());
        }
        return;
    }
    if app.session.prefs.middle_click_paste {
        // A middle click in another program pastes this copy then, as a browser's does (#1934).
        let _ = app.services.set_primary_text.as_ref().is_some_and(|f| f(text));
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

/// The document's selection last put on the primary selection (#1934): the range it covered, so
/// the same selection isn't put there again, and the text that was put there.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SharedSelection {
    /// The selection's range, as the session orders it.
    pub range: (wordcraft_doc::Pos, wordcraft_doc::Pos),
    /// The text that was put on the primary selection.
    pub text: String,
}

/// Put the document's selection on the primary selection (#1934).
///
/// The primary selection is the mouse selection programs share on Linux, and it is what a middle
/// click pastes. The freedesktop clipboard spec makes it "the current selection": selecting text
/// here puts it there, so a middle click in another program pastes what is selected here, as it
/// does with a browser's selection. A copy puts it there too (see [`put`]), but that is not
/// enough: a selection made without a copy is the current selection all the same.
///
/// Only with "Use the middle mouse button to paste instead of autoscroll" on: without it
/// WordCraft never reads the primary selection, so it leaves the mouse selection to the programs
/// that share it. The text is plain (a selection carries no formatting) and bounded by
/// [`MAX_PASTE_BYTES`], as a paste from another program is.
///
/// A selection that is still moving isn't published: while a button is down it changes with the
/// pointer, and it settles when the pointer comes up, which is when it is published. The same
/// selection isn't published twice, and a selection too large to paste isn't published at all.
pub(crate) fn share_selection(app: &mut WordApp, ctx: &egui::Context) {
    if !app.session.prefs.middle_click_paste {
        return;
    }
    let Some(set) = &app.services.set_primary_text else {
        return;
    };
    if ctx.input(|i| i.pointer.any_down()) {
        // The selection is still moving.
        return;
    }
    let range = app.session.sel.ordered();
    if app.shared_selection.as_ref().is_some_and(|s| s.range == range) {
        return;
    }
    let text = app.session.selected_text();
    if text.is_empty() || text.len() > MAX_PASTE_BYTES {
        return;
    }
    let _ = set(&text);
    app.shared_selection = Some(SharedSelection { range, text });
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

    /// #1934: a copy also reaches the primary selection, so a middle click pastes what was
    /// copied here — here or in another program, as it does from a browser — rather than only
    /// what another program left there. It is plain text: the primary selection carries no
    /// formatting.
    #[test]
    fn copying_also_puts_the_text_on_the_primary_selection() {
        // #1934: a middle click pastes the primary selection, so a copy here has to land on it
        // too — as it does from a browser — or copying inside WordCraft and pasting with the
        // middle button would reach only what another program left there.
        let primary = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut a = app(Services {
            set_primary_text: Some(Box::new({
                let s = primary.clone();
                move |t: &str| {
                    s.borrow_mut().push(t.to_string());
                    true
                }
            })),
            ..Default::default()
        });
        a.session.prefs.middle_click_paste = true;
        a.run("document.setText", json!({"text": "Studio"})).unwrap();
        let ctx = egui::Context::default();
        a.run("select.all", json!({})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&a, &ctx, &r);
        assert_eq!(primary.borrow().as_slice(), ["Studio"], "the copy reached the primary selection");

        // It goes there whatever the ordinary clipboard does, HTML or not.
        let mut a = app(Services {
            set_primary_text: Some(Box::new({
                let s = primary.clone();
                move |t: &str| {
                    s.borrow_mut().push(t.to_string());
                    false
                }
            })),
            set_clipboard_html: Some(Box::new(|_, _, _| true)),
            ..Default::default()
        });
        a.session.prefs.middle_click_paste = true;
        a.run("document.setText", json!({"text": "Studio"})).unwrap();
        let r = a.run("select.all", json!({})).and_then(|_| a.run("edit.copy", json!({}))).unwrap();
        put_result(&a, &ctx, &r);
        assert_eq!(primary.borrow().last().map(String::as_str), Some("Studio"), "set even when the clipboard write failed");

        // With the option off a copy leaves the primary selection alone, so the mouse selection
        // other programs keep stays theirs.
        let mut a = app(Services {
            set_primary_text: Some(Box::new({
                let s = primary.clone();
                move |t: &str| {
                    s.borrow_mut().push(t.to_string());
                    true
                }
            })),
            ..Default::default()
        });
        a.run("document.setText", json!({"text": "Studio"})).unwrap();
        a.run("select.all", json!({})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&a, &ctx, &r);
        assert_eq!(primary.borrow().len(), 2, "with the option off the copy left the primary selection alone");
    }

    /// #1934: the document's selection goes on the primary selection, so a middle click in
    /// another program pastes what is selected here — no copy needed, as the freedesktop
    /// clipboard spec asks ("the current selection"). With the option off the mouse selection
    /// other programs keep stays theirs.
    #[test]
    fn a_selection_goes_on_the_primary_selection_without_a_copy() {
        let primary = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut a = app(Services {
            set_primary_text: Some(Box::new({
                let s = primary.clone();
                move |t: &str| {
                    s.borrow_mut().push(t.to_string());
                    true
                }
            })),
            ..Default::default()
        });
        a.session.prefs.middle_click_paste = true;
        a.run("document.setText", json!({"text": "Studio of text"})).unwrap();
        let ctx = egui::Context::default();

        // No selection yet: nothing to share.
        share_selection(&mut a, &ctx);
        assert!(primary.borrow().is_empty(), "no selection, nothing on the primary selection");

        // A selection, and no copy: it goes on the primary selection all the same.
        a.session.run("select.range", &json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 6}})).unwrap();
        share_selection(&mut a, &ctx);
        assert_eq!(primary.borrow().as_slice(), ["Studio"], "the selection is on the primary selection");

        // The same selection isn't put there again.
        share_selection(&mut a, &ctx);
        assert_eq!(primary.borrow().len(), 1, "the same selection isn't put there twice");

        // A different selection replaces it.
        a.session.run("select.range", &json!({"anchor": {"block": 0, "off": 7}, "focus": {"block": 0, "off": 9}})).unwrap();
        share_selection(&mut a, &ctx);
        assert_eq!(primary.borrow().last().map(String::as_str), Some("of"), "the new selection is on it");

        // A selection too large to paste isn't put there.
        a.session.run("select.all", &json!({})).unwrap();
        let big = "x".repeat(MAX_PASTE_BYTES + 1);
        a.run("document.setText", json!({"text": big})).unwrap();
        a.session.run("select.all", &json!({})).unwrap();
        share_selection(&mut a, &ctx);
        assert_eq!(primary.borrow().last().map(String::as_str), Some("of"), "an oversized selection isn't put there");

        // With the option off the mouse selection other programs keep stays theirs.
        let mut b = app(Services {
            set_primary_text: Some(Box::new({
                let s = primary.clone();
                move |t: &str| {
                    s.borrow_mut().push(t.to_string());
                    true
                }
            })),
            ..Default::default()
        });
        b.run("document.setText", json!({"text": "Studio of text"})).unwrap();
        b.session.run("select.all", &json!({})).unwrap();
        share_selection(&mut b, &ctx);
        assert_eq!(primary.borrow().len(), 2, "with the option off the selection isn't put there");
    }

    /// #816: a copy of an equation gives the host HTML with the equation as MathML beside the
    /// text, and a copy of anything else gives it HTML too, so other programs keep the
    /// formatting (#1523).
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
}
