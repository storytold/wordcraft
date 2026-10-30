use super::{
    Block, CharProps, CmdError, CmdResult, Float, InlineObject, Paragraph, PartKind, Path, Pos, Rgb, Selection, Session, ShapeKind, StoryRef, Table,
    Value, Wrap, delete_selection, json, p, para_block, sel_result,
};

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

pub(super) fn table(s: &mut Session, v: &Value) -> CmdResult {
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

/// The media extension of an encoded image by its magic bytes, as the picture probe accepted it
/// (TIFF included when its compression is one the `image` crate decodes within limits).
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
    // Natural size: pixels at 96 ppi, or a metafile's own frame (WMF, EMF: #802). A picture that
    // is never decoded (a TIFF with JPEG strips, #3645) isn't inserted.
    let (natural_w, natural_h, ext) = match wordcraft_geom::picture::probe(&bytes) {
        Ok(pic) => (pic.width as f32 * 0.75, pic.height as f32 * 0.75, image_ext(&bytes)),
        Err(wordcraft_geom::picture::Refusal::Unsupported(why)) => return Err(CmdError::Failed(why.into())),
        Err(_) => match wordcraft_render::metafile_size(&bytes) {
            Some(size) => size,
            None => return Err(CmdError::Failed("not a supported image (PNG, JPEG, GIF, WebP, BMP, WMF, EMF)".into())),
        },
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
    let obj = InlineObject::Image { media: key.clone(), w, h, alt: alt.to_string(), float: Float::default(), crop: [0.0; 4], ole: None, shape: None };
    // A selected Drawing Canvas takes it in.
    if let Some(i) = crate::cmd::canvas::add_member(s, obj.clone())? {
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
pub(super) fn icon(s: &mut Session, v: &Value) -> CmdResult {
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

pub(super) fn shape(s: &mut Session, v: &Value) -> CmdResult {
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
    if crate::cmd::canvas::add_member(s, obj.clone())?.is_some() {
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

pub(super) fn text_box(s: &mut Session, v: &Value) -> CmdResult {
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
    if crate::cmd::canvas::add_member(s, obj.clone())?.is_none() {
        let props = s.typing_props();
        let at = delete_selection(s)?;
        s.doc.insert_object(&at, obj, &props)?;
    }
    // Like Word, type straight into the new box.
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}
