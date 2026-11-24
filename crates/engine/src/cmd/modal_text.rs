//! Transactional text-buffer edits for an external modal editor. The document remains the
//! authority: offsets refer to a bounded projection, stale text is refused, and unchanged
//! runs, objects and table structure are never round-tripped through a plain-text importer.

use serde::Deserialize;
use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos, StoryRef};

use crate::{CmdError, CmdResult, CommandSpec, Selection, Session};

/// Maximum projected story, in UTF-8 bytes.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_EDITS: usize = 10_000;

/// A story's text and the native position at each projected character boundary. Paragraph
/// separators are LF; a soft line break is U+2028 so it cannot become a paragraph by accident.
pub struct Projection {
    pub text: String,
    spans: Vec<Span>,
}

struct Span {
    start: usize,
    end: usize,
    story: StoryRef,
    path: wordcraft_doc::Path,
    soft_breaks: Vec<(usize, usize)>,
}

impl Projection {
    pub fn new(doc: &Document, story: StoryRef) -> Result<Self, CmdError> {
        let mut text = String::new();
        let mut spans = Vec::new();
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            let projected_len = para
                .text
                .len()
                .checked_add(para.text.bytes().filter(|c| *c == b'\n').count().saturating_mul(2))
                .and_then(|n| n.checked_add(text.len()))
                .and_then(|n| n.checked_add(usize::from(!spans.is_empty())));
            if projected_len.is_none_or(|n| n > MAX_BYTES) {
                return Err(CmdError::Disabled("the story is too large for modal text editing".into()));
            }
            if !spans.is_empty() {
                text.push('\n');
            }
            let start = text.len();
            let mut soft_breaks = Vec::new();
            for (native, c) in para.text.char_indices() {
                if c == '\n' {
                    soft_breaks.push((native, text.len() - start));
                }
                text.push(if c == '\n' { '\u{2028}' } else { c });
            }
            spans.push(Span { start, end: text.len(), story, path, soft_breaks });
        }
        Ok(Self { text, spans })
    }

    pub fn pos(&self, offset: usize) -> Result<Pos, CmdError> {
        let index = self.spans.partition_point(|span| span.end < offset);
        let span = self.spans.get(index).filter(|span| (span.start..=span.end).contains(&offset));
        if let Some(span) = span
            && let Some(prefix) = self.text.get(span.start..offset)
        {
            let off = prefix.len() - span.soft_breaks.iter().filter(|(_, at)| *at < prefix.len()).count() * 2;
            return Ok(Pos { story: span.story, path: span.path.clone(), off });
        }
        Err(CmdError::Params("text offset is not a character boundary".into()))
    }

    pub fn offset(&self, pos: &Pos) -> Option<usize> {
        let index = self.spans.binary_search_by(|span| span.path.cmp(&pos.path)).ok()?;
        let span = self.spans.get(index).filter(|span| span.story == pos.story)?;
        let projected = pos.off.checked_add(span.soft_breaks.iter().filter(|(at, _)| *at < pos.off).count() * 2)?;
        let offset = span.start.checked_add(projected)?;
        (offset <= span.end && self.text.is_char_boundary(offset)).then_some(offset)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Patch {
    story: StoryRef,
    before: String,
    edits: Vec<Edit>,
    anchor: usize,
    focus: usize,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("text.applyBufferEdits", "Apply Text Buffer Edits", "Editing", apply)
            .params(r#"{story: StoryRef, before: string, edits: [{start: byte, end: byte, text: string}], anchor: byte, focus: byte}"#),
    ]
}

fn apply(s: &mut Session, value: &Value) -> CmdResult {
    let patch: Patch = serde_json::from_value(value.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if patch.before.len() > MAX_BYTES || patch.edits.len() > MAX_EDITS {
        return Err(CmdError::Params("text buffer edit limit exceeded".into()));
    }
    let projection = Projection::new(&s.doc, patch.story)?;
    if projection.text != patch.before {
        return Err(CmdError::Disabled("the document changed while the text buffer was being edited".into()));
    }
    let mut previous_end = 0;
    let mut expected = projection.text.clone();
    // Validate the entire patch before the first mutation. The normal command transaction also
    // rolls back any later protection/content-control refusal, including its undo history.
    let mut positions = Vec::with_capacity(patch.edits.len());
    for edit in &patch.edits {
        if edit.start < previous_end || edit.end < edit.start || edit.text.len() > MAX_BYTES {
            return Err(CmdError::Params("text edits must be bounded, ordered and disjoint".into()));
        }
        let a = projection.pos(edit.start)?;
        let b = projection.pos(edit.end)?;
        let first = projection.spans.partition_point(|span| span.path < a.path);
        let last = projection.spans.partition_point(|span| span.path <= b.path);
        let crosses_cell = projection.spans.get(first..last).is_none_or(|spans| spans.iter().any(|span| span.path.parent() != a.path.parent()));
        if a.path.parent() != b.path.parent() || crosses_cell {
            return Err(CmdError::Disabled("a text-buffer edit cannot join different table cells".into()));
        }
        // A text buffer cannot recreate an inline object, field or tracked-deleted run.
        // Refuse rather than turn it into text or silently discard its formatting on Undo.
        let old = projection.text.get(edit.start..edit.end).ok_or_else(|| CmdError::Params("invalid text range".into()))?;
        if old.contains(wordcraft_doc::para::OBJ) || edit.text.contains(wordcraft_doc::para::OBJ) {
            return Err(CmdError::Disabled("edit embedded objects in the document view".into()));
        }
        previous_end = edit.end;
        positions.push((a, b));
    }
    for edit in patch.edits.iter().rev() {
        expected.replace_range(edit.start..edit.end, &edit.text);
        if expected.len() > MAX_BYTES {
            return Err(CmdError::Params("the edited text buffer is too large".into()));
        }
    }
    let typing_at = s.sel.is_collapsed().then(|| s.sel.focus.clone());
    let typing_props = s.pending.clone();
    for (edit, (anchor, focus)) in patch.edits.iter().zip(positions).rev() {
        let typed_at_caret = anchor == focus && typing_at.as_ref() == Some(&anchor);
        s.sel = Selection { anchor, focus };
        s.pending = if typed_at_caret { typing_props.clone() } else { None };
        // LF in the projection is a paragraph; U+2028 remains a soft line break. Paste's
        // low-level path keeps list/paragraph properties and skips AutoCorrect.
        let text = edit.text.replace('\n', "\r").replace('\u{2028}', "\n");
        super::paste_text(s, &text)?;
    }
    let after = Projection::new(&s.doc, patch.story)?;
    if after.text != expected {
        return Err(CmdError::Disabled("the document could not apply the text edit without changing its structure".into()));
    }
    s.set_selection(Selection { anchor: after.pos(patch.anchor)?, focus: after.pos(patch.focus)? });
    Ok(json!({"text": after.text}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_edits_preserve_untouched_formatting_and_undo_as_one_step() {
        let mut s = Session::new(Document::from_text("one MIDDLE three"));
        s.run("select.range", &json!({"anchor": {"block": 0,"off":4},"focus":{"block":0,"off":10}})).unwrap();
        s.run("format.bold", &json!({})).unwrap();
        let original = s.doc.clone();
        s.run(
            "text.applyBufferEdits",
            &json!({"story":"body","before":"one MIDDLE three",
            "edits":[{"start":0,"end":3,"text":"1"},{"start":11,"end":16,"text":"3"}],"anchor":1,"focus":1}),
        )
        .unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body), "1 MIDDLE 3");
        let middle = s.doc.para_at(&Pos::body(0, 2)).unwrap();
        assert_eq!(middle.props_of_char(2).bold, Some(true));
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.same_content(&original));
    }

    #[test]
    fn stale_or_invalid_patches_leave_document_and_history_unchanged() {
        let mut s = Session::new(Document::from_text("café"));
        let before = s.doc.clone();
        let depth = s.undo_depth();
        for (text, end) in [("stale", 1), ("café", 4)] {
            assert!(
                s.run(
                    "text.applyBufferEdits",
                    &json!({"story":"body","before":text,
                "edits":[{"start":0,"end":end,"text":"x"}],"anchor":0,"focus":0})
                )
                .is_err()
            );
            assert!(s.doc.same_content(&before));
            assert_eq!(s.undo_depth(), depth);
        }
    }
    #[test]
    fn text_edits_keep_tables_and_generated_embedded_images_through_docx_save() {
        use std::sync::Arc;
        use wordcraft_doc::{Block, Table};
        let mut doc = Document::from_text("body");
        let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([20, 40, 60, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels).write_to(&mut png, image::ImageFormat::Png).unwrap();
        doc.media.insert("generated.png".into(), Arc::new(png.into_inner()));
        let picture = wordcraft_doc::para::InlineObject::Image {
            media: "generated.png".into(),
            w: 12.0,
            h: 12.0,
            alt: "Generated test pixel".into(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        };
        doc.insert_object(&Pos::body(0, 4), picture, &Default::default()).unwrap();
        doc.body.push(Arc::new(Block::Table(Table::new(1, 2, 200.0))));
        let table = doc.body[1].clone();
        let picture = doc.para_at(&Pos::body(0, 0)).unwrap().objects.clone();
        let mut s = Session::new(doc);
        let before = Projection::new(&s.doc, StoryRef::Body).unwrap().text;
        s.run(
            "text.applyBufferEdits",
            &json!({"story":"body","before":before,
            "edits":[{"start":0,"end":4,"text":"changed"}],"anchor":0,"focus":0}),
        )
        .unwrap();
        assert_eq!(s.doc.body[1], table);
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().objects, picture);
        let bytes = wordcraft_docx::write(&s.doc).unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert!(matches!(reopened.body.get(1).map(Arc::as_ref), Some(Block::Table(t)) if t.rows.len()==1 && t.rows[0].cells.len()==2));
        assert!(reopened.media.values().any(|data| s.doc.media.values().any(|original| data == original)));
    }

    #[test]
    fn projection_maps_unicode_and_soft_breaks_to_native_byte_positions() {
        let mut doc = Document::from_text("café");
        doc.insert_text(&Pos::body(0, 5), "\n☕\u{2028}end", &Default::default()).unwrap();
        let projection = Projection::new(&doc, StoryRef::Body).unwrap();
        let para = doc.para_at(&Pos::body(0, 0)).unwrap();
        for off in para.text.char_indices().map(|(off, _)| off).chain(std::iter::once(para.text.len())) {
            let pos = Pos::body(0, off);
            assert_eq!(projection.pos(projection.offset(&pos).unwrap()).unwrap(), pos);
        }
    }
    #[test]
    fn insertion_at_the_native_caret_keeps_pending_ribbon_formatting() {
        let mut s = Session::new(Document::from_text("plain"));
        s.run("format.bold", &json!({})).unwrap();
        s.run(
            "text.applyBufferEdits",
            &json!({"story":"body","before":"plain",
            "edits":[{"start":0,"end":0,"text":"X"}],"anchor":1,"focus":1}),
        )
        .unwrap();
        let para = s.doc.para_at(&Pos::body(0, 0)).unwrap();
        assert_eq!(para.props_of_char(0).bold, Some(true));
        assert_ne!(para.props_of_char(1).bold, Some(true));
    }
}
