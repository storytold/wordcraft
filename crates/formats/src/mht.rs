//! MHT / MHTML (Single File Web Page): a web page and its pictures in one MIME message (RFC 2557
//! `multipart/related`, RFC 2045 transfer encodings). The page is the part the message's `start`
//! parameter names, else its first HTML part; it is read by [`crate::html`], with `img` sources
//! found among the other parts by `Content-Location` (also relative to the page's) or `cid:`
//! `Content-ID`, and the page opens as Word opens one ([`crate::html::import_page`]). Word keeps
//! HTML `w:altChunk`s in this form too.

use std::collections::HashMap;
use std::sync::Arc;

use wordcraft_doc::Document;

/// Most parts read from one message (nested multiparts included).
const MAX_PARTS: usize = 10_000;
/// Deepest `multipart` nesting followed.
const MAX_NESTING: usize = 8;
/// Longest header block looked at for one part.
const MAX_HEADER: usize = 64 * 1024;
/// Longest boundary (RFC 2046 §5.1.1).
const MAX_BOUNDARY: usize = 70;
/// Picture bytes one page may load, counting every `img` that names a part.
const PICTURE_BUDGET: usize = 1 << 30;

/// One leaf part of the message: its headers (names lower-cased) and decoded body.
struct Part {
    content_type: String,
    location: Option<String>,
    id: Option<String>,
    /// Shared by every `img` that names it.
    body: Arc<Vec<u8>>,
}

impl Part {
    fn mime(&self) -> String {
        self.content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
    }
}

/// Does `bytes` look like a MIME message (an `.mht` file)?
pub fn is_mht(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(4096)).unwrap_or(bytes)).to_ascii_lowercase();
    head.starts_with("mime-version:") || head.starts_with("content-type: multipart/") || head.contains("\ncontent-type: multipart/related")
}

/// Import an MHT file. A message without an HTML part reads as its text (an empty document
/// when there's none).
pub fn import(bytes: &[u8]) -> Document {
    import_on(bytes, &wordcraft_doc::SectionProps::default())
}

/// [`import`] onto `page`: like HTML, an MHT file gives no page size (#1381).
pub fn import_on(bytes: &[u8], page: &wordcraft_doc::SectionProps) -> Document {
    let mut parts = Vec::new();
    let (headers, body) = split_headers(bytes);
    let start = param(header(&headers, "content-type").unwrap_or(""), "start").map(|s| s.trim_matches(['<', '>']).to_string());
    collect(&headers, body, &mut parts, 0);
    let root = start
        .as_deref()
        .and_then(|s| parts.iter().position(|p| p.id.as_deref() == Some(s)))
        .or_else(|| parts.iter().position(|p| matches!(p.mime().as_str(), "text/html" | "application/xhtml+xml")));
    let Some(root) = root.and_then(|i| parts.get(i)) else {
        let text = parts.iter().find(|p| p.mime() == "text/plain").map(|p| p.body.as_slice()).unwrap_or(&[]);
        return crate::txt::import_on(text, page);
    };
    let html = crate::html::decode_as(&root.body, param(&root.content_type, "charset"));
    let base = root.location.as_deref().unwrap_or("");
    // Parts by `Content-Location` (lower case) and `Content-ID`; the first of each wins.
    let mut by_location = HashMap::new();
    let mut by_id = HashMap::new();
    for p in &parts {
        if let Some(l) = &p.location {
            by_location.entry(l.to_ascii_lowercase()).or_insert(p);
        }
        if let Some(i) = &p.id {
            by_id.entry(i.as_str()).or_insert(p);
        }
    }
    let left = std::cell::Cell::new(PICTURE_BUDGET);
    let loader = |src: &str| -> Option<Arc<Vec<u8>>> {
        let src = src.trim();
        let part = match src.get(..4).filter(|s| s.eq_ignore_ascii_case("cid:")).and(src.get(4..)) {
            Some(cid) => by_id.get(cid),
            // By its location as written, or resolved against the page's.
            None => by_location.get(&src.to_ascii_lowercase()).or_else(|| by_location.get(&resolve(base, src).to_ascii_lowercase())),
        }?;
        left.set(left.get().checked_sub(part.body.len())?);
        Some(part.body.clone())
    };
    crate::html::import_on(&html, &loader, page)
}

/// `reference` resolved against the page's location `base`, as RFC 2557 §5 asks (RFC 3986
/// §5.2.2): a reference with a scheme stays as it is, `//host/…` takes the base's scheme, `/…`
/// its scheme and host, an empty path its path and query, anything else is merged with its path
/// (§5.2.3); `.` and `..` segments go (§5.2.4) and fragments are dropped. Nothing is fetched.
fn resolve(base: &str, reference: &str) -> String {
    let (b, r) = (UriParts::of(base), UriParts::of(reference));
    let t = if r.scheme.is_some() {
        UriParts { path: remove_dots(&r.path), ..r }
    } else if r.authority.is_some() {
        UriParts { scheme: b.scheme, path: remove_dots(&r.path), ..r }
    } else if r.path.is_empty() {
        UriParts { query: r.query.or(b.query), ..b }
    } else if r.path.starts_with('/') {
        UriParts { path: remove_dots(&r.path), query: r.query, ..b }
    } else {
        let merged = if b.authority.is_some() && b.path.is_empty() {
            format!("/{}", r.path)
        } else {
            format!("{}{}", b.path.rfind('/').and_then(|i| b.path.get(..=i)).unwrap_or(""), r.path)
        };
        UriParts { path: remove_dots(&merged), query: r.query, ..b }
    };
    t.to_string()
}

/// A URI's parts (RFC 3986 §3): `scheme:`, `//authority`, path and `?query`; the fragment left out.
struct UriParts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: String,
    query: Option<&'a str>,
}

impl<'a> UriParts<'a> {
    fn of(uri: &'a str) -> Self {
        let uri = uri.split('#').next().unwrap_or("");
        // A one-letter scheme is a Windows drive (`C:/…`), not a scheme.
        let scheme = uri.split_once(':').map(|(k, _)| k).filter(|k| {
            k.len() > 1 && k.starts_with(|c: char| c.is_ascii_alphabetic()) && k.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        });
        let rest = scheme.and_then(|k| uri.get(k.len() + 1..)).unwrap_or(uri);
        let (authority, rest) = match rest.strip_prefix("//") {
            Some(r) => {
                let end = r.find(['/', '?']).unwrap_or(r.len());
                (r.get(..end), r.get(end..).unwrap_or(""))
            }
            None => (None, rest),
        };
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        UriParts { scheme, authority, path: path.to_string(), query }
    }
}

impl std::fmt::Display for UriParts<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(s) = self.scheme {
            write!(f, "{s}:")?;
        }
        if let Some(a) = self.authority {
            write!(f, "//{a}")?;
        }
        f.write_str(&self.path)?;
        if let Some(q) = self.query {
            write!(f, "?{q}")?;
        }
        Ok(())
    }
}

/// `path` without its `.` and `..` segments (RFC 3986 §5.2.4); `..` stops at the root.
fn remove_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segments: Vec<&str> = path.split('/').collect();
    for (i, seg) in segments.iter().enumerate() {
        match *seg {
            "." => {}
            ".." => {
                if out.len() > 1 || out.first().is_some_and(|f| !f.is_empty()) {
                    out.pop();
                }
            }
            s => out.push(s),
        }
        if i + 1 == segments.len() && matches!(*seg, "." | "..") {
            out.push("");
        }
    }
    out.join("/")
}

/// Split a part into its unfolded header lines and its body (after the first empty line).
fn split_headers(b: &[u8]) -> (Vec<(String, String)>, &[u8]) {
    // Whichever blank line comes first ends the headers.
    let blank = [find_bytes(b, b"\r\n\r\n").map(|i| (i, 4)), find_bytes(b, b"\n\n").map(|i| (i, 2))].into_iter().flatten().min();
    let (head, body) = match blank {
        Some((i, n)) => (b.get(..i).unwrap_or(&[]), b.get(i + n..).unwrap_or(&[])),
        // Headers only, or a body without them.
        None if b.first().is_some_and(|c| c.is_ascii_alphabetic()) && find_bytes(b, b":").is_some() => (b, &[][..]),
        None => (&[][..], b),
    };
    let head = String::from_utf8_lossy(head.get(..head.len().min(MAX_HEADER)).unwrap_or(head));
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = headers.last_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    (headers, body)
}

fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// A `; name=value` parameter of a header value (quotes removed), name case-insensitive.
fn param<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    value.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().trim_matches('"'))
    })
}

/// Gather the leaf parts of a (possibly multipart) entity.
fn collect(headers: &[(String, String)], body: &[u8], out: &mut Vec<Part>, depth: usize) {
    if out.len() >= MAX_PARTS {
        return;
    }
    let ct = header(headers, "content-type").unwrap_or("text/plain").to_string();
    if ct.trim_start().to_ascii_lowercase().starts_with("multipart/") {
        let Some(boundary) = param(&ct, "boundary").filter(|b| !b.is_empty() && b.len() <= MAX_BOUNDARY) else { return };
        if depth >= MAX_NESTING {
            return;
        }
        for chunk in multipart_bodies(body, boundary.as_bytes()) {
            let (h, b) = split_headers(chunk);
            collect(&h, b, out, depth + 1);
            if out.len() >= MAX_PARTS {
                return;
            }
        }
        return;
    }
    let enc = header(headers, "content-transfer-encoding").unwrap_or("").trim().to_ascii_lowercase();
    let body = match enc.as_str() {
        "quoted-printable" => quoted_printable(body),
        "base64" => crate::model::base64_decode(&String::from_utf8_lossy(body)).unwrap_or_default(),
        _ => body.to_vec(),
    };
    let clean = |s: &str| s.trim().trim_matches(['<', '>']).to_string();
    out.push(Part {
        content_type: ct,
        location: header(headers, "content-location").map(clean),
        id: header(headers, "content-id").map(clean),
        body: Arc::new(body),
    });
}

/// The bodies between `--boundary` lines, up to the closing `--boundary--`.
fn multipart_bodies<'a>(body: &'a [u8], boundary: &[u8]) -> Vec<&'a [u8]> {
    let mut delim = b"--".to_vec();
    delim.extend_from_slice(boundary);
    let mut out = Vec::new();
    // Delimiters sit at line starts.
    let mut starts = Vec::new();
    let mut i = 0;
    while let Some(k) = body.get(i..).and_then(|rest| find_bytes(rest, &delim)) {
        let at = i + k;
        if at == 0 || body.get(at - 1) == Some(&b'\n') {
            starts.push(at);
            if starts.len() > MAX_PARTS {
                break;
            }
        }
        i = at + delim.len();
    }
    for w in starts.windows(2) {
        let (Some(&a), Some(&b)) = (w.first(), w.get(1)) else { continue };
        let Some(seg) = body.get(a + delim.len()..b) else { continue };
        if seg.starts_with(b"--") {
            break;
        }
        // The rest of the delimiter line, then the part; the line break before the next
        // delimiter belongs to it.
        let seg = match find_bytes(seg, b"\n") {
            Some(n) => seg.get(n + 1..).unwrap_or(&[]),
            None => &[],
        };
        let seg = seg.strip_suffix(b"\r\n").or_else(|| seg.strip_suffix(b"\n")).unwrap_or(seg);
        out.push(seg);
        if out.len() >= MAX_PARTS {
            break;
        }
    }
    out
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Decode quoted-printable (RFC 2045 §6.7): `=XX` bytes and `=` soft line breaks; anything
/// malformed is kept as it stands.
fn quoted_printable(b: &[u8]) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c != b'=' {
            out.push(c);
            i += 1;
            continue;
        }
        match (b.get(i + 1).copied(), b.get(i + 2).copied()) {
            (Some(b'\r'), Some(b'\n')) => i += 3,
            (Some(b'\n'), _) => i += 2,
            (Some(h), Some(l)) if let (Some(h), Some(l)) = (hex(h), hex(l)) => {
                out.push((h << 4) | l);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{Block, InlineObject};

    const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    fn message(html_part: &str) -> String {
        format!(
            "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"----=_B\"\r\n\r\n\
             ------=_B\r\n{html_part}\r\n\
             ------=_B\r\nContent-Location: file:///C:/page_files/dot.png\r\nContent-Transfer-Encoding: base64\r\nContent-Type: image/png\r\n\r\n{PNG_1X1}\r\n\
             ------=_B--\r\n"
        )
    }

    #[test]
    fn reads_the_quoted_printable_page_and_its_pictures() {
        let page = "Content-Location: file:///C:/page.htm\r\nContent-Transfer-Encoding: quoted-printable\r\nContent-Type: text/html; charset=\"utf-8\"\r\n\r\n\
                    <html><body><p class=3D\"x\">Dobar dan, gospo=C4=91o, ovo je =\r\njedan red.</p><p><img src=3D\"page_files/dot.png\" width=3D\"10\" height=3D\"10\"></p></body></html>";
        let d = import(message(page).as_bytes());
        let text = d.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(text.contains("Dobar dan, gospođo, ovo je jedan red."), "{text:?}");
        let pictures =
            d.body.iter().filter(|b| matches!(&***b, Block::Para(p) if p.objects.iter().any(|o| matches!(o, InlineObject::Image { .. })))).count();
        assert_eq!(pictures, 1);
        assert_eq!(d.media.len(), 1);
    }

    #[test]
    fn pictures_named_relative_to_the_page_resolve_as_uris() {
        // The page in `reports/`, its picture in `images/` (RFC 2557 §5: relative URIs resolve
        // against the page's Content-Location).
        let m = |src: &str| {
            format!(
                "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=b\r\n\r\n\
                 --b\r\nContent-Location: https://example.test/reports/page.html\r\nContent-Type: text/html\r\n\r\n<p>x</p><img src=\"{src}\">\r\n\
                 --b\r\nContent-Location: https://example.test/images/logo.png\r\nContent-Transfer-Encoding: base64\r\nContent-Type: image/png\r\n\r\n{PNG_1X1}\r\n--b--\r\n"
            )
        };
        for src in [
            "../images/logo.png",
            "/images/logo.png",
            "./../images/./logo.png",
            "//example.test/images/logo.png",
            "https://example.test/images/logo.png",
        ] {
            assert_eq!(import(m(src).as_bytes()).media.len(), 1, "{src}");
        }
        // Nothing past the root: `..` there stays at the root.
        assert_eq!(import(m("../../../images/logo.png").as_bytes()).media.len(), 1);
        assert_eq!(import(m("images/logo.png").as_bytes()).media.len(), 0);
        assert_eq!(resolve("https://h/a/b/c.html", "../d/./e.png#f"), "https://h/a/d/e.png");
        assert_eq!(resolve("file:///C:/x/page.htm", "page_files/dot.png"), "file:///C:/x/page_files/dot.png");
        assert_eq!(resolve("", "dot.png"), "dot.png");
        // A host with no path, and a base whose query holds slashes (RFC 3986 §5.2.3: the path
        // merges, not the query; an authority with an empty path gets its `/`).
        assert_eq!(resolve("https://example.test", "logo.png"), "https://example.test/logo.png");
        assert_eq!(resolve("https://example.test/page.html?redirect=/other/", "logo.png"), "https://example.test/logo.png");
        assert_eq!(resolve("https://h/a/p.html?q", "?r"), "https://h/a/p.html?r");
        assert_eq!(resolve("https://h/a/p.html?q", "x.png?v=1"), "https://h/a/x.png?v=1");
        assert_eq!(resolve("https://h/a/p.html", "mailto:x@y"), "mailto:x@y");
        for page in ["https://example.test", "https://example.test/page.html?redirect=/other/"] {
            let m = format!(
                "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=b\r\n\r\n\
                 --b\r\nContent-Location: {page}\r\nContent-Type: text/html\r\n\r\n<p>x</p><img src=\"logo.png\">\r\n\
                 --b\r\nContent-Location: https://example.test/logo.png\r\nContent-Transfer-Encoding: base64\r\nContent-Type: image/png\r\n\r\n{PNG_1X1}\r\n--b--\r\n"
            );
            assert_eq!(import(m.as_bytes()).media.len(), 1, "{page}");
        }
    }

    #[test]
    fn the_part_charset_decides_the_text() {
        let page = "Content-Type: text/html; charset=windows-1250\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<p>=E8lanak</p>";
        let d = import(message(page).as_bytes());
        assert_eq!(d.plain_text(wordcraft_doc::StoryRef::Body).trim(), "članak");
    }

    #[test]
    fn the_start_part_is_the_page_and_cid_names_pictures() {
        let m = format!(
            "MIME-Version: 1.0\nContent-Type: multipart/related; type=\"text/html\"; start=\"<page>\"; boundary=b\n\n\
             --b\nContent-Type: text/html\n\n<p>Not this one</p>\n\
             --b\nContent-ID: <page>\nContent-Type: text/html\n\n<p>This one</p><img src=\"cid:dot\">\n\
             --b\nContent-ID: <dot>\nContent-Transfer-Encoding: base64\nContent-Type: image/png\n\n{PNG_1X1}\n--b--\n"
        );
        let d = import(m.as_bytes());
        assert_eq!(d.plain_text(wordcraft_doc::StoryRef::Body).trim(), "This one");
        assert_eq!(d.media.len(), 1);
        // Without a page, a message reads as its text.
        let d = import(b"MIME-Version: 1.0\nContent-Type: multipart/mixed; boundary=b\n\n--b\nContent-Type: text/plain\n\nJust text\n--b--\n");
        assert_eq!(d.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Just text");
        assert!(is_mht(m.as_bytes()) && !is_mht(b"<html><p>x</p>"));
    }

    #[test]
    fn hostile_messages_never_panic() {
        for m in [
            &b""[..],
            b"MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"\"\r\n\r\n",
            b"MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\n--x--",
            b"MIME-Version: 1.0\nContent-Type: multipart/related; boundary=x\n\n--x\nContent-Transfer-Encoding: quoted-printable\nContent-Type: text/html\n\n=\n=Z=4",
            b"Content-Type: text/html\r\nContent-Transfer-Encoding: base64\r\n\r\n!!!!",
        ] {
            let _ = import(m);
        }
        let mut deep = String::from("MIME-Version: 1.0\r\n");
        for i in 0..64 {
            deep.push_str(&format!("Content-Type: multipart/related; boundary=b{i}\r\n\r\n--b{i}\r\n"));
        }
        let _ = import(deep.as_bytes());
    }
}
