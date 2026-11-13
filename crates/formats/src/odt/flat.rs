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
    // Every step moves past a tag, so this ends; nesting is bounded like any XML here (deeper is
    // malformed), however many elements of the name sit side by side within.
    loop {
        let rest = xml.get(at..).unwrap_or("");
        let next_close = rest.find(&close);
        // Only opens before that close count (and only they are looked for).
        let before = next_close.and_then(|c| rest.get(..c)).unwrap_or(rest);
        let next_open = before.match_indices(&open).map(|(i, _)| i).find(|&i| {
            let after = rest.get(i + open.len()..).unwrap_or("");
            after.chars().next().is_some_and(|c| c == '>' || c == '/' || c.is_whitespace())
                // Not self-closed.
                && after.find('>').and_then(|e| after.get(..e)).is_some_and(|t| !t.ends_with('/'))
        });
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                if depth > super::limits::MAX_XML_DEPTH {
                    return xml.get(start..).unwrap_or("");
                }
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
            // The element holding the data: the start tag just before it.
            let holder = before.rfind('<').filter(|_| !data.is_empty()).and_then(|i| {
                let tag = before.get(i..).unwrap_or("");
                ["<draw:image", "<draw:object-ole"]
                    .into_iter()
                    .find(|el| tag.starts_with(el) && tag.get(el.len()..).is_some_and(|r| r.starts_with([' ', '>', '\n', '\t', '\r'])))
                    .map(|el| (i, el))
            });
            match holder {
                Some((at, el)) => {
                    let name = if el == "<draw:image" { format!("Pictures/flat{n}.{}", picture_ext(&data)) } else { format!("Object flat{n}") };
                    let (head, tag) = before.split_at(at);
                    out.push_str(head);
                    out.push_str(&format!("{el} xlink:href=\"{}\"", if el == "<draw:image" { name.clone() } else { format!("./{name}") }));
                    out.push_str(tag.get(el.len()..).unwrap_or(""));
                    files.push((name, data));
                }
                None => out.push_str(before),
            }
        } else {
            let dir = format!("Object flat{n}");
            out.push_str(before);
            out.push_str(&format!("<draw:object xlink:href=\"./{dir}\"/>"));
            // A sub-document (a chart, a spreadsheet) goes back into its files; a formula is
            // its `content.xml`.
            match package_parts(body.trim()) {
                Some(parts) => {
                    for (name, xml) in ["content.xml", "styles.xml", "meta.xml", "settings.xml"].into_iter().zip(parts) {
                        files.push((format!("{dir}/{name}"), xml.into_bytes()));
                    }
                }
                None => files.push((format!("{dir}/content.xml"), body.as_bytes().to_vec())),
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Open a flat OpenDocument text document.
pub fn import_flat(bytes: &[u8]) -> Result<Document, String> {
    import_flat_on(bytes, &wordcraft_doc::SectionProps::default())
}

/// [`import_flat`] onto `page` (see [`super::import_on`]).
pub fn import_flat_on(bytes: &[u8], page: &wordcraft_doc::SectionProps) -> Result<Document, String> {
    let xml = std::str::from_utf8(bytes).map_err(|_| "FODT: not UTF-8 text".to_string())?;
    let root = element(xml, "office:document");
    let [content, styles, meta, settings] = package_parts(root).ok_or("FODT: no office:document")?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let content = take_out(&content, &mut files);
    let styles = take_out(&styles, &mut files);
    let entries: Vec<(String, Vec<u8>)> = [
        ("mimetype".to_string(), b"application/vnd.oasis.opendocument.text".to_vec()),
        ("content.xml".to_string(), content.into_bytes()),
        ("styles.xml".to_string(), styles.into_bytes()),
        ("meta.xml".to_string(), meta.into_bytes()),
        ("settings.xml".to_string(), settings.into_bytes()),
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
    super::import_on(&package, page)
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
    let manifest = super::objects::manifest(&bytes_of("META-INF/manifest.xml"));
    let body = flat_parts(&content, &styles, &meta, &settings);
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
        // A picture, or an embedded object's replacement picture.
        let picture = target.starts_with("Pictures/") || target.trim_start_matches("./").starts_with("ObjectReplacements/");
        // The rest of the linking element's start tag, whether it closes itself, and what follows
        // the tag (markup kept as read may have `<draw:image …></draw:image>`).
        let gt = after.find('>').unwrap_or(after.len());
        let closed = after.get(..gt).is_some_and(|t| t.ends_with('/'));
        let tag_rest = after.get(..gt).unwrap_or("").trim_end_matches('/');
        let past_tag = after.get(gt + 1..).unwrap_or("");
        // What follows the element: past its tag, or past its end tag.
        let past = |el: &str| {
            if closed { past_tag } else { past_tag.find(&format!("</{el}>")).and_then(|e| past_tag.get(e + el.len() + 3..)).unwrap_or("") }
        };
        // The link's other attributes go with it.
        let attrs: String = tag_rest.split_whitespace().filter(|a| !a.starts_with("xlink:")).map(|a| format!(" {a}")).collect();
        if tag.starts_with("<draw:image") && picture {
            // `<draw:image xlink:href="…" …/>` → `<draw:image …><office:binary-data>…`, before
            // anything else it holds.
            let data = bytes_of(target.trim_start_matches("./"));
            flat.push_str(before.trim_end());
            flat.push_str(&attrs);
            flat.push_str(&format!("><office:binary-data>{}</office:binary-data>", STANDARD.encode(&data)));
            if closed {
                flat.push_str("</draw:image>");
            }
            rest = past_tag;
        } else if tag.starts_with("<draw:object-ole") && target.starts_with("./Object ") {
            // An OLE object's file inline (§10.4.7 allows `office:binary-data` in it).
            let data = bytes_of(target.trim_start_matches("./"));
            flat.push_str(before.rfind("<draw:object-ole").and_then(|i| before.get(..i)).unwrap_or(before));
            flat.push_str(&format!("<draw:object-ole><office:binary-data>{}</office:binary-data></draw:object-ole>", STANDARD.encode(&data)));
            rest = past("draw:object-ole");
        } else if tag.starts_with("<draw:object") && target.starts_with("./Object ") {
            let dir = target.trim_start_matches("./");
            let mut text = |name: &str| String::from_utf8_lossy(&bytes_of(&format!("{dir}/{name}"))).into_owned();
            let content = text("content.xml");
            // A sub-document (a chart, a spreadsheet) goes in as an `office:document` (§3.1.2)
            // of its media type, its files together; a formula as its `math:math`.
            let math = if content.contains("<office:document-content") {
                let mime = manifest.get(&format!("{dir}/")).map_or("application/vnd.oasis.opendocument.graphics", String::as_str);
                let (styles, meta, settings) = (text("styles.xml"), text("meta.xml"), text("settings.xml"));
                flat_document(mime, &content, &styles, &meta, &settings)
            } else {
                content.split_once("?>").map_or(content.as_str(), |(_, m)| m).trim().to_string()
            };
            // A plain `draw:object` around it.
            flat.push_str(before.rfind("<draw:object").and_then(|i| before.get(..i)).unwrap_or(before));
            flat.push_str(&format!("<draw:object>{math}</draw:object>"));
            rest = past("draw:object");
        } else {
            flat.push_str(before);
            flat.push_str(from.get(..target_start + len + 1).unwrap_or(""));
            rest = after;
        }
    }
    flat.push_str(rest);
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document {} office:version=\"1.3\" office:mimetype=\"application/vnd.oasis.opendocument.text\">{flat}</office:document>\n",
        merged_namespaces(&[&content, &styles, &meta, &settings])
    )
    .into_bytes())
}

/// Every namespace the parts declare (the settings' `config`, say), each prefix once.
fn merged_namespaces(parts: &[&str]) -> String {
    let mut ns: Vec<&str> = Vec::new();
    for part in parts {
        for decl in namespaces(part) {
            let prefix = decl.split('=').next().unwrap_or("");
            if !ns.iter().any(|d| d.split('=').next() == Some(prefix)) {
                ns.push(decl);
            }
        }
    }
    ns.join(" ")
}

/// What a flat document holds, from a package's parts in the order §3.1.2 gives them: one
/// font declaration list and one automatic styles list for its content and its styles together.
fn flat_parts(content: &str, styles: &str, meta: &str, settings: &str) -> String {
    let both = |name: &str| {
        let (a, b) = (element(styles, name), element(content, name));
        if a.is_empty() && b.is_empty() { String::new() } else { format!("<{name}>{}{}</{name}>", inner(a), inner(b)) }
    };
    [
        element(meta, META).to_string(),
        element(settings, SETTINGS).to_string(),
        both(FONTS),
        element(styles, STYLES).to_string(),
        both(AUTOMATIC),
        element(styles, MASTERS).to_string(),
        element(content, BODY).to_string(),
    ]
    .concat()
}

/// A sub-document's parts as one `office:document` of media type `mime`.
fn flat_document(mime: &str, content: &str, styles: &str, meta: &str, settings: &str) -> String {
    format!(
        "<office:document {} office:version=\"1.3\" office:mimetype=\"{}\">{}</office:document>",
        merged_namespaces(&[content, styles, meta, settings]),
        super::writer::x(mime),
        flat_parts(content, styles, meta, settings)
    )
}

/// An `office:document` element taken apart into a package's `content.xml`, `styles.xml`,
/// `meta.xml` and `settings.xml` (each part's root with the document's attributes but its
/// media type). `None` when `root` is no `office:document`.
fn package_parts(root: &str) -> Option<[String; 4]> {
    if !root.starts_with("<office:document") || root.starts_with("<office:document-") {
        return None;
    }
    let attrs = root.find('>').and_then(|e| root.get("<office:document".len()..e)).map(|a| a.trim_end_matches('/'))?;
    // Without its media type, which only the flat root has.
    let attrs = match attrs.find(" office:mimetype=\"") {
        Some(i) => {
            let from = i + " office:mimetype=\"".len();
            let end = attrs.get(from..).and_then(|r| r.find('"')).map_or(attrs.len(), |e| from + e + 1);
            format!("{}{}", attrs.get(..i).unwrap_or(""), attrs.get(end..).unwrap_or(""))
        }
        None => attrs.to_string(),
    };
    let doc = |name: &str, parts: &[&str]| -> String {
        let body: String = parts.iter().map(|p| element(root, p)).collect();
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:{name}{attrs}>{body}</office:{name}>")
    };
    Some([
        doc("document-content", &[FONTS, AUTOMATIC, BODY]),
        doc("document-styles", &[FONTS, STYLES, AUTOMATIC, MASTERS]),
        doc("document-meta", &[META]),
        doc("document-settings", &[SETTINGS]),
    ])
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

#[cfg(test)]
mod element_tests {
    use super::element;

    /// An element ends at its own close tag however many elements of its name it holds side by
    /// side (inline objects' own `office:body`s), not after a count of tags.
    #[test]
    fn an_element_holding_many_of_its_name_ends_at_its_own_close() {
        let inner = "<office:body><text:p>x</text:p></office:body>".repeat(1200);
        let outer = format!("<office:body>{inner}</office:body>");
        let xml = format!("<office:document>{outer}<office:settings/></office:document>");
        assert_eq!(element(&xml, "office:body"), outer);
        // Nested deeper than any XML here: malformed, to the end, as an unclosed one.
        let deep = format!("{}{}", "<office:body>".repeat(600), "</office:body>".repeat(600));
        assert_eq!(element(&deep, "office:body"), deep);
        assert_eq!(element("<office:body><x/>", "office:body"), "<office:body><x/>");
    }
}
