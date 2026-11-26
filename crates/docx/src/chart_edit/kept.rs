//! A chart kept from a file, edited in the document: its chart part and workbook found through
//! the object's [`Embedded`] and the embedded-part manifest, and the edited parts stored under
//! new part names (copy on write), so a copy of the chart elsewhere in the document, and the
//! undo history, keep the parts they had.
//!
//! The manifest (`wordcraft:embedded.parts` in `Document::passthrough`) is craft-docx's: one
//! `P \t part \t content type` line per kept part and one `R \t part \t id \t type \t target \t
//! I|E` line per relationship of a kept part. The writer copies only the parts an object
//! reaches, naming them anew, so parts no object uses any more are left out of the file.

use std::sync::Arc;

use wordcraft_doc::Document;
use wordcraft_doc::graphic::{Embedded, EmbeddedRef};

use super::{ChartData, EditError, PartChart, PartEdit, edit_part, edit_workbook, err, read_part};

/// The manifest's key in `Document::passthrough`.
pub const MANIFEST: &str = "wordcraft:embedded.parts";
/// Most kept parts the manifest lists (craft-docx's limit).
const MAX_PARTS: usize = 4096;

/// The chart part a kept chart object draws from.
pub fn chart_part(src: &Embedded) -> Option<&EmbeddedRef> {
    src.refs.iter().find(|r| !r.external && rel_tail(&r.kind) == "chart")
}

/// The last segment of a relationship type (`…/relationships/chart` → `chart`).
fn rel_tail(kind: &str) -> &str {
    kind.rsplit('/').next().unwrap_or(kind)
}

/// What Chart Design shows of the kept chart `src` in `doc`.
pub fn read_kept(doc: &Document, src: &Embedded) -> Option<PartChart> {
    let part = chart_part(src)?;
    read_part(doc.passthrough.get(&part.target)?)
}

/// One relationship line of the manifest.
#[derive(Clone, Debug, PartialEq)]
struct Rel {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

/// The manifest, as its lines.
#[derive(Clone, Debug, Default)]
struct Manifest {
    /// (part, content type)
    parts: Vec<(String, String)>,
    /// (part, relationship)
    rels: Vec<(String, Rel)>,
}

impl Manifest {
    fn read(doc: &Document) -> Manifest {
        let mut m = Manifest::default();
        let Some(bytes) = doc.passthrough.get(MANIFEST) else { return m };
        for line in String::from_utf8_lossy(bytes).lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["P", part, ct] => m.parts.push(((*part).to_string(), (*ct).to_string())),
                ["R", part, id, kind, target, mode] => m.rels.push((
                    (*part).to_string(),
                    Rel { id: (*id).to_string(), kind: (*kind).to_string(), target: (*target).to_string(), external: *mode == "E" },
                )),
                _ => {}
            }
        }
        m
    }

    fn content_type(&self, part: &str) -> Option<&str> {
        self.parts.iter().find(|(p, _)| p == part).map(|(_, ct)| ct.as_str())
    }

    fn rels_of(&self, part: &str) -> Vec<Rel> {
        self.rels.iter().filter(|(p, _)| p == part).map(|(_, r)| r.clone()).collect()
    }

    /// List part `part` as a copy of `like`: its content type, and `like`'s relationships with
    /// targets replaced through `map`.
    fn add_copy(&mut self, part: &str, like: &str, map: &[(String, String)]) {
        let ct = self.content_type(like).unwrap_or("application/octet-stream").to_string();
        self.parts.push((part.to_string(), ct));
        for mut r in self.rels_of(like) {
            if let Some((_, to)) = map.iter().find(|(from, _)| *from == r.target) {
                r.target = to.clone();
            }
            self.rels.push((part.to_string(), r));
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let ok = |s: &str| !s.is_empty() && !s.contains(['\t', '\r', '\n']);
        let mut s = String::new();
        for (p, ct) in &self.parts {
            if ok(p) && ok(ct) {
                s.push_str(&format!("P\t{p}\t{ct}\n"));
            }
        }
        for (p, r) in &self.rels {
            if [p, &r.id, &r.kind, &r.target].iter().all(|f| ok(f)) {
                s.push_str(&format!("R\t{p}\t{}\t{}\t{}\t{}\n", r.id, r.kind, r.target, if r.external { "E" } else { "I" }));
            }
        }
        s.into_bytes()
    }
}

/// A part name not in `doc`'s passthrough, next to `like` (`word/charts/chart1.xml` →
/// `word/charts/chart1-e2.xml`).
fn fresh_name(doc: &Document, like: &str, taken: &[String]) -> String {
    let (dir, file) = like.rsplit_once('/').map_or(("", like), |(d, f)| (d, f));
    let (stem, ext) = file.rsplit_once('.').map_or((file, ""), |(s, e)| (s, e));
    // An earlier edit's suffix is replaced, not repeated.
    let stem = match stem.rsplit_once("-e") {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => stem,
    };
    (1..)
        .map(|k| {
            let name = format!("{stem}-e{k}");
            let file = if ext.is_empty() { name } else { format!("{name}.{ext}") };
            if dir.is_empty() { file } else { format!("{dir}/{file}") }
        })
        .find(|n| !doc.passthrough.keys().any(|k| k.eq_ignore_ascii_case(n)) && !taken.iter().any(|t| t.eq_ignore_ascii_case(n)))
        .unwrap_or_else(|| like.to_string())
}

/// What editing a kept chart did.
#[derive(Clone, Debug)]
pub struct KeptEdit {
    /// The object's new markup and relationships (the same markup, the new chart part).
    pub source: Arc<Embedded>,
    /// The edited chart part's bytes (also stored in the document).
    pub part: Vec<u8>,
    /// The chart's workbook: `"updated"`, `"linked"` (a workbook outside the document, left
    /// as it is), or `"none"` (the chart keeps its data in itself).
    pub workbook: &'static str,
}

/// Make `edit` to the kept chart `src` of `doc`: its chart part (and, for new data, its
/// workbook) are stored edited under new names and listed in the manifest. `doc` is unchanged
/// on error.
pub fn edit_kept(doc: &mut Document, src: &Embedded, edit: &PartEdit) -> Result<KeptEdit, EditError> {
    let Some(chart_ref) = chart_part(src) else { return err("this chart's part wasn't kept") };
    let Some(old_bytes) = doc.passthrough.get(&chart_ref.target).cloned() else { return err("this chart's part is missing") };
    let Some(old) = read_part(&old_bytes) else { return err("this chart can't be read") };
    let new_bytes = edit_part(&old_bytes, edit)?;
    let mut manifest = Manifest::read(doc);
    if manifest.parts.len() + 2 > MAX_PARTS {
        return err("the document keeps too many parts to edit this chart");
    }
    let chart_name = fresh_name(doc, &chart_ref.target, &[]);
    let mut map: Vec<(String, String)> = vec![(chart_ref.target.clone(), chart_name.clone())];
    let mut stores: Vec<(String, Vec<u8>)> = Vec::new();
    let mut workbook = "none";
    if matches!(edit, PartEdit::Data { .. })
        && let Some(id) = old.workbook.as_deref()
    {
        let rel = manifest.rels_of(&chart_ref.target).into_iter().find(|r| r.id == id);
        match rel {
            Some(r) if r.external => workbook = "linked",
            Some(r) => {
                let Some(new) = read_part(&new_bytes) else { return err("the edited chart can't be read") };
                let wb = updated_workbook(doc, &manifest, &r.target, &old, &new.data)?;
                let wb_name = fresh_name(doc, &r.target, std::slice::from_ref(&chart_name));
                manifest.add_copy(&wb_name, &r.target, &[]);
                map.push((r.target.clone(), wb_name.clone()));
                stores.push((wb_name, wb));
                workbook = "updated";
            }
            // A relationship the chart names but the file didn't have: nothing to update.
            None => {}
        }
    }
    manifest.add_copy(&chart_name, &chart_ref.target, &map);
    for (name, bytes) in stores {
        doc.passthrough.insert(name, Arc::new(bytes));
    }
    doc.passthrough.insert(chart_name.clone(), Arc::new(new_bytes.clone()));
    doc.passthrough.insert(MANIFEST.to_string(), Arc::new(manifest.to_bytes()));
    let refs =
        src.refs.iter().map(|r| if r.id == chart_ref.id { EmbeddedRef { target: chart_name.clone(), ..r.clone() } } else { r.clone() }).collect();
    Ok(KeptEdit { source: Arc::new(Embedded { xml: src.xml.clone(), refs }), part: new_bytes, workbook })
}

/// The workbook at `part` with `data` written in, or why it can't be.
fn updated_workbook(doc: &Document, manifest: &Manifest, part: &str, old: &PartChart, data: &ChartData) -> Result<Vec<u8>, EditError> {
    let Some(bytes) = doc.passthrough.get(part) else { return err("the chart's workbook is missing") };
    let ct = manifest.content_type(part).unwrap_or("");
    let lower = part.to_ascii_lowercase();
    let package = ct.contains("spreadsheetml") || lower.ends_with(".xlsx") || lower.ends_with(".xlsm") || bytes.starts_with(b"PK\x03\x04");
    if !package {
        return err("this chart keeps its data in an Excel 97-2003 workbook, which WordCraft can't edit yet");
    }
    let old_rows = old.data.points() + 1;
    let old_cols = old.data.series.len() + 1;
    edit_workbook(bytes, old.sheet.as_deref(), data, old_rows, old_cols)
}
