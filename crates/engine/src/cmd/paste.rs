//! Paste Special: paste the clipboard in a chosen format (formatted, RTF, HTML, picture,
//! unformatted), and the formatted content Paste and Merge Formatting take from other programs.
//!
//! The clipboard can hold several representations of the same content. WordCraft's own copy keeps
//! a rich fragment; the system clipboard gives text, and callers (agents, the CLI, a front end
//! with a richer clipboard) may pass `html`, `rtf` or a `picture` too. Text that is itself RTF or
//! HTML source can also be pasted as that format. Paste keeps the source formatting of another
//! program's HTML or RTF; Merge Formatting keeps only its emphasis (#687).
//! Plain text has no formatting to keep, so Paste inserts it as Keep Text Only does.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Block, Document, Paragraph};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Largest clipboard payload Paste Special accepts, in bytes.
pub const MAX_PASTE_BYTES: usize = 16 << 20;
/// Largest clipboard picture Paste Special accepts (base64), in bytes.
pub const MAX_PICTURE_BYTES: usize = 96 << 20;
/// Nesting depth we follow when adjusting pasted tables.
const MAX_DEPTH: usize = 64;

/// The formats, in the order the dialog lists them: (id, label).
pub const FORMATS: [(&str, &str); 5] = [
    ("formatted", "Formatted Text"),
    ("rtf", "Rich Text Format (RTF)"),
    ("html", "HTML Format"),
    ("picture", "Picture"),
    ("text", "Unformatted Text"),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("edit.pasteSpecial", "Paste Special", "Home › Clipboard › Paste", paste_special)
            .key("Mod+Alt+V")
            .params(r#"{"as"?: "formatted"|"rtf"|"html"|"picture"|"text", "text"?: string, "html"?: string, "rtf"?: string, "picture"?: base64}"#),
        CommandSpec::new("edit.pasteChange", "Paste Options", "Home › Clipboard › Paste", change_paste)
            .params(r#"{"mode": "source"|"text"|"merge"|"picture" (Keep Source Formatting, Keep Text Only, Merge Formatting, Picture)}"#)
            .when(|s| if paste_options(s).is_empty() { Some("there is no paste to change") } else { None }),
    ]
}

/// Clipboard text from the params (system clipboard), else WordCraft's own copy.
fn clip_text(s: &Session, v: &Value) -> Option<String> {
    match p::str(v, "text") {
        Some(t) => Some(t.replace("\r\n", "\n")),
        None => s.clipboard.as_ref().map(Fragment::plain_text),
    }
}

fn looks_like_rtf(t: &str) -> bool {
    t.trim_start().starts_with("{\\rtf")
}

fn looks_like_html(t: &str) -> bool {
    let t = t.trim_start();
    t.starts_with('<') && (t.contains("</") || t.contains("/>"))
}

/// The source for a format, if the clipboard has it.
fn source(s: &Session, v: &Value, fmt: &str) -> Option<String> {
    let non_empty = |t: String| if t.is_empty() { None } else { Some(t) };
    match fmt {
        "formatted" => {
            let f = s.clipboard.as_ref()?;
            // System clipboard text that isn't our own copy means someone else copied since.
            match p::str(v, "text") {
                Some(t) if f.clipboard_text() != t.replace("\r\n", "\n") => None,
                _ => Some(String::new()),
            }
        }
        "rtf" => p::str(v, "rtf").map(str::to_string).or_else(|| clip_text(s, v).filter(|t| looks_like_rtf(t))).and_then(non_empty),
        "html" => p::str(v, "html").map(str::to_string).or_else(|| clip_text(s, v).filter(|t| looks_like_html(t))).and_then(non_empty),
        "picture" => p::str(v, "picture").map(str::to_string).and_then(non_empty),
        "text" => clip_text(s, v).and_then(non_empty),
        _ => None,
    }
}

/// Formats the clipboard can be pasted as, best first.
pub fn available(s: &Session, v: &Value) -> Vec<&'static str> {
    FORMATS.iter().map(|(id, _)| *id).filter(|id| source(s, v, id).is_some()).collect()
}

fn formats_json(ids: &[&str]) -> Value {
    json!(ids.iter().map(|id| json!({"id": id, "label": FORMATS.iter().find(|(f, _)| f == id).map(|(_, l)| *l).unwrap_or(*id)})).collect::<Vec<_>>())
}

fn paste_special(s: &mut Session, v: &Value) -> CmdResult {
    for k in ["text", "html", "rtf"] {
        if p::str(v, k).is_some_and(|t| t.len() > MAX_PASTE_BYTES) {
            return Err(CmdError::Params(format!("`{k}` is larger than {} MB", MAX_PASTE_BYTES >> 20)));
        }
    }
    if p::str(v, "picture").is_some_and(|t| t.len() > MAX_PICTURE_BYTES) {
        return Err(CmdError::Params(format!("`picture` is larger than {} MB", MAX_PICTURE_BYTES >> 20)));
    }
    let ids = available(s, v);
    let Some(fmt) = p::str(v, "as") else {
        // No format chosen: list them and ask a front end (if any) to show the dialog.
        let mut req = json!({"open": "pasteSpecial", "formats": ids});
        for k in ["text", "html", "rtf", "picture"] {
            if let Some(t) = p::str(v, k) {
                req[k] = json!(t);
            }
        }
        // Only the newest dialog request matters; older ones would pin their payloads.
        s.ui_requests.retain(|r| r["open"] != "pasteSpecial");
        s.push_ui_request(req);
        return Ok(json!({"formats": formats_json(&ids)}));
    };
    let fmt = match fmt.to_ascii_lowercase().as_str() {
        "formatted" | "keep" | "wordcraft" => "formatted",
        "rtf" => "rtf",
        "html" => "html",
        "picture" | "png" | "image" => "picture",
        "text" | "unformatted" | "plain" => "text",
        other => return Err(CmdError::Params(format!("unknown format `{other}` (formatted, rtf, html, picture or text)"))),
    };
    let Some(src) = source(s, v, fmt) else {
        return Err(CmdError::Failed(if ids.is_empty() {
            "the clipboard is empty".into()
        } else {
            format!("the clipboard has no {fmt} content (available: {})", ids.join(", "))
        }));
    };
    let frag = match fmt {
        "text" => {
            super::paste_text(s, &src.replace("\r\n", "\r").replace('\n', "\r"))?;
            let mut r = sel_result(s)?;
            r["pastedAs"] = json!(fmt);
            return Ok(r);
        }
        "picture" => {
            let mut r = super::insert::picture(s, &json!({"data": src}))?;
            r["pastedAs"] = json!(fmt);
            return Ok(r);
        }
        "formatted" => s.clipboard.clone().ok_or_else(|| CmdError::Failed("the clipboard is empty".into()))?,
        "html" => adopt(s, import_html(&src)),
        _ => adopt(s, wordcraft_formats::rtf::import(src.as_bytes()).map_err(|e| CmdError::Failed(format!("bad RTF: {e}")))?),
    };
    insert(s, frag)?;
    let mut r = sel_result(s)?;
    r["pastedAs"] = json!(fmt);
    Ok(r)
}

/// Indent each side of a pasted `<blockquote>` whose page gives it no margins, points.
const BLOCKQUOTE_INDENT: f32 = 36.0;

/// Another program's HTML clipboard payload as a document. A `<blockquote>` there is how
/// LibreOffice (and many web pages) write a paragraph indented on both sides, not a quotation
/// (#1270): it pastes as a Normal paragraph indented by the page's `blockquote` margins, else half
/// an inch each side, rather than in the (centred, italic) Quote style. Opening an .html file keeps
/// `<blockquote>` as Quote, so HTML round-trips.
fn import_html(html: &str) -> Document {
    let mut doc = wordcraft_formats::html::import(html_fragment(html));
    let (left, right) = blockquote_margins(html);
    for b in &mut doc.body {
        each_para(Arc::make_mut(b), 0, &mut |p| {
            if p.props.style.as_deref() == Some("Quote") {
                p.props.style = None;
                p.props.indent_left.get_or_insert(left);
                p.props.indent_right.get_or_insert(right);
                p.touch();
            }
        });
    }
    doc
}

/// The left and right margins (points) the payload's `<style>` sheets give `blockquote` — LibreOffice
/// writes `blockquote { margin-left: 1cm; margin-right: 1cm }` — else [`BLOCKQUOTE_INDENT`].
fn blockquote_margins(html: &str) -> (f32, f32) {
    let len = |v: &str| wordcraft_formats::html::css_len_pt(v).map(|x| x.clamp(0.0, 1584.0));
    let (mut left, mut right) = (BLOCKQUOTE_INDENT, BLOCKQUOTE_INDENT);
    let lower = html.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("<style") {
        let tag = rest.get(at..).unwrap_or("");
        let sheet = tag.find('>').and_then(|j| tag.get(j + 1..)).unwrap_or("");
        let end = sheet.find("</style").unwrap_or(sheet.len());
        for rule in sheet.get(..end).unwrap_or("").split('}') {
            let Some((sel, decls)) = rule.split_once('{') else { continue };
            if !sel.split(',').any(|s| s.trim().trim_start_matches("<!--").trim() == "blockquote") {
                continue;
            }
            for d in decls.split(';') {
                let Some((name, v)) = d.split_once(':') else { continue };
                match name.trim() {
                    "margin-left" => left = len(v).unwrap_or(left),
                    "margin-right" => right = len(v).unwrap_or(right),
                    "margin" => {
                        let vals: Vec<&str> = v.split_whitespace().take(4).collect();
                        let (r, l) = match vals.as_slice() {
                            [a] => (*a, *a),
                            [_, h] | [_, h, _] => (*h, *h),
                            [_, r, _, l] => (*r, *l),
                            _ => continue,
                        };
                        right = len(r).unwrap_or(right);
                        left = len(l).unwrap_or(left);
                    }
                    _ => {}
                }
            }
        }
        rest = sheet.get(end..).unwrap_or("");
    }
    (left, right)
}

/// The HTML itself from a Windows "HTML Format" (CF_HTML) clipboard payload — a `Version:` header
/// giving byte offsets — or the text unchanged. Hostile offsets fall back to the text after the
/// header.
pub fn html_fragment(html: &str) -> &str {
    if !html.starts_with("Version:") {
        return html;
    }
    let offset = |key: &str| -> Option<usize> {
        let at = html.find(key)?.checked_add(key.len())?;
        let digits: &str = html.get(at..)?.split(['\r', '\n']).next()?;
        digits.trim().parse().ok()
    };
    if let (Some(a), Some(b)) = (offset("StartFragment:"), offset("EndFragment:"))
        && let Some(frag) = html.get(a..b)
    {
        return frag;
    }
    html.find('<').and_then(|i| html.get(i..)).unwrap_or("")
}

/// Whether an imported document has anything to paste: text, a table, or a picture it carries.
fn has_content(d: &Document) -> bool {
    d.body.iter().any(|b| match &**b {
        Block::Para(p) => !p.text.trim().is_empty() || (!p.objects.is_empty() && !d.media.is_empty()),
        Block::Table(_) => true,
    })
}

/// Another program's formatted copy, from the system clipboard's HTML, else its RTF (#687), as a
/// fragment of this document. `None` when neither is given, either is larger than
/// [`MAX_PASTE_BYTES`], or it holds nothing to paste (the caller pastes the text then).
pub fn rich_fragment(s: &mut Session, v: &Value) -> Option<Fragment> {
    let fits = |k: &str| p::str(v, k).filter(|t| !t.trim().is_empty() && t.len() <= MAX_PASTE_BYTES);
    let doc = fits("html").map(import_html);
    let doc = match doc.filter(has_content) {
        Some(mut d) => {
            drop_computed_left(&mut d);
            d
        }
        None => {
            let rtf = fits("rtf").filter(|t| looks_like_rtf(t))?;
            wordcraft_formats::rtf::import(rtf.as_bytes()).ok().filter(has_content)?
        }
    };
    Some(adopt(s, doc))
}

/// Browsers write the computed `text-align: left` (or `start`) on every paragraph they copy: that
/// is the default, not a choice, so it must not override a centered destination for all but the
/// first paragraph. Only HTML is treated so: in RTF `\ql` is a property the author set.
fn drop_computed_left(doc: &mut Document) {
    for b in &mut doc.body {
        each_para(Arc::make_mut(b), 0, &mut |p| {
            if p.props.align == Some(wordcraft_doc::Align::Left) {
                p.props.align = None;
            }
        });
    }
}

/// What a Paste brings in.
pub enum Pasted {
    /// WordCraft's own copy, or another program's HTML or RTF: it keeps the formatting it carries.
    Fragment(Fragment),
    /// The system clipboard's text, lines separated by `\n`.
    Text(String),
}

/// What Paste inserts: WordCraft's own copy while the system clipboard still holds it, else
/// another program's formatted copy (HTML, RTF), else the system clipboard's text.
pub fn paste_content(s: &mut Session, v: &Value) -> Result<Pasted, CmdError> {
    let text = p::str(v, "text").map(|t| t.replace("\r\n", "\n"));
    if let Some(f) = &s.clipboard
        && text.as_ref().is_none_or(|t| f.clipboard_text() == *t)
        && (text.is_some() || (p::str(v, "html").is_none() && p::str(v, "rtf").is_none()))
    {
        return Ok(Pasted::Fragment(f.clone()));
    }
    if let Some(f) = rich_fragment(s, v) {
        return Ok(Pasted::Fragment(f));
    }
    match text {
        Some(t) if !t.is_empty() => Ok(Pasted::Text(t)),
        _ => Err(CmdError::Failed("the clipboard is empty".into())),
    }
}

/// [`paste_content`] as a fragment. Text becomes paragraphs with no formatting of their own.
pub fn paste_fragment(s: &mut Session, v: &Value) -> Result<Fragment, CmdError> {
    Ok(match paste_content(s, v)? {
        Pasted::Fragment(f) => f,
        Pasted::Text(t) => Fragment::from_text(&t),
    })
}

/// Paste plain text the way Keep Text Only does, which is what Word does with text that carries
/// no formatting: the text takes the formatting before the caret, and each line after the first
/// starts a paragraph formatted like the one the text lands in (direction, style, alignment,
/// indents, list). Empty lines become empty paragraphs.
pub fn insert_text(s: &mut Session, text: &str) -> Result<(), CmdError> {
    // Only our own cut, pasted straight after it, becomes a move (see `insert`).
    s.pending_move = None;
    super::paste_text(s, &text.replace("\r\n", "\r").replace('\n', "\r"))
}

/// Merge Formatting (#687): the pasted text takes the formatting where it lands — the paragraph's
/// and the caret's — keeping only its emphasis (bold, italic, underline, strikethrough).
pub fn merge_formatting(s: &Session, frag: &mut Fragment) {
    let (base, para) = destination_formatting(s);
    merge_into(frag, &base, &para, &s.doc.styles);
}

/// The formatting a paste lands in: the caret's character formatting and its paragraph's.
fn destination_formatting(s: &Session) -> (wordcraft_doc::CharProps, wordcraft_doc::ParaProps) {
    let mut base = s.typing_props();
    base.ins = None;
    base.del = None;
    base.fmt_change = None;
    let (start, _) = s.sel.ordered();
    let para = s.doc.para_at(&start).map(|p| p.props.clone()).unwrap_or_default();
    (base, para)
}

fn merge_into(frag: &mut Fragment, base: &wordcraft_doc::CharProps, para: &wordcraft_doc::ParaProps, styles: &wordcraft_doc::styles::StyleSheet) {
    // Emphasis a paragraph style gives (a heading's bold) is emphasis too.
    let style_flag = |id: Option<&str>, get: fn(&wordcraft_doc::CharProps) -> Option<bool>| {
        id.map(|id| styles.chain(id)).and_then(|chain| chain.iter().rev().find_map(|st| get(&st.chr))).unwrap_or(false)
    };
    for b in &mut frag.blocks {
        each_para(b, 0, &mut |p| {
            let source = std::mem::replace(&mut p.props, para.clone());
            let (style_bold, style_italic) = (style_flag(source.style.as_deref(), |c| c.bold), style_flag(source.style.as_deref(), |c| c.italic));
            // A list stays a list, at its levels: bullets, numbers and their indents are structure.
            if source.numbering.is_some() {
                p.props.numbering = source.numbering;
                p.props.indent_left = source.indent_left;
                p.props.indent_first = source.indent_first;
            }
            for run in &mut p.runs {
                let src = std::mem::replace(&mut run.props, base.clone());
                if src.bold == Some(true) || (src.bold.is_none() && style_bold) {
                    run.props.bold = Some(true);
                }
                if src.italic == Some(true) || (src.italic.is_none() && style_italic) {
                    run.props.italic = Some(true);
                }
                if src.underline.is_some_and(|u| u != wordcraft_doc::props::Underline::None) {
                    run.props.underline = src.underline;
                }
                if src.strike == Some(true) {
                    run.props.strike = Some(true);
                }
                run.props.link = src.link;
                // Tracked changes are not formatting: text deleted in the source stays deleted.
                run.props.ins = src.ins;
                run.props.del = src.del;
            }
            p.touch();
        });
    }
}

/// The paste just made, kept so the Paste Options button (#1747) can redo it another way in
/// place: where it went, what the source was, and the document revision right after it.
#[derive(Clone, Debug)]
pub struct LastPaste {
    pub start: wordcraft_doc::Pos,
    pub end: wordcraft_doc::Pos,
    pub source: Fragment,
    /// The formatting the paste landed in, from before it: what Merge Formatting and Keep Text
    /// Only take (the document start would otherwise read the pasted text's own).
    pub dest: (wordcraft_doc::CharProps, wordcraft_doc::ParaProps),
    /// The option the paste is in now (one of [`PASTE_OPTIONS`]'s ids).
    pub mode: &'static str,
    /// Set once the command that pasted has finished (the session fills it in).
    pub rev: Option<u64>,
}

/// How a paste can be changed afterwards: (id, label), in the menu's order.
pub const PASTE_OPTIONS: [(&str, &str); 4] =
    [("source", "Keep Source Formatting"), ("text", "Keep Text Only"), ("merge", "Merge Formatting"), ("picture", "Picture")];

/// Pixels per point of the picture a paste is turned into (twice screen size, so it stays sharp).
const PICTURE_SCALE: f32 = 2.0;
/// Why Picture is refused for long content: it is shown to the person, so it says what to do.
const TOO_LONG_FOR_PICTURE: &str =
    "the pasted content is too long to turn into a picture (more than two pages tall): choose another option, or paste less";
/// Tallest content turned into a picture, in points (the raster's side is limited).
const PICTURE_MAX_H: f32 = 7000.0;

/// The pasted content drawn as a PNG, cropped to what is on it, with its size (width, height) in
/// points. The destination's styles and media render it, so pictures and styled text look as they
/// did.
fn fragment_picture(s: &Session, frag: &Fragment) -> Result<(Vec<u8>, f32, f32), CmdError> {
    let mut d = s.doc.clone();
    d.body = frag.blocks.iter().cloned().map(Arc::new).collect();
    d.last_section.landscape = false;
    let width = d.sections().first().map(|(_, sp)| sp.text_width()).filter(|w| w.is_finite() && *w > 10.0).unwrap_or(468.0).min(1500.0);
    let sect = &mut d.last_section;
    sect.page_w = width;
    sect.page_h = PICTURE_MAX_H;
    sect.margin_top = 0.0;
    sect.margin_bottom = 0.0;
    sect.margin_left = 0.0;
    sect.margin_right = 0.0;
    sect.header = 0.0;
    sect.footer = 0.0;
    sect.gutter = 0.0;
    sect.headers = Default::default();
    sect.footers = Default::default();
    let layout = wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
    let page = layout.pages.first().ok_or_else(|| CmdError::Failed("nothing to turn into a picture".into()))?;
    // More than one page of it can't be drawn whole: refuse rather than leave the rest out.
    if layout.pages.len() > 1 {
        return Err(CmdError::Failed(TOO_LONG_FOR_PICTURE.into()));
    }
    // Draw only as much page as the content takes, not the whole tall one it was laid out on.
    let used = page.content_bottom();
    let layout = if used > 0.0 && used + 8.0 < PICTURE_MAX_H {
        d.last_section.page_h = (used + 8.0).ceil();
        wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &Default::default())
    } else {
        layout
    };
    if layout.pages.len() > 1 {
        return Err(CmdError::Failed(TOO_LONG_FOR_PICTURE.into()));
    }
    let page = layout.pages.first().ok_or_else(|| CmdError::Failed("nothing to turn into a picture".into()))?;
    let img = wordcraft_render::render_page(&d, page, PICTURE_SCALE, &Default::default());
    // Crop to the ink: everything that isn't paper, with a small margin.
    // The page's own corner: a table at the margin touches the top-left one.
    let paper = img.pixel(img.width.saturating_sub(1), img.height.saturating_sub(1));
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..img.height {
        for x in 0..img.width {
            if img.pixel(x, y) != paper {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        return Err(CmdError::Failed("nothing to turn into a picture".into()));
    }
    let pad = 4;
    let (x0, y0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
    let (x1, y1) = ((x1 + pad).min(img.width.saturating_sub(1)), (y1 + pad).min(img.height.saturating_sub(1)));
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut cropped = wordcraft_render::Rendered { width: w, height: h, pixels: Vec::with_capacity(w as usize * h as usize * 4) };
    for y in y0..=y1 {
        let row = (y as usize * img.width as usize + x0 as usize) * 4;
        match img.pixels.get(row..row + w as usize * 4) {
            Some(r) => cropped.pixels.extend_from_slice(r),
            None => return Err(CmdError::Failed("the picture could not be cropped".into())),
        }
    }
    let png = cropped.to_png();
    if png.is_empty() {
        return Err(CmdError::Failed("the picture could not be encoded".into()));
    }
    Ok((png, w as f32 / PICTURE_SCALE, h as f32 / PICTURE_SCALE))
}

/// The widest the picture may be so that it also fits the page's text area in height: its width
/// when it already does, else scaled down in proportion. Content more than twice a page tall would
/// shrink to nothing legible, so it is refused instead.
fn fit_to_page_height(s: &Session, width: f32, height: f32) -> Result<f32, CmdError> {
    let page_h = s.doc.sections().first().map(|(_, sp)| sp.text_height()).filter(|h| h.is_finite() && *h > 10.0).unwrap_or(648.0);
    if height <= page_h {
        return Ok(width);
    }
    if height > page_h * 2.0 {
        return Err(CmdError::Failed(TOO_LONG_FOR_PICTURE.into()));
    }
    Ok(width * page_h / height)
}

/// The fragment's words as plain paragraphs, the way Keep Text Only pastes them: a table row is
/// one paragraph with its cells' text separated by commas.
fn text_only(frag: &Fragment) -> Fragment {
    fn cell_text(cell: &wordcraft_doc::Cell) -> String {
        let doc = Fragment { blocks: cell.blocks.iter().map(|b| (**b).clone()).collect(), parts: Default::default() };
        doc.plain_text().split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
    }
    fn lines(b: &Block, out: &mut Vec<String>, depth: usize) {
        match b {
            Block::Table(t) if depth < MAX_DEPTH => {
                for row in &t.rows {
                    out.push(row.cells.iter().map(cell_text).collect::<Vec<_>>().join(", "));
                }
            }
            other => {
                let one = Fragment { blocks: vec![other.clone()], parts: Default::default() };
                out.extend(one.plain_text().split('\n').map(str::to_string));
            }
        }
    }
    let mut out = Vec::new();
    for b in &frag.blocks {
        lines(b, &mut out, 0);
    }
    // The empty paragraph a table needs after it isn't text.
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    Fragment::from_text(&out.join("\n"))
}

/// Whether `frag` can be pasted more than one way: plain text has nothing to choose between.
fn has_choices(frag: &Fragment) -> bool {
    Fragment::from_text(&frag.plain_text()).blocks != frag.blocks
}

/// The paste options available now: empty unless the caret is right after a paste that had a
/// choice and nothing has changed since.
pub fn paste_options(s: &Session) -> Vec<&'static str> {
    match &s.last_paste {
        Some(lp) if lp.rev == Some(s.rev()) && s.sel == Selection::caret(lp.end.clone()) => PASTE_OPTIONS.iter().map(|o| o.0).collect(),
        _ => Vec::new(),
    }
}

/// The option the current paste is in, while the offer stands (`source` right after a paste).
pub fn paste_mode(s: &Session) -> Option<&'static str> {
    if paste_options(s).is_empty() { None } else { s.last_paste.as_ref().map(|lp| lp.mode) }
}

/// Redo the last paste in place with the chosen option (`source`, `merge` or `text`).
pub fn change_paste(s: &mut Session, v: &Value) -> CmdResult {
    let mode = p::str(v, "mode").unwrap_or("source");
    if !PASTE_OPTIONS.iter().any(|o| o.0 == mode) {
        return Err(CmdError::Failed(format!("unknown paste option `{mode}`")));
    }
    if paste_options(s).is_empty() {
        return Err(CmdError::Disabled("there is no paste to change".into()));
    }
    let Some(lp) = s.last_paste.clone() else { return Err(CmdError::Disabled("there is no paste to change".into())) };
    if mode == "picture" {
        let (png, width, height) = fragment_picture(s, &lp.source)?;
        let width = fit_to_page_height(s, width, height)?;
        super::controls::prepare_edit(s)?;
        s.sel = Selection { anchor: lp.start.clone(), focus: lp.end.clone() };
        let at = delete_selection(s)?;
        // A paste at a paragraph's start restyled that paragraph: it goes back to the destination's.
        if at.off == 0
            && let Ok(para) = s.doc.para_mut(at.story, &at.path)
        {
            para.props = lp.dest.1.clone();
            para.touch();
        }
        s.sel = Selection::caret(at);
        let r = super::insert::place_picture(s, png, Some(width), "Pasted content")?;
        // The picture is still a paste: the options stay, so it can be turned back.
        s.last_paste = Some(LastPaste { start: lp.start, end: s.sel.focus.clone(), source: lp.source, dest: lp.dest, mode: "picture", rev: None });
        return Ok(r);
    }
    let mut frag = lp.source.clone();
    if mode != "source" {
        // The destination's formatting is what is there without the paste: at the start of a
        // document the caret would otherwise read the pasted text's own.
        super::controls::prepare_edit(s)?;
        s.sel = Selection { anchor: lp.start.clone(), focus: lp.end.clone() };
        let at = delete_selection(s)?;
        // A paste at a paragraph's start gave that paragraph the source's properties (a heading's
        // style): what is left of it goes back to the destination's.
        if at.off == 0
            && let Ok(para) = s.doc.para_mut(at.story, &at.path)
        {
            para.props = lp.dest.1.clone();
            para.touch();
        }
        s.sel = Selection::caret(at);
        if mode == "text" {
            frag = text_only(&frag);
        }
        merge_into(&mut frag, &lp.dest.0, &lp.dest.1, &s.doc.styles);
        let source = lp.source.clone();
        insert_from(s, frag, Some(source))?;
        if let Some(now) = s.last_paste.as_mut() {
            now.mode = if mode == "text" { "text" } else { "merge" };
            now.dest = lp.dest;
        }
        return sel_result(s);
    }
    // Back to the source: take the earlier paste out, give the paragraph it restyled back its own
    // properties, and paste the source again from that clean spot.
    super::controls::prepare_edit(s)?;
    s.sel = Selection { anchor: lp.start.clone(), focus: lp.end.clone() };
    let at = delete_selection(s)?;
    if at.off == 0
        && let Ok(para) = s.doc.para_mut(at.story, &at.path)
    {
        para.props = lp.dest.1.clone();
        para.touch();
    }
    s.sel = Selection::caret(at);
    insert_from(s, frag, Some(lp.source))?;
    if let Some(now) = s.last_paste.as_mut() {
        now.mode = "source";
        // The destination is what the first paste landed in, not the pasted text now there.
        now.dest = lp.dest;
    }
    sel_result(s)
}

/// Replace the selection with `frag`, marking it as an insertion under Track Changes. This is a
/// paste: the Paste Options button is offered after it when it had a choice.
pub fn insert(s: &mut Session, frag: Fragment) -> Result<(), CmdError> {
    insert_from(s, frag, None)
}

/// [`insert`] for content that is not a paste (Copy To): the fragment goes in as it is, and no
/// Paste Options are offered.
pub fn insert_plain(s: &mut Session, frag: Fragment) -> Result<(), CmdError> {
    insert_inner(s, frag, None, false)
}

/// [`insert`], remembering `source` (the fragment before Merge Formatting changed it) as what a
/// later Paste Options choice starts from.
pub fn insert_from(s: &mut Session, frag: Fragment, source: Option<Fragment>) -> Result<(), CmdError> {
    insert_inner(s, frag, source, true)
}

fn insert_inner(s: &mut Session, mut frag: Fragment, source: Option<Fragment>, paste: bool) -> Result<(), CmdError> {
    super::controls::prepare_edit(s)?;
    let source = source.unwrap_or_else(|| frag.clone());
    let dest = destination_formatting(s);
    // Plain text has no paragraph formatting of its own: every paragraph it makes takes the
    // destination's (a centered paragraph stays centered through all of them), as in Word.
    if !paste {
        // Not a paste: keep the paragraphs as they are.
    } else if !has_choices(&frag) {
        for b in &mut frag.blocks {
            each_para(b, 0, &mut |p| p.props = dest.1.clone());
        }
    } else {
        // Formatted paragraphs that state no alignment, indent or spacing take the destination's
        // too: the first paragraph joins the destination's, and the rest must not fall back to
        // the style while it is centered. Paragraphs inside a table cell keep their own.
        let d = &dest.1;
        for b in &mut frag.blocks {
            if let Block::Para(p) = b {
                p.props.align = p.props.align.or(d.align);
                if p.props.numbering.is_none() {
                    p.props.indent_left = p.props.indent_left.or(d.indent_left);
                    p.props.indent_right = p.props.indent_right.or(d.indent_right);
                    p.props.indent_first = p.props.indent_first.or(d.indent_first);
                }
                p.props.space_before = p.props.space_before.or(d.space_before);
                p.props.space_after = p.props.space_after.or(d.space_after);
                p.props.line_spacing = p.props.line_spacing.or(d.line_spacing);
                p.touch();
            }
        }
    }
    let at = delete_selection(s)?;
    let track = s.doc.settings.track_changes;
    let mut rid = None;
    if track {
        // What the last command cut, pasted again: the two ends of a move.
        let r = match super::moves::take_paste(s, &frag) {
            Some(r) => r,
            None => super::new_revision(s, wordcraft_doc::RevisionKind::Insert),
        };
        rid = Some(r);
        for b in &mut frag.blocks {
            each_para(b, 0, &mut |p| {
                for run in &mut p.runs {
                    // Text deleted in the source stays deleted: inserted, then deleted.
                    run.props.ins = Some(r);
                    run.props.fmt_change = None;
                }
                p.mark.ins = None;
                p.mark.del = None;
            });
        }
    }
    let end = s.doc.insert_fragment(&at, &frag)?;
    // The paragraph marks the paste added are inserted too, so rejecting it joins the
    // paragraphs again. The last paragraph ends with the original mark and keeps it.
    if let Some(r) = rid
        && at.story == end.story
        && at.path.parent() == end.path.parent()
    {
        for i in at.path.last()..end.path.last() {
            if let Ok(p) = s.doc.para_mut(at.story, &at.path.with_last(i)) {
                p.mark.ins = Some(r);
                p.mark.del = None;
                p.touch();
            }
        }
    }
    s.last_paste =
        (paste && has_choices(&source)).then(|| LastPaste { start: at.clone(), end: end.clone(), source, dest, mode: "source", rev: None });
    s.sel = Selection::caret(end);
    Ok(())
}

/// Every paragraph in a block, tables included (bounded depth).
pub(crate) fn each_para(b: &mut Block, depth: usize, f: &mut dyn FnMut(&mut Paragraph)) {
    if depth > MAX_DEPTH {
        return;
    }
    match b {
        Block::Para(p) => f(p),
        Block::Table(t) => {
            for row in &mut t.rows {
                for cell in &mut row.cells {
                    for cb in &mut cell.blocks {
                        each_para(Arc::make_mut(cb), depth + 1, f);
                    }
                }
            }
        }
    }
}

/// Point a tracked formatting change at its renumbered revision (dropped when it has none).
fn remap_change<T>(c: &mut Option<Box<wordcraft_doc::props::PropChange<T>>>, rev: &dyn Fn(&mut Option<u32>)) {
    let mut r = c.as_ref().map(|c| c.rev);
    rev(&mut r);
    match (r, c.as_mut()) {
        (Some(v), Some(x)) => x.rev = v,
        _ => *c = None,
    }
}

/// [`remap_change`] for the table, row and cell formatting changes in `b` (bounded depth).
fn remap_table_changes(b: &mut Block, depth: usize, rev: &dyn Fn(&mut Option<u32>)) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Block::Table(t) = b {
        remap_change(&mut t.props.fmt_change, rev);
        for row in &mut t.rows {
            remap_change(&mut row.props.fmt_change, rev);
            for cell in &mut row.cells {
                remap_change(&mut cell.props.fmt_change, rev);
                for cb in &mut cell.blocks {
                    remap_table_changes(Arc::make_mut(cb), depth + 1, rev);
                }
            }
        }
    }
}

/// Section breaks brought in from another document keep their page, but their header and
/// footer ids are that document's: clear them (and the different first page that went with
/// them) so the sections link to the previous section's headers and footers, as in Word.
pub(crate) fn unlink_sections(blocks: &mut [Block]) {
    for b in blocks {
        each_para(b, 0, &mut |p| {
            if let Some(sec) = p.section.as_deref_mut() {
                sec.headers = Default::default();
                sec.footers = Default::default();
                sec.title_page = false;
            }
        });
    }
}

/// Turn an imported document into a fragment of the session's document: bring over the media,
/// lists and styles it refers to (renumbered so they can't collide with ours).
fn adopt(s: &mut Session, src: Document) -> Fragment {
    let mut media = BTreeMap::new();
    for (key, data) in &src.media {
        let ext = key.rsplit_once('.').map(|(_, e)| e).unwrap_or("png");
        media.insert(key.clone(), s.doc.add_media(data.to_vec(), ext));
    }
    let next_abs = s.doc.numbering.abstracts.iter().map(|a| a.id.saturating_add(1)).max().unwrap_or(0);
    let mut abs_map = BTreeMap::new();
    for (i, a) in src.numbering.abstracts.iter().enumerate() {
        let id = next_abs.saturating_add(i as u32);
        abs_map.insert(a.id, id);
        s.doc.numbering.abstracts.push(wordcraft_doc::numbering::AbstractNum { id, ..a.clone() });
    }
    let next_num = s.doc.numbering.nums.iter().map(|n| n.id.saturating_add(1)).max().unwrap_or(1).max(1);
    let mut num_map = BTreeMap::new();
    for (i, n) in src.numbering.nums.iter().enumerate() {
        let id = next_num.saturating_add(i as u32);
        num_map.insert(n.id, id);
        let abstract_id = abs_map.get(&n.abstract_id).copied().unwrap_or(n.abstract_id);
        s.doc.numbering.nums.push(wordcraft_doc::numbering::Num { id, abstract_id, ..n.clone() });
    }
    for st in &src.styles.styles {
        if s.doc.styles.get(&st.id).is_none() {
            s.doc.styles.upsert(st.clone());
        }
    }
    // Tracked changes the source carries (RTF revisions) point at its own revisions: add them
    // after ours and renumber.
    let rev_base = u32::try_from(s.doc.revisions.len()).unwrap_or(u32::MAX);
    let src_revs = src.revisions.len();
    s.doc.revisions.extend(src.revisions.iter().cloned());
    let rev = |i: &mut Option<u32>| {
        *i = i.filter(|v| (*v as usize) < src_revs).and_then(|v| rev_base.checked_add(v));
    };
    let mut blocks: Vec<Block> = src.body.iter().map(|b| (**b).clone()).collect();
    // Pasted pictures fit our text width, like Insert › Pictures (layout keeps their size).
    let max_w = crate::io::text_width(&s.doc);
    for b in &mut blocks {
        crate::io::fit_block_pictures(b, max_w);
    }
    for b in &mut blocks {
        each_para(b, 0, &mut |p| {
            if let Some(n) = p.props.numbering.as_mut()
                && n.num != 0
            {
                n.num = num_map.get(&n.num).copied().unwrap_or(0);
            }
            for r in &mut p.runs {
                rev(&mut r.props.ins);
                rev(&mut r.props.del);
                remap_change(&mut r.props.fmt_change, &rev);
            }
            rev(&mut p.mark.ins);
            rev(&mut p.mark.del);
            remap_change(&mut p.mark.fmt_change, &rev);
            remap_change(&mut p.props.fmt_change, &rev);
            let mut n = p.props.num_change.as_ref().map(|c| c.rev);
            rev(&mut n);
            match (n, p.props.num_change.as_mut()) {
                (Some(v), Some(c)) => c.rev = v,
                _ => p.props.num_change = None,
            }
            for o in &mut p.objects {
                if let wordcraft_doc::InlineObject::Image { media: key, .. } = o
                    && let Some(k) = media.get(key.as_str())
                {
                    *key = k.clone();
                }
            }
        });
    }
    for b in &mut blocks {
        remap_table_changes(b, 0, &rev);
    }
    unlink_sections(&mut blocks);
    // Text box stories the imported shapes show; pasting gives each a new story of its own.
    let parts = src.parts.into_iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::TextBox).collect();
    Fragment { blocks, parts }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table at the margin touches the picture's corner: the crop must still be the table's own
    /// size, not the whole tall page it was drawn on.
    #[test]
    fn a_table_becomes_a_picture_cropped_to_the_table() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let html = "<table border=\"1\"><tr><th>Item</th><th>Qty</th></tr><tr><td>Pencil</td><td>12</td></tr></table>";
        s.run("edit.paste", &json!({"text": "Item\tQty\nPencil\t12", "html": html})).unwrap();
        let source = s.last_paste.as_ref().map(|lp| lp.source.clone()).unwrap();
        let (png, width_pt, _) = fragment_picture(&s, &source).unwrap();
        let (w, h) = wordcraft_render::image_size(&png).unwrap();
        assert!(h < 400, "{w}×{h}px: cropped to the table, not the page");
        assert!(width_pt > 20.0 && width_pt < 500.0, "{width_pt}pt");
    }

    /// More than a page of content isn't drawn with the rest left out: it is refused.
    #[test]
    fn content_taller_than_the_picture_page_is_refused() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let html: String = (0..900).map(|i| format!("<p><b>line {i}</b></p>")).collect();
        let text: String = (0..900).map(|i| format!("line {i}\n")).collect();
        s.run("edit.paste", &json!({"text": text, "html": html})).unwrap();
        let source = s.last_paste.as_ref().map(|lp| lp.source.clone()).unwrap();
        let err = fragment_picture(&s, &source).unwrap_err();
        assert!(err.to_string().contains("too long"), "{err}");
    }

    /// A centered destination gives top-level pasted paragraphs its alignment, but the paragraphs
    /// inside a pasted table's cells keep their own.
    #[test]
    fn table_cell_paragraphs_do_not_take_the_destination_alignment() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("para.alignCenter", &json!({})).unwrap();
        let html = "<p><b>Top</b></p><table><tr><td>Cell</td></tr></table>";
        s.run("edit.paste", &json!({"text": "Top\nCell", "html": html})).unwrap();
        let mut aligns = Vec::new();
        for b in &s.doc.body {
            match &**b {
                Block::Para(p) => aligns.push(("para", p.props.align)),
                Block::Table(t) => {
                    for row in &t.rows {
                        for cell in &row.cells {
                            for cb in &cell.blocks {
                                if let Block::Para(p) = &**cb {
                                    aligns.push(("cell", p.props.align));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(aligns.iter().any(|(k, a)| *k == "para" && *a == Some(wordcraft_doc::Align::Center)), "{aligns:?}");
        assert!(aligns.iter().filter(|(k, _)| *k == "cell").all(|(_, a)| *a != Some(wordcraft_doc::Align::Center)), "{aligns:?}");
    }

    /// The offer does not depend on how deep the undo history is: with the history full (steps
    /// evicted, its length constant) a paste still offers Paste Options, and changing it works.
    #[test]
    fn the_offer_stands_when_the_undo_history_is_full() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        for i in 0..520 {
            s.run("text.insert", &json!({"text": "x"})).unwrap();
            s.run("format.bold", &json!({"value": i % 2 == 0})).unwrap();
        }
        assert!(s.undo_steps_so_far() > 500, "steps were evicted: {}", s.undo_steps_so_far());
        let html = "<p><b>Hello</b> <i>world</i></p><p>Second</p>";
        s.run("edit.paste", &json!({"text": "Hello world\nSecond", "html": html})).unwrap();
        assert!(!paste_options(&s).is_empty(), "the offer shows with a full history");
        s.run("edit.pasteChange", &json!({"mode": "text"})).unwrap();
        assert_eq!(paste_mode(&s), Some("text"));
        s.run("edit.pasteChange", &json!({"mode": "source"})).unwrap();
        assert_eq!(paste_mode(&s), Some("source"));
    }

    /// Copying something else after a paste does not spoil changing it: the offer keeps the
    /// pasted content itself, not the clipboard's.
    #[test]
    fn changing_a_paste_survives_a_copy_made_after_it() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let html = "<p><b>Bold start</b> tail</p><p>Two</p>";
        s.run("edit.paste", &json!({"text": "Bold start tail\nTwo", "html": html})).unwrap();
        let end = s.sel.clone();
        // Something else is copied while the offer stands: the caret and the document stay.
        s.run("select.all", &json!({})).unwrap();
        s.run("edit.copy", &json!({})).unwrap();
        s.sel = end;
        assert!(!paste_options(&s).is_empty(), "copying does not end the offer");
        s.run("edit.pasteChange", &json!({"mode": "merge"})).unwrap();
        let text = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(text.contains("Bold start tail") && text.contains("Two"), "{text:?}");
        s.run("edit.pasteChange", &json!({"mode": "source"})).unwrap();
        let first = s.doc.para_at(&s.doc.start_of(wordcraft_doc::StoryRef::Body)).unwrap();
        assert!(first.runs.iter().any(|r| r.props.bold == Some(true)), "the source's bold is back: {:?}", first.runs);
    }

    /// `\ql` in pasted RTF is a left alignment the author chose: it is kept in a centered
    /// destination. Only a browser's computed `text-align: left` is dropped.
    #[test]
    fn rtf_left_alignment_is_kept_in_a_centered_destination() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("para.alignCenter", &json!({})).unwrap();
        let rtf = "{\\rtf1\\ansi\\pard\\ql First\\par\\pard\\ql Second\\par}";
        s.run("edit.paste", &json!({"text": "First\nSecond", "rtf": rtf})).unwrap();
        let aligns: Vec<_> = s.doc.body.iter().filter_map(|b| if let Block::Para(p) = &**b { Some(p.props.align) } else { None }).collect();
        // The first paragraph joins the destination's; the second keeps the \ql the source set.
        assert_eq!(aligns.last(), Some(&Some(wordcraft_doc::Align::Left)), "{aligns:?}");
        // The same text from a browser (computed left) takes the destination's alignment.
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("para.alignCenter", &json!({})).unwrap();
        let html = "<p style=\"text-align:left\">First</p><p style=\"text-align:left\">Second</p>";
        s.run("edit.paste", &json!({"text": "First\nSecond", "html": html})).unwrap();
        let aligns: Vec<_> = s.doc.body.iter().filter_map(|b| if let Block::Para(p) = &**b { Some(p.props.align) } else { None }).collect();
        assert!(aligns.iter().all(|a| *a == Some(wordcraft_doc::Align::Center)), "{aligns:?}");
    }

    /// The copy of the pasted content that Paste Options keeps is freed when the offer ends: the
    /// next edit drops it, but moving the caret away does not (the offer comes back at the end).
    #[test]
    fn the_pasted_copy_is_freed_when_the_offer_ends() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let html = "<p><b>Hello</b> world</p><p>Second</p>";
        s.run("edit.paste", &json!({"text": "Hello world\nSecond", "html": html})).unwrap();
        assert!(s.last_paste.is_some() && !paste_options(&s).is_empty());
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(s.last_paste.is_none(), "typing ends the offer and frees the copy");
        assert!(paste_options(&s).is_empty());
    }

    /// A picture taller than the page's text area is scaled to fit it, not left running off the
    /// page; content more than two pages tall is refused.
    #[test]
    fn a_tall_picture_fits_the_page() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let text_h = s.doc.sections().first().map(|(_, sp)| sp.text_height()).unwrap();
        let html: String = (0..45).map(|i| format!("<p><b>line {i}</b></p>")).collect();
        let text: String = (0..45).map(|i| format!("line {i}\n")).collect();
        s.run("edit.paste", &json!({"text": text, "html": html})).unwrap();
        let source = s.last_paste.as_ref().map(|lp| lp.source.clone()).unwrap();
        let (_, _, height) = fragment_picture(&s, &source).unwrap();
        assert!(height > text_h, "the test content is taller than the page: {height} vs {text_h}");
        s.run("edit.pasteChange", &json!({"mode": "picture"})).unwrap();
        let mut found = None;
        for b in &s.doc.body {
            if let Block::Para(p) = &**b {
                for o in &p.objects {
                    if let wordcraft_doc::InlineObject::Image { h, .. } = o {
                        found = Some(*h);
                    }
                }
            }
        }
        let h = found.expect("a picture was inserted");
        assert!(h <= text_h + 0.5, "the picture ({h} pt) fits the page's text height ({text_h} pt)");
        // Several pages of it would shrink to nothing legible: refused, and the paste stays.
        let mut s = Session::new(wordcraft_doc::Document::new());
        let html: String = (0..200).map(|i| format!("<p><b>line {i}</b></p>")).collect();
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        s.run("edit.paste", &json!({"text": text, "html": html})).unwrap();
        let err = s.run("edit.pasteChange", &json!({"mode": "picture"})).unwrap_err();
        assert!(err.to_string().contains("too long"), "{err}");
        assert!(!paste_options(&s).is_empty(), "the paste is unchanged and still offers its options");
    }

    /// Picture at the start of a document puts the paragraph the paste restyled back.
    #[test]
    fn a_picture_restores_the_destination_paragraph() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let html = "<h2 style=\"font-weight:bold\">Heading</h2><p>Body</p>";
        s.run("edit.paste", &json!({"text": "Heading\nBody", "html": html})).unwrap();
        s.run("edit.pasteChange", &json!({"mode": "picture"})).unwrap();
        let first = s.doc.para_at(&s.doc.start_of(wordcraft_doc::StoryRef::Body)).unwrap();
        assert!(first.props.style.is_none(), "{:?}", first.props.style);
    }
}
