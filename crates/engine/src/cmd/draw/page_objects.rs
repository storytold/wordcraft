//! Per-call object lookups for drawing-order page walks, which can revisit a paragraph's
//! offsets out of text order. Scan only as far as the furthest requested object, once.

use std::collections::HashMap;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Document, InlineObject, Path, Pos, StoryRef};

#[derive(Default)]
struct ParagraphObjects<'a> {
    scanned: usize,
    ordinal: usize,
    objects: Vec<(usize, &'a InlineObject)>,
}

pub(super) struct PageObjects<'a> {
    doc: &'a Document,
    paragraphs: HashMap<(StoryRef, Path), ParagraphObjects<'a>>,
}

impl<'a> PageObjects<'a> {
    pub(super) fn new(doc: &'a Document) -> Self {
        Self { doc, paragraphs: HashMap::new() }
    }

    pub(super) fn get(&mut self, pos: &Pos) -> Option<&'a InlineObject> {
        let p = self.doc.para_at(pos)?;
        if !p.text.get(pos.off..)?.starts_with(OBJ) {
            return None;
        }
        let end = pos.off.checked_add(OBJ.len_utf8())?;
        let cached = self.paragraphs.entry((pos.story, pos.path.clone())).or_default();
        if end > cached.scanned {
            // Start after the last scanned marker, not at the paragraph's start. In particular,
            // a lone object near the start never scans a long text suffix it doesn't need.
            for (off, c) in p.text.get(cached.scanned..end)?.char_indices() {
                if c == OBJ {
                    if let Some(object) = p.objects.get(cached.ordinal) {
                        cached.objects.push((cached.scanned + off, object));
                    }
                    cached.ordinal += 1;
                }
            }
            cached.scanned = end;
        }
        let i = cached.objects.binary_search_by_key(&pos.off, |(off, _)| *off).ok()?;
        cached.objects.get(i).map(|(_, object)| *object)
    }
}
