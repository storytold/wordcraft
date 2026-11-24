//! The app-side edits a session carries, and how they land on the document.
//!
//! An [`Edit`] is the application's own intent, not a wire message: it names a session block id
//! ([`crate::ids`]) and carries the text or the range. Nothing here serialises, and nothing decodes:
//! the shared contract, when it lands, carries these values between processes; the adapter only
//! has to apply them, in the one order the relay sequenced them, to every replica alike.
//!
//! Every edit is applied through the document's own primitives
//! (`insert_text`, `delete_range`, `insert_block`, `remove_block`), so a block keeps its formatting,
//! its kept source markup and everything the model holds that this adapter does not name. That is
//! what lets an unmodelled drawing or a custom part survive a session round-trip: the adapter never
//! re-encodes a part it does not model.
//!
//! Each applied edit reports its inverse ([`Effect::inverse`]), which is what a per-user undo stack
//! stores. Undo itself stays where the collaboration branch put it; this increment only owes the
//! inverse, and only for the four edits it knows. These inverses restore text and nothing else: the
//! payloads carry no model fragment, so a removal's inverse re-creates a paragraph with default
//! properties and a deleted range's inverse re-inserts characters without the formatting they had.
//! The fragment-carrying inverse (`collab-doc` on the collaboration branch) is the next increment.

use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, Path, Pos, StoryRef};

use crate::ids::{BlockId, IdMap};

/// One edit of the body's top-level blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Insert `text` into the paragraph at byte `at`.
    InsertText { block: BlockId, at: usize, text: String },
    /// Delete the bytes `from..to` of the paragraph.
    DeleteRange { block: BlockId, from: usize, to: usize },
    /// Remove the paragraph and with it everything the model holds for it.
    RemoveParagraph { block: BlockId },
    /// Insert a new paragraph, carrying the id every replica will know it by.
    InsertParagraph { after: Option<BlockId>, id: BlockId, text: String },
}

impl Edit {
    /// The block whose text this edit changes, when it changes one.
    pub fn text_block(&self) -> Option<BlockId> {
        match self {
            Edit::InsertText { block, .. } | Edit::DeleteRange { block, .. } => Some(*block),
            Edit::RemoveParagraph { .. } | Edit::InsertParagraph { .. } => None,
        }
    }

    /// The structural edit's order is the relay's to keep: it does not commute with anything else
    /// that addresses the same container.
    pub fn is_structural(&self) -> bool {
        matches!(self, Edit::RemoveParagraph { .. } | Edit::InsertParagraph { .. })
    }

    /// The text the author typed, when the edit carries some.
    pub fn typed_text(&self) -> Option<&str> {
        match self {
            Edit::InsertText { text, .. } | Edit::InsertParagraph { text, .. } => Some(text),
            Edit::DeleteRange { .. } | Edit::RemoveParagraph { .. } => None,
        }
    }
}

/// Why an edit did not land. Every variant is decided before the document changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The block is gone: a structural edit that was sequenced first removed it.
    Gone,
    /// The edit addresses something this increment does not model (a table, a part).
    Unsupported,
    /// An offset past the paragraph or inside a character.
    BadOffset,
    /// The paragraph is the body's last one, which a document keeps.
    LastParagraph,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EditError::Gone => "the block is gone",
            EditError::Unsupported => "the block is not a paragraph this adapter models",
            EditError::BadOffset => "the offset is past the paragraph or inside a character",
            EditError::LastParagraph => "a document keeps its last paragraph",
        })
    }
}

/// What applying an edit produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    /// The edit that takes this one back, for the per-user undo stack.
    ///
    /// It restores text only. [`Edit::RemoveParagraph`]'s inverse re-creates the paragraph with
    /// default paragraph and character properties and without the non-text content the removal
    /// dropped; [`Edit::DeleteRange`]'s inverse re-inserts the characters without their formatting.
    /// A test that checks a restoration checks the text, not the block as it was.
    pub inverse: Edit,
    /// The blocks whose content the edit touched, including the one it created.
    pub touched: Vec<BlockId>,
}

/// Apply an edit to the document, through the document's own primitives.
///
/// The caller applies one edit at a time, in the order the relay sequenced them, on every
/// replica. An edit that cannot land changes nothing.
pub fn apply(doc: &mut Document, ids: &mut IdMap, edit: &Edit) -> Result<Effect, EditError> {
    match edit {
        Edit::InsertText { block, at, text } => {
            let index = index_of(ids, *block)?;
            let para = paragraph(doc, index)?;
            check_offset(para, *at)?;
            doc.insert_text(&Pos::body(index, *at), text, &CharProps::default()).map_err(|_| EditError::BadOffset)?;
            let end = at.saturating_add(text.len());
            Ok(Effect { inverse: Edit::DeleteRange { block: *block, from: *at, to: end }, touched: vec![*block] })
        }
        Edit::DeleteRange { block, from, to } => {
            let index = index_of(ids, *block)?;
            let para = paragraph(doc, index)?;
            if from > to {
                return Err(EditError::BadOffset);
            }
            check_offset(para, *from)?;
            check_offset(para, *to)?;
            let text = para.plain_text().get(*from..*to).map(str::to_owned).ok_or(EditError::BadOffset)?;
            doc.delete_range(&Pos::body(index, *from), &Pos::body(index, *to)).map_err(|_| EditError::BadOffset)?;
            Ok(Effect { inverse: Edit::InsertText { block: *block, at: *from, text }, touched: vec![*block] })
        }
        Edit::RemoveParagraph { block } => {
            let index = index_of(ids, *block)?;
            let para = paragraph(doc, index)?;
            if doc.body.len() <= 1 {
                return Err(EditError::LastParagraph);
            }
            let text = para.plain_text();
            let after = index.checked_sub(1).and_then(|i| ids.id_at(i));
            doc.remove_block(StoryRef::Body, &Path::top(index)).map_err(|_| EditError::Unsupported)?;
            ids.removed(index);
            Ok(Effect { inverse: Edit::InsertParagraph { after, id: *block, text }, touched: vec![*block] })
        }
        Edit::InsertParagraph { after, id, text } => {
            let index = match after {
                Some(block) => index_of(ids, *block)?.saturating_add(1),
                None => 0,
            };
            if ids.index(*id).is_some() {
                return Err(EditError::Unsupported);
            }
            let block = Block::Para(Paragraph::with_text(text, CharProps::default()));
            doc.insert_block(StoryRef::Body, &Path::top(index), block).map_err(|_| EditError::BadOffset)?;
            ids.shifted(index);
            ids.put(index, *id);
            Ok(Effect { inverse: Edit::RemoveParagraph { block: *id }, touched: vec![*id] })
        }
    }
}

fn index_of(ids: &IdMap, block: BlockId) -> Result<usize, EditError> {
    ids.index(block).ok_or(EditError::Gone)
}

fn paragraph(doc: &Document, index: usize) -> Result<&Paragraph, EditError> {
    doc.para(StoryRef::Body, &Path::top(index)).ok_or(EditError::Unsupported)
}

fn check_offset(para: &Paragraph, at: usize) -> Result<(), EditError> {
    let text = para.plain_text();
    if at > text.len() || !text.is_char_boundary(at) {
        return Err(EditError::BadOffset);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(texts: &[&str]) -> Document {
        let mut doc = Document::new();
        doc.body.clear();
        for text in texts {
            doc.body.push(std::sync::Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
        }
        doc
    }

    fn body_text(doc: &Document) -> Vec<String> {
        doc.body.iter().map(|block| block.as_para().map(Paragraph::plain_text).unwrap_or_default()).collect::<Vec<_>>()
    }

    #[test]
    fn typing_reaches_the_paragraph_and_reports_its_inverse() {
        let mut doc = document(&["Hello"]);
        let mut ids = IdMap::bind(&doc, 1);
        let block = ids.id_at(0).expect("an id");
        let effect = apply(&mut doc, &mut ids, &Edit::InsertText { block, at: 5, text: " there".into() }).expect("it lands");
        assert_eq!(body_text(&doc), ["Hello there"]);
        assert_eq!(effect.inverse, Edit::DeleteRange { block, from: 5, to: 11 });
        apply(&mut doc, &mut ids, &effect.inverse).expect("the inverse lands");
        assert_eq!(body_text(&doc), ["Hello"]);
    }

    #[test]
    fn a_removed_paragraph_comes_back_with_its_text_from_the_inverse() {
        let mut doc = document(&["one", "two", "three"]);
        let mut ids = IdMap::bind(&doc, 1);
        let two = ids.id_at(1).expect("an id");
        let effect = apply(&mut doc, &mut ids, &Edit::RemoveParagraph { block: two }).expect("it lands");
        assert_eq!(body_text(&doc), ["one", "three"]);
        assert_eq!(ids.index(two), None);
        apply(&mut doc, &mut ids, &effect.inverse).expect("the inverse lands");
        assert_eq!(body_text(&doc), ["one", "two", "three"]);
        assert_eq!(ids.index(two), Some(1));
    }

    #[test]
    fn an_edit_inside_a_character_or_past_the_end_changes_nothing() {
        let mut doc = document(&["привет"]);
        let mut ids = IdMap::bind(&doc, 1);
        let block = ids.id_at(0).expect("an id");
        assert_eq!(apply(&mut doc, &mut ids, &Edit::InsertText { block, at: 1, text: "x".into() }), Err(EditError::BadOffset));
        assert_eq!(apply(&mut doc, &mut ids, &Edit::InsertText { block, at: 7, text: "x".into() }), Err(EditError::BadOffset));
        assert_eq!(body_text(&doc), ["привет"]);
    }

    #[test]
    fn the_last_paragraph_is_kept() {
        let mut doc = document(&["only"]);
        let mut ids = IdMap::bind(&doc, 1);
        let block = ids.id_at(0).expect("an id");
        assert_eq!(apply(&mut doc, &mut ids, &Edit::RemoveParagraph { block }), Err(EditError::LastParagraph));
        assert_eq!(body_text(&doc), ["only"]);
    }

    #[test]
    fn a_gone_block_is_refused_before_anything_moves() {
        let mut doc = document(&["one", "two"]);
        let mut ids = IdMap::bind(&doc, 1);
        let one = ids.id_at(0).expect("an id");
        apply(&mut doc, &mut ids, &Edit::RemoveParagraph { block: one }).expect("it lands");
        assert_eq!(apply(&mut doc, &mut ids, &Edit::InsertText { block: one, at: 0, text: "x".into() }), Err(EditError::Gone));
        assert_eq!(body_text(&doc), ["two"]);
    }
}
