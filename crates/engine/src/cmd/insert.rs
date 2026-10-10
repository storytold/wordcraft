//! Insert tab: pages, tables, pictures, shapes, links, bookmarks, headers/footers, text, symbols.

use serde_json::{Value, json};
use wordcraft_doc::para::{Float, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::props::{Align, CharProps, Rgb, TabAlign, TabStop, TextColor};
use wordcraft_doc::{Block, Paragraph, PartKind, Path, Pos, StoryRef, Table, para_block};

use super::{delete_selection, sel_result, split_para, type_text};
use crate::sample::Lang;
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
        CommandSpec::new("insert.coverPage", "Cover Page", "Insert › Pages", cover_page)
            .params(r#"{"title"?: string, "subtitle"?: string, "author"?: string, "language"?: "de" (date as 10. Oktober 2026)}"#),
        CommandSpec::new("insert.table", "Table", "Insert › Tables", table).params(r#"{"rows": n, "cols": n, "style"?: string}"#),
        CommandSpec::new("insert.picture", "Pictures", "Insert › Illustrations", picture).params(r#"{"path"?: string, "data"?: base64, "width"?: pt, "alt"?: string}"#),
        CommandSpec::new("insert.shape", "Shapes", "Insert › Illustrations", shape)
            .params(r#"{"kind": "rectangle|roundedRectangle|ellipse|triangle|diamond|line|arrow|star|heart", "width"?: pt, "height"?: pt, "fill"?: "RRGGBB", "stroke"?: "RRGGBB"}"#),
        CommandSpec::new("insert.textBox", "Text Box", "Insert › Text", text_box).params(r#"{"text"?: string, "width"?: pt, "height"?: pt}"#),
        CommandSpec::new("insert.link", "Link", "Insert › Links", link).key("Mod+K").params(r#"{"url": string, "text"?: string}"#),
        CommandSpec::new("insert.removeLink", "Remove Hyperlink", "Insert › Links", |s, _| super::format::apply(s, &|c| {
            c.link = None;
            if c.style.as_deref() == Some("Hyperlink") {
                c.style = None;
            }
        })),
        CommandSpec::new("insert.bookmark", "Bookmark", "Insert › Links", bookmark).params(r#"{"name": string}"#),
        CommandSpec::new("insert.header", "Header", "Insert › Header & Footer", |s, v| header_footer(s, v, true)).params(r#"{"text"?: string, "preset"?: "blank|blankThree|title"}"#),
        CommandSpec::new("insert.footer", "Footer", "Insert › Header & Footer", |s, v| header_footer(s, v, false)).params(r#"{"text"?: string, "preset"?: "blank|blankThree|pageNumber"}"#),
        CommandSpec::new("insert.pageNumber", "Page Number", "Insert › Header & Footer", page_number).params(r#"{"position"?: "top|bottom|current", "align"?: "left|center|right", "format"?: "x of y"}"#),
        CommandSpec::new("insert.editHeader", "Edit Header", "Insert › Header & Footer › Header", |s, _| edit_hf(s, true)).pure(),
        CommandSpec::new("insert.editFooter", "Edit Footer", "Insert › Header & Footer › Footer", |s, _| edit_hf(s, false)).pure(),
        CommandSpec::new("insert.closeHeader", "Close Header and Footer", "Header & Footer", |s, _| {
            s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
            sel_result(s)
        })
        .pure(),
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
        CommandSpec::new("insert.dateTime", "Date & Time", "Insert › Text", date_time)
            .params(r#"{"format"?: "M/d/yyyy" (German: "dd.MM.yyyy"), "update"?: bool, "language"?: "de"}"#),
        CommandSpec::new("insert.symbol", "Symbol", "Insert › Symbols", |s, v| {
            let c = p::req_str(v, "char")?;
            type_text(s, c)?;
            sel_result(s)
        })
        .params(r#"{"char": string}"#),
        CommandSpec::new("insert.equation", "Equation", "Insert › Symbols", |s, v| {
            let lin = p::str(v, "linear").unwrap_or("a^2+b^2=c^2").to_string();
            let props = s.typing_props();
            let at = delete_selection(s)?;
            let end = s.doc.insert_object(&at, InlineObject::Equation { linear: lin, display: false }, &props)?;
            s.sel = Selection::caret(end);
            sel_result(s)
        })
        .key("Alt+=")
        .params(r#"{"linear"?: string}"#),
        CommandSpec::new("insert.field", "Field", "Insert › Text › Quick Parts", field).key("Mod+F9").params(r#"{"instr": string, "result"?: string}"#),
        CommandSpec::new("insert.dropCap", "Drop Cap", "Insert › Text", |s, v| {
            let lines = p::u64(v, "lines").unwrap_or(3).min(10) as u8;
            super::para::fmt(s, &|p| p.drop_cap = Some(lines))
        })
        .params(r#"{"lines"?: n (0 = none)}"#),
        CommandSpec::new("insert.horizontalLine", "Horizontal Line", "Home › Paragraph › Borders", |s, _| {
            super::para::fmt(s, &|p| {
                p.borders = Some(wordcraft_doc::props::Borders { bottom: Some(wordcraft_doc::props::Border::single(1.5)), ..Default::default() })
            })
        }),
        CommandSpec::new("insert.wordArt", "WordArt", "Insert › Text", |s, v| {
            let text = p::str(v, "text").unwrap_or("Your text here").to_string();
            let props = CharProps { size: Some(36.0), bold: Some(true), color: Some(TextColor::Rgb(Rgb(0x15, 0x60, 0x82))), outline: Some(false), ..Default::default() };
            let at = delete_selection(s)?;
            let end = s.doc.insert_text(&at, &text, &props)?;
            s.sel = Selection { anchor: at, focus: end };
            sel_result(s)
        }),
        CommandSpec::new("insert.textFromFile", "Text from File", "Insert › Text › Object", |s, v| {
            let path = p::req_str(v, "path")?;
            let doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
            let frag = wordcraft_doc::edit::Fragment { blocks: doc.body.iter().map(|b| (**b).clone()).collect() };
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

fn picture(s: &mut Session, v: &Value) -> CmdResult {
    let bytes = if let Some(d) = p::str(v, "data") {
        base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?
    } else if let Some(path) = p::str(v, "path") {
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::read(path).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            return Err(CmdError::Failed("reading files by path isn't available on the web".into()));
        }
    } else {
        s.ui_requests.push(json!({"open": "insertPicture"}));
        return sel_result(s);
    };
    if bytes.len() > 200 << 20 {
        return Err(CmdError::Failed("image is larger than 200 MB".into()));
    }
    let (pw, ph) =
        wordcraft_render::image_size(&bytes).ok_or_else(|| CmdError::Failed("not a supported image (PNG, JPEG, GIF, WebP, BMP)".into()))?;
    let ext = match bytes.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => "png",
        Some([0xFF, 0xD8, ..]) => "jpeg",
        Some([b'G', b'I', b'F', _]) => "gif",
        Some([b'R', b'I', b'F', b'F']) => "webp",
        Some([b'B', b'M', ..]) => "bmp",
        _ => "png",
    };
    let key = s.doc.add_media(bytes, ext);
    // Natural size at 96 ppi, fitted to the text width.
    let max_w = s.doc.sections().first().map(|(_, sp)| sp.text_width()).unwrap_or(468.0);
    let (mut w, mut h) = (pw as f32 * 0.75, ph as f32 * 0.75);
    if let Some(want) = p::f32(v, "width") {
        let k = want.max(4.0) / w.max(1.0);
        w *= k;
        h *= k;
    }
    if w > max_w {
        h *= max_w / w;
        w = max_w;
    }
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let obj =
        InlineObject::Image { media: key.clone(), w, h, alt: p::str(v, "alt").unwrap_or("").to_string(), float: Float::default(), crop: [0.0; 4] };
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection { anchor: at, focus: end };
    Ok(json!({"media": key, "width": w, "height": h}))
}

fn shape(s: &mut Session, v: &Value) -> CmdResult {
    let kind: ShapeKind =
        serde_json::from_value(v.get("kind").cloned().unwrap_or(json!("rectangle"))).map_err(|e| CmdError::Params(e.to_string()))?;
    let w = p::f32(v, "width").unwrap_or(108.0).clamp(4.0, 2000.0);
    let h = p::f32(v, "height").unwrap_or(if kind == ShapeKind::Line { 1.0 } else { 72.0 }).clamp(1.0, 2000.0);
    let fill = match p::str(v, "fill") {
        Some(c) => Rgb::parse(c),
        None if kind == ShapeKind::Line => None,
        None => Some(Rgb(0x15, 0x60, 0x82)),
    };
    let stroke = p::str(v, "stroke").and_then(Rgb::parse).or(Some(Rgb(0x0E, 0x40, 0x5A)));
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let obj =
        InlineObject::Shape { kind, w, h, fill, stroke, stroke_width: 1.0, float: Float { wrap: Wrap::Inline, ..Default::default() }, story: None };
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection { anchor: at, focus: end };
    sel_result(s)
}

fn text_box(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    let w = p::f32(v, "width").unwrap_or(144.0).clamp(18.0, 2000.0);
    let h = p::f32(v, "height").unwrap_or(72.0).clamp(18.0, 2000.0);
    let id = s.doc.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(&text, CharProps::default()))]);
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let obj = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w,
        h,
        fill: Some(Rgb::WHITE),
        stroke: Some(Rgb::BLACK),
        stroke_width: 0.75,
        float: Float::default(),
        story: Some(id),
    };
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection::caret(end);
    Ok(json!({"story": id}))
}

fn link(s: &mut Session, v: &Value) -> CmdResult {
    let Some(url) = p::str(v, "url").map(str::to_string) else {
        s.ui_requests.push(json!({"open": "link"}));
        return sel_result(s);
    };
    if url.is_empty() || url.len() > 4096 {
        return Err(CmdError::Params("bad url".into()));
    }
    let style = Some("Hyperlink".to_string());
    if s.sel.is_collapsed() {
        let text = p::str(v, "text").unwrap_or(&url).to_string();
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

fn bookmark(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if name.is_empty() || name.contains(' ') || name.len() > 40 || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Err(CmdError::Params("bookmark names start with a letter, have no spaces and at most 40 characters".into()));
    }
    // Replace an existing bookmark of the same name.
    for (_, pos) in s.doc.bookmarks().into_iter().filter(|(n, _)| *n == name).rev() {
        let para = s.doc.para_mut(pos.story, &pos.path)?;
        para.delete(pos.off, pos.off + wordcraft_doc::para::OBJ.len_utf8())?;
        let offs: Vec<usize> = para.object_offsets();
        if let Some(e) = offs.into_iter().find(|o| matches!(para.object_at(*o), Some(InlineObject::BookmarkEnd { name: n }) if *n == name)) {
            para.delete(e, e + wordcraft_doc::para::OBJ.len_utf8())?;
        }
    }
    let (a, b) = s.sel.ordered();
    let props = CharProps::default();
    let end = s.doc.insert_object(&b, InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&a, InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    let _ = end;
    sel_result(s)
}

fn hf_part(s: &mut Session, header: bool) -> u32 {
    let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
    let sect = s.doc.section_mut(block).clone();
    let existing = if header { sect.headers.default } else { sect.footers.default };
    if let Some(id) = existing.filter(|id| s.doc.parts.contains_key(id)) {
        return id;
    }
    let style = if header { "Header" } else { "Footer" };
    let id = s.doc.add_part(if header { PartKind::Header } else { PartKind::Footer }, vec![para_block(Paragraph::new().styled(style))]);
    let sect = s.doc.section_mut(block);
    if header {
        sect.headers.default = Some(id);
    } else {
        sect.footers.default = Some(id);
    }
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
                para.insert_object(0, InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false }, &CharProps::default())?;
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

fn edit_hf(s: &mut Session, header: bool) -> CmdResult {
    let id = hf_part(s, header);
    s.touch();
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

fn page_number(s: &mut Session, v: &Value) -> CmdResult {
    let position = p::str(v, "position").unwrap_or("bottom");
    let align = match p::str(v, "align").unwrap_or("center") {
        "left" => Align::Left,
        "right" => Align::Right,
        _ => Align::Center,
    };
    let x_of_y = p::str(v, "format").is_some_and(|f| f.contains("of"));
    let field = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false };
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
            InlineObject::Field { instr: "NUMPAGES".into(), result: "1".into(), locked: false },
            &CharProps { bold: Some(true), ..Default::default() },
        )?;
    } else {
        para.insert_object(0, field, &CharProps::default())?;
    }
    s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    Ok(json!({"story": id}))
}

/// The language dates are written in: the call's `language`, else the session's (the app's
/// interface language; English for headless sessions).
fn date_lang(s: &Session, v: &Value) -> Lang {
    Lang::from_tag(p::str(v, "language").unwrap_or(&s.template_language))
}

/// Word's default date picture in a language: `10/10/2026`, or `10.10.2026` in German.
pub(crate) fn default_date_picture(lang: Lang) -> &'static str {
    match lang {
        Lang::English => "M/d/yyyy",
        Lang::German => "dd.MM.yyyy",
    }
}

fn date_time(s: &mut Session, v: &Value) -> CmdResult {
    let lang = date_lang(s, v);
    let fmt = p::str(v, "format").unwrap_or(default_date_picture(lang));
    let text = format_date_in(fmt, lang);
    if p::bool(v, "update").unwrap_or(false) {
        let props = s.typing_props();
        let at = delete_selection(s)?;
        let end = s.doc.insert_object(&at, InlineObject::Field { instr: format!("DATE \\@ \"{fmt}\""), result: text, locked: false }, &props)?;
        s.sel = Selection::caret(end);
    } else {
        type_text(s, &text)?;
    }
    sel_result(s)
}

/// Format today's date with a Word picture (`M/d/yyyy`, `MMMM d, yyyy`, `yyyy-MM-dd`, `dddd`…).
pub fn format_date(fmt: &str) -> String {
    format_date_in(fmt, Lang::English)
}

/// [`format_date`] with month and day names in `lang` (`Oktober`, `Samstag`, `Okt`, `Sa`).
pub fn format_date_in(fmt: &str, lang: Lang) -> String {
    let iso = super::now_iso();
    let y: i64 = iso.get(0..4).and_then(|x| x.parse().ok()).unwrap_or(2026);
    let m: usize = iso.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(1);
    let d: u32 = iso.get(8..10).and_then(|x| x.parse().ok()).unwrap_or(1);
    let hh: u32 = iso.get(11..13).and_then(|x| x.parse().ok()).unwrap_or(0);
    let mm: u32 = iso.get(14..16).and_then(|x| x.parse().ok()).unwrap_or(0);
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    const DE_MONTHS: [&str; 12] =
        ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
    const DE_DAYS: [&str; 7] = ["Donnerstag", "Freitag", "Samstag", "Sonntag", "Montag", "Dienstag", "Mittwoch"];
    let (months, days, day_abbr) = match lang {
        Lang::English => (MONTHS, DAYS, 3),
        // German abbreviates days to two letters (`Sa`) and months to three (`Okt`).
        Lang::German => (DE_MONTHS, DE_DAYS, 2),
    };
    let days_since = {
        // Days since epoch for weekday.
        let secs = {
            #[cfg(not(target_arch = "wasm32"))]
            {
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
            }
            #[cfg(target_arch = "wasm32")]
            {
                0u64
            }
        };
        (secs / 86_400) as usize
    };
    let month = months.get(m.saturating_sub(1)).copied().unwrap_or("January");
    let day = days.get(days_since % 7).copied().unwrap_or("Monday");
    let abbr = |name: &'static str, n: usize| name.char_indices().nth(n).map_or(name, |(i, _)| name.get(..i).unwrap_or(name));
    let mut out = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        match (c, n) {
            ('y', 4..) => out.push_str(&format!("{y:04}")),
            ('y', _) => out.push_str(&format!("{:02}", y % 100)),
            ('M', 4..) => out.push_str(month),
            ('M', 3) => out.push_str(abbr(month, 3)),
            ('M', 2) => out.push_str(&format!("{m:02}")),
            ('M', 1) => out.push_str(&m.to_string()),
            ('d', 4..) => out.push_str(day),
            ('d', 3) => out.push_str(abbr(day, day_abbr)),
            ('d', 2) => out.push_str(&format!("{d:02}")),
            ('d', 1) => out.push_str(&d.to_string()),
            ('H', 2..) => out.push_str(&format!("{hh:02}")),
            ('H', _) => out.push_str(&hh.to_string()),
            ('h', _) => out.push_str(&(if hh.is_multiple_of(12) { 12 } else { hh % 12 }).to_string()),
            ('m', 2..) => out.push_str(&format!("{mm:02}")),
            ('m', _) => out.push_str(&mm.to_string()),
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

fn field(s: &mut Session, v: &Value) -> CmdResult {
    let instr = p::req_str(v, "instr")?.trim().to_string();
    if instr.is_empty() || instr.len() > 4096 {
        return Err(CmdError::Params("bad field code".into()));
    }
    let result = p::str(v, "result").unwrap_or("").to_string();
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let end = s.doc.insert_object(&at, InlineObject::Field { instr, result, locked: false }, &props)?;
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
    let lang = date_lang(s, v);
    let picture = match lang {
        Lang::English => "MMMM d, yyyy",
        Lang::German => "d. MMMM yyyy",
    };
    let mut date = Paragraph::with_text(&format_date_in(picture, lang), CharProps::default());
    date.insert_text(date.len(), "\u{000C}", &CharProps::default())?;
    blocks.push(Block::Para(date));
    let frag = wordcraft_doc::edit::Fragment { blocks };
    let start = s.doc.start_of(StoryRef::Body);
    let at = s.doc.split_paragraph(&start)?;
    let _ = at;
    s.doc.insert_fragment(&start, &frag)?;
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    sel_result(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        let data = b"hello world!?";
        assert_eq!(base64_decode(&base64_encode(data)).unwrap(), data);
        assert!(base64_decode("!!!").is_none());
    }

    #[test]
    fn date_pictures() {
        let d = format_date("yyyy-MM-dd");
        assert_eq!(d.len(), 10);
        assert!(format_date("MMMM d, yyyy").contains(", "));
    }

    #[test]
    fn german_date_pictures() {
        const MONTHS: [&str; 12] =
            ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
        const DAYS: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
        let short = format_date_in("dd.MM.yyyy", Lang::German);
        let b = short.as_bytes();
        assert!(short.len() == 10 && b[2] == b'.' && b[5] == b'.', "{short}");
        assert!(MONTHS.contains(&format_date_in("MMMM", Lang::German).as_str()));
        assert!(DAYS.contains(&format_date_in("dddd", Lang::German).as_str()));
        let abbr = format_date_in("ddd", Lang::German);
        assert!(DAYS.iter().any(|d| d.starts_with(&abbr)) && abbr.chars().count() == 2, "{abbr}");
        let long = format_date_in("d. MMMM yyyy", Lang::German);
        assert!(MONTHS.iter().any(|m| long.contains(m)), "{long}");
        // English is unchanged.
        assert_eq!(format_date_in("yyyy-MM-dd", Lang::English), format_date("yyyy-MM-dd"));
        assert_eq!(default_date_picture(Lang::German), "dd.MM.yyyy");
        assert_eq!(default_date_picture(Lang::English), "M/d/yyyy");
    }
}
