//! Exact `w:numberingChange` snapshot overrides in a DOCX custom XML part.
//!
//! ECMA-376 §17.3.1.20 stores only `w:original` on `<w:numberingChange>`, never a `w:numId`.
//! When two lists share the same level format, or when a list is edited in place after a revision
//! was recorded, `w:original` alone cannot distinguish the paragraph's prior list from another
//! list whose definition matches the historical string. WordCraft writes this namespaced store
//! only when a paragraph's in-memory snapshot differs from what `Numbering::change_originals`
//! recovers from `w:original`, leaving `w:original` untouched as the historical definition.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use craft_docx::DocxError;
use serde::{Deserialize, Serialize};
use wordcraft_doc::props::NumRef;

/// Package path of the custom XML store.
pub(crate) const PART: &str = "customxml/wordcraftnumchange.xml";
/// Root element of the store.
const ROOT: &str = "wordcraftNumChange";
/// Namespace of the store.
const NS: &str = "urn:wordcraft:numchange";
/// Versioned marker attribute (`El` strips `xmlns` attributes).
const KIND: &str = "wordcraft-numchange-v1";
/// Maximum byte length accepted or emitted for the custom XML store.
pub(crate) const MAX_STORE_BYTES: usize = 1024 * 1024;
/// Maximum number of snapshot overrides serialized or loaded.
const MAX_ENTRIES: usize = 10_000;
/// Maximum nested table/group depth traversed when indexing stories.
const MAX_DEPTH: usize = 32;

const CUSTOM_XML_ROOTS: &str = "wordcraft:customXml.roots";
const EMBEDDED_PARTS: &str = "wordcraft:embedded.parts";

/// Snapshot state for one paragraph's `<w:numberingChange>` when it differs from cache inference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SnapshotEntry {
    /// Canonical story and block path (`"b:0"`, `"s0:hd:1/0/0/0"`, `"g0:0"`).
    pub loc: String,
    /// Exported `w:original` string guarding against external paragraph insertions/edits.
    pub original: String,
    /// Exact prior numbering reference, or `None` when the revision is unnumbered/unresolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<NumRef>,
}

/// Parsed store result with completeness and integrity metadata.
pub(crate) struct SnapshotStore {
    pub entries: HashMap<String, SnapshotEntry>,
    /// When `false`, the store was truncated by size/entry limits on write. The reader treats
    /// every `numberingChange` *without* an entry as unresolved rather than falling back to
    /// cache inference.
    pub complete: bool,
    /// Hash of the ordered `(loc, original)` pairs for every `numberingChange` in the document
    /// and each override's snapshot list identity at the time the document was saved. The reader
    /// recomputes this from the shared model and invalidates all overrides when the document was
    /// changed externally (e.g., another editor inserted a paragraph or renumbered lists).
    pub fingerprint: Option<u64>,
}

/// Serialize `entries` as the namespaced custom XML store.
///
/// `fingerprint` is a hash of the ordered `(loc, original)` pairs for every `numberingChange`
/// in the document and each override's snapshot list identity in `w:numbering`. The reader
/// verifies it against the shared model on import and invalidates all overrides when the
/// document was changed externally.
///
/// When the full set exceeds `MAX_ENTRIES` or `MAX_STORE_BYTES`, a small stub with
/// `complete="false"` is written instead so the reader treats every `numberingChange` without
/// an entry as unresolved rather than falling back to cache inference.
pub(crate) fn write(entries: &[SnapshotEntry], fingerprint: u64) -> Result<Option<Vec<u8>>, DocxError> {
    if entries.is_empty() {
        return Ok(None);
    }
    let capped = entries.get(..MAX_ENTRIES).unwrap_or(entries);
    let json = serde_json::to_string(capped).map_err(|e| DocxError::Xml(e.to_string()))?;
    let esc_json = craft_xml::esc(&json);
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><{ROOT} xmlns="{NS}" kind="{KIND}" fp="{fingerprint}"><entries>{esc_json}</entries></{ROOT}>"#
    )
    .into_bytes();
    if xml.len() > MAX_STORE_BYTES || capped.len() < entries.len() {
        // Emit a minimal stub so the reader knows the store existed but was incomplete.
        let stub = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><{ROOT} xmlns="{NS}" kind="{KIND}" complete="false" fp="{fingerprint}"><entries>[]</entries></{ROOT}>"#
        )
        .into_bytes();
        return Ok(Some(stub));
    }
    Ok(Some(xml))
}

/// Quick check whether `bytes` is a WordCraft numbering-change snapshot store (even if another
/// editor renamed the package part to `customXml/item1.xml`).
pub(crate) fn is_store(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_STORE_BYTES && std::str::from_utf8(bytes).is_ok_and(|s| s.contains(KIND))
}

/// Read snapshot overrides from a custom XML part's bytes, if it is ours and within limits.
pub(crate) fn read_from_bytes(bytes: &[u8]) -> Option<SnapshotStore> {
    if bytes.len() > MAX_STORE_BYTES {
        return None;
    }
    let root = craft_xml::parse(bytes).ok()?;
    if root.local() != ROOT || root.attr("kind") != Some(KIND) {
        return None;
    }
    let complete = root.attr("complete") != Some("false");
    let fingerprint = root.attr("fp").and_then(|s| s.parse::<u64>().ok());
    let text = root.child_text("entries")?;
    if text.len() > MAX_STORE_BYTES {
        return None;
    }
    let entries: Vec<SnapshotEntry> = serde_json::from_str(text.as_ref()).ok()?;
    let entries = entries.into_iter().take(MAX_ENTRIES).map(|e| (e.loc.clone(), e)).collect();
    Some(SnapshotStore { entries, complete, fingerprint })
}

/// Remove `part` and any manifest/root bookkeeping lines naming it from `passthrough`.
pub(crate) fn remove_custom_xml_part(passthrough: &mut BTreeMap<String, Arc<Vec<u8>>>, part: &str) {
    let keys: Vec<String> = passthrough.keys().filter(|k| k.eq_ignore_ascii_case(part)).cloned().collect();
    for key in keys {
        passthrough.remove(&key);
    }
    strip_manifest_part(passthrough, EMBEDDED_PARTS, part, 1);
    strip_manifest_part(passthrough, CUSTOM_XML_ROOTS, part, 2);
}

fn strip_manifest_part(passthrough: &mut BTreeMap<String, Arc<Vec<u8>>>, key: &str, part: &str, field_idx: usize) {
    let Some(existing) = passthrough.get(key) else { return };
    let text = String::from_utf8_lossy(existing);
    let mut kept = String::new();
    for line in text.lines() {
        let matches_part = line.split('\t').nth(field_idx).is_some_and(|f| f.eq_ignore_ascii_case(part));
        if !matches_part {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if kept.is_empty() {
        passthrough.remove(key);
    } else {
        passthrough.insert(key.into(), Arc::new(kept.into_bytes()));
    }
}

/// Insert `part` into `passthrough` alongside its embedded-part and custom-XML-root lines.
pub(crate) fn insert_custom_xml_part(passthrough: &mut BTreeMap<String, Arc<Vec<u8>>>, part: &str, xml: Vec<u8>) {
    remove_custom_xml_part(passthrough, part);
    passthrough.insert(part.into(), Arc::new(xml));

    let manifest_entry = format!("P\t{part}\tapplication/xml\n");
    let manifest = match passthrough.get(EMBEDDED_PARTS) {
        Some(existing) => {
            let mut text = String::from_utf8_lossy(existing).to_string();
            text.push_str(&manifest_entry);
            Arc::new(text.into_bytes())
        }
        None => Arc::new(manifest_entry.into_bytes()),
    };
    passthrough.insert(EMBEDDED_PARTS.into(), manifest);

    let root_entry = format!("D\thttp://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\t{part}\n");
    let roots = match passthrough.get(CUSTOM_XML_ROOTS) {
        Some(existing) => {
            let mut text = String::from_utf8_lossy(existing).to_string();
            text.push_str(&root_entry);
            Arc::new(text.into_bytes())
        }
        None => Arc::new(root_entry.into_bytes()),
    };
    passthrough.insert(CUSTOM_XML_ROOTS.into(), roots);
}

/// Assigns stable, ID-independent labels to every `Part` in `doc` by reference order so `loc`
/// strings match before `craft_docx::write` and after `craft_docx::read`.
pub(crate) fn part_labels(doc: &craft_wordmodel::Document) -> HashMap<u32, String> {
    let mut labels = HashMap::new();
    let mut sec_idx = 0usize;
    let mut section_part_order = Vec::new();
    for block in &doc.body {
        if let craft_wordmodel::Block::Para(p) = block.as_ref()
            && let Some(sec) = p.section.as_deref()
        {
            label_section_parts(sec, sec_idx, &mut labels, &mut section_part_order);
            sec_idx = sec_idx.saturating_add(1);
        }
    }
    label_section_parts(&doc.last_section, sec_idx, &mut labels, &mut section_part_order);

    let mut comment_part_order = Vec::new();
    for (i, cmt) in doc.comments.values().enumerate() {
        labels.entry(cmt.part).or_insert_with(|| {
            comment_part_order.push(cmt.part);
            format!("c{i}")
        });
    }

    let mut counters = ObjectPartCounters::default();
    label_blocks_object_parts(&doc.body, &mut labels, &mut counters, 0);
    for pid in section_part_order {
        if let Some(part) = doc.parts.get(&pid) {
            label_blocks_object_parts(&part.blocks, &mut labels, &mut counters, 0);
        }
    }
    for pid in comment_part_order {
        if let Some(part) = doc.parts.get(&pid) {
            label_blocks_object_parts(&part.blocks, &mut labels, &mut counters, 0);
        }
    }
    for g in &doc.glossary {
        label_blocks_object_parts(&g.blocks, &mut labels, &mut counters, 0);
    }

    let mut unref_ord = 0usize;
    for &id in doc.parts.keys() {
        labels.entry(id).or_insert_with(|| {
            let ord = unref_ord;
            unref_ord = unref_ord.saturating_add(1);
            format!("u{ord}")
        });
    }
    labels
}

fn label_section_parts(sec: &craft_wordmodel::section::SectionProps, sec_idx: usize, labels: &mut HashMap<u32, String>, order: &mut Vec<u32>) {
    for (slot, opt_id) in [
        ("hd", sec.headers.default),
        ("hf", sec.headers.first),
        ("he", sec.headers.even),
        ("fd", sec.footers.default),
        ("ff", sec.footers.first),
        ("fe", sec.footers.even),
    ] {
        if let Some(id) = opt_id {
            labels.entry(id).or_insert_with(|| {
                order.push(id);
                format!("s{sec_idx}:{slot}")
            });
        }
    }
}

#[derive(Default)]
struct ObjectPartCounters {
    notes: usize,
    boxes: usize,
}

fn label_blocks_object_parts(blocks: &craft_wordmodel::Blocks, labels: &mut HashMap<u32, String>, counters: &mut ObjectPartCounters, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for block in blocks {
        match block.as_ref() {
            craft_wordmodel::Block::Para(p) => {
                for obj in &p.objects {
                    label_object_parts(obj, labels, counters, 0);
                }
            }
            craft_wordmodel::Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        label_blocks_object_parts(&cell.blocks, labels, counters, depth + 1);
                    }
                }
            }
        }
    }
}

fn label_object_parts(obj: &craft_wordmodel::para::InlineObject, labels: &mut HashMap<u32, String>, counters: &mut ObjectPartCounters, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match obj {
        craft_wordmodel::para::InlineObject::NoteRef { id, .. } => {
            labels.entry(*id).or_insert_with(|| {
                let ord = counters.notes;
                counters.notes = counters.notes.saturating_add(1);
                format!("n{ord}")
            });
        }
        craft_wordmodel::para::InlineObject::Shape { story: Some(id), .. } => {
            labels.entry(*id).or_insert_with(|| {
                let ord = counters.boxes;
                counters.boxes = counters.boxes.saturating_add(1);
                format!("tb{ord}")
            });
        }
        craft_wordmodel::para::InlineObject::Group { children, .. } => {
            for child in children {
                label_object_parts(&child.obj, labels, counters, depth + 1);
            }
        }
        _ => {}
    }
}

/// Collects snapshot overrides across `source` wherever a paragraph's `NumChange.snapshot`
/// differs from what `Numbering::change_originals` would recover from its exported `w:original`.
pub(crate) fn collect_overrides(source: &wordcraft_doc::Document, shared: &craft_wordmodel::Document) -> Vec<SnapshotEntry> {
    let mut collector = Collector { numbering: source.numbering.as_ref(), originals: None, entries: Vec::new() };
    collector.blocks(&source.body, "b", 0);
    if source.parts.values().any(|part| blocks_have_num_change(&part.blocks, 0)) {
        let labels = part_labels(shared);
        for (&id, part) in &source.parts {
            if let Some(label) = labels.get(&id) {
                collector.blocks(&part.blocks, label, 0);
            }
        }
    }
    for (i, g) in source.glossary.iter().enumerate() {
        collector.blocks(&g.blocks, &format!("g{i}"), 0);
    }
    collector.entries
}

const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0100_0000_01b3;

fn fnv1a_bytes(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(FNV1A_PRIME);
    }
    // Length/field separator so ("a", "bc") != ("ab", "c").
    *h ^= 0xff;
    *h = h.wrapping_mul(FNV1A_PRIME);
}

fn fnv1a_pair(h: &mut u64, loc: &str, original: &str) {
    fnv1a_bytes(h, loc.as_bytes());
    fnv1a_bytes(h, original.as_bytes());
}

fn fnv1a_u32(h: &mut u64, val: u32) {
    fnv1a_bytes(h, &val.to_le_bytes());
}

fn fnv1a_u8(h: &mut u64, val: u8) {
    fnv1a_bytes(h, &[val]);
}

/// Recompute the fingerprint from a `craft_wordmodel::Document` for verification or serialization.
///
/// Hashes:
/// 1. Ordered `(loc, original)` pairs for every `numberingChange` in the document, detecting
///    external paragraph insertions, deletions, reordering, and `w:original` edits.
/// 2. For each snapshot override entry (sorted by `loc`):
///    `loc`, `original`, and its snapshot list identity in `source.numbering` (abstract definition
///    ID, template text, format, and start index), detecting external list renumbering, swapping
///    of `w:numId`s, or abstract definition edits.
pub(crate) fn fingerprint_shared_model<'a>(source: &craft_wordmodel::Document, overrides: impl IntoIterator<Item = &'a SnapshotEntry>) -> u64 {
    let mut h = FNV1A_OFFSET;
    fn walk(blocks: &craft_wordmodel::Blocks, prefix: &str, h: &mut u64, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for (i, block) in blocks.iter().enumerate() {
            let loc = format!("{prefix}:{i}");
            match block.as_ref() {
                craft_wordmodel::Block::Para(p) => {
                    if let Some(nc) = p.props.num_change.as_deref() {
                        fnv1a_pair(h, &loc, &nc.original);
                    }
                }
                craft_wordmodel::Block::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (c, cell) in row.cells.iter().enumerate() {
                            walk(&cell.blocks, &format!("{loc}/{r}/{c}"), h, depth + 1);
                        }
                    }
                }
            }
        }
    }
    walk(&source.body, "b", &mut h, 0);
    let labels = part_labels(source);
    let mut labeled: Vec<(&str, &craft_wordmodel::Part)> =
        source.parts.iter().filter_map(|(&id, part)| labels.get(&id).map(|l| (l.as_str(), part))).collect();
    labeled.sort_by_key(|(label, _)| *label);
    for (label, part) in labeled {
        walk(&part.blocks, label, &mut h, 0);
    }
    for (i, g) in source.glossary.iter().enumerate() {
        walk(&g.blocks, &format!("g{i}"), &mut h, 0);
    }

    let mut sorted_overrides: Vec<&SnapshotEntry> = overrides.into_iter().collect();
    sorted_overrides.sort_by_key(|e| &e.loc);
    for entry in sorted_overrides {
        fnv1a_pair(&mut h, &entry.loc, &entry.original);
        match entry.snapshot {
            Some(nr) => {
                fnv1a_u8(&mut h, 1);
                fnv1a_u32(&mut h, nr.num);
                fnv1a_u8(&mut h, nr.level);
                if let Some(num) = source.numbering.num(nr.num) {
                    fnv1a_u8(&mut h, 1);
                    fnv1a_u32(&mut h, num.abstract_id);
                    if let Some(abs) = source.numbering.abstract_of(nr.num) {
                        fnv1a_u8(&mut h, 1);
                        fnv1a_u32(&mut h, abs.id);
                        if let Some(nsid) = abs.nsid.as_deref() {
                            fnv1a_bytes(&mut h, nsid.as_bytes());
                        }
                    } else {
                        fnv1a_u8(&mut h, 0);
                    }
                    for lvl_idx in 0..=nr.level {
                        if let Some(lvl) = source.numbering.level(nr.num, lvl_idx) {
                            fnv1a_u8(&mut h, 1);
                            fnv1a_u8(&mut h, lvl_idx);
                            fnv1a_bytes(&mut h, lvl.text.as_bytes());
                            fnv1a_bytes(&mut h, lvl.format.ooxml().as_bytes());
                            if let Some((_, start)) = num.start_overrides.iter().find(|(l, _)| *l == lvl_idx) {
                                fnv1a_u32(&mut h, *start);
                            } else {
                                fnv1a_u32(&mut h, lvl.start);
                            }
                        } else {
                            fnv1a_u8(&mut h, 0);
                        }
                    }
                } else {
                    fnv1a_u8(&mut h, 0);
                }
            }
            None => {
                fnv1a_u8(&mut h, 0);
            }
        }
    }

    h
}

fn blocks_have_num_change(blocks: &wordcraft_doc::Blocks, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    blocks.iter().any(|b| match b.as_ref() {
        wordcraft_doc::Block::Para(p) => p.props.num_change.is_some(),
        wordcraft_doc::Block::Table(t) => t.rows.iter().any(|r| r.cells.iter().any(|c| blocks_have_num_change(&c.blocks, depth + 1))),
    })
}

struct Collector<'a> {
    numbering: &'a wordcraft_doc::numbering::Numbering,
    originals: Option<BTreeMap<String, NumRef>>,
    entries: Vec<SnapshotEntry>,
}

impl Collector<'_> {
    fn inferred(&mut self, original: &str) -> Option<NumRef> {
        let trimmed = original.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            return None;
        }
        let originals = self.originals.get_or_insert_with(|| self.numbering.change_originals());
        originals.get(trimmed).copied()
    }

    fn blocks(&mut self, blocks: &wordcraft_doc::Blocks, prefix: &str, depth: usize) {
        if depth > MAX_DEPTH || self.entries.len() > MAX_ENTRIES {
            return;
        }
        for (i, block) in blocks.iter().enumerate() {
            if self.entries.len() > MAX_ENTRIES {
                break;
            }
            let loc = format!("{prefix}:{i}");
            match block.as_ref() {
                wordcraft_doc::Block::Para(p) => {
                    let Some(nc) = p.props.num_change.as_deref() else { continue };
                    let exported_original = if !nc.original.is_empty() {
                        nc.original.clone()
                    } else if let Some(nr) = nc.snapshot {
                        self.numbering.change_original(nr.num, nr.level).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let inferred = self.inferred(&exported_original);
                    if nc.snapshot != inferred {
                        self.entries.push(SnapshotEntry { loc, original: exported_original, snapshot: nc.snapshot });
                    }
                }
                wordcraft_doc::Block::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (c, cell) in row.cells.iter().enumerate() {
                            self.blocks(&cell.blocks, &format!("{loc}/{r}/{c}"), depth + 1);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrips_and_rejects_oversized_or_foreign_markup() {
        let entries = vec![
            SnapshotEntry { loc: "b:0".into(), original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 2, level: 0 }) },
            SnapshotEntry { loc: "b:1".into(), original: "%1:1:0:.".into(), snapshot: None },
        ];
        let xml = write(&entries, 42).expect("serialize").expect("nonempty");
        let store = read_from_bytes(&xml).expect("parse");
        assert!(store.complete);
        assert_eq!(store.fingerprint, Some(42));
        assert_eq!(store.entries.get("b:0").and_then(|e| e.snapshot), Some(NumRef { num: 2, level: 0 }));
        assert_eq!(store.entries.get("b:1").and_then(|e| e.snapshot), None);

        assert!(write(&[], 0).expect("empty").is_none());
        assert!(read_from_bytes(b"<other kind=\"wordcraft-numchange-v1\"/>").is_none());
        let oversized = vec![b'x'; MAX_STORE_BYTES + 1];
        assert!(read_from_bytes(&oversized).is_none());
    }

    #[test]
    fn oversized_store_emits_incomplete_stub() {
        // Exceed MAX_ENTRIES: generate more entries than the limit.
        let entries: Vec<SnapshotEntry> = (0..MAX_ENTRIES + 1)
            .map(|i| SnapshotEntry { loc: format!("b:{i}"), original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 1, level: 0 }) })
            .collect();
        let xml = write(&entries, 99).expect("serialize").expect("should emit stub, not None");
        let store = read_from_bytes(&xml).expect("parse stub");
        assert!(!store.complete, "stub must be marked incomplete");
        assert!(store.entries.is_empty(), "stub has no entries");
        assert_eq!(store.fingerprint, Some(99));
    }

    #[test]
    fn byte_limit_emits_incomplete_stub() {
        // A single entry with a huge original string to exceed MAX_STORE_BYTES.
        let big_original = "x".repeat(MAX_STORE_BYTES);
        let entries = vec![SnapshotEntry { loc: "b:0".into(), original: big_original, snapshot: None }];
        let xml = write(&entries, 77).expect("serialize").expect("should emit stub, not None");
        let store = read_from_bytes(&xml).expect("parse stub");
        assert!(!store.complete, "stub must be marked incomplete");
        assert!(store.entries.is_empty(), "stub has no entries");
    }
}
