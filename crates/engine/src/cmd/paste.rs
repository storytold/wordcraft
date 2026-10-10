//! Paste Special: paste the clipboard in a chosen format (formatted, RTF, HTML, unformatted).
//!
//! The clipboard can hold several representations of the same content. WordCraft's own copy keeps
//! a rich fragment; the system clipboard gives text, and callers (agents, the CLI, a front end
//! with a richer clipboard) may pass `html` or `rtf` too. Text that is itself RTF or HTML source
//! can also be pasted as that format.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Block, Document, Paragraph};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Largest clipboard payload Paste Special accepts, in bytes.
pub const MAX_PASTE_BYTES: usize = 16 << 20;
/// Nesting depth we follow when adjusting pasted tables.
const MAX_DEPTH: usize = 64;

/// The formats, in the order the dialog lists them: (id, label).
pub const FORMATS: [(&str, &str); 4] =
    [("formatted", "Formatted Text"), ("rtf", "Rich Text Format (RTF)"), ("html", "HTML Format"), ("text", "Unformatted Text")];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("edit.pasteSpecial", "Paste Special", "Home › Clipboard › Paste", paste_special)
            .key("Mod+Alt+V")
            .params(r#"{"as"?: "formatted"|"rtf"|"html"|"text", "text"?: string, "html"?: string, "rtf"?: string}"#),
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
                Some(t) if f.plain_text() != t.replace("\r\n", "\n") => None,
                _ => Some(String::new()),
            }
        }
        "rtf" => p::str(v, "rtf").map(str::to_string).or_else(|| clip_text(s, v).filter(|t| looks_like_rtf(t))).and_then(non_empty),
        "html" => p::str(v, "html").map(str::to_string).or_else(|| clip_text(s, v).filter(|t| looks_like_html(t))).and_then(non_empty),
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
    let ids = available(s, v);
    let Some(fmt) = p::str(v, "as") else {
        // No format chosen: list them and ask a front end (if any) to show the dialog.
        let mut req = json!({"open": "pasteSpecial", "formats": ids});
        for k in ["text", "html", "rtf"] {
            if let Some(t) = p::str(v, k) {
                req[k] = json!(t);
            }
        }
        s.ui_requests.push(req);
        return Ok(json!({"formats": formats_json(&ids)}));
    };
    let fmt = match fmt.to_ascii_lowercase().as_str() {
        "formatted" | "keep" | "wordcraft" => "formatted",
        "rtf" => "rtf",
        "html" => "html",
        "text" | "unformatted" | "plain" => "text",
        other => return Err(CmdError::Params(format!("unknown format `{other}` (formatted, rtf, html or text)"))),
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
            super::type_text(s, &src.replace('\n', "\r"))?;
            let mut r = sel_result(s)?;
            r["pastedAs"] = json!(fmt);
            return Ok(r);
        }
        "formatted" => s.clipboard.clone().ok_or_else(|| CmdError::Failed("the clipboard is empty".into()))?,
        "html" => adopt(s, wordcraft_formats::html::import(&src)),
        _ => adopt(s, wordcraft_formats::rtf::import(src.as_bytes()).map_err(|e| CmdError::Failed(format!("bad RTF: {e}")))?),
    };
    insert(s, frag)?;
    let mut r = sel_result(s)?;
    r["pastedAs"] = json!(fmt);
    Ok(r)
}

/// Replace the selection with `frag`, marking it as an insertion under Track Changes.
pub fn insert(s: &mut Session, mut frag: Fragment) -> Result<(), CmdError> {
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
                    run.props.ins = Some(r);
                    run.props.del = None;
                    run.props.fmt_change = None;
                }
                p.mark.ins = None;
                p.mark.del = None;
            });
        }
    }
    let end = s.doc.insert_fragment(&at, &frag)?;
    // The paragraph marks the paste added are inserted too.
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
fn each_para(b: &mut Block, depth: usize, f: &mut dyn FnMut(&mut Paragraph)) {
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
    let mut blocks: Vec<Block> = src.body.iter().map(|b| (**b).clone()).collect();
    for b in &mut blocks {
        each_para(b, 0, &mut |p| {
            if let Some(n) = p.props.numbering.as_mut()
                && n.num != 0
            {
                n.num = num_map.get(&n.num).copied().unwrap_or(0);
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
    // Text box stories the imported shapes show; pasting gives each a new story of its own.
    let parts = src.parts.into_iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::TextBox).collect();
    Fragment { blocks, parts }
}
