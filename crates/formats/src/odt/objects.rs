//! Embedded objects WordCraft shows but doesn't edit (ODF 1.3 §10.4.6 `draw:object` holding a
//! sub-document folder, §10.4.7 `draw:object-ole` an OLE file): their files gathered from the
//! package so the writer can put them back, and a placeholder picture for one whose replacement
//! picture WordCraft can't show (LibreOffice writes its own metafile format there).

use std::collections::HashMap;
use std::sync::Arc;

use crate::model::OdfObject;
use crate::model::odf_object::MAX_FILES;

/// The media type of an OLE object (§10.4.7; LibreOffice's manifest names it so).
pub(in crate::odt) const OLE_MIME: &str = "application/vnd.sun.star.oleobject";

/// The package path an object link names (`./Object 1` → `Object 1`), when it is one inside the
/// package: not empty, not absolute, not leaving the package, not a URL.
fn inside(href: &str) -> Option<&str> {
    let p = href.trim().trim_start_matches("./").trim_end_matches('/');
    let bad = p.is_empty() || p.starts_with('/') || p.contains(':') || p.split('/').any(|s| s == ".." || s == "." || s.is_empty());
    (!bad).then_some(p)
}

/// The media type a sub-document's `content.xml` says it is, by its body's content.
fn guess_mime(content: &[u8]) -> &'static str {
    let head = String::from_utf8_lossy(content.get(..content.len().min(64 * 1024)).unwrap_or_default());
    for (el, mime) in [
        ("<office:spreadsheet", "application/vnd.oasis.opendocument.spreadsheet"),
        ("<office:presentation", "application/vnd.oasis.opendocument.presentation"),
        ("<office:chart", "application/vnd.oasis.opendocument.chart"),
        ("<office:text", "application/vnd.oasis.opendocument.text"),
    ] {
        if head.contains(el) {
            return mime;
        }
    }
    "application/vnd.oasis.opendocument.graphics"
}

/// The object `href` links to (an OLE file when `ole`), its files read with `fetch` from the
/// package's `entries`; its media type from the package `manifest` (path → media type) or its
/// content. `None` when it has no files there.
pub(in crate::odt) fn gather(
    href: &str,
    ole: bool,
    entries: &[String],
    manifest: Option<&HashMap<String, String>>,
    fetch: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> Option<OdfObject> {
    let path = inside(href)?;
    let mut files = Vec::new();
    if ole {
        files.push((String::new(), Arc::new(fetch(path)?)));
    } else {
        let prefix = format!("{path}/");
        for name in entries.iter().filter(|n| n.starts_with(&prefix) && !n.ends_with('/')).take(MAX_FILES) {
            let rel = name.get(prefix.len()..).unwrap_or_default();
            if inside(rel).is_some()
                && let Some(bytes) = fetch(name)
            {
                files.push((rel.to_string(), Arc::new(bytes)));
            }
        }
    }
    if files.is_empty() {
        return None;
    }
    let mime = manifest.and_then(|m| m.get(&format!("{path}/")).or_else(|| m.get(path))).cloned().unwrap_or_else(|| {
        if ole {
            OLE_MIME.to_string()
        } else {
            let content = files.iter().find(|(n, _)| n == "content.xml").map(|(_, b)| b.as_slice()).unwrap_or_default();
            guess_mime(content).to_string()
        }
    });
    Some(OdfObject { mime, ole, files, replacement: None })
}

/// Each `manifest:file-entry`'s path and media type in a package's `META-INF/manifest.xml`.
pub(in crate::odt) fn manifest(xml: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    super::events(xml, |ev| {
        if let super::Ev::Start(q, a) = ev
            && q == "manifest:file-entry"
            && out.len() < 100_000
            && let (Some(p), Some(m)) = (super::get(&a, "manifest:full-path"), super::get(&a, "manifest:media-type"))
        {
            out.insert(p.to_string(), m.to_string());
        }
    });
    out
}

/// A placeholder picture for an object whose own picture WordCraft can't show: a pale panel
/// with a frame, ruled as a grid for a spreadsheet. Drawn here, in WordCraft's own greys.
pub(in crate::odt) fn placeholder(mime: &str) -> Option<Vec<u8>> {
    let (w, h) = (120u32, 72u32);
    let grid = mime.ends_with("spreadsheet");
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
        let rule = grid && (x % 30 == 0 || y % 12 == 0);
        image::Rgb(if edge {
            [150, 150, 150]
        } else if rule {
            [214, 214, 214]
        } else {
            [246, 246, 246]
        })
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}
