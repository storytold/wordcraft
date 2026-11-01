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
        Some(d) => d,
        None => {
            let rtf = fits("rtf").filter(|t| looks_like_rtf(t))?;
            wordcraft_formats::rtf::import(rtf.as_bytes()).ok().filter(has_content)?
        }
    };
    Some(adopt(s, doc))
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
    let mut base = s.typing_props();
    base.ins = None;
    base.del = None;
    base.fmt_change = None;
    let (start, _) = s.sel.ordered();
    let para = s.doc.para_at(&start).map(|p| p.props.clone()).unwrap_or_default();
    for b in &mut frag.blocks {
        each_para(b, 0, &mut |p| {
            p.props = para.clone();
            for run in &mut p.runs {
                let src = std::mem::replace(&mut run.props, base.clone());
                if src.bold == Some(true) {
                    run.props.bold = Some(true);
                }
                if src.italic == Some(true) {
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

/// Replace the selection with `frag`, marking it as an insertion under Track Changes.
pub fn insert(s: &mut Session, mut frag: Fragment) -> Result<(), CmdError> {
    super::controls::prepare_edit(s)?;
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
