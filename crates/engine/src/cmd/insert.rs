//! Insert tab: pages, tables, pictures, shapes, links, bookmarks, headers/footers, text, symbols.

use serde_json::{Value, json};
use wordcraft_doc::para::{Float, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::props::{Align, CharProps, Rgb, TabAlign, TabStop, TextColor};
use wordcraft_doc::{Block, Paragraph, PartKind, Path, Pos, StoryRef, Table, para_block};

use super::{delete_selection, sel_result, split_para, type_text};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.pageBreak", "Page Break", "Insert › Pages", |s, _| {
            type_text(s, "\u{000C}")?;
            sel_result(s)
        }),
        CommandSpec::new("insert.blankPage", "Blank Page", "Insert › Pages", |s, _| {
            type_text(s, "\u{000C}")?;
            let at = s.sel.focus.clone();
            let at = split_para(s, &at)?;
            s.doc.insert_text(&at, "\u{000C}", &CharProps::default())?;
            s.sel = Selection::caret(at);
            sel_result(s)
        }),
        CommandSpec::new("insert.coverPage", "Cover Page", "Insert › Pages", cover_page).params(r#"{"title"?: string, "subtitle"?: string, "author"?: string}"#),
        CommandSpec::new("insert.table", "Table", "Insert › Tables", table).params(r#"{"rows": n, "cols": n, "style"?: string}"#),
        CommandSpec::new("insert.picture", "Pictures", "Insert › Illustrations", picture).params(r#"{"path"?: string, "data"?: base64, "width"?: pt, "alt"?: string}"#),
        CommandSpec::new("insert.icon", "Icons", "Insert › Illustrations", icon)
            .params(r#"{"id"?: string (omit to list the library), "color"?: "RRGGBB", "size"?: pt, "query"?: string, "category"?: string}"#),
        CommandSpec::new("insert.shape", "Shapes", "Insert › Illustrations", shape)
            .params(r#"{"kind": "rectangle|roundedRectangle|ellipse|triangle|diamond|line|arrow|star|heart|straightConnector|elbowConnector|curvedConnector", "width"?: pt, "height"?: pt, "fill"?: "RRGGBB", "stroke"?: "RRGGBB", "arrow"?: "none"|"start"|"end"|"both" (lines and connectors; glue a connector's ends with shape.connect)}"#),
        CommandSpec::new("insert.textBox", "Text Box", "Insert › Text", text_box)
            .params(r#"{"text"?: string, "width"?: pt, "height"?: pt}"#)
            .when(|s| {
                // As in Word: no text box inside a text box.
                matches!(s.sel.focus.story, StoryRef::Part(id) if s.doc.parts.get(&id).is_some_and(|p| p.kind == PartKind::TextBox))
                    .then_some("text boxes can't go inside a text box")
            }),
        CommandSpec::new("insert.link", "Link", "Insert › Links", link).key("Mod+K").params(
            r#"{"url": string} | {"bookmark": name} | {"heading": block (see link.targets)} | {"top": true}, "text"?: string (Place in This Document: a bookmark, a heading or the top of the document)"#,
        ),
        CommandSpec::new("link.targets", "Link", "Insert › Links", link_targets).pure(),
        CommandSpec::new("insert.removeLink", "Remove Hyperlink", "Insert › Links", |s, _| super::format::apply(s, &|c| {
            c.link = None;
            if c.style.as_deref() == Some("Hyperlink") {
                c.style = None;
            }
        })),
        CommandSpec::new("insert.bookmark", "Bookmark", "Insert › Links", bookmark).params(r#"{"name": string}"#).key("Mod+Shift+F5"),
        CommandSpec::new("insert.deleteBookmark", "Delete Bookmark", "Insert › Links", delete_bookmark).params(r#"{"name": string}"#),
        CommandSpec::new("insert.header", "Header", "Insert › Header & Footer", |s, v| header_footer(s, v, true)).params(r#"{"text"?: string, "preset"?: "blank|blankThree|title"}"#),
        CommandSpec::new("insert.footer", "Footer", "Insert › Header & Footer", |s, v| header_footer(s, v, false)).params(r#"{"text"?: string, "preset"?: "blank|blankThree|pageNumber"}"#),
        CommandSpec::new("insert.pageNumber", "Page Number", "Insert › Header & Footer", page_number).params(r#"{"position"?: "top|bottom|current", "align"?: "left|center|right", "format"?: "x of y"}"#),
        // Pure: going into an existing header/footer changes nothing; one the page doesn't have yet
        // is made by the mutating `insert.createHeader`/`insert.createFooter` (undoable, refused in
        // a protected document).
        CommandSpec::new("insert.editHeader", "Edit Header", "Insert › Header & Footer › Header", |s, v| edit_hf(s, v, true))
            .params(r#"{"page"?: number (0-based: the header that page shows, created if it has none)}"#)
            .pure(),
        CommandSpec::new("insert.editFooter", "Edit Footer", "Insert › Header & Footer › Footer", |s, v| edit_hf(s, v, false))
            .params(r#"{"page"?: number (0-based: the footer that page shows, created if it has none)}"#)
            .pure(),
        CommandSpec::new("insert.createHeader", "Header", "Insert › Header & Footer", |s, v| create_hf(s, v, true))
            .params(r#"{"page"?: number (0-based; default: the caret's section)} — the header that page shows, created if it has none, with the caret in it"#),
        CommandSpec::new("insert.createFooter", "Footer", "Insert › Header & Footer", |s, v| create_hf(s, v, false))
            .params(r#"{"page"?: number (0-based; default: the caret's section)} — the footer that page shows, created if it has none, with the caret in it"#),
        CommandSpec::new("insert.closeHeader", "Close Header and Footer", "Header & Footer", |s, _| {
            // Back where the caret was in the body, as Word does, not on page 1.
            let back = s.hf_return.take().filter(|_| s.in_header_footer());
            if !new_empty_headers(s).is_empty() {
                // Its own undo step; a protected document keeps it, and the caret still leaves.
                let _ = s.run("insert.removeEmptyHeaders", &json!({}));
            }
            s.sel = back.unwrap_or_else(|| Selection::caret(s.doc.start_of(StoryRef::Body)));
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("insert.removeEmptyHeaders", "Close Header and Footer", "Header & Footer", |s, _| {
            let ids = new_empty_headers(s);
            if strip_headers(&mut s.doc, &ids) {
                s.touch();
            }
            sel_result(s)
        }),
        CommandSpec::new("insert.removeHeader", "Remove Header", "Insert › Header & Footer › Header", |s, _| {
            s.doc.last_section.headers = Default::default();
            for path in s.doc.para_paths(StoryRef::Body) {
                if s.doc.para(StoryRef::Body, &path).is_some_and(|p| p.section.is_some())
                    && let Ok(p) = s.doc.para_mut(StoryRef::Body, &path)
                    && let Some(sec) = p.section.as_mut()
                {
                    sec.headers = Default::default();
                }
            }
            sel_result(s)
        }),
        CommandSpec::new("insert.removeFooter", "Remove Footer", "Insert › Header & Footer › Footer", |s, _| {
            s.doc.last_section.footers = Default::default();
            sel_result(s)
        }),
        CommandSpec::new("insert.dateTime", "Date & Time", "Insert › Text", date_time).params(r#"{"format"?: "M/d/yyyy", "update"?: bool}"#),
        CommandSpec::new("insert.symbol", "Symbol", "Insert › Symbols", symbol).params(r#"{"char": string, "font"?: family (the symbol's own font)}"#),
        CommandSpec::new("insert.field", "Field", "Insert › Text › Quick Parts", field).key("Mod+F9").params(r#"{"instr": string, "result"?: string}"#),
        // Word's Alt+Shift+D / T / P (Control+Shift+… in Word for Mac): DATE, TIME and PAGE fields.
        CommandSpec::new("insert.dateField", "Insert Date Field", "Insert › Text › Quick Parts", |s, _| field(s, &json!({"instr": "DATE"})))
            .key(super::mac_or("Ctrl+Shift+D", "Alt+Shift+D")),
        CommandSpec::new("insert.timeField", "Insert Time Field", "Insert › Text › Quick Parts", |s, _| field(s, &json!({"instr": "TIME"})))
            .key(super::mac_or("Ctrl+Shift+T", "Alt+Shift+T")),
        CommandSpec::new("insert.pageField", "Insert Page Field", "Insert › Text › Quick Parts", |s, _| field(s, &json!({"instr": "PAGE"})))
            .key(super::mac_or("Ctrl+Shift+P", "Alt+Shift+P")),
        CommandSpec::new("insert.dropCap", "Drop Cap", "Insert › Text", drop_cap)
            .params(r#"{"position"?: "none"|"dropped"|"margin" (default dropped), "lines"?: 1..10 (default 3; 0 = none), "distance"?: pt from the text (0..1584, default 0), "font"?: family (omit for the text's font)}"#)
            .when(drop_cap_off),
        CommandSpec::new("insert.horizontalLine", "Horizontal Line", "Home › Paragraph › Borders", |s, _| {
            super::para::fmt(s, &|p| {
                p.borders = Some(wordcraft_doc::props::Borders { bottom: Some(wordcraft_doc::props::Border::single(1.5)), ..Default::default() })
            })
        }),
        CommandSpec::new("insert.textFromFile", "Text from File", "Insert › Text › Object", |s, v| {
            let path = p::req_str(v, "path")?;
            let doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
            let mut frag = doc.body_fragment();
            super::paste::unlink_sections(&mut frag.blocks);
            let at = delete_selection(s)?;
            let end = s.doc.insert_fragment(&at, &frag)?;
            s.sel = Selection::caret(end);
            sel_result(s)
        })
        .params(r#"{"path": string}"#),
    ]
}

/// Insert a block after the caret's paragraph (splitting it if the caret is mid-paragraph).
fn insert_block_at_caret(s: &mut Session, block: Block) -> Result<Path, CmdError> {
    let at = delete_selection(s)?;
    let len = s.doc.para_at(&at).map(|p| p.len()).unwrap_or(0);
    let insert_at = if at.off == 0 {
        at.path.clone()
    } else {
        let new = s.doc.split_paragraph(&at)?;
        if at.off >= len { at.path.with_last(at.path.last() + 1) } else { new.path }
    };
    s.doc.insert_block(at.story, &insert_at, block)?;
    // Word keeps a paragraph after a table.
    let next = insert_at.with_last(insert_at.last() + 1);
    if s.doc.block(at.story, &next).is_none() {
        s.doc.insert_block(at.story, &next, Block::Para(Paragraph::new()))?;
    }
    // An empty paragraph we inserted before? Remove if the table took its place at the start.
    Ok(insert_at)
}

fn table(s: &mut Session, v: &Value) -> CmdResult {
    let rows = p::u64(v, "rows").unwrap_or(2).clamp(1, 1000) as usize;
    let cols = p::u64(v, "cols").unwrap_or(2).clamp(1, 63) as usize;
    let width = s.doc.sections().first().map(|(_, sp)| sp.text_width()).unwrap_or(468.0);
    let mut t = Table::new(rows, cols, width);
    if let Some(st) = p::str(v, "style") {
        let id = s.doc.styles.find(st).map(|x| x.id.clone()).ok_or_else(|| CmdError::Params(format!("no table style `{st}`")))?;
        t.props.style = Some(id);
    }
    // Like Word, a table inserted in a right-to-left paragraph is a right-to-left table (#2634):
    // its first cell stands at the right, so Tab, which follows reading order, moves leftwards.
    if s.doc.para_at(&s.sel.focus).is_some_and(|p| s.doc.styles.resolve_para(&p.props).bidi) {
        t.props.bidi_visual = true;
    }
    let story = s.sel.focus.story;
    let path = insert_block_at_caret(s, Block::Table(t))?;
    let mut first = path.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story, path: Path(first), off: 0 });
    Ok(json!({"table": path.0}))
}

/// Minimal base64 (standard alphabet, padding optional, whitespace ignored).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut n = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((buf >> n) as u8);
            buf &= (1 << n) - 1;
        }
    }
    Some(out)
}

pub fn base64_encode(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for ch in b.chunks(3) {
        let n =
            (ch.first().copied().unwrap_or(0) as u32) << 16 | (ch.get(1).copied().unwrap_or(0) as u32) << 8 | ch.get(2).copied().unwrap_or(0) as u32;
        for i in 0..4 {
            if i <= ch.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub(crate) fn picture(s: &mut Session, v: &Value) -> CmdResult {
    let bytes = if let Some(d) = p::str(v, "data") {
        base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?
    } else if let Some(path) = p::str(v, "path") {
        crate::io::read_file(path).map_err(CmdError::Failed)?
    } else {
        s.push_ui_request(json!({"open": "insertPicture"}));
        return sel_result(s);
    };
    place_picture(s, bytes, p::f32(v, "width"), p::str(v, "alt").unwrap_or(""))
}

/// The media extension of an encoded image by its magic bytes, as `image_size` accepted it
/// (TIFF included: the `image` crate decodes it).
pub(crate) fn image_ext(bytes: &[u8]) -> &'static str {
    match bytes.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => "png",
        Some([0xFF, 0xD8, ..]) => "jpeg",
        Some([b'G', b'I', b'F', _]) => "gif",
        Some([b'R', b'I', b'F', b'F']) => "webp",
        Some([b'B', b'M', ..]) => "bmp",
        Some([b'I', b'I', 0x2A, 0x00]) | Some([b'M', b'M', 0x00, 0x2A]) => "tiff",
        _ => "png",
    }
}

/// Insert an encoded image at the selection as an inline picture, `width` points wide (default:
/// its natural size at 96 ppi), never wider than the text column. The caret ends after it, so a
/// script's next typing doesn't replace it (#965); Picture Format commands still act on it. The
/// app selects a picture the person inserts (`WordApp::run`).
fn place_picture(s: &mut Session, bytes: Vec<u8>, width: Option<f32>, alt: &str) -> CmdResult {
    if bytes.len() > 200 << 20 {
        return Err(CmdError::Failed("image is larger than 200 MB".into()));
    }
    // Natural size: pixels at 96 ppi, or a metafile's own frame (WMF, EMF: #802).
    let (natural_w, natural_h, ext) = if let Some((pw, ph)) = wordcraft_render::image_size(&bytes) {
        let ext = image_ext(&bytes);
        (pw as f32 * 0.75, ph as f32 * 0.75, ext)
    } else if let Some(size) = wordcraft_render::metafile_size(&bytes) {
        size
    } else {
        return Err(CmdError::Failed("not a supported image (PNG, JPEG, GIF, WebP, BMP, WMF, EMF)".into()));
    };
    let key = s.doc.add_media(bytes, ext);
    // Fitted to the text width.
    let max_w = s.doc.sections().first().map(|(_, sp)| sp.text_width()).unwrap_or(468.0);
    let (mut w, mut h) = (natural_w, natural_h);
    if let Some(want) = width {
        let k = want.max(4.0) / w.max(1.0);
        w *= k;
        h *= k;
    }
    if w > max_w {
        h *= max_w / w;
        w = max_w;
    }
    let obj = InlineObject::Image { media: key.clone(), w, h, alt: alt.to_string(), float: Float::default(), crop: [0.0; 4], ole: None };
    // A selected Drawing Canvas takes it in.
    if let Some(i) = super::canvas::add_member(s, obj.clone())? {
        return Ok(json!({"media": key, "canvasMember": i}));
    }
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection::caret(end);
    Ok(json!({"media": key, "width": w, "height": h}))
}

/// Insert › Icons: a library icon (original artwork drawn in code, `wordcraft_render::icon_lib`)
/// rendered to a PNG picture `size` points square (default 1 inch) in `color` (default black).
/// Without an `id` it lists the library (narrowed by `query` and `category`) and asks a front end
/// for its icon picker.
fn icon(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_render::icon_lib;
    let Some(id) = p::str(v, "id") else {
        let category = p::str(v, "category").and_then(icon_lib::Category::parse);
        let icons: Vec<Value> = icon_lib::search(p::str(v, "query").unwrap_or(""), category)
            .iter()
            .map(|i| json!({"id": i.id, "name": i.name, "category": i.category.name(), "keywords": i.keywords}))
            .collect();
        s.push_ui_request(json!({"open": "icons"}));
        return Ok(json!({"icons": icons}));
    };
    let icon = icon_lib::find(id).ok_or_else(|| CmdError::Params(format!("no icon {id:?} (insert.icon without an id lists them)")))?;
    let ink = match p::str(v, "color") {
        Some(c) => Rgb::parse(c).ok_or_else(|| CmdError::Params(format!("bad color {c:?}: use RRGGBB")))?,
        None => Rgb::BLACK,
    };
    let size = p::f32(v, "size").unwrap_or(72.0).clamp(4.0, 1584.0);
    // About 300 pixels per inch, so it prints sharply.
    let px = (size * 300.0 / 72.0).round().clamp(64.0, 1024.0) as u32;
    let png = icon_lib::render(icon, px, ink).to_png();
    if png.is_empty() {
        return Err(CmdError::Failed("couldn't draw the icon".into()));
    }
    let mut r = place_picture(s, png, Some(size), icon.name)?;
    r["icon"] = json!(icon.id);
    Ok(r)
}

fn shape(s: &mut Session, v: &Value) -> CmdResult {
    let kind: ShapeKind =
        serde_json::from_value(v.get("kind").cloned().unwrap_or(json!("rectangle"))).map_err(|e| CmdError::Params(e.to_string()))?;
    let w = p::f32(v, "width").unwrap_or(108.0).clamp(4.0, 2000.0);
    let h =
        p::f32(v, "height").unwrap_or(if kind == ShapeKind::Line { 1.0 } else { 72.0 }).clamp(if kind.is_connector() { 0.0 } else { 1.0 }, 2000.0);
    let fill = match p::str(v, "fill") {
        Some(c) => Rgb::parse(c),
        None if kind.is_open() => None,
        None => Some(Rgb(0x15, 0x60, 0x82)),
    };
    // Arrowheads on lines and connectors.
    let (arrow_start, arrow_end) = match p::str(v, "arrow") {
        Some("end") => (false, true),
        Some("start") => (true, false),
        Some("both") => (true, true),
        Some("none") | None => (false, false),
        Some(x) => return Err(CmdError::Params(format!("`arrow`: none, start, end or both, not `{x}`"))),
    };
    let extra = wordcraft_doc::connector::ShapeExtra { arrow_start, arrow_end, ..Default::default() };
    // Connectors float in front of the text, so they can be glued to floating shapes.
    let float = if kind.is_connector() {
        Float { wrap: Wrap::InFrontOfText, ..Default::default() }
    } else {
        Float { wrap: Wrap::Inline, ..Default::default() }
    };
    let stroke = p::str(v, "stroke").and_then(Rgb::parse).or(Some(Rgb(0x0E, 0x40, 0x5A)));
    let obj = InlineObject::Shape {
        kind,
        w,
        h,
        fill: fill.map(wordcraft_doc::fill::Fill::solid),
        stroke,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra,
    };
    // A selected Drawing Canvas takes it in (and stays selected for the next one).
    if super::canvas::add_member(s, obj.clone())?.is_some() {
        return sel_result(s);
    }
    // Word draws a new shape beside what is selected and never deletes it: the shape just
    // inserted (which is selected) stays, and so does selected text (#2019). It goes after the
    // selection and becomes the selection. (A picture does replace the selection, as in Word.)
    let props = s.typing_props();
    let (_, at) = s.sel.ordered();
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection { anchor: at, focus: end };
    sel_result(s)
}

fn text_box(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    let w = p::f32(v, "width").unwrap_or(144.0).clamp(18.0, 2000.0);
    let h = p::f32(v, "height").unwrap_or(72.0).clamp(18.0, 2000.0);
    let id = s.doc.try_add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(&text, CharProps::default()))])?;
    let obj = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w,
        h,
        fill: Some(wordcraft_doc::fill::Fill::solid(Rgb::WHITE)),
        stroke: Some(Rgb::BLACK),
        stroke_width: 0.75,
        float: Float::default(),
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    // A selected Drawing Canvas takes it in; otherwise it goes at the caret.
    if super::canvas::add_member(s, obj.clone())?.is_none() {
        let props = s.typing_props();
        let at = delete_selection(s)?;
        s.doc.insert_object(&at, obj, &props)?;
    }
    // Like Word, type straight into the new box.
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

/// What a link can point to inside the document (Insert › Link › Place in This Document): the top
/// of the document, its headings (body block, level, text) and its bookmarks, without the hidden
/// ones (names starting with `_`).
fn link_targets(s: &mut Session, _: &Value) -> CmdResult {
    let headings: Vec<Value> =
        super::references::headings(s).into_iter().map(|(block, level, text, _)| json!({"block": block, "level": level, "text": text})).collect();
    let mut bookmarks: Vec<String> = Vec::new();
    for (name, _) in s.doc.bookmarks() {
        if !name.starts_with('_') && !bookmarks.contains(&name) {
            bookmarks.push(name);
        }
    }
    Ok(json!({"top": true, "headings": headings, "bookmarks": bookmarks}))
}

/// The address and the default text of a link to a place in this document, from `top`, `bookmark`
/// or `heading` (a heading gets a hidden `_Toc` bookmark, as in Word); None when none is given.
fn place_target(s: &mut Session, v: &Value) -> Result<Option<(String, String)>, CmdError> {
    if p::bool(v, "top") == Some(true) {
        return Ok(Some(("#_top".into(), "Top of the Document".into())));
    }
    if let Some(name) = p::str(v, "bookmark") {
        if !s.doc.bookmarks().iter().any(|(n, _)| n == name) {
            return Err(CmdError::Params(format!("no bookmark `{name}`")));
        }
        return Ok(Some((format!("#{name}"), name.to_string())));
    }
    if let Some(block) = p::u64(v, "heading") {
        let Some((i, _, text, lead)) = super::references::headings(s).into_iter().find(|h| h.0 as u64 == block) else {
            return Err(CmdError::Params(format!("block {block} is not a heading")));
        };
        let name = super::references::toc_bookmark(s, i, lead)?;
        return Ok(Some((format!("#{name}"), text)));
    }
    Ok(None)
}

fn link(s: &mut Session, v: &Value) -> CmdResult {
    let (url, label) = match p::str(v, "url") {
        Some(url) => (url.to_string(), url.to_string()),
        None => match place_target(s, v)? {
            Some(t) => t,
            None => {
                s.push_ui_request(json!({"open": "link"}));
                return sel_result(s);
            }
        },
    };
    if url.is_empty() || url.len() > 4096 {
        return Err(CmdError::Params("bad url".into()));
    }
    let style = Some("Hyperlink".to_string());
    if s.sel.is_collapsed() {
        let text = p::str(v, "text").unwrap_or(&label).to_string();
        let mut props = s.typing_props();
        props.link = Some(url.clone());
        props.style = style;
        let at = s.sel.focus.clone();
        let end = s.doc.insert_text(&at, &text, &props)?;
        s.sel = Selection::caret(end);
        s.pending = Some(CharProps::default());
    } else {
        let (a, b) = s.sel.ordered();
        s.doc.format_range(&a, &b, &|c| {
            c.link = Some(url.clone());
            c.style = style.clone();
        })?;
    }
    sel_result(s)
}

/// Whether `name` can name a bookmark: starts with a letter or underscore, contains only alphanumeric characters
/// or underscores, and is at most 40 characters (#3491). Word allows leading underscores for hidden bookmarks (_Toc, _Ref).
pub fn bookmark_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_') && name.chars().count() <= 40
}

/// Where bookmark `name` is: see [`bookmark_span`]. `None` when there is no such bookmark.
pub fn bookmark_range(doc: &wordcraft_doc::Document, name: &str) -> Option<(Pos, Pos)> {
    let (_, start) = doc.bookmarks().into_iter().find(|(n, _)| n == name)?;
    Some(bookmark_span(doc, name, &start))
}

/// The text of bookmark `name` whose start mark is at `start`: from just after that mark to its
/// end mark — in a later paragraph when it spans several (the start again when it has no end).
pub fn bookmark_span(doc: &wordcraft_doc::Document, name: &str, start: &Pos) -> (Pos, Pos) {
    let from = Pos { off: start.off.saturating_add(wordcraft_doc::para::OBJ.len_utf8()), ..start.clone() };
    let end = doc.para_paths(start.story).into_iter().skip_while(|p| *p != start.path).find_map(|path| {
        let para = doc.para(start.story, &path)?;
        let off = para.object_offsets().into_iter().find(|o| {
            (path != start.path || *o > start.off) && matches!(para.object_at(*o), Some(InlineObject::BookmarkEnd { name: n }) if n == name)
        })?;
        Some(Pos { story: start.story, path, off })
    });
    let end = end.unwrap_or_else(|| from.clone());
    (from, end)
}

/// Remove bookmark `name`'s start and end marks, in whichever paragraphs and stories they are,
/// keeping the selection on the same text. Returns whether the bookmark was there.
fn remove_bookmark(s: &mut Session, name: &str) -> Result<bool, CmdError> {
    let mark_len = wordcraft_doc::para::OBJ.len_utf8();
    let mut found = false;
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for story in stories {
        for path in s.doc.para_paths(story) {
            let Some(p) = s.doc.para(story, &path) else { continue };
            let mut marks = Vec::new();
            for off in p.object_offsets() {
                match p.object_at(off) {
                    Some(InlineObject::BookmarkStart { name: n }) if n == name => {
                        found = true;
                        marks.push(off);
                    }
                    Some(InlineObject::BookmarkEnd { name: n }) if n == name => marks.push(off),
                    _ => {}
                }
            }
            if marks.is_empty() {
                continue;
            }
            let para = s.doc.para_mut(story, &path)?;
            for off in marks.into_iter().rev() {
                para.delete(off, off + mark_len)?;
                for at in [&mut s.sel.anchor, &mut s.sel.focus] {
                    if at.story == story && at.path == path && at.off > off {
                        at.off = at.off.saturating_sub(mark_len);
                    }
                }
            }
        }
    }
    s.clamp_selection();
    Ok(found)
}

fn delete_bookmark(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim();
    if !remove_bookmark(s, name)? {
        return Err(CmdError::Params(format!("no bookmark `{name}`")));
    }
    sel_result(s)
}

fn bookmark(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if !bookmark_name_ok(&name) {
        return Err(CmdError::Params("bookmark names start with a letter, have no spaces and at most 40 characters".into()));
    }
    // Replace an existing bookmark of the same name.
    remove_bookmark(s, &name)?;
    let (a, b) = s.sel.ordered();
    let props = CharProps::default();
    let end = s.doc.insert_object(&b, InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&a, InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    let _ = end;
    sel_result(s)
}

/// The header (or footer) of the caret's section in the slot the caret's page uses (first page,
/// even page or the default), made when there is none: with Different First Page on, a header
/// inserted on page 1 is the one page 1 shows (#2826).
fn hf_part(s: &mut Session, header: bool) -> u32 {
    let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
    let layout = s.layout();
    let page = if s.sel.focus.story == StoryRef::Body { layout.page_of(&s.sel.focus) } else { Some(s.page_hint) };
    let slot = page.and_then(|pg| layout.pages.get(pg)).map(|pg| pg.hf_slot).unwrap_or_default();
    fn slot_of(slot: wordcraft_layout::HfSlot, set: &mut wordcraft_doc::section::HeaderSet) -> &mut Option<u32> {
        match slot {
            wordcraft_layout::HfSlot::First => &mut set.first,
            wordcraft_layout::HfSlot::Even => &mut set.even,
            wordcraft_layout::HfSlot::Default => &mut set.default,
        }
    }
    let mut sect = s.doc.section_mut(block).clone();
    let existing = *slot_of(slot, if header { &mut sect.headers } else { &mut sect.footers });
    if let Some(id) = existing.filter(|id| s.doc.parts.contains_key(id)) {
        return id;
    }
    let style = if header { "Header" } else { "Footer" };
    let id = s.doc.add_part(if header { PartKind::Header } else { PartKind::Footer }, vec![para_block(Paragraph::new().styled(style))]);
    let sect = s.doc.section_mut(block);
    *slot_of(slot, if header { &mut sect.headers } else { &mut sect.footers }) = Some(id);
    id
}

fn header_footer(s: &mut Session, v: &Value, header: bool) -> CmdResult {
    let id = hf_part(s, header);
    let style = if header { "Header" } else { "Footer" };
    if let Some(text) = p::str(v, "text") {
        let blocks = text.split('\n').map(|l| para_block(Paragraph::with_text(l, CharProps::default()).styled(style))).collect();
        s.doc.set_story(StoryRef::Part(id), blocks)?;
    } else if let Some(preset) = p::str(v, "preset") {
        let mut para = Paragraph::new().styled(style);
        match preset {
            "blankThree" => {
                para.insert_text(0, "[Type here]\t[Type here]\t[Type here]", &CharProps::default())?;
            }
            "title" => {
                para.insert_text(0, if s.doc.core.title.is_empty() { "[Document title]" } else { &s.doc.core.title }, &CharProps::default())?;
                para.props.align = Some(Align::Center);
            }
            "pageNumber" => {
                para.props.align = Some(Align::Center);
                para.insert_object(
                    0,
                    InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
                    &CharProps::default(),
                )?;
            }
            _ => {
                para.insert_text(0, "[Type here]", &CharProps::default())?;
            }
        }
        s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

fn edit_hf(s: &mut Session, v: &Value, header: bool) -> CmdResult {
    let mut page = p::u64(v, "page").map(|page| usize::try_from(page).unwrap_or(usize::MAX));
    if page.is_none() && s.sel.focus.story == StoryRef::Body {
        // The caret's page: with Different First Page on, page 1 edits its own header (#2826).
        let focus = s.sel.focus.clone();
        page = s.layout().page_of(&focus);
    }
    let shown = match page {
        Some(page) => shown_hf(s, page, header)?,
        None => {
            let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
            let set = s
                .doc
                .sections()
                .into_iter()
                .find(|(end, _)| block <= *end)
                .or_else(|| s.doc.sections().pop())
                .map(|(_, sp)| if header { sp.headers.default } else { sp.footers.default });
            set.flatten().filter(|id| s.doc.parts.contains_key(id))
        }
    };
    let Some(id) = shown else {
        // None yet: made by the mutating command, so it is an undo step and protection applies.
        let cmd = if header { "insert.createHeader" } else { "insert.createFooter" };
        let params = page.map_or_else(|| json!({}), |page| json!({ "page": page }));
        return s.run(cmd, &params);
    };
    if let Some(page) = page {
        s.page_hint = page;
    }
    s.touch();
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

/// The header (or footer) page `page` shows, if any; an error for a page that isn't there.
fn shown_hf(s: &mut Session, page: usize, header: bool) -> Result<Option<u32>, CmdError> {
    let layout = s.layout();
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    let shown = if header { pg.header_story } else { pg.footer_story };
    Ok(shown.filter(|id| s.doc.parts.contains_key(id)))
}

fn create_hf(s: &mut Session, v: &Value, header: bool) -> CmdResult {
    let id = match p::u64(v, "page") {
        Some(page) => {
            let page = usize::try_from(page).unwrap_or(usize::MAX);
            s.page_hint = page;
            page_hf_part(s, page, header)?
        }
        None => {
            let before = s.doc.parts.len();
            let id = hf_part(s, header);
            if s.doc.parts.len() > before {
                s.new_header_parts.push(id);
            }
            id
        }
    };
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

/// The header (or footer) page `page` shows; when it shows none, a new empty one in the slot the
/// page uses (first page, even page or the default), as Word creates on a double-click in the
/// empty header area (#960). It goes in the first section: sections link to the previous one by
/// default, so as in Word it shows on every page that uses that slot and has none of its own.
fn page_hf_part(s: &mut Session, page: usize, header: bool) -> Result<u32, CmdError> {
    if let Some(id) = shown_hf(s, page, header)? {
        return Ok(id);
    }
    let layout = s.layout();
    let slot = layout.pages.get(page).map(|pg| pg.hf_slot).unwrap_or_default();
    // The page shows none, so no section up to its own has this slot: the first section's is
    // inherited all the way to it.
    let block = s.doc.sections().first().map(|(end, _)| *end).unwrap_or(0);
    let style = if header { "Header" } else { "Footer" };
    let id = s.doc.add_part(if header { PartKind::Header } else { PartKind::Footer }, vec![para_block(Paragraph::new().styled(style))]);
    let sect = s.doc.section_mut(block);
    let set = if header { &mut sect.headers } else { &mut sect.footers };
    match slot {
        wordcraft_layout::HfSlot::First => set.first = Some(id),
        wordcraft_layout::HfSlot::Even => set.even = Some(id),
        wordcraft_layout::HfSlot::Default => set.default = Some(id),
    }
    s.new_header_parts.push(id);
    Ok(id)
}

/// The headers/footers made by `insert.createHeader`/`insert.createFooter` that are still empty
/// (one empty paragraph). The list keeps them after they are taken away, so one an undo brings
/// back is taken away again on the next close.
fn new_empty_headers(s: &Session) -> Vec<u32> {
    s.new_header_parts
        .iter()
        .copied()
        .filter(|id| {
            s.doc.parts.get(id).is_some_and(|part| {
                matches!(part.kind, PartKind::Header | PartKind::Footer)
                    && part.blocks.len() == 1
                    && part.blocks.first().is_some_and(|b| matches!(&**b, Block::Para(p) if p.text.is_empty() && p.objects.is_empty()))
            })
        })
        .collect()
}

/// The document as saved: without the headers/footers just made and still empty, so an unwritten
/// one is never saved (`None` when there are none, to save the document as it is).
pub(crate) fn saved_copy(s: &Session) -> Option<wordcraft_doc::Document> {
    let ids = new_empty_headers(s);
    if ids.is_empty() {
        return None;
    }
    let mut doc = s.doc.clone();
    strip_headers(&mut doc, &ids).then_some(doc)
}

/// Takes headers/footers `ids` out of the document and every section that refers to them;
/// whether anything was taken away.
fn strip_headers(doc: &mut wordcraft_doc::Document, ids: &[u32]) -> bool {
    if ids.is_empty() {
        return false;
    }
    let clear = |set: &mut wordcraft_doc::section::HeaderSet| {
        for slot in [&mut set.default, &mut set.first, &mut set.even] {
            if slot.is_some_and(|id| ids.contains(&id)) {
                *slot = None;
            }
        }
    };
    clear(&mut doc.last_section.headers);
    clear(&mut doc.last_section.footers);
    for path in doc.para_paths(StoryRef::Body) {
        if doc.para(StoryRef::Body, &path).is_some_and(|p| p.section.is_some())
            && let Ok(p) = doc.para_mut(StoryRef::Body, &path)
            && let Some(sec) = p.section.as_mut()
        {
            clear(&mut sec.headers);
            clear(&mut sec.footers);
        }
    }
    let mut any = false;
    for id in ids {
        any |= doc.parts.remove(id).is_some();
    }
    any
}

fn page_number(s: &mut Session, v: &Value) -> CmdResult {
    let position = p::str(v, "position").unwrap_or("bottom");
    let align = match p::str(v, "align").unwrap_or("center") {
        "left" => Align::Left,
        "right" => Align::Right,
        _ => Align::Center,
    };
    let x_of_y = p::str(v, "format").is_some_and(|f| f.contains("of"));
    let field = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() };
    if position == "current" {
        let props = s.typing_props();
        let at = delete_selection(s)?;
        let end = s.doc.insert_object(&at, field, &props)?;
        s.sel = Selection::caret(end);
        return sel_result(s);
    }
    let header = position == "top";
    let id = hf_part(s, header);
    let mut para = Paragraph::new().styled(if header { "Header" } else { "Footer" });
    para.props.align = Some(align);
    para.props.tabs = Some(vec![TabStop { pos: 0.0, align: TabAlign::Clear, leader: Default::default() }]);
    if x_of_y {
        para.insert_text(0, "Page  of ", &CharProps::default())?;
        para.insert_object(5, field, &CharProps { bold: Some(true), ..Default::default() })?;
        let end = para.len();
        para.insert_object(
            end,
            InlineObject::Field { instr: "NUMPAGES".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps { bold: Some(true), ..Default::default() },
        )?;
    } else {
        para.insert_object(0, field, &CharProps::default())?;
    }
    s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    Ok(json!({"story": id}))
}

fn date_time(s: &mut Session, v: &Value) -> CmdResult {
    let fmt = p::str(v, "format").unwrap_or("M/d/yyyy");
    let text = format_date(fmt);
    if p::bool(v, "update").unwrap_or(false) {
        let props = s.typing_props();
        let at = delete_selection(s)?;
        let end = s.doc.insert_object(
            &at,
            InlineObject::Field { instr: format!("DATE \\@ \"{fmt}\""), result: text, locked: false, dirty: false, code: false, form: String::new() },
            &props,
        )?;
        s.sel = Selection::caret(end);
    } else {
        type_text(s, &text)?;
    }
    sel_result(s)
}

/// Format today's date with a Word picture (`M/d/yyyy`, `MMMM d, yyyy`, `yyyy-MM-dd`, `dddd`…).
pub fn format_date(fmt: &str) -> String {
    let iso = super::now_iso();
    let y: i64 = iso.get(0..4).and_then(|x| x.parse().ok()).unwrap_or(2026);
    let m: usize = iso.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(1);
    let d: u32 = iso.get(8..10).and_then(|x| x.parse().ok()).unwrap_or(1);
    let hh: u32 = iso.get(11..13).and_then(|x| x.parse().ok()).unwrap_or(0);
    let mm: u32 = iso.get(14..16).and_then(|x| x.parse().ok()).unwrap_or(0);
    let ss: u32 = iso.get(17..19).and_then(|x| x.parse().ok()).unwrap_or(0);
    format_date_parts(fmt, y, m, d, (hh, mm, ss), (super::now_unix() / 86_400) as i64)
}

/// Format a date and time with a Word picture; `days_since` (days since 1970-01-01) gives the weekday.
pub fn format_date_parts(fmt: &str, y: i64, m: usize, d: u32, (hh, mm, ss): (u32, u32, u32), days_since: i64) -> String {
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    let month = MONTHS.get(m.saturating_sub(1)).copied().unwrap_or("January");
    let day = DAYS.get(days_since.rem_euclid(7) as usize).copied().unwrap_or("Monday");
    let mut out = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        // `am/pm` (any case) is one token: AM or PM.
        if chars.get(i..i + 5).is_some_and(|w| w.iter().collect::<String>().eq_ignore_ascii_case("am/pm")) {
            out.push_str(if hh < 12 { "AM" } else { "PM" });
            i += 5;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        match (c, n) {
            ('y' | 'Y', 4..) => out.push_str(&format!("{y:04}")),
            ('y' | 'Y', _) => out.push_str(&format!("{:02}", y % 100)),
            ('M', 4..) => out.push_str(month),
            ('M', 3) => out.push_str(month.get(..3).unwrap_or(month)),
            ('M', 2) => out.push_str(&format!("{m:02}")),
            ('M', 1) => out.push_str(&m.to_string()),
            ('d' | 'D', 4..) => out.push_str(day),
            ('d' | 'D', 3) => out.push_str(day.get(..3).unwrap_or(day)),
            ('d' | 'D', 2) => out.push_str(&format!("{d:02}")),
            ('d' | 'D', 1) => out.push_str(&d.to_string()),
            ('H', 2..) => out.push_str(&format!("{hh:02}")),
            ('H', _) => out.push_str(&hh.to_string()),
            ('h', 2..) => out.push_str(&format!("{:02}", if hh.is_multiple_of(12) { 12 } else { hh % 12 })),
            ('h', _) => out.push_str(&(if hh.is_multiple_of(12) { 12 } else { hh % 12 }).to_string()),
            ('m', 2..) => out.push_str(&format!("{mm:02}")),
            ('m', _) => out.push_str(&mm.to_string()),
            ('s', 2..) => out.push_str(&format!("{ss:02}")),
            ('s', _) => out.push_str(&ss.to_string()),
            ('a' | 'A', _) => out.push_str(if hh < 12 { "AM" } else { "PM" }),
            _ => {
                for _ in 0..n {
                    out.push(c);
                }
            }
        }
        i += n;
    }
    out
}

/// Insert › Symbol: the character at the caret, in its own font when `font` names one (the rest of
/// the text keeps the caret's font).
fn symbol(s: &mut Session, v: &Value) -> CmdResult {
    let c = p::req_str(v, "char")?;
    if c.is_empty() || c.chars().count() > 64 {
        return Err(CmdError::Params("char: 1–64 characters".into()));
    }
    let font = p::str(v, "font").map(str::trim).filter(|f| !f.is_empty());
    let Some(font) = font else {
        type_text(s, c)?;
        return sel_result(s);
    };
    if font.len() > 256 {
        return Err(CmdError::Params("font: a family name".into()));
    }
    let base = s.typing_props();
    // A symbol font's character remembers it as its own font (`w:sym`), so changing the text's
    // font later keeps the symbol; other fonts are ordinary run formatting. `w:sym` holds a 16-bit
    // code, so a character beyond the Basic Multilingual Plane stays ordinary text in that font.
    // Each character decides for itself, so a mixed string keeps its symbols and its text apart.
    let font_is_symbol = wordcraft_doc::symbol_fonts::is_symbol_font(font);
    let mut r = Ok(());
    let mut buf = [0u8; 4];
    for ch in c.chars() {
        let mut props = base.clone();
        let private_use = ('\u{F020}'..='\u{F0FF}').contains(&ch);
        if u32::from(ch) <= 0xFFFF && (font_is_symbol || private_use) {
            props.sym_font = Some(font.to_string());
        } else {
            props.font = Some(font.to_string());
        }
        s.pending = Some(props);
        r = type_text(s, ch.encode_utf8(&mut buf));
        if r.is_err() {
            break;
        }
    }
    // Typing on after the symbol goes on in the text's own font.
    s.pending = Some(base);
    r?;
    sel_result(s)
}

fn field(s: &mut Session, v: &Value) -> CmdResult {
    let instr = p::req_str(v, "instr")?.trim().to_string();
    if instr.is_empty() || instr.len() > 4096 {
        return Err(CmdError::Params("bad field code".into()));
    }
    let result = p::str(v, "result").unwrap_or("").to_string();
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let end =
        s.doc.insert_object(&at, InlineObject::Field { instr, result, locked: false, dirty: false, code: false, form: String::new() }, &props)?;
    s.sel = Selection::caret(end);
    let _ = super::references::update_fields(s);
    sel_result(s)
}

fn cover_page(s: &mut Session, v: &Value) -> CmdResult {
    let title = p::str(v, "title").unwrap_or("Document Title").to_string();
    let subtitle = p::str(v, "subtitle").unwrap_or("Document subtitle").to_string();
    let author = p::str(v, "author").unwrap_or(&s.author).to_string();
    let accent = Rgb(0x15, 0x60, 0x82);
    let mut blocks = Vec::new();
    for _ in 0..8 {
        blocks.push(Block::Para(Paragraph::new()));
    }
    let mut t =
        Paragraph::with_text(&title, CharProps { size: Some(44.0), color: Some(TextColor::Rgb(accent)), ..Default::default() }).styled("Title");
    t.props.borders = Some(wordcraft_doc::props::Borders {
        bottom: Some(wordcraft_doc::props::Border { style: wordcraft_doc::props::BorderStyle::Single, width: 2.0, color: Some(accent), space: 6.0 }),
        ..Default::default()
    });
    blocks.push(Block::Para(t));
    blocks.push(Block::Para(Paragraph::with_text(&subtitle, CharProps::default()).styled("Subtitle")));
    for _ in 0..14 {
        blocks.push(Block::Para(Paragraph::new()));
    }
    blocks
        .push(Block::Para(Paragraph::with_text(&author, CharProps { bold: Some(true), color: Some(TextColor::Rgb(accent)), ..Default::default() })));
    let mut date = Paragraph::with_text(&format_date("MMMM d, yyyy"), CharProps::default());
    date.insert_text(date.len(), "\u{000C}", &CharProps::default())?;
    blocks.push(Block::Para(date));
    let frag = wordcraft_doc::edit::Fragment { blocks, ..Default::default() };
    let start = s.doc.start_of(StoryRef::Body);
    let at = s.doc.split_paragraph(&start)?;
    let _ = at;
    s.doc.insert_fragment(&start, &frag)?;
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    sel_result(s)
}

/// Why Insert › Drop Cap is off at the caret, as in Word: in a table cell, in an empty paragraph
/// and in one that starts with a space.
fn drop_cap_off(s: &Session) -> Option<&'static str> {
    if s.sel.focus.path.cell().is_some() {
        return Some("drop caps can't go in a table");
    }
    let first = s.doc.para_at(&s.sel.focus).and_then(|p| p.text.chars().next());
    match first {
        None => Some("the paragraph is empty"),
        Some(c) if c.is_whitespace() => Some("the paragraph starts with a space"),
        Some(_) => None,
    }
}

/// Insert › Drop Cap and Drop Cap Options: none, or the letter dropped into the text or hung in
/// the margin, `lines` tall, `distance` points from the text, in `font` (unset: the text's).
/// None takes all of it away, the letter's font with it.
fn drop_cap(s: &mut Session, v: &Value) -> CmdResult {
    let lines = v.get("lines").and_then(Value::as_f64).filter(|l| l.is_finite());
    let none = p::str(v, "position") == Some("none") || lines == Some(0.0);
    let margin = p::str(v, "position") == Some("margin");
    let lines = lines.map_or(3.0, |l| l.round().clamp(1.0, 10.0)) as u8;
    let distance = p::f32(v, "distance").map_or(0.0, |d| d.clamp(0.0, wordcraft_doc::resolve::MAX_DROP_CAP_DISTANCE));
    // A family name, not a paragraph of text.
    let font = p::str(v, "font").map(str::trim).filter(|f| !f.is_empty()).map(|f| f.chars().take(64).collect::<String>());
    super::para::fmt(s, &|p| {
        p.drop_cap = (!none).then_some(lines);
        p.drop_cap_margin = (!none && margin).then_some(true);
        p.drop_cap_distance = (!none && distance > 0.0).then_some(distance);
        p.drop_cap_font = font.clone().filter(|_| !none);
    })
}

/// Insert › Drop Cap at the caret: the menu's highlight and the Drop Cap dialog's values.
#[derive(Clone, Debug, PartialEq)]
pub struct DropCapState {
    /// `none`, `dropped` or `margin`.
    pub position: &'static str,
    /// Lines to drop (3 when there is no drop cap).
    pub lines: u8,
    /// Distance from the text, points.
    pub distance: f32,
    /// The letter's own font, if it has one.
    pub font: Option<String>,
    /// The font of the paragraph's first character.
    pub text_font: String,
}

pub fn drop_cap_state(s: &Session) -> DropCapState {
    let para = s.doc.para_at(&s.sel.focus);
    let props = para.map(|p| p.props.clone()).unwrap_or_default();
    let rp = s.doc.styles.resolve_para(&props);
    let first = para.map(|p| p.props_at(0).clone()).unwrap_or_default();
    let text_font = s.doc.styles.resolve_char_in(props.style.as_deref(), None, &first).font;
    DropCapState {
        position: match (rp.drop_cap, rp.drop_cap_margin) {
            (0, _) => "none",
            (_, true) => "margin",
            _ => "dropped",
        },
        lines: if rp.drop_cap == 0 { 3 } else { rp.drop_cap },
        distance: if rp.drop_cap == 0 { 0.0 } else { rp.drop_cap_distance },
        font: rp.drop_cap_font.filter(|_| rp.drop_cap > 0),
        text_font,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closing the header or footer (Esc, the ribbon) puts the caret back where it was in the
    /// body, not on page 1 (#3331).
    #[test]
    fn close_header_returns_to_the_body_selection() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "One"})).unwrap();
        s.run("insert.pageBreak", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Two words here"})).unwrap();
        s.run("select.text", &json!({"text": "words"})).unwrap();
        let before = s.sel.clone();
        // A header written to stays.
        s.run("insert.editHeader", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Head"})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(s.sel, before);
        // An empty one made by a double-click on page 2 is taken away again.
        s.run("insert.editFooter", &json!({"page": 1})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(s.sel, before);
        assert_eq!(s.doc.last_section.footers, Default::default());
    }

    /// #960: editing the header of a page that has none creates the one that page shows (first
    /// page, even page or default); typing goes into it, and closing an unwritten one takes it
    /// away again so the saved file stays as it was.
    #[test]
    fn edit_header_on_a_page_creates_its_slot() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "One"})).unwrap();
        s.run("insert.pageBreak", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Two"})).unwrap();
        assert_eq!(s.layout().pages.len(), 2);
        // Plain document: the default header, then closing it unwritten leaves no header.
        let r = s.run("insert.editHeader", &json!({"page": 1})).unwrap();
        let id = r["story"].as_u64().unwrap() as u32;
        assert_eq!(s.sel.focus.story, StoryRef::Part(id));
        assert_eq!(s.doc.last_section.headers.default, Some(id));
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(s.doc.last_section.headers, Default::default());
        assert!(!s.doc.parts.contains_key(&id));
        let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        assert!(back.parts.values().all(|p| !matches!(p.kind, PartKind::Header | PartKind::Footer)), "no empty header saved");
        // Written footer stays.
        let r = s.run("insert.editFooter", &json!({"page": 0})).unwrap();
        let foot = r["story"].as_u64().unwrap() as u32;
        s.run("text.insert", &json!({"text": "Foot"})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(s.doc.last_section.footers.default, Some(foot));
        assert_eq!(s.layout().pages[1].footer_story, Some(foot));
        // Different first page: page 0's header is the first-page one, page 1's the default.
        s.run("layout.differentFirstPage", &json!({"value": true})).unwrap();
        let first = s.run("insert.editHeader", &json!({"page": 0})).unwrap()["story"].as_u64().unwrap() as u32;
        s.run("text.insert", &json!({"text": "Cover"})).unwrap();
        assert_eq!(s.doc.last_section.headers.first, Some(first));
        assert_eq!(s.doc.last_section.headers.default, None);
        // Odd & even: page 1 (page number 2) gets the even header.
        s.run("layout.differentOddEven", &json!({"value": true})).unwrap();
        let even = s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64().unwrap() as u32;
        s.run("text.insert", &json!({"text": "Even"})).unwrap();
        assert_eq!(s.doc.last_section.headers.even, Some(even));
        // A page that shows a header edits that one; a junk page is an error, not a panic.
        assert_eq!(s.run("insert.editHeader", &json!({"page": 0})).unwrap()["story"].as_u64(), Some(first as u64));
        assert!(s.run("insert.editHeader", &json!({"page": 99})).is_err());
        assert!(s.run("insert.editHeader", &json!({"page": u64::MAX})).is_err());
    }

    /// #2826: with Different First Page on, Insert › Header and Edit Header with the caret on
    /// page 1 give page 1 its own header; page 2 keeps the default one.
    #[test]
    fn different_first_page_header_is_reachable_from_page_one() {
        let mut s = two_pages();
        s.run("insert.header", &json!({"text": "Head"})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        let head = s.doc.last_section.headers.default.unwrap();
        s.run("layout.differentFirstPage", &json!({"value": true})).unwrap();
        assert_eq!(s.layout().pages[0].header_story, None, "page 1 shows the empty first-page header");
        s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
        let first = s.run("insert.header", &json!({"preset": "title"})).unwrap()["story"].as_u64().unwrap() as u32;
        assert_ne!(first, head);
        assert_eq!(s.doc.last_section.headers.first, Some(first));
        let pages = s.layout().pages.clone();
        assert_eq!((pages[0].header_story, pages[1].header_story), (Some(first), Some(head)));
        // Edit Footer from page 1 (no page given): page 1's own footer, not the default.
        s.run("insert.closeHeader", &json!({})).unwrap();
        s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
        let foot = s.run("insert.editFooter", &json!({})).unwrap()["story"].as_u64().unwrap() as u32;
        assert_eq!((s.doc.last_section.footers.first, s.doc.last_section.footers.default), (Some(foot), None));
    }

    fn two_pages() -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "One"})).unwrap();
        s.run("insert.pageBreak", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Two"})).unwrap();
        s
    }

    fn headers(s: &Session) -> usize {
        s.doc.parts.values().filter(|p| matches!(p.kind, PartKind::Header | PartKind::Footer)).count()
    }

    /// #960 review: making a header is an edit — a read-only document refuses it (an existing
    /// header can still be gone into), and Undo takes a made one away again.
    #[test]
    fn creating_a_header_is_an_undoable_protected_edit() {
        let mut s = two_pages();
        s.doc.settings.protection = Some("readOnly".into());
        assert!(s.run("insert.editHeader", &json!({"page": 0})).is_err(), "read only: no new header");
        assert!(s.run("insert.editFooter", &json!({})).is_err(), "read only: no new footer");
        assert_eq!(headers(&s), 0);
        s.doc.settings.protection = None;
        let id = s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64().unwrap() as u32;
        assert!(s.doc.parts.contains_key(&id));
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(headers(&s), 0, "undo takes the new header away");
        assert_eq!(s.doc.last_section.headers, Default::default());
        // Going into one that is there is no edit: allowed read-only, and no undo step.
        let id = s.run("insert.editHeader", &json!({"page": 0})).unwrap()["story"].as_u64().unwrap() as u32;
        s.run("text.insert", &json!({"text": "Head"})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        let steps = s.undo_labels().len();
        s.doc.settings.protection = Some("readOnly".into());
        assert_eq!(s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64(), Some(id as u64));
        assert_eq!(s.undo_labels().len(), steps);
    }

    /// #960 review: a new header still empty isn't saved, even while it is open; and one an Undo
    /// brings back after closing is taken away on the next close.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_empty_new_header_is_never_saved() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hf960-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.docx");
        let mut s = two_pages();
        s.run("insert.editHeader", &json!({"page": 0})).unwrap();
        s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        let back = wordcraft_docx::read(&std::fs::read(&path).unwrap()).unwrap();
        assert!(back.parts.values().all(|p| p.kind != PartKind::Header), "no empty header saved");
        assert_eq!(headers(&s), 1, "still open for typing");
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(headers(&s), 0);
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(headers(&s), 1, "undo brings it back");
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert_eq!(headers(&s), 0, "and the next close takes it away again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #960 review: with no header anywhere, one made on a later section's page goes in the first
    /// section, linked through to the later ones (Word's "Link to Previous" default); with Odd &
    /// Even on and only a default header, an even page edits the one it shows.
    #[test]
    fn new_header_links_to_previous_and_even_pages_edit_what_they_show() {
        let mut s = two_pages();
        s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
        s.run("text.insert", &json!({"text": "Three"})).unwrap();
        let pages = s.layout().pages.len();
        assert!(pages >= 3 && s.layout().pages[pages - 1].section == 1, "a second section");
        let id = s.run("insert.editHeader", &json!({"page": pages - 1})).unwrap()["story"].as_u64().unwrap() as u32;
        s.run("text.insert", &json!({"text": "All"})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert!(s.layout().pages.iter().all(|p| p.header_story == Some(id)), "every page shows it");
        assert_eq!(s.doc.sections()[0].1.headers.default, Some(id));
        s.run("layout.differentOddEven", &json!({"value": true})).unwrap();
        assert_eq!(s.layout().pages[1].header_story, Some(id));
        assert_eq!(s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64(), Some(id as u64));
        assert_eq!(s.doc.sections()[0].1.headers.even, None, "nothing made");
    }

    #[test]
    fn base64_round_trip() {
        let data = b"hello world!?";
        assert_eq!(base64_decode(&base64_encode(data)).unwrap(), data);
        assert!(base64_decode("!!!").is_none());
    }

    /// Every bookmark mark in the body, in order: (paragraph, start or end, name).
    fn bookmark_marks(s: &Session) -> Vec<(usize, bool, String)> {
        let mut out = Vec::new();
        for (i, path) in s.doc.para_paths(StoryRef::Body).iter().enumerate() {
            let p = s.doc.para(StoryRef::Body, path).unwrap();
            for off in p.object_offsets() {
                match p.object_at(off) {
                    Some(InlineObject::BookmarkStart { name }) => out.push((i, true, name.clone())),
                    Some(InlineObject::BookmarkEnd { name }) => out.push((i, false, name.clone())),
                    _ => {}
                }
            }
        }
        out
    }

    /// Insert › Bookmark › Delete (#1236): a bookmark across two paragraphs loses both its marks,
    /// the text and the selection stay where they were, and one undo brings it back. Go To
    /// selects the bookmark's text, as in Word.
    #[test]
    fn deleting_a_bookmark_removes_its_start_and_end() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Alpha beta\ngamma delta"));
        s.sel = Selection { anchor: Pos::body(0, 6), focus: Pos::body(1, 5) };
        s.run("insert.bookmark", &json!({"name": "Across"})).unwrap();
        assert_eq!(bookmark_marks(&s), vec![(0, true, "Across".into()), (1, false, "Across".into())]);
        s.run("edit.goto", &json!({"bookmark": "Across"})).unwrap();
        let picked = s.selected_text();
        assert!(picked.starts_with("beta") && picked.ends_with("gamma"), "{picked:?}");
        s.run("select.text", &json!({"text": "delta"})).unwrap();
        s.run("insert.deleteBookmark", &json!({"name": "Across"})).unwrap();
        assert!(bookmark_marks(&s).is_empty(), "{:?}", bookmark_marks(&s));
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Alpha beta\ngamma delta");
        assert_eq!(s.selected_text(), "delta", "the selection stays on its text");
        assert!(s.run("insert.deleteBookmark", &json!({"name": "Across"})).is_err(), "gone");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(bookmark_marks(&s).len(), 2, "one undo step");
    }

    /// Adding a bookmark under a name in use moves it, and the old end goes too — it was only
    /// looked for in the start's paragraph, so a bookmark across paragraphs left a stray end.
    #[test]
    fn adding_a_bookmark_again_moves_all_of_it() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Alpha beta\ngamma delta"));
        s.sel = Selection { anchor: Pos::body(0, 6), focus: Pos::body(1, 5) };
        s.run("insert.bookmark", &json!({"name": "Spot"})).unwrap();
        s.run("select.text", &json!({"text": "delta"})).unwrap();
        s.run("insert.bookmark", &json!({"name": "Spot"})).unwrap();
        assert_eq!(bookmark_marks(&s), vec![(1, true, "Spot".into()), (1, false, "Spot".into())]);
        assert_eq!(crate::cmd::references::bookmark_text(&s.doc, "Spot").as_deref(), Some("delta"));
    }

    /// #3491: Bookmark names are validated by Unicode character count and character classes.
    #[test]
    fn bookmark_name_validation_rules() {
        // Valid names:
        assert!(bookmark_name_ok("Mark1"));
        assert!(bookmark_name_ok("A"));
        assert!(bookmark_name_ok("Alpha_Beta_123"));
        // Valid leading underscore (Word hidden bookmarks, e.g. _Toc, _Ref, _Hidden):
        assert!(bookmark_name_ok("_Hidden"));
        assert!(bookmark_name_ok("_foo"));
        // Valid 27-character Unicode Cyrillic name (51 UTF-8 bytes):
        assert!(bookmark_name_ok("быстрый_доступ_к_документам"));
        // Valid 40-character name:
        let exactly_40 = "a".repeat(40);
        assert!(bookmark_name_ok(&exactly_40));
        let exactly_40_unicode = "я".repeat(40);
        assert!(bookmark_name_ok(&exactly_40_unicode));

        // Invalid: empty
        assert!(!bookmark_name_ok(""));
        // Invalid: leading digit
        assert!(!bookmark_name_ok("1mark"));
        // Invalid: symbols / punctuation
        assert!(!bookmark_name_ok("a&b"));
        assert!(!bookmark_name_ok("a!b"));
        assert!(!bookmark_name_ok("a-b"));
        assert!(!bookmark_name_ok("a.b"));
        assert!(!bookmark_name_ok("foo bar"));
        // Invalid: more than 40 characters
        assert!(!bookmark_name_ok(&"a".repeat(41)));
        assert!(!bookmark_name_ok(&"я".repeat(41)));

        // Verification through `insert.bookmark` command
        let mut s = Session::new(wordcraft_doc::Document::from_text("Test content"));
        assert!(s.run("insert.bookmark", &json!({"name": "быстрый_доступ_к_документам"})).is_ok());
        assert!(s.run("insert.bookmark", &json!({"name": "_Hidden"})).is_ok());
        assert!(s.run("insert.bookmark", &json!({"name": "1mark"})).is_err());
        assert!(s.run("insert.bookmark", &json!({"name": "a&b"})).is_err());
        assert!(s.run("insert.bookmark", &json!({"name": "a!b"})).is_err());
    }

    /// Insert › Symbol with a font: only the symbol takes it, the caret's font carries on after.
    #[test]
    fn a_mixed_symbol_string_keeps_symbols_and_text_apart() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("a"));
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.symbol", &json!({"char": "\u{F041}A", "font": "My Custom Font"})).unwrap();
        let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(p.props_of_char(1).sym_font.as_deref(), Some("My Custom Font"), "the private-use glyph");
        assert_eq!(p.props_of_char(4).sym_font, None, "the plain letter");
        assert_eq!(p.props_of_char(4).font.as_deref(), Some("My Custom Font"));
    }

    #[test]
    fn an_astral_symbol_stays_text_in_its_font() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("a"));
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.symbol", &json!({"char": "\u{1F600}", "font": "Wingdings"})).unwrap();
        let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(p.props_of_char(1).sym_font, None, "w:sym can't hold it");
        assert_eq!(p.props_of_char(1).font.as_deref(), Some("Wingdings"));
    }

    #[test]
    fn symbol_takes_its_own_font() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("ab"));
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.symbol", &json!({"char": "\u{2665}", "font": "DejaVu Sans"})).unwrap();
        s.run("text.insert", &json!({"text": "c"})).unwrap();
        let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(p.text, "ab\u{2665}c");
        assert_eq!(p.props_of_char(2).font.as_deref(), Some("DejaVu Sans"), "the symbol");
        assert_eq!(p.props_of_char(5).font, None, "typing after it (byte offsets)");
        assert!(s.run("insert.symbol", &json!({"char": ""})).is_err());
    }

    #[test]
    fn date_pictures() {
        // Word accepts either case for the day and year instructions.
        assert_eq!(format_date_parts("MM/DD/YYYY", 2026, 10, 21, (17, 27, 3), 0), "10/21/2026");
        assert_eq!(format_date_parts("D MMMM YYYY", 2026, 10, 21, (17, 27, 3), 0), "21 October 2026");
        // Both 12-hour instructions pad a single-digit hour; only the 12-hour one wraps at noon.
        assert_eq!(format_date_parts("hh:mm", 2026, 10, 21, (17, 27, 3), 0), "05:27");
        assert_eq!(format_date_parts("h:mm", 2026, 10, 21, (17, 27, 3), 0), "5:27");
        assert_eq!(format_date_parts("HH:mm", 2026, 10, 21, (5, 7, 3), 0), "05:07");
        let d = format_date("yyyy-MM-dd");
        assert_eq!(d.len(), 10);
        assert!(format_date("MMMM d, yyyy").contains(", "));
        let t = format_date("h:mm:ss am/pm");
        assert!(t.ends_with(" AM") || t.ends_with(" PM"), "{t}");
        assert_eq!(t.matches(':').count(), 2, "{t}");
        use super::super::references::date_picture;
        assert_eq!(date_picture(r#"DATE \@ "MMMM d, yyyy" \* MERGEFORMAT"#).as_deref(), Some("MMMM d, yyyy"));
        assert_eq!(date_picture(r#"TIME \@ HH:mm"#).as_deref(), Some("HH:mm"));
        assert_eq!(date_picture("DATE"), None);
    }
}
