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

/// Straight RGBA8 pixels for a native image copy. Decode limits are applied before allocation.
#[derive(Clone, Debug)]
pub struct CopyImage {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Raw uncompressed RGBA pixel bytes (4 bytes per pixel).
    pub rgba: Vec<u8>,
}

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
    // A caller that captured ownership already (Paste Special's dialog) keeps that value.
    if app.services.clipboard_is_own.is_some() && !o.contains_key("privateClipboard") {
        let own = app.clipboard_published && app.services.clipboard_is_own.as_ref().is_some_and(|f| f());
        o.insert("privateClipboard".into(), Value::Bool(own));
    }
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
    // Native bitmap/image formats are for other programs. Our own paste keeps the original
    // fragment (size, cropping and floating-picture properties), while its owner mark lasts.
    if app.session.clipboard.is_some() && o.get("privateClipboard").and_then(Value::as_bool) == Some(true) {
        o.insert("text".into(), Value::String(app.session.clipboard_text.clone()));
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
/// and equations as MathML, #816; a lone equation in the MathML formats too, #1297) when the host
/// can set them, else the text through egui. egui sets its text at the end of the frame, which
/// would replace the rest, so only one of them runs.
pub(crate) fn put(app: &WordApp, ctx: &egui::Context, text: &str, html: &str, rtf: &str, mathml: &str) -> bool {
    if text.is_empty() {
        // A copied picture or shape has no text, only HTML (#2276): empty the clipboard's text,
        // else Paste would find the text copied before it and paste that instead of our copy.
        if !html.is_empty() {
            if !(html.is_empty() && rtf.is_empty()) && app.services.set_clipboard_rich.as_ref().is_some_and(|f| f(text, html, rtf, mathml)) {
                return true;
            }
            ctx.copy_text(String::new());
        }
        return false;
    }
    if !(html.is_empty() && rtf.is_empty()) && app.services.set_clipboard_rich.as_ref().is_some_and(|f| f(text, html, rtf, mathml)) {
        return true;
    }
    ctx.copy_text(text.to_string());
    false
}

/// Return whether the command result `r` contains any copied payload (text, HTML, RTF, or MathML).
fn has_copied_content(r: &Value) -> bool {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    !s("text").is_empty() || !s("html").is_empty() || !s("rtf").is_empty() || !s("mathml").is_empty()
}

/// Put what Copy or Cut returned on the system clipboard (see [`put`]).
///
/// When `r` contains no copied content (e.g. Copy on a collapsed selection),
/// this does nothing: it avoids overwriting external clipboard contents with
/// previously-copied fragments or modifying the published ownership state.
pub(crate) fn put_result(app: &mut WordApp, ctx: &egui::Context, r: &Value) {
    if !has_copied_content(r) {
        return;
    }
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    app.clipboard_published = false;
    if app.services.set_clipboard_image.is_some()
        && let Some(res) = copied_image(app)
    {
        match res {
            Ok(image) => {
                let set = app.services.set_clipboard_image.as_ref().map(|f| f(&image, s("html"), s("text"))).unwrap_or(false);
                if set {
                    app.clipboard_published = true;
                    return;
                }
            }
            Err(e) => {
                app.status(format!("Couldn't copy the picture: {e}"));
            }
        }
    }
    app.clipboard_published = put(app, ctx, s("text"), s("html"), s("rtf"), s("mathml"));
}

/// Extract the single copied image from the clipboard fragment, if that is all that was copied.
fn copied_image(app: &WordApp) -> Option<Result<CopyImage, String>> {
    use wordcraft_doc::{Block, para::InlineObject};
    let fragment = app.session.clipboard.as_ref()?;
    let [Block::Para(para)] = fragment.blocks.as_slice() else { return None };
    let [InlineObject::Image { media, .. }] = para.objects.as_slice() else { return None };
    if para.text != wordcraft_doc::para::OBJ.to_string() {
        return None;
    }
    let bytes = fragment.media.get(media).or_else(|| app.session.doc.media.get(media))?;
    Some(match wordcraft_render::metafile_size(bytes) {
        Some((w, h, _)) => {
            let rect = wordcraft_geom::Rect::new(0.0, 0.0, w, h);
            let items = [wordcraft_layout::display::Draw::Image {
                rect,
                media: media.clone(),
                crop: [0.0; 4],
                alpha: 1.0,
                alt: String::new(),
                fx: Default::default(),
                decorative: false,
            }];
            let doc = fragment.to_document();
            let image = wordcraft_render::render_items(&doc, &items, rect, 96.0 / 72.0, &Default::default());
            Ok(CopyImage { width: image.width as usize, height: image.height as usize, rgba: image.to_straight() })
        }
        None => decode_image(bytes),
    })
}

/// Decode raster image bytes into unpremultiplied RGBA8 pixels under bounded size limits.
fn decode_image(bytes: &[u8]) -> Result<CopyImage, String> {
    if bytes.len() as u64 > paste_picture::MAX_FILE_BYTES {
        return Err("the picture file is larger than 200 MB".into());
    }
    let reader = || image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string());
    let (w, h) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
    let pixels = (w as usize).checked_mul(h as usize);
    if w == 0
        || h == 0
        || w as usize > paste_picture::MAX_SIDE
        || h as usize > paste_picture::MAX_SIDE
        || pixels.is_none_or(|n| n > paste_picture::MAX_PIXELS)
    {
        return Err(format!("the picture is too large to copy ({w} × {h} pixels)"));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(paste_picture::MAX_SIDE as u32);
    limits.max_image_height = Some(paste_picture::MAX_SIDE as u32);
    limits.max_alloc = Some((paste_picture::MAX_PIXELS as u64).saturating_mul(4));
    let mut reader = reader()?;
    reader.limits(limits);
    let rgba = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    Ok(CopyImage { width: w as usize, height: h as usize, rgba: rgba.into_raw() })
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
        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(String, String, String, String)>::new()));
        let sink = got.clone();
        let mut a = app(Services {
            set_clipboard_rich: Some(Box::new(move |t, h, r, m| {
                sink.borrow_mut().push((h.to_string(), t.to_string(), m.to_string(), r.to_string()));
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
        put_result(&mut a, &ctx, &r);
        let got_now = got.borrow().clone();
        assert_eq!(got_now.len(), 1);
        assert!(got_now[0].0.contains("<math") && got_now[0].1.contains("y=2x_s"), "{got_now:?}");
        // With text around it, no MathML formats: Word would paste the equation alone (#1297).
        assert!(got_now[0].2.is_empty(), "{got_now:?}");
        // Text without an equation gets HTML too, so other programs keep its formatting (#1523),
        // and RTF for the programs that read that (gap-clipboard-rtf).
        a.run("select.range", json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        let got_now = got.borrow().clone();
        assert_eq!(got_now.len(), 2);
        assert!(got_now[1].0.contains("Speed") && !got_now[1].0.contains("<math") && got_now[1].1 == "Speed", "{got_now:?}");
        assert!(got_now[1].2.is_empty(), "{got_now:?}");
        assert!(got_now[1].3.starts_with("{\\rtf1") && got_now[1].3.contains("Speed"), "{got_now:?}");
        // The equation alone: its MathML for the platform's MathML formats too (#1297).
        a.run("select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 9}})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        let got_now = got.borrow().clone();
        assert_eq!(got_now.len(), 3);
        assert!(got_now[2].2.starts_with("<math") && got_now[2].2.contains("<msub>") && got_now[2].1 == "y=2x_s", "{got_now:?}");
    }

    #[test]
    fn copying_an_image_publishes_image_and_html_to_system_clipboard() {
        let got_image = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(usize, usize, String, String)>::new()));
        let sink = got_image.clone();
        let services = Services {
            set_clipboard_image: Some(Box::new(move |img, html, text| {
                sink.borrow_mut().push((img.width, img.height, html.to_string(), text.to_string()));
                true
            })),
            ..Default::default()
        };
        let mut a = app(services);
        let png = paste_picture::png_from_rgba(2, 2, vec![255; 16]).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(&png);
        a.run("insert.picture", json!({"data": data})).unwrap();
        a.run("select.all", json!({})).unwrap();
        let ctx = egui::Context::default();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        assert!(a.clipboard_published);
        let got = got_image.borrow().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, 2);
        assert_eq!(got[0].1, 2);
        assert!(got[0].2.contains("<img src=\"data:image/png;base64,"));
    }

    #[test]
    fn pasting_own_image_uses_private_clipboard_fragment() {
        let services =
            Services { set_clipboard_image: Some(Box::new(|_, _, _| true)), clipboard_is_own: Some(Box::new(|| true)), ..Default::default() };
        let mut a = app(services);
        let png = paste_picture::png_from_rgba(2, 2, vec![255; 16]).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(&png);
        a.run("insert.picture", json!({"data": data})).unwrap();
        a.run("select.all", json!({})).unwrap();
        let ctx = egui::Context::default();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        assert!(a.clipboard_published);

        a.run("document.setText", json!({"text": ""})).unwrap();
        assert_eq!(a.session.doc.body.len(), 1);
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty());

        let mut params = json!({});
        fill(&mut a, "edit.paste", &mut params);
        assert_eq!(params.get("privateClipboard"), Some(&Value::Bool(true)));
        a.run("edit.paste", params).unwrap();

        assert!(!a.session.doc.media.is_empty());
    }

    /// Paste Special captures ownership when it opens; `fill` must keep that captured value
    /// when the dialog submits, even if the clipboard changed meanwhile. Callers that pass no
    /// flag still get it read from the clipboard ownership service.
    #[test]
    fn fill_keeps_a_callers_private_clipboard_flag() {
        let services = Services { clipboard_is_own: Some(Box::new(|| true)), ..Default::default() };
        let mut a = app(services);
        a.clipboard_published = true;

        let mut params = json!({"privateClipboard": false, "text": "Bold"});
        fill(&mut a, "edit.pasteSpecial", &mut params);
        assert_eq!(params.get("privateClipboard"), Some(&Value::Bool(false)), "the captured flag wins");

        let mut params = json!({});
        fill(&mut a, "edit.paste", &mut params);
        assert_eq!(params.get("privateClipboard"), Some(&Value::Bool(true)), "without a flag fill reads ownership");
    }

    #[test]
    fn empty_copy_does_not_publish_or_overwrite_clipboard_state() {
        let got_image = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(usize, usize, String, String)>::new()));
        let sink = got_image.clone();
        let services = Services {
            set_clipboard_image: Some(Box::new(move |img, html, text| {
                sink.borrow_mut().push((img.width, img.height, html.to_string(), text.to_string()));
                true
            })),
            ..Default::default()
        };
        let mut a = app(services);
        let png = paste_picture::png_from_rgba(2, 2, vec![255; 16]).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(&png);
        a.run("insert.picture", json!({"data": data})).unwrap();
        a.run("select.all", json!({})).unwrap();
        let ctx = egui::Context::default();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        assert!(a.clipboard_published);
        assert_eq!(got_image.borrow().len(), 1);

        a.run("caret.docEnd", json!({})).unwrap();
        assert!(a.session.sel.is_collapsed());

        let empty_r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &empty_r);

        assert_eq!(got_image.borrow().len(), 1, "empty copy must not call set_clipboard_image");
        assert!(a.clipboard_published, "empty copy must not change published-copy state");

        let mut a2 = app(Services { set_clipboard_image: Some(Box::new(|_, _, _| true)), ..Default::default() });
        assert!(!a2.clipboard_published);
        let empty_r2 = a2.run("edit.copy", json!({})).unwrap();
        put_result(&mut a2, &ctx, &empty_r2);
        assert!(!a2.clipboard_published, "empty copy must not change published-copy state when false");
    }
}
