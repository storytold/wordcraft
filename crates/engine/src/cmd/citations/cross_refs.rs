use super::generated::page_of;
use super::{CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, Pos, Selection, Session, StoryRef, Value, delete_selection, json, p};

pub(super) fn cross_ref(s: &mut Session, v: &Value) -> CmdResult {
    let to = p::str(v, "to").unwrap_or("heading");
    let targets = ref_targets(s, to);
    let Some(target) = p::str(v, "target") else {
        return Ok(Value::Array(targets.iter().map(|(t, _)| json!(t)).collect()));
    };
    let find = |ts: Vec<(String, Pos)>| ts.into_iter().find(|(t, _)| t == target || t.starts_with(target));
    let (text, pos) = find(targets).ok_or_else(|| CmdError::Params(format!("no {to} `{target}`")))?;
    let show = p::str(v, "show").unwrap_or("text");
    let page = page_of(s, &pos);
    let result = match show {
        "page" => page.to_string(),
        "number" => text.split(':').next().unwrap_or(&text).trim().to_string(),
        "aboveBelow" => {
            if pos < s.sel.focus {
                "above".into()
            } else {
                "below".into()
            }
        }
        _ if to == "bookmark" => crate::cmd::references::bookmark_text(&s.doc, &text).unwrap_or_default(),
        _ => text.clone(),
    };
    let props = s.typing_props();
    let at = delete_selection(s)?;
    // The field points at a bookmark: the named one, or a hidden `_Ref` bookmark on the target
    // paragraph (as Word does), so Update Field and Word can resolve it.
    let (name, at) = if to == "bookmark" {
        (text, at)
    } else {
        // Deleting the selection can move paragraphs: look the target up again.
        let (_, pos) = find(ref_targets(s, to)).ok_or_else(|| CmdError::Params(format!("no {to} `{target}`")))?;
        ref_bookmark(s, &pos, show == "number", at)?
    };
    let instr = format!("{} {name} \\h", if show == "page" { "PAGEREF" } else { "REF" });
    let end = s.doc.insert_object(
        &at,
        InlineObject::Field { instr, result: result.clone(), locked: false, dirty: false, code: false, form: String::new() },
        &props,
    )?;
    s.sel = Selection::caret(end);
    Ok(json!({"inserted": result}))
}

/// The label of a caption: the text before its first field (`Plate ` in `Plate `, `SEQ Plate`,
/// `: A rock`). When the caption starts with its chapter number, the text between that field and
/// the `SEQ` number, unless it is only a separator. Failing both, the `SEQ` identifier names it.
/// A caption without a `SEQ` field (typed by hand) has what comes before its first digit.
pub(super) fn caption_label(p: &Paragraph, text: &str) -> Option<String> {
    let fields: Vec<(usize, &str)> = p
        .object_offsets()
        .into_iter()
        .filter_map(|o| match p.object_at(o) {
            Some(InlineObject::Field { instr, .. }) => Some((o, instr.as_str())),
            _ => None,
        })
        .collect();
    let seq = fields.iter().position(|(_, instr)| instr.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("SEQ")));
    let label = match seq.and_then(|i| Some((i, fields.get(i)?))) {
        Some((i, (seq_at, instr))) => {
            // The text in `from..to` with the bookmark marks dropped, if it says anything.
            let said = |from: usize, to: usize| {
                let t: String = p.text.get(from..to).unwrap_or_default().chars().filter(|c| *c != wordcraft_doc::para::OBJ).collect();
                let t = t.trim();
                t.chars().any(char::is_alphanumeric).then(|| t.to_string())
            };
            let first = fields.first().map_or(0, |(o, _)| *o);
            let after_previous =
                i.checked_sub(1).and_then(|k| fields.get(k)).map_or(0, |(o, _)| o.saturating_add(wordcraft_doc::para::OBJ.len_utf8()));
            said(0, first).or_else(|| said(after_previous, *seq_at)).or_else(|| instr.split_whitespace().nth(1).map(|id| id.replace('_', " ")))?
        }
        None => text.get(..text.find(|c: char| c.is_ascii_digit())?)?.trim().to_string(),
    };
    (!label.is_empty() && label.chars().count() <= 40).then_some(label)
}

/// Whether `p` is a caption: its style is Caption or one based on it, as a caption restyled with a
/// style of its own (a `Table Caption` based on Caption) still is (#4411).
pub(super) fn is_caption(doc: &Document, p: &Paragraph) -> bool {
    p.props.style.as_deref().is_some_and(|id| id == "Caption" || doc.styles.chain(id).iter().any(|s| s.id == "Caption"))
}

/// The caption paragraphs of the body: (text, position, label).
fn captions(s: &Session) -> Vec<(String, Pos, Option<String>)> {
    s.doc
        .para_paths(StoryRef::Body)
        .into_iter()
        .filter_map(|path| {
            let p = s.doc.para(StoryRef::Body, &path).filter(|p| is_caption(&s.doc, p))?;
            let text = p.plain_text();
            let label = caption_label(p, &text);
            Some((text, Pos { story: StoryRef::Body, path, off: 0 }, label))
        })
        .collect()
}

/// The labels the body's captions use, first seen first: Figure, Table and Equation, and any the
/// person made with Insert Caption › New Label. Each is a cross-reference type, `label:<Name>` (#2940).
pub fn caption_labels(s: &Session) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for l in captions(s).into_iter().filter_map(|(_, _, l)| l) {
        if !labels.contains(&l) {
            labels.push(l);
        }
    }
    labels
}

/// Cross-reference targets of a kind: (shown text or bookmark name, position).
pub fn ref_targets(s: &Session, to: &str) -> Vec<(String, Pos)> {
    let mut targets: Vec<(String, Pos)> = Vec::new();
    match to {
        "bookmark" => targets = s.doc.bookmarks(),
        // A caption label of the person's own: the captions that use exactly that label.
        t if t.starts_with("label:") => {
            let want = t.get("label:".len()..).unwrap_or_default();
            targets = captions(s)
                .into_iter()
                .filter(|(_, _, label)| !want.is_empty() && label.as_deref() == Some(want))
                .map(|(text, pos, _)| (text, pos))
                .collect();
        }
        "figure" | "table" | "equation" => {
            let label = match to {
                "table" => "Table",
                "equation" => "Equation",
                _ => "Figure",
            };
            for path in s.doc.para_paths(StoryRef::Body) {
                if let Some(p) = s.doc.para(StoryRef::Body, &path)
                    && is_caption(&s.doc, p)
                    && p.plain_text().starts_with(label)
                {
                    targets.push((p.plain_text(), Pos { story: StoryRef::Body, path: path.clone(), off: 0 }));
                }
            }
        }
        _ => {
            for path in s.doc.para_paths(StoryRef::Body) {
                if let Some(p) = s.doc.para(StoryRef::Body, &path)
                    && s.doc.styles.resolve_para(&p.props).outline_level.is_some()
                    && !p.plain_text().trim().is_empty()
                {
                    targets.push((p.plain_text().trim().to_string(), Pos { story: StoryRef::Body, path: path.clone(), off: 0 }));
                }
            }
        }
    }
    targets
}

/// The hidden `_Ref` bookmark around the target paragraph at `pos` (its label and number only
/// when `number_only`), reusing one already there. Returns its name and `at` moved past the
/// bookmark marks inserted before it.
fn ref_bookmark(s: &mut Session, pos: &Pos, number_only: bool, mut at: Pos) -> Result<(String, Pos), CmdError> {
    const MARK: usize = wordcraft_doc::para::OBJ.len_utf8();
    let para = s.doc.para(pos.story, &pos.path).ok_or_else(|| CmdError::Params("cross-reference target is gone".into()))?;
    let end = if number_only { para.text.find(':').unwrap_or(para.len()) } else { para.len() };
    if let Some(InlineObject::BookmarkStart { name }) = para.object_at(0)
        && name.starts_with("_Ref")
    {
        // Reuse it when it covers the same extent: up to the colon, or the whole paragraph.
        let same = para.object_offsets().into_iter().any(|o| {
            matches!(para.object_at(o), Some(InlineObject::BookmarkEnd { name: n }) if n == name)
                && para.text.get(o.saturating_add(MARK)..).is_some_and(|rest| if number_only { rest.starts_with(':') } else { rest.is_empty() })
        });
        if same {
            return Ok((name.clone(), at));
        }
    }
    let next =
        s.doc.bookmarks().iter().filter_map(|(n, _)| n.strip_prefix("_Ref").and_then(|d| d.parse::<u64>().ok())).max().unwrap_or(0).saturating_add(1);
    let name = format!("_Ref{next:09}");
    let props = CharProps::default();
    s.doc.insert_object(&Pos { off: end, ..pos.clone() }, InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&Pos { off: 0, ..pos.clone() }, InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    if at.story == pos.story && at.path == pos.path {
        if at.off >= end {
            at.off = at.off.saturating_add(2 * MARK);
        } else if at.off > 0 {
            at.off = at.off.saturating_add(MARK);
        }
    }
    Ok((name, at))
}
