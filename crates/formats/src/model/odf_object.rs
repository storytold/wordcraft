//! ODF embedded objects ([`OdfObject`]) in a document: their files in
//! [`Document::passthrough`] under `odf-object/<n>/`, linked from the picture showing them by an
//! [`Embedded`] whose markup only names the object's media type, and taken back out for the ODT
//! writer. Other formats see the picture alone.

use std::sync::Arc;

use wordcraft_doc::Document;
use wordcraft_doc::graphic::{Embedded, EmbeddedRef};

use super::types::OdfObject;

/// Relationship kind of an object's file, and of its replacement picture.
const FILE: &str = "file";
const REPLACEMENT: &str = "replacement";
/// Most objects a document keeps, and most files in one.
pub const MAX_OBJECTS: usize = 1000;
pub const MAX_FILES: usize = 256;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unesc(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Keep `obj` in `doc`: its files as passthrough parts, and the link to them. `None` when the
/// document already holds as many objects as it keeps.
pub fn store(doc: &mut Document, obj: &OdfObject) -> Option<Arc<Embedded>> {
    let taken = |n: usize| {
        let prefix = format!("odf-object/{n}/");
        doc.passthrough.range(prefix.clone()..).next().is_some_and(|(k, _)| k.starts_with(&prefix))
    };
    let n = (0..MAX_OBJECTS).find(|&n| !taken(n))?;
    let mut refs = Vec::new();
    for (k, (path, bytes)) in obj.files.iter().take(MAX_FILES).enumerate() {
        let target = format!("odf-object/{n}/{k}");
        doc.passthrough.insert(target.clone(), bytes.clone());
        refs.push(EmbeddedRef { id: path.clone(), kind: FILE.into(), target, external: false });
    }
    if let Some(r) = &obj.replacement {
        let target = format!("odf-object/{n}/replacement");
        doc.passthrough.insert(target.clone(), r.clone());
        refs.push(EmbeddedRef { id: String::new(), kind: REPLACEMENT.into(), target, external: false });
    }
    let xml = format!("{} mime=\"{}\" ole=\"{}\"/>", Embedded::ODF_OBJECT, esc(&obj.mime), obj.ole);
    Some(Arc::new(Embedded { xml, refs }))
}

/// The object `e` links to in `doc`, when it is an ODF object whose files are all there.
pub fn load(doc: &Document, e: &Embedded) -> Option<Arc<OdfObject>> {
    if !e.is_odf_object() {
        return None;
    }
    let attr = |k: &str| {
        let pat = format!(" {k}=\"");
        let from = e.xml.find(&pat)? + pat.len();
        let len = e.xml.get(from..)?.find('"')?;
        e.xml.get(from..from + len).map(unesc)
    };
    let mut obj = OdfObject { mime: attr("mime").unwrap_or_default(), ole: attr("ole").as_deref() == Some("true"), ..Default::default() };
    for r in e.refs.iter().take(MAX_FILES + 1) {
        let bytes = doc.passthrough.get(&r.target)?.clone();
        match r.kind.as_str() {
            FILE => obj.files.push((r.id.clone(), bytes)),
            REPLACEMENT => obj.replacement = Some(bytes),
            _ => {}
        }
    }
    (!obj.files.is_empty()).then(|| Arc::new(obj))
}
