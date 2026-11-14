//! MHT / MHTML (Single File Web Page) import via `craft-mime` and `craft-html`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

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
    let pictures = Pictures::new(&msg.root, root.content_location().unwrap_or(""), PICTURE_BUDGET);
    let loader = |src: &str| pictures.load(src);
    crate::html::import_on(&html, &loader, page)
}

/// The picture parts a page names. While the document keeps a part's copy, every `img` that names
/// the part shares it; a copy the document does not keep (not a picture) is freed.
struct Pictures<'a> {
    resources: craft_mime::Resources<'a>,
    base: &'a str,
    loaded: RefCell<HashMap<*const Part, Weak<Vec<u8>>>>,
    left: Cell<usize>,
}

impl<'a> Pictures<'a> {
    fn new(root: &'a Part, base: &'a str, budget: usize) -> Self {
        Pictures { resources: craft_mime::Resources::of(root), base, loaded: RefCell::new(HashMap::new()), left: Cell::new(budget) }
    }

    /// The bytes of the part `src` names, charged to the budget for every `img`.
    fn load(&self, src: &str) -> Option<Arc<Vec<u8>>> {
        let part = self.resources.find(src.trim(), self.base)?;
        let data = part_bytes(part);
        self.left.set(self.left.get().checked_sub(data.len())?);
        let key: *const Part = part;
        let mut loaded = self.loaded.borrow_mut();
        if let Some(kept) = loaded.get(&key).and_then(Weak::upgrade) {
            return Some(kept);
        }
        let copy = Arc::new(data.to_vec());
        loaded.insert(key, Arc::downgrade(&copy));
        Some(copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &[u8] = b"MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/html\r\nContent-Location: file:///c:/page.htm\r\n\r\n<p><img src=\"a.png\"><img src=\"a.png\"></p>\r\n--b\r\nContent-Type: image/png\r\nContent-Location: file:///c:/a.png\r\n\r\nabcdef\r\n--b--\r\n";

    #[test]
    fn a_part_named_twice_is_copied_once() {
        let msg = craft_mime::parse(PAGE, &craft_mime::Limits::DEFAULT);
        let pictures = Pictures::new(&msg.root, "file:///c:/page.htm", PICTURE_BUDGET);
        let (Some(a), Some(b)) = (pictures.load("a.png"), pictures.load(" a.png ")) else {
            panic!("the picture part was not found");
        };
        assert_eq!(a.as_slice(), b"abcdef");
        assert!(Arc::ptr_eq(&a, &b), "each reference made its own copy of the part");
        // A copy nobody keeps is not held for later references.
        drop((a, b));
        assert!(pictures.loaded.borrow().values().all(|w| w.strong_count() == 0));
        assert!(pictures.load("a.png").is_some());
    }

    #[test]
    fn every_reference_is_charged_to_the_budget() {
        let msg = craft_mime::parse(PAGE, &craft_mime::Limits::DEFAULT);
        let pictures = Pictures::new(&msg.root, "file:///c:/page.htm", 10);
        assert!(pictures.load("a.png").is_some());
        assert!(pictures.load("a.png").is_none(), "6 more bytes do not fit in the 4 left");
    }
}
