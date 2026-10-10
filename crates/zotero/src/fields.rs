//! Zotero's fields and preferences as WordCraft stores them (the Word plugin's layout).
//!
//! A citation is a range field with code `ADDIN ZOTERO_ITEM CSL_CITATION {…}`; Zotero itself
//! sees the code without the `ADDIN ZOTERO_` prefix (`ITEM CSL_CITATION {…}`). Fields in
//! footnotes and endnotes are listed at their note reference, with the note's number.

use wordcraft_doc::para::{NoteKind, OBJ};
use wordcraft_doc::{Document, InlineObject, PartKind, Pos, StoryRef};

/// The stored code prefix.
pub const PREFIX: &str = "ADDIN ZOTERO_";
/// Custom property names holding the document preferences, numbered from 1.
pub const PREF_NAME: &str = "ZOTERO_PREF_";
/// Longest chunk per preference property (Word's custom property limit).
pub const PREF_CHUNK: usize = 255;
/// Most preference chunks read back.
const MAX_PREF_CHUNKS: usize = 10_000;

/// A Zotero field.
#[derive(Clone, Debug, PartialEq)]
pub struct ZField {
    /// Position of the `FieldStart` marker.
    pub start: Pos,
    /// Position of the `FieldEnd` marker.
    pub end: Pos,
    /// Zotero's code (without the stored prefix).
    pub code: String,
    /// The note number for fields in a footnote or endnote, else 0.
    pub note_index: u32,
}

impl ZField {
    /// Where the result begins (just after the start marker).
    pub fn content_start(&self) -> Pos {
        Pos { off: self.start.off + OBJ.len_utf8(), ..self.start.clone() }
    }
    /// Just after the end marker.
    pub fn after(&self) -> Pos {
        Pos { off: self.end.off + OBJ.len_utf8(), ..self.end.clone() }
    }
    /// Whether `p` is inside the field (after its start marker, up to its end marker).
    pub fn contains(&self, p: &Pos) -> bool {
        p.story == self.start.story && *p >= self.content_start() && *p <= self.end
    }
}

/// Zotero's code for a stored field code, if it is a Zotero field.
pub fn zotero_code(instr: &str) -> Option<&str> {
    let t = instr.trim_start();
    let head = t.get(..PREFIX.len())?;
    head.eq_ignore_ascii_case(PREFIX).then(|| t.get(PREFIX.len()..).unwrap_or("").trim())
}

/// The stored field code for Zotero's code.
pub fn stored_code(code: &str) -> String {
    format!("{PREFIX}{}", code.trim())
}

fn story_fields(doc: &Document, story: StoryRef, note_index: u32) -> Vec<ZField> {
    doc.field_ranges(story)
        .into_iter()
        .filter_map(|r| zotero_code(&r.instr).map(|c| ZField { start: r.start, end: r.end, code: c.to_string(), note_index }))
        .collect()
}

/// Every Zotero field in document order: the body's, with each note's fields at its reference.
pub fn list(doc: &Document) -> Vec<ZField> {
    enum Ev {
        Field(ZField),
        Note(u32, u32),
    }
    let mut evs: Vec<(Pos, Ev)> = story_fields(doc, StoryRef::Body, 0).into_iter().map(|f| (f.start.clone(), Ev::Field(f))).collect();
    let (mut foot, mut end) = (0u32, 0u32);
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for off in p.object_offsets() {
            if let Some(InlineObject::NoteRef { kind, id, custom }) = p.object_at(off) {
                let n = match kind {
                    NoteKind::Footnote => &mut foot,
                    NoteKind::Endnote => &mut end,
                };
                if custom.is_empty() {
                    *n = n.saturating_add(1);
                }
                evs.push((Pos::new(StoryRef::Body, path.clone(), off), Ev::Note(*id, (*n).max(1))));
            }
        }
    }
    evs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for (_, ev) in evs {
        match ev {
            Ev::Field(f) => out.push(f),
            Ev::Note(id, n) => out.extend(story_fields(doc, StoryRef::Part(id), n)),
        }
    }
    out
}

/// Whether a citation can go at `p`: the body, a footnote or an endnote.
pub fn can_insert_at(doc: &Document, p: &Pos) -> bool {
    match p.story {
        StoryRef::Body => true,
        StoryRef::Part(id) => doc.parts.get(&id).is_some_and(|part| matches!(part.kind, PartKind::Footnote | PartKind::Endnote)),
    }
}

fn pref_index(name: &str) -> Option<usize> {
    let head = name.get(..PREF_NAME.len())?;
    if !head.eq_ignore_ascii_case(PREF_NAME) {
        return None;
    }
    name.get(PREF_NAME.len()..)?.parse().ok()
}

/// The document preferences Zotero stored, or `""`.
pub fn get_prefs(doc: &Document) -> String {
    let mut out = String::new();
    for i in 1..=MAX_PREF_CHUNKS {
        match doc.custom_prop(&format!("{PREF_NAME}{i}")) {
            Some(v) => out.push_str(v),
            None => break,
        }
    }
    // Zotero sees the field type it knows this protocol by; the file keeps Word's.
    out.replace(r#"name="fieldType" value="Field""#, r#"name="fieldType" value="ReferenceMark""#)
}

/// Store Zotero's document preferences in `ZOTERO_PREF_1…n`.
pub fn set_prefs(doc: &mut Document, data: &str) {
    let data = data.replace(r#"name="fieldType" value="ReferenceMark""#, r#"name="fieldType" value="Field""#);
    doc.custom_props.retain(|p| pref_index(&p.name).is_none());
    let chars: Vec<char> = data.chars().collect();
    for (i, chunk) in chars.chunks(PREF_CHUNK).take(MAX_PREF_CHUNKS).enumerate() {
        doc.set_custom_prop(&format!("{PREF_NAME}{}", i + 1), &chunk.iter().collect::<String>());
    }
}
