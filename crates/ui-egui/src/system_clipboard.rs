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

/// Put a copy on the system clipboard: its HTML and RTF beside the text (the formatting, #1523,
/// and equations as MathML, #816) when the host can set them, else the text through egui. egui
/// sets its text at the end of the frame, which would replace the rest, so only one of them runs.
pub(crate) fn put(app: &WordApp, ctx: &egui::Context, text: &str, html: &str, rtf: &str) {
    if text.is_empty() {
        return;
    }
    if !(html.is_empty() && rtf.is_empty()) && app.services.set_clipboard_rich.as_ref().is_some_and(|f| f(text, html, rtf)) {
        return;
    }
    ctx.copy_text(text.to_string());
}

/// Put what Copy or Cut returned on the system clipboard (see [`put`]).
pub(crate) fn put_result(app: &WordApp, ctx: &egui::Context, r: &Value) {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    put(app, ctx, s("text"), s("html"), s("rtf"));
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
        // #816: the host gets HTML with the equation as MathML beside the text, and RTF.
        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(String, String, String)>::new()));
        let sink = got.clone();
        let mut a = app(Services {
            set_clipboard_rich: Some(Box::new(move |t, h, r| {
                sink.borrow_mut().push((h.to_string(), t.to_string(), r.to_string()));
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
        // Text without an equation gets HTML too, so other programs keep its formatting (#1523),
        // and RTF for the programs that read that (gap-clipboard-rtf).
        a.run("select.range", json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&a, &ctx, &r);
        let got_now = got.borrow().clone();
        assert_eq!(got_now.len(), 2);
        assert!(got_now[1].0.contains("Speed") && !got_now[1].0.contains("<math") && got_now[1].1 == "Speed", "{got_now:?}");
        assert!(got_now[1].2.starts_with("{\\rtf1") && got_now[1].2.contains("Speed"), "{got_now:?}");
    }
}
