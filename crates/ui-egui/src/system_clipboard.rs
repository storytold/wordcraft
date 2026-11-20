//! The system clipboard for the Paste and Copy commands (#584, #687, #833).
//!
//! Everything goes through the installed clipboard, reached with `ClipboardHandle::of(ctx)`:
//! another program's formatted copy as HTML or RTF (Paste keeps its formatting, Merge Formatting
//! its emphasis), a picture for Paste Special, and the names of the formats there so a paste that
//! finds nothing usable says what the clipboard holds instead of "empty". Everything read here
//! comes from other programs: the platform read bounds every size, and the text payloads are
//! capped again before they reach the engine.

use std::sync::Arc;

use craft::platform::clipboard::{HTML, RTF, Written};
// PLAN(app-hub): craft facade path of the framework's clipboard handle, and the craft-platform dependency
use craft::ui::runtime::commands::{ClipItem, ClipboardHandle};
use serde_json::Value;
use wordcraft_engine::cmd::paste::{MAX_PASTE_BYTES, MAX_PICTURE_BYTES};

use crate::{WordApp, paste_picture};

/// Longest side of a copied picture, in pixels.
const MAX_SIDE: usize = 16_000;
/// Most pixels in a copied picture (200 MB of RGBA).
const MAX_PIXELS: usize = 50_000_000;

/// The commands that paste the clipboard.
pub(crate) fn is_paste(id: &str) -> bool {
    matches!(id, "edit.paste" | "edit.pasteText" | "edit.pasteMerge" | "edit.pasteSpecial")
}

/// Most format names an error lists.
const MAX_NAMES: usize = 6;
/// Longest format name shown.
const MAX_NAME_CHARS: usize = 60;

/// The clipboard of the app's window; `None` before the app has a context.
pub(crate) fn handle(app: &WordApp) -> Option<ClipboardHandle> {
    app.ctx.as_ref().map(ClipboardHandle::of)
}

/// The text of the `mime` flavour of the clipboard when it is there, not blank and within
/// [`MAX_PASTE_BYTES`].
fn read_markup(clip: &ClipboardHandle, mime: &str) -> Option<String> {
    let flavor = clip.read(&[mime]).ok().flatten()?;
    if flavor.data.len() > MAX_PASTE_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&flavor.data).into_owned()).filter(|t| !t.trim().is_empty())
}

/// Add what the system clipboard holds to a paste command's params: its text (the ribbon and
/// context-menu Paste send none), its HTML and RTF, and for Paste Special its picture. Params a
/// caller already gave are kept. `Some` when the paste was done here instead (copied picture
/// files, which reach us as their paths).
pub(crate) fn fill(app: &mut WordApp, id: &str, params: &mut Value) -> Option<Result<Value, String>> {
    if !is_paste(id) {
        return None;
    }
    let clip = handle(app)?;
    let o = params.as_object_mut()?;
    // A caller that captured ownership already (Paste Special's dialog) keeps that value.
    if !o.contains_key("privateClipboard") {
        let own = app.clipboard_published && clip.holds_ours();
        o.insert("privateClipboard".into(), Value::Bool(own));
    }
    if !o.contains_key("text")
        && let Some(text) = clip.read_text().ok().flatten().filter(|t| !t.is_empty())
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
    if let Some(html) = read_markup(&clip, HTML) {
        o.insert("html".into(), Value::String(html));
    }
    if let Some(rtf) = read_markup(&clip, RTF) {
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

/// Return whether the command result `r` contains any copied payload (text, HTML, RTF, or MathML).
fn has_copied_content(r: &Value) -> bool {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    !s("text").is_empty() || !s("html").is_empty() || !s("rtf").is_empty() || !s("mathml").is_empty()
}

/// Put what Copy or Cut returned on the system clipboard: a lone picture as an image with its
/// HTML and text; anything else as text, HTML and RTF (equations as MathML, #816; a lone
/// equation in the MathML formats too, #1297; formatting for other programs, #1523).
///
/// When `r` contains no copied content (e.g. Copy on a collapsed selection),
/// this does nothing: it avoids overwriting external clipboard contents with
/// previously-copied fragments or modifying the published ownership state.
pub(crate) fn put_result(app: &mut WordApp, ctx: &egui::Context, r: &Value) {
    if !has_copied_content(r) {
        return;
    }
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    let item = |make: fn(Arc<str>) -> ClipItem, text: &str| (!text.is_empty()).then(|| make(Arc::from(text)));
    let items: Vec<ClipItem> = match copied_image(app) {
        Some(Ok(image)) => {
            [Some(ClipItem::Image(Arc::new(image))), item(ClipItem::Html, s("html")), item(ClipItem::Text, s("text"))].into_iter().flatten().collect()
        }
        Some(Err(e)) => {
            app.clipboard_published = false;
            app.status(format!("Couldn't copy the picture: {e}"));
            return;
        }
        None => {
            [item(ClipItem::Text, s("text")), item(ClipItem::Html, s("html")), item(ClipItem::Rtf, s("rtf")), item(ClipItem::MathMl, s("mathml"))]
                .into_iter()
                .flatten()
                .collect()
        }
    };
    let result = ClipboardHandle::of(ctx).offer(&items);
    if let Err(e) = &result {
        log::warn!("clipboard: {e}");
    }
    app.clipboard_published = matches!(result, Ok(Written::Written));
}

/// RGBA8 pixels as an egui image, refused when the length doesn't match the size.
fn color_image(width: usize, height: usize, rgba: &[u8]) -> Result<egui::ColorImage, String> {
    if width.checked_mul(height).and_then(|n| n.checked_mul(4)) != Some(rgba.len()) {
        return Err("the picture's size doesn't match its pixels".into());
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied([width, height], rgba))
}

/// Extract the single copied image from the clipboard fragment, if that is all that was copied.
fn copied_image(app: &WordApp) -> Option<Result<egui::ColorImage, String>> {
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
            color_image(image.width as usize, image.height as usize, &image.to_straight())
        }
        None => decode_image(bytes),
    })
}

/// Decode raster image bytes into an unpremultiplied RGBA8 image under bounded size limits.
fn decode_image(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    if bytes.len() as u64 > paste_picture::MAX_FILE_BYTES {
        return Err("the picture file is larger than 200 MB".into());
    }
    let (w, h) = wordcraft_geom::picture::dimensions(bytes).ok_or_else(|| "unsupported or unreadable picture".to_string())?;
    let pixels = (w as usize).checked_mul(h as usize);
    if w == 0 || h == 0 || w as usize > MAX_SIDE || h as usize > MAX_SIDE || pixels.is_none_or(|n| n > MAX_PIXELS) {
        return Err(format!("the picture is too large to copy ({w} × {h} pixels)"));
    }
    let budget = (MAX_PIXELS as u64).saturating_mul(8);
    let img = wordcraft_geom::picture::decode_for_drawing(bytes, budget, MAX_SIDE as u32).map_err(|e| e.to_string())?;
    color_image(w as usize, h as usize, &img.into_rgba8().into_raw())
}

/// Format names worth showing: not the clipboard protocol's own bookkeeping targets.
fn shown(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && !matches!(n, "TARGETS" | "TIMESTAMP" | "MULTIPLE" | "SAVE_TARGETS" | "DELETE" | "INCR")
        && !n.starts_with("chromium/")
        && !n.starts_with("application/x-qt-")
}

/// What to say when a paste found nothing it can use: the formats on the clipboard when it lists
/// them (#833), else that it is empty.
pub(crate) fn nothing_to_paste(app: &WordApp) -> String {
    let names: Vec<String> = handle(app).map(|h| h.formats()).unwrap_or_default();
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

/// An app over an installed in-memory clipboard, for the paste and picture tests.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;

    use craft::platform::clipboard::{Clipboard, ClipboardBackend, ClipboardError, ClipboardImage, Flavor, MemoryClipboard, RGBA8, URI_LIST};
    use wordcraft_engine::Session;

    use crate::{Services, WordApp};

    /// [`MemoryClipboard`] that also answers `read_files` from a held `text/uri-list`, as the system and event
    /// clipboards do (the framework's memory clipboard answers none, so a test of copied files would find the
    /// clipboard holding only the list's text).
    #[derive(Default)]
    struct FilesClipboard(MemoryClipboard);

    impl ClipboardBackend for FilesClipboard {
        fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
            self.0.write(flavors)
        }

        fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
            self.0.read(mimes, limit)
        }

        fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
            self.0.read_image(limit)
        }

        fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
            let Some(list) = self.0.read(&[URI_LIST], u64::MAX)? else { return Ok(None) };
            let files: Vec<PathBuf> = String::from_utf8_lossy(&list.data)
                .lines()
                .filter_map(|line| line.trim().strip_prefix("file://").map(PathBuf::from).filter(|path| path.is_absolute()))
                .take(limit.saturating_add(1))
                .collect();
            Ok((!files.is_empty()).then_some(files))
        }

        fn has(&mut self, mimes: &[&str]) -> bool {
            self.0.has(mimes)
        }

        fn formats(&mut self) -> Vec<String> {
            self.0.formats()
        }
    }

    /// An empty document whose window's clipboard holds `flavors`.
    pub(crate) fn app_holding(flavors: &[Flavor]) -> WordApp {
        let mut clipboard = Clipboard::new(FilesClipboard::default());
        if !flavors.is_empty() {
            let _ = clipboard.write(flavors).unwrap();
        }
        let ctx = egui::Context::default();
        // PLAN: assumed craft::ui::install(&Context, InstallSpec) and craft::ui::testing::install_spec() from craft-ui-contract
        craft::ui::install(&ctx, crate::test_harness::install_spec().clipboard(clipboard)).unwrap();
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.ctx = Some(ctx);
        a
    }

    /// A `width` × `height` picture of one grey value, as an `RGBA8` flavour.
    pub(crate) fn rgba8(width: u32, height: u32, value: u8) -> Flavor {
        let mut data = Vec::new();
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend(std::iter::repeat_n(value, (width * height * 4) as usize));
        Flavor::new(RGBA8, data)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{app_holding, rgba8};
    use super::*;
    use craft::platform::clipboard::{Flavor, TEXT};
    use serde_json::json;

    fn bold_at(a: &WordApp, off: usize) -> bool {
        let focus = a.session.sel.focus.clone();
        a.session.doc.para_at(&focus).and_then(|p| p.props_of_char(off).bold).unwrap_or(false)
    }

    fn first_para(a: &WordApp) -> String {
        let focus = a.session.sel.focus.clone();
        a.session.doc.para_at(&focus).map(|p| p.text.clone()).unwrap_or_default()
    }

    /// Which flavours each paste command sends: HTML formatting for Paste and Merge Formatting,
    /// none for Keep Text Only, RTF when the HTML is absent or over the cap.
    #[test]
    fn paste_keeps_formatting_from_the_system_clipboard_flavours() {
        let html = Flavor::new(HTML, b"<p>plain <b>strong</b></p>".to_vec());
        for (id, bold) in [("edit.paste", true), ("edit.pasteMerge", true), ("edit.pasteText", false)] {
            let mut a = app_holding(&[Flavor::text("plain strong"), html.clone()]);
            a.run(id, json!({})).unwrap();
            assert_eq!(first_para(&a), "plain strong", "{id}");
            assert_eq!(bold_at(&a, 7), bold, "{id}");
            assert!(!bold_at(&a, 0), "{id}");
        }
        let mut a = app_holding(&[Flavor::text("plain strong"), html.clone()]);
        let r = a.run("edit.pasteSpecial", json!({})).unwrap();
        let ids: Vec<&str> = r["formats"].as_array().unwrap().iter().filter_map(|f| f["id"].as_str()).collect();
        assert_eq!(ids, ["html", "text"]);
        let rtf = Flavor::new(RTF, br"{\rtf1\ansi {\b bold}}".to_vec());
        let huge = Flavor::new(HTML, format!("<b>{}</b>", "x".repeat(MAX_PASTE_BYTES)).into_bytes());
        for flavors in [vec![Flavor::text("bold"), rtf.clone()], vec![Flavor::text("bold"), huge, rtf]] {
            let mut a = app_holding(&flavors);
            a.run("edit.paste", json!({})).unwrap();
            assert_eq!(first_para(&a), "bold");
            assert!(bold_at(&a, 0), "the RTF's bold");
        }
    }

    /// Paste Special lists a picture on the system clipboard and pastes it.
    #[test]
    fn paste_special_offers_the_clipboard_picture() {
        let mut a = app_holding(&[rgba8(4, 4, 200)]);
        let r = a.run("edit.pasteSpecial", json!({})).unwrap();
        let ids: Vec<&str> = r["formats"].as_array().unwrap().iter().filter_map(|f| f["id"].as_str()).collect();
        assert_eq!(ids, ["picture"]);
        let r = a.run("edit.pasteSpecial", json!({"as": "picture"})).unwrap();
        assert_eq!(r["pastedAs"], "picture");
        assert_eq!(a.session.doc.media.len(), 1);
    }

    /// A clipboard holding nothing pasteable says what it holds; an empty one says it is empty; a
    /// long list is capped; an unreadable picture says it couldn't be read.
    #[test]
    fn nothing_to_paste_says_what_the_clipboard_holds() {
        let a = app_holding(&[]);
        let empty = nothing_to_paste(&a);
        let mut a = app_holding(&[Flavor::new("text/x-moz-url", b"x".to_vec())]);
        let holds = a.run("edit.paste", json!({})).unwrap_err();
        assert!(holds.contains("text/x-moz-url") && !empty.contains("text/x-moz-url"), "{holds} / {empty}");
        assert_ne!(holds, empty);
        assert_eq!(a.run("edit.pasteMerge", json!({})).unwrap_err(), holds);
        let many: Vec<Flavor> = (0..20).map(|i| Flavor::new(format!("x-f/{i}"), vec![1])).collect();
        let capped = nothing_to_paste(&app_holding(&many));
        assert!(capped.contains("more") && capped.len() < many.iter().map(|f| f.mime.len() + 2).sum::<usize>() + holds.len() + 40, "{capped}");
        // A picture whose pixels don't match its header can't be read.
        let mut broken = 1u32.to_le_bytes().to_vec();
        broken.extend_from_slice(&1u32.to_le_bytes());
        let mut b = app_holding(&[Flavor::new(RGBA8_MIME, broken)]);
        assert!(b.run("edit.paste", json!({})).is_err());
        assert!(b.session.doc.media.is_empty(), "no picture is inserted from an unreadable one");
    }

    const RGBA8_MIME: &str = craft::platform::clipboard::RGBA8;

    /// The flavours written for an equation alone, text around an equation, and plain text.
    #[test]
    fn copying_an_equation_puts_its_html_on_the_system_clipboard() {
        let mut a = app_holding(&[]);
        let ctx = a.ctx.clone().unwrap();
        let clip = ClipboardHandle::of(&ctx);
        let flavor = |mime: &str| clip.read(&[mime]).ok().flatten().map(|f| String::from_utf8_lossy(&f.data).into_owned());
        let has_mathml = || clip.formats().iter().any(|f| f.contains("mathml"));
        a.run("document.setText", json!({"text": "Speed "})).unwrap();
        a.run("caret.docEnd", json!({})).unwrap();
        a.run("insert.equation", json!({"linear": "y=2x_s", "edit": false})).unwrap();
        a.run("select.all", json!({})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        assert!(flavor(HTML).is_some_and(|h| h.contains("<math")));
        assert!(flavor(TEXT).is_some_and(|t| t.contains("y=2x_s")));
        assert!(!has_mathml(), "text around an equation has no MathML flavour");
        a.run("select.range", json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        assert!(flavor(HTML).is_some_and(|h| h.contains("Speed") && !h.contains("<math")));
        assert_eq!(flavor(TEXT).as_deref(), Some("Speed"));
        assert!(flavor(RTF).is_some_and(|r| r.starts_with("{\\rtf1") && r.contains("Speed")));
        assert!(!has_mathml());
        a.run("select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 9}})).unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        assert!(has_mathml(), "the equation alone has a MathML flavour");
        assert_eq!(flavor(TEXT).as_deref(), Some("y=2x_s"));
    }

    fn app_with_copied_picture() -> WordApp {
        let mut a = app_holding(&[]);
        let png = paste_picture::png_from_rgba(2, 2, vec![255; 16]).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(&png);
        a.run("insert.picture", json!({"data": data})).unwrap();
        a.run("select.all", json!({})).unwrap();
        let ctx = a.ctx.clone().unwrap();
        let r = a.run("edit.copy", json!({})).unwrap();
        put_result(&mut a, &ctx, &r);
        a
    }

    #[test]
    fn copying_an_image_publishes_image_and_html_to_system_clipboard() {
        let a = app_with_copied_picture();
        assert!(a.clipboard_published);
        let clip = handle(&a).unwrap();
        let image = clip.read_image().unwrap().unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        let html = clip.read(&[HTML]).unwrap().unwrap();
        assert!(String::from_utf8_lossy(&html.data).contains("<img src=\"data:image/png;base64,"));
    }

    /// After our own copy the paste keeps the fragment, not the clipboard's bitmap.
    #[test]
    fn pasting_own_image_uses_private_clipboard_fragment() {
        let mut a = app_with_copied_picture();
        a.run("document.setText", json!({"text": ""})).unwrap();
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty());
        let mut params = json!({});
        fill(&mut a, "edit.paste", &mut params);
        assert_eq!(params.get("privateClipboard"), Some(&Value::Bool(true)));
        a.run("edit.paste", params).unwrap();
        assert!(!a.session.doc.media.is_empty());
    }
}
