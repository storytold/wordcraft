//! MHT / MHTML (Single File Web Page) import via `craft-mime` and `craft-html`.

use std::sync::Arc;

use craft_mime::{Body, Part};
use wordcraft_doc::{Document, SectionProps};

/// Picture bytes one page may load, counting every `img` that names a part.
const PICTURE_BUDGET: usize = 1 << 30;

/// Does `bytes` look like a MIME message (an `.mht` file)?
pub fn is_mht(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(4096)).unwrap_or(bytes)).to_ascii_lowercase();
    head.starts_with("mime-version:") || head.starts_with("content-type: multipart/") || head.contains("\ncontent-type: multipart/related")
}

/// Import an MHT file.
pub fn import(bytes: &[u8]) -> Document {
    import_on(bytes, &SectionProps::default())
}

fn collect_leaves<'a>(part: &'a Part, out: &mut Vec<&'a Part>) {
    match &part.body {
        Body::Data(_) => out.push(part),
        Body::Parts(parts) => {
            for p in parts {
                collect_leaves(p, out);
            }
        }
    }
}

fn part_bytes(part: &Part) -> &[u8] {
    match &part.body {
        Body::Data(b) => b,
        Body::Parts(_) => &[],
    }
}

/// [`import`] onto `page`.
pub fn import_on(bytes: &[u8], page: &SectionProps) -> Document {
    let msg = craft_mime::parse(bytes, &craft_mime::Limits::DEFAULT);
    let mut leaves = Vec::new();
    collect_leaves(&msg.root, &mut leaves);
    let root = craft_mime::start_part(&msg.root)
        .or_else(|| leaves.iter().copied().find(|p| matches!(p.declared_media_type(), Some("text/html" | "application/xhtml+xml"))));
    let Some(root) = root else {
        let text = leaves.iter().copied().find(|p| p.declared_media_type().unwrap_or("text/plain") == "text/plain").map(part_bytes).unwrap_or(&[]);
        return crate::txt::import_on(text, page);
    };
    let html = crate::html::decode_as(part_bytes(root), root.content_type.get("charset"));
    let base = root.content_location().unwrap_or("");
    let resources = craft_mime::Resources::of(&msg.root);
    let left = std::cell::Cell::new(PICTURE_BUDGET);
    let loader = |src: &str| -> Option<Arc<Vec<u8>>> {
        let part = resources.find(src.trim(), base)?;
        let data = part_bytes(part);
        left.set(left.get().checked_sub(data.len())?);
        Some(Arc::new(data.to_vec()))
    };
    crate::html::import_on(&html, &loader, page)
}
