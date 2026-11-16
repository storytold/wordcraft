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

/// Serialize `entries` as the namespaced custom XML store, returning `None` if oversized.
pub(crate) fn write(entries: &[SnapshotEntry]) -> Result<Option<Vec<u8>>, DocxError> {
    if entries.is_empty() {
        return Ok(None);
    }
    let capped = entries.get(..MAX_ENTRIES).unwrap_or(entries);
    let json = serde_json::to_string(capped).map_err(|e| DocxError::Xml(e.to_string()))?;
    let esc_json = craft_xml::esc(&json);
    let xml =
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><{ROOT} xmlns="{NS}" kind="{KIND}"><entries>{esc_json}</entries></{ROOT}>"#).into_bytes();
    if xml.len() > MAX_STORE_BYTES {
        return Ok(None);
    }
    Ok(Some(xml))
}

/// Quick check whether `bytes` is a WordCraft numbering-change snapshot store (even if another
/// editor renamed the package part to `customXml/item1.xml`).
pub(crate) fn is_store(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_STORE_BYTES && std::str::from_utf8(bytes).is_ok_and(|s| s.contains(KIND))
}

/// Read snapshot overrides from a custom XML part's bytes, if it is ours and within limits.
pub(crate) fn read_from_bytes(bytes: &[u8]) -> Option<HashMap<String, SnapshotEntry>> {
    if bytes.len() > MAX_STORE_BYTES {
        return None;
    }
    let root = craft_xml::parse(bytes).ok()?;
    if root.local() != ROOT || root.attr("kind") != Some(KIND) {
        return None;
    }
    let text = root.child_text("entries")?;
    if text.len() > MAX_STORE_BYTES {
        return None;
    }
    let entries: Vec<SnapshotEntry> = serde_json::from_str(text.as_ref()).ok()?;
    Some(entries.into_iter().take(MAX_ENTRIES).map(|e| (e.loc.clone(), e)).collect())
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
        if depth > MAX_DEPTH || self.entries.len() >= MAX_ENTRIES {
            return;
        }
        for (i, block) in blocks.iter().enumerate() {
            if self.entries.len() >= MAX_ENTRIES {
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
        let xml = write(&entries).expect("serialize").expect("nonempty");
        let parsed = read_from_bytes(&xml).expect("parse");
        assert_eq!(parsed.get("b:0").and_then(|e| e.snapshot), Some(NumRef { num: 2, level: 0 }));
        assert_eq!(parsed.get("b:1").and_then(|e| e.snapshot), None);

        assert!(write(&[]).expect("empty").is_none());
        assert!(read_from_bytes(b"<other kind=\"wordcraft-numchange-v1\"/>").is_none());
        let oversized = vec![b'x'; MAX_STORE_BYTES + 1];
        assert!(read_from_bytes(&oversized).is_none());
    }
}
