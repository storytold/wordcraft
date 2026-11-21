//! MHT / MHTML (Single File Web Page) import via `craft-mime` and `craft-html`.

use std::collections::HashMap;
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
    let by_location = locations(&msg.root);
    let left = std::cell::Cell::new(PICTURE_BUDGET);
    let loader = |src: &str| -> Option<Arc<Vec<u8>>> {
        let reference = src.trim();
        let cid = reference.get(..4).filter(|s| s.eq_ignore_ascii_case("cid:")).and(reference.get(4..));
        let part = match cid {
            Some(id) => resources.by_content_id(id),
            // A path names its part as written (RFC 2557 §8.2, RFC 3986 §6.2.2.1): the scheme
            // and host compare case-insensitively, the path does not, so `…/A.png` and `…/a.png`
            // are two different resources (`craft_mime::Resources` folds all letter case).
            None => by_location
                .get(&location_key(reference))
                .or_else(|| by_location.get(&location_key(&craft_mime::resolve_location(base, reference))))
                .copied(),
        }?;
        let data = part_bytes(part);
        left.set(left.get().checked_sub(data.len())?);
        Some(Arc::new(data.to_vec()))
    };
    crate::html::import_on(&html, &loader, page)
}

/// The leaves of the message by their `Content-Location`, for the references of a page to its
/// pictures. The first part of each location wins.
fn locations(root: &Part) -> HashMap<String, &Part> {
    let mut by_location = HashMap::new();
    for part in root.leaves() {
        if let Some(location) = part.content_location() {
            by_location.entry(location_key(location)).or_insert(part);
        }
    }
    by_location
}

/// What two locations must agree on to name the same resource: their scheme and host compare
/// without regard to ASCII letter case (RFC 3986 §6.2.2.1), their path and query stay as
/// written, and fragments go. A one-letter scheme is a Windows drive, not a scheme. A
/// scheme-relative `//host/path` has no scheme to fold, so its marker `//` stays and only the
/// host folds (§4.2) — inventing the page's scheme would keep two writings of one location apart.
fn location_key(location: &str) -> String {
    let uri = location.split('#').next().unwrap_or("");
    if let Some(rest) = uri.strip_prefix("//") {
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        return format!("//{}{}", rest.get(..end).unwrap_or("").to_ascii_lowercase(), rest.get(end..).unwrap_or(""));
    }
    let Some((scheme, rest)) = uri.split_once(':').filter(|(s, _)| {
        s.len() > 1 && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    }) else {
        return uri.to_string();
    };
    let (authority, path) = match rest.strip_prefix("//") {
        Some(r) => {
            let end = r.find(['/', '?']).unwrap_or(r.len());
            (Some(r.get(..end).unwrap_or("")), r.get(end..).unwrap_or(""))
        }
        None => (None, rest),
    };
    match authority {
        Some(host) => format!("{}://{}{}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase(), path),
        None => format!("{}:{}", scheme.to_ascii_lowercase(), path),
    }
}
