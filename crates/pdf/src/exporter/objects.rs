//! Per-export figure lookups in drawing order, which can revisit paragraph offsets out of order.
//! Scan only the requested prefixes, once, and borrow objects without copying their contents.

use std::collections::HashMap;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Document, InlineObject, Path, StoryRef};

#[derive(Default)]
struct ParagraphObjects<'a> {
    scanned: usize,
    ordinal: usize,
    objects: Vec<(usize, &'a InlineObject)>,
}

pub(crate) struct ObjectLookup<'a> {
    doc: &'a Document,
    paragraphs: HashMap<(StoryRef, Path), ParagraphObjects<'a>>,
}

impl<'a> ObjectLookup<'a> {
    pub(crate) fn new(doc: &'a Document) -> Self {
        Self { doc, paragraphs: HashMap::new() }
    }

    pub(crate) fn get(&mut self, story: StoryRef, path: &Path, off: usize) -> Option<&'a InlineObject> {
        let p = self.doc.para(story, path)?;
        if !p.text.get(off..)?.starts_with(OBJ) {
            return None;
        }
        let end = off.checked_add(OBJ.len_utf8())?;
        let cached = self.paragraphs.entry((story, path.clone())).or_default();
        if end > cached.scanned {
            // A lone early figure must not index a long unused suffix. Later figures extend
            // this prefix; repeated/reordered pages and drawing order reuse previous entries.
            for (at, c) in p.text.get(cached.scanned..end)?.char_indices() {
                if c == OBJ {
                    if let Some(object) = p.objects.get(cached.ordinal) {
                        cached.objects.push((cached.scanned + at, object));
                    }
                    cached.ordinal += 1;
                }
            }
            cached.scanned = end;
        }
        let i = cached.objects.binary_search_by_key(&off, |(at, _)| *at).ok()?;
        cached.objects.get(i).map(|(_, object)| *object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{CharProps, Paragraph, Part, PartKind, Table, para_block};

    #[test]
    fn lookups_keep_utf8_offsets_story_paths_and_missing_storage() {
        let para = |name: &str| {
            let mut p = Paragraph::with_text("éβ", CharProps::default());
            p.insert_object(2, InlineObject::BookmarkStart { name: name.into() }, &Default::default()).unwrap();
            p.insert_object(p.len(), InlineObject::BookmarkEnd { name: name.into() }, &Default::default()).unwrap();
            p
        };
        let mut d = Document::new();
        let mut table = Table::new(1, 1, 200.0);
        table.rows[0].cells[0].blocks = vec![para_block(para("cell"))];
        d.body = vec![para_block(para("body")), std::sync::Arc::new(wordcraft_doc::Block::Table(table))];
        d.parts.insert(7, Part { kind: PartKind::Header, blocks: vec![para_block(para("header"))], ..Default::default() });
        let mut lookup = ObjectLookup::new(&d);
        for (story, path, name) in
            [(StoryRef::Body, Path::top(0), "body"), (StoryRef::Body, Path(vec![1, 0, 0, 0]), "cell"), (StoryRef::Part(7), Path::top(0), "header")]
        {
            // The later marker first, then the earlier one, including duplicates.
            for off in [7, 2, 7, 2] {
                let obj = lookup.get(story, &path, off).unwrap();
                assert!(matches!(obj, InlineObject::BookmarkStart { name: n } | InlineObject::BookmarkEnd { name: n } if n == name));
            }
            for off in [0, 1, 3, 5, 6, 8, 10, usize::MAX] {
                assert!(lookup.get(story, &path, off).is_none(), "{off}");
            }
        }
        assert!(lookup.get(StoryRef::Part(99), &Path::top(0), 2).is_none());
        assert!(lookup.get(StoryRef::Body, &Path::top(99), 2).is_none());
        drop(lookup);
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.objects.truncate(1);
        p.text.push_str(&"x".repeat(100_000));
        let mut lookup = ObjectLookup::new(&d);
        assert!(lookup.get(StoryRef::Body, &Path::top(0), 2).is_some());
        assert_eq!(lookup.paragraphs[&(StoryRef::Body, Path::top(0))].scanned, 5, "unused suffix scanned");
        assert!(lookup.get(StoryRef::Body, &Path::top(0), 7).is_none());
        assert!(lookup.get(StoryRef::Body, &Path::top(0), 2).is_some());
    }
}
