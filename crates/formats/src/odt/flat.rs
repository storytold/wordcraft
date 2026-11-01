//! Flat OpenDocument text (`.fodt`, ODF 1.3 Part 3 §2.1.2): the whole document in one
//! `office:document` element, its pictures inline (`office:binary-data`, base64) and its
//! formulas inline (`math:math` inside `draw:object`). Read by taking it apart into a package's
//! files and opening that; written by putting a package's files together.

use super::limits::MAX_ENTRY;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::io::{Cursor, Read, Write};
use wordcraft_doc::Document;

/// The top-level parts of a flat document, in the order a package's files take them.
const META: &str = "office:meta";
const SETTINGS: &str = "office:settings";
const FONTS: &str = "office:font-face-decls";
const STYLES: &str = "office:styles";
const AUTOMATIC: &str = "office:automatic-styles";
const MASTERS: &str = "office:master-styles";
const BODY: &str = "office:body";

/// Most pictures and formulas taken out of a flat document (hostile files).
const MAX_INLINE: usize = 10_000;

/// Element `name` in `xml` (its first one, start to end, or self-closed); "" when there is none.
fn element<'a>(xml: &'a str, name: &str) -> &'a str {
    let open = format!("<{name}");
    let Some(start) = xml
        .match_indices(&open)
        .map(|(i, _)| i)
        .find(|&i| xml.get(i + open.len()..).and_then(|r| r.chars().next()).is_some_and(|c| c == '>' || c == '/' || c.is_whitespace()))
    else {
        return "";
    };
    let Some(tag_end) = xml.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { return "" };
    if xml.get(..tag_end).is_some_and(|t| t.ends_with('/')) {
        return xml.get(start..=tag_end).unwrap_or("");
    }
    // Its end: the close tag at its own depth (a chart object inside holds an `office:body`,
    // an `office:document` of its own).
    let close = format!("</{name}>");
    let mut depth = 0usize;
    let mut at = tag_end;
    // Nesting is bounded like any XML here: deeper is malformed.
    for _ in 0..super::limits::MAX_XML_DEPTH.saturating_mul(2) {
        let rest = xml.get(at..).unwrap_or("");
        let next_close = rest.find(&close);
        let next_open = rest.match_indices(&open).map(|(i, _)| i).find(|&i| {
            let after = rest.get(i + open.len()..).unwrap_or("");
            after.chars().next().is_some_and(|c| c == '>' || c == '/' || c.is_whitespace())
                // Not self-closed.
                && after.find('>').and_then(|e| after.get(..e)).is_some_and(|t| !t.ends_with('/'))
        });
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                at += o + open.len();
            }
            (_, Some(c)) if depth > 0 => {
                depth -= 1;
                at += c + close.len();
            }
            (_, Some(c)) => return xml.get(start..at + c + close.len()).unwrap_or(""),
            // Unclosed (malformed): to the end.
            (_, None) => return xml.get(start..).unwrap_or(""),
        }
    }
    xml.get(start..).unwrap_or("")
}

/// What is inside element `el` (an element string as [`element`] gives).
fn inner(el: &str) -> &str {
    let Some(tag_end) = el.find('>') else { return "" };
    if el.get(..tag_end).is_some_and(|t| t.ends_with('/')) {
        return "";
    }
    let end = el.rfind("</").unwrap_or(el.len());
    el.get(tag_end + 1..end.max(tag_end + 1)).unwrap_or("")
}

/// The extension of picture `data`, by its first bytes.
fn picture_ext(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG") {
        "png"
    } else if data.starts_with(b"\xff\xd8") {
        "jpg"
    } else if data.starts_with(b"GIF8") {
        "gif"
    } else if data.starts_with(b"<svg") || data.starts_with(b"<?xml") {
        "svg"
    } else {
        "bin"
    }
}

/// Where the first `draw:object` holding its object inline (no `xlink:href`, not self-closed)
/// starts in `xml`.
fn inline_object(xml: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = xml.get(from..).and_then(|r| r.find("<draw:object")).map(|i| from + i) {
        let tag = xml.get(i..).and_then(|r| r.find('>').and_then(|e| r.get(..e))).unwrap_or("");
        let next = tag.get("<draw:object".len()..).and_then(|r| r.chars().next());
        if next.is_none_or(|c| c.is_whitespace()) && !tag.contains("xlink:href") && !tag.ends_with('/') {
            return Some(i);
        }
        from = i + "<draw:object".len();
    }
    None
}

/// `xml` with its inline pictures and formulas moved into `files` (named as a package names
/// them) and pointed at there, as a package's content holds them.
fn take_out(xml: &str, files: &mut Vec<(String, Vec<u8>)>) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    loop {
        let pic = rest.find("<office:binary-data>");
        let obj = inline_object(rest);
        let (at, picture) = match (pic, obj) {
            (Some(p), Some(o)) if o < p => (o, false),
            (Some(p), _) => (p, true),
            (None, Some(o)) => (o, false),
            (None, None) => break,
        };
        let (before, from) = rest.split_at(at);
        let close = if picture { "</office:binary-data>" } else { "</draw:object>" };
        let Some(end) = from.find(close) else {
            out.push_str(before);
            rest = from;
            break;
        };
        let open_len = if picture { "<office:binary-data>".len() } else { from.find('>').map_or(0, |e| e + 1) };
        let body = from.get(open_len..end).unwrap_or("");
        let after = from.get(end + close.len()..).unwrap_or("");
        let n = files.len() + 1;
        if files.len() >= MAX_INLINE {
            out.push_str(before);
            rest = after;
            continue;
        }
        if picture {
            // The picture's `draw:image` start tag (just before) gets the package's reference.
            let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
            let data = STANDARD.decode(b64.as_bytes()).ok().filter(|d| d.len() <= MAX_ENTRY as usize).unwrap_or_default();
            let name = format!("Pictures/flat{n}.{}", picture_ext(&data));
            match before.rfind("<draw:image") {
                Some(img) if !data.is_empty() => {
                    let (head, tag) = before.split_at(img);
                    out.push_str(head);
                    out.push_str(&format!("<draw:image xlink:href=\"{name}\""));
                    out.push_str(tag.get("<draw:image".len()..).unwrap_or(""));
                    files.push((name, data));
                }
                _ => out.push_str(before),
            }
        } else {
            let dir = format!("Object flat{n}");
            out.push_str(before);
            out.push_str(&format!("<draw:object xlink:href=\"./{dir}\"/>"));
            files.push((format!("{dir}/content.xml"), body.as_bytes().to_vec()));
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Open a flat OpenDocument text document.
pub fn import_flat(bytes: &[u8]) -> Result<Document, String> {
    let xml = std::str::from_utf8(bytes).map_err(|_| "FODT: not UTF-8 text".to_string())?;
    let root = element(xml, "office:document");
    let attrs =
        root.find('>').and_then(|e| root.get("<office:document".len()..e)).map(|a| a.trim_end_matches('/')).ok_or("FODT: no office:document")?;
    let doc = |name: &str, parts: &[&str]| -> String {
        let body: String = parts.iter().map(|p| element(root, p)).collect();
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:{name}{attrs}>{body}</office:{name}>")
    };
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let content = take_out(&doc("document-content", &[FONTS, AUTOMATIC, BODY]), &mut files);
    let styles = take_out(&doc("document-styles", &[FONTS, STYLES, AUTOMATIC, MASTERS]), &mut files);
    let entries: Vec<(String, Vec<u8>)> = [
        ("mimetype".to_string(), b"application/vnd.oasis.opendocument.text".to_vec()),
        ("content.xml".to_string(), content.into_bytes()),
        ("styles.xml".to_string(), styles.into_bytes()),
        ("meta.xml".to_string(), doc("document-meta", &[META]).into_bytes()),
        ("settings.xml".to_string(), doc("document-settings", &[SETTINGS]).into_bytes()),
    ]
    .into_iter()
    .chain(files)
    .collect();
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in &entries {
        z.start_file(name.as_str(), stored).map_err(|e| format!("FODT: {e}"))?;
        z.write_all(data).map_err(|e| format!("FODT: {e}"))?;
    }
    let package = z.finish().map_err(|e| format!("FODT: {e}"))?.into_inner();
    super::import(&package)
}

/// Save `doc` as flat OpenDocument text: its package's files in one `office:document`, the
/// automatic styles of its content and its styles together, pictures and formulas inline.
pub fn export_flat(doc: &Document) -> Result<Vec<u8>, String> {
    let package = super::export(doc)?;
    let mut z = zip::ZipArchive::new(Cursor::new(package)).map_err(|e| format!("FODT: {e}"))?;
    let mut read = |name: &str| -> String {
        let mut s = String::new();
        if let Ok(mut f) = z.by_name(name) {
            let _ = f.read_to_string(&mut s);
        }
        s
    };
    let (content, styles, meta, settings) = (read("content.xml"), read("styles.xml"), read("meta.xml"), read("settings.xml"));
    let mut bytes_of = |name: &str| -> Vec<u8> {
        let mut v = Vec::new();
        if let Ok(mut f) = z.by_name(name) {
            let _ = f.read_to_end(&mut v);
        }
        v
    };
    let automatic = format!("<{AUTOMATIC}>{}{}</{AUTOMATIC}>", inner(element(&content, AUTOMATIC)), inner(element(&styles, AUTOMATIC)));
    let mut body = String::new();
    for part in [
        element(&meta, META),
        element(&settings, SETTINGS),
        element(&styles, FONTS),
        element(&content, FONTS),
        element(&styles, STYLES),
        &automatic,
        element(&styles, MASTERS),
        element(&content, BODY),
    ] {
        body.push_str(part);
    }
    // Pictures and formulas in the document, not beside it.
    let mut flat = String::with_capacity(body.len());
    let mut rest = body.as_str();
    while let Some(at) = rest.find("xlink:href=\"") {
        let (before, from) = rest.split_at(at);
        let target_start = "xlink:href=\"".len();
        let Some(len) = from.get(target_start..).and_then(|r| r.find('"')) else { break };
        let target = from.get(target_start..target_start + len).unwrap_or("");
        let after = from.get(target_start + len + 1..).unwrap_or("");
        let tag = before.rfind('<').and_then(|i| before.get(i..)).unwrap_or("");
        if tag.starts_with("<draw:image") && target.starts_with("Pictures/") {
            // `<draw:image xlink:href="…" …/>` → `<draw:image …><office:binary-data>…`.
            let data = bytes_of(target);
            let rest_tag_end = after.find("/>").unwrap_or(0);
            // The link's other attributes go with it.
            let attrs: String =
                after.get(..rest_tag_end).unwrap_or("").split_whitespace().filter(|a| !a.starts_with("xlink:")).map(|a| format!(" {a}")).collect();
            flat.push_str(before.trim_end());
            flat.push_str(&attrs);
            flat.push_str(&format!("><office:binary-data>{}</office:binary-data></draw:image>", STANDARD.encode(&data)));
            rest = after.get(rest_tag_end + 2..).unwrap_or("");
        } else if tag.starts_with("<draw:object") && target.starts_with("./Object ") {
            let math = bytes_of(&format!("{}/content.xml", target.trim_start_matches("./")));
            let math = String::from_utf8_lossy(&math);
            let math = math.split_once("?>").map_or(math.as_ref(), |(_, m)| m).trim();
            // A sub-document (a chart) goes in as an `office:document` (§3.1.2), its root.
            let math = match math.strip_prefix("<office:document-content").and_then(|m| m.strip_suffix("</office:document-content>")) {
                Some(inner) => format!("<office:document{inner}</office:document>"),
                None => math.to_string(),
            };
            // A plain `draw:object` around it: the reference's other attributes go with it.
            let rest_tag_end = after.find("/>").unwrap_or(0);
            flat.push_str(before.rfind("<draw:object").and_then(|i| before.get(..i)).unwrap_or(before));
            flat.push_str(&format!("<draw:object>{math}</draw:object>"));
            rest = after.get(rest_tag_end + 2..).unwrap_or("");
        } else {
            flat.push_str(before);
            flat.push_str(from.get(..target_start + len + 1).unwrap_or(""));
            rest = after;
        }
    }
    flat.push_str(rest);
    // Every namespace the parts declare (the settings' `config`, say), each prefix once.
    let mut ns: Vec<String> = Vec::new();
    for part in [&content, &styles, &meta, &settings] {
        for decl in namespaces(part) {
            let prefix = decl.split('=').next().unwrap_or("");
            if !ns.iter().any(|d| d.split('=').next() == Some(prefix)) {
                ns.push(decl.to_string());
            }
        }
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document {} office:version=\"1.3\" office:mimetype=\"application/vnd.oasis.opendocument.text\">{flat}</office:document>\n",
        ns.join(" ")
    )
    .into_bytes())
}

/// The namespace declarations (`xmlns:p="…"`) on the root element of `xml`.
fn namespaces(xml: &str) -> Vec<&str> {
    let start = xml.find("<office:").unwrap_or(0);
    let tag = xml.get(start..).and_then(|r| r.find('>').and_then(|e| r.get(..e))).unwrap_or("");
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(i) = rest.find("xmlns:") {
        let from = rest.get(i..).unwrap_or("");
        // `xmlns:p="…"`: up to the closing quote.
        let Some(q1) = from.find('"') else { break };
        let Some(q2) = from.get(q1 + 1..).and_then(|r| r.find('"')) else { break };
        out.push(from.get(..q1 + 1 + q2 + 1).unwrap_or(""));
        rest = from.get(q1 + 1 + q2 + 1..).unwrap_or("");
    }
    out
}
