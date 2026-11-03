//! Markup kept from the file an ODT document was opened from (see `wordcraft_doc::kept`): the
//! definitions kept paragraphs refer to, read here, and written back with them.
//!
//! A kept paragraph names its automatic styles (`P12`, `T5`), and through them common styles,
//! list styles, data styles and font faces. They are kept as read, by name: `auto:` for
//! `content.xml`'s automatic styles, `common:` for `styles.xml`'s common styles, `font:` for
//! font faces. On save, the ones a written paragraph needs (and what they need in turn) go back
//! in; WordCraft's own automatic styles take another prefix, so names can't meet.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::xml::{Ev, events_at, get};

/// Most definitions kept from one part (hostile input).
const MAX_DEFS: usize = 100_000;

/// The definitions in `xml` (a `content.xml` or `styles.xml`): each child of
/// `office:automatic-styles` as `auto:<name>` (only for `content.xml`, `content` true), of
/// `office:styles` as `common:<name>`, of `office:font-face-decls` as `font:<name>`; and the
/// namespaces its root declares, as `ns:<prefix>` (the URI).
pub(super) fn defs(xml: &[u8], content: bool) -> Vec<(String, Arc<str>)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut group: Option<&'static str> = None;
    let mut open: Option<(String, usize)> = None;
    events_at(xml, |ev, at| match ev {
        Ev::Start(q, a) => {
            depth += 1;
            // The namespaces the root declares, which kept markup may use.
            if depth == 1 {
                for (k, v) in &a {
                    if let Some(p) = k.strip_prefix("xmlns:").filter(|p| !p.is_empty() && p.len() <= 64)
                        && out.len() < MAX_DEFS
                    {
                        out.push((format!("ns:{p}"), Arc::from(v.as_str())));
                    }
                }
            } else if depth == 2 {
                group = match q.as_str() {
                    "office:automatic-styles" if content => Some("auto"),
                    "office:styles" => Some("common"),
                    "office:font-face-decls" => Some("font"),
                    _ => None,
                };
            } else if depth == 3
                && let Some(g) = group
                && let Some(key) = child_key(g, &q, &a)
                && out.len() < MAX_DEFS
            {
                open = Some((key, at.0));
            }
        }
        Ev::End(_) => {
            if depth == 3
                && let Some((key, from)) = open.take()
                && let Some(s) = xml.get(from..at.1).and_then(|b| std::str::from_utf8(b).ok())
            {
                out.push((key, Arc::from(s)));
            }
            if depth == 2 {
                group = None;
            }
            depth = depth.saturating_sub(1);
        }
        Ev::Text(_) => {}
    });
    out
}

/// The key a definition (element `q` with attributes `a`, a child of group `g`) is kept under:
/// `<g>:<name>` (its `style:name`, or a drawing definition's `draw:name`); an unnamed child of `office:styles` (a default style, the outline style, a note
/// or line numbering configuration) as `<g>~<element>~<its family or note class>`.
fn child_key(g: &str, q: &str, a: &super::xml::Attrs) -> Option<String> {
    // Drawing definitions (dashes, gradients, arrowheads…) are named by `draw:name`.
    match get(a, "style:name").or_else(|| get(a, "draw:name")).filter(|n| !n.is_empty()) {
        Some(name) => Some(format!("{g}:{name}")),
        None if g == "common" => {
            let which = get(a, "style:family").or_else(|| get(a, "text:note-class")).unwrap_or("");
            Some(format!("{g}~{q}~{which}"))
        }
        None => None,
    }
}

/// The children of `office:styles` in `xml` (a `styles.xml` WordCraft wrote): each one's key
/// (as [`defs`] gives it) and its markup's byte range.
pub(super) fn common_children(xml: &str) -> Vec<(String, (usize, usize))> {
    let mut out = Vec::new();
    let (mut depth, mut inside) = (0usize, false);
    let mut open: Option<(String, usize)> = None;
    events_at(xml.as_bytes(), |ev, at| match ev {
        Ev::Start(q, a) => {
            depth += 1;
            if depth == 2 {
                inside = q == "office:styles";
            } else if depth == 3
                && inside
                && let Some(key) = child_key("common", &q, &a)
            {
                open = Some((key, at.0));
            }
        }
        Ev::End(_) => {
            if depth == 3
                && let Some((key, from)) = open.take()
            {
                out.push((key, (from, at.1)));
            }
            depth = depth.saturating_sub(1);
        }
        Ev::Text(_) => {}
    });
    out
}

/// Whether a definition (its markup) is a drawing's: a graphic style or default style, or a
/// dash, gradient, marker or fill picture. Drawings are always written from the model, so
/// their definitions are WordCraft's where both have one by that name.
fn drawing_def(xml: &str) -> bool {
    let head = xml.split('>').next().unwrap_or("");
    head.starts_with("<draw:") || head.contains(r#"style:family="graphic""#)
}

/// `styles` (a `styles.xml` WordCraft wrote) with its `office:styles` as read: every kept
/// child of the file's (`order`, from `defs`), then WordCraft's own children the file had no
/// such of. A drawing's definition both have stays WordCraft's (see [`drawing_def`]). For a
/// document whose style definitions are as read.
pub(super) fn with_common_as_read(styles: &str, defs: &BTreeMap<String, Arc<str>>, order: &[String]) -> String {
    let kids = common_children(styles);
    let (Some(first), Some(last)) = (kids.first(), kids.last()) else { return styles.to_string() };
    let (from, to) = (first.1.0, last.1.1);
    let ours = |k: &str| kids.iter().find(|(key, _)| key == k).and_then(|(_, (a, b))| styles.get(*a..*b));
    let mut body = String::new();
    for k in order.iter().filter(|k| k.starts_with("common")) {
        match (defs.get(k), ours(k)) {
            (Some(_), Some(o)) if drawing_def(o) => body.push_str(o),
            // The default graphic style applies to every drawing, which WordCraft writes from the
            // model under its own (none: the standard's).
            (Some(_), None) if k == "common~style:default-style~graphic" => {}
            (Some(d), _) => body.push_str(d),
            (None, _) => {}
        }
    }
    for (k, (a, b)) in &kids {
        if !defs.contains_key(k)
            && let Some(x) = styles.get(*a..*b)
        {
            body.push_str(x);
        }
    }
    let (Some(head), Some(tail)) = (styles.get(..from), styles.get(to..)) else { return styles.to_string() };
    format!("{head}{body}{tail}")
}

/// Attribute names whose values name a definition kept markup may refer to.
fn refers(attr: &str) -> bool {
    attr.ends_with("style-name") || attr.starts_with("style:font-name") || attr == "style:list-style-name"
}

/// The names `xml` refers to (the values of its style- and font-name attributes).
pub(super) fn names(xml: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    super::xml::events(xml.as_bytes(), |ev| {
        if let Ev::Start(_, a) = ev {
            for (k, v) in a {
                if refers(&k) && !v.is_empty() {
                    out.insert(v);
                }
            }
        }
    });
    out
}

/// Most package files kept definitions may refer to.
const MAX_FILES: usize = 1000;

/// The package files `defs` refer to (`xlink:href` to a path inside the package, as a style's
/// fill picture), read with `fetch`: a definition written as read needs them in the package.
pub(super) fn files(defs: &[(String, Arc<str>)], fetch: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Vec<(String, Vec<u8>)> {
    let mut paths = BTreeSet::new();
    for (_, xml) in defs.iter().filter(|(_, x)| x.contains("xlink:href")) {
        super::xml::events(xml.as_bytes(), |ev| {
            if let Ev::Start(_, a) = ev
                && let Some(h) = get(&a, "xlink:href")
                && inside_package(h)
                && paths.len() < MAX_FILES
            {
                paths.insert(h.trim_start_matches("./").to_string());
            }
        });
    }
    paths.into_iter().filter_map(|p| fetch(&p).map(|b| (p, b))).collect()
}

/// Whether link `h` names a file inside the package (a relative path, not a URL or a fragment).
fn inside_package(h: &str) -> bool {
    !h.is_empty() && !h.contains(':') && !h.starts_with(['#', '/']) && !h.contains("..")
}

/// The sequences (§7.4.13 `text:sequence`) `xml` counts, which the document declares.
pub(super) fn sequences(xml: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if xml.contains("<text:sequence ") {
        super::xml::events(xml.as_bytes(), |ev| {
            if let Ev::Start(q, a) = ev
                && q == "text:sequence"
                && let Some(n) = get(&a, "text:name").filter(|n| !n.is_empty() && n.len() <= 256)
            {
                out.insert(n.to_string());
            }
        });
    }
    out
}

/// The kept definitions markup naming `start` needs, each once, what they name included:
/// `(automatic, common, font faces)`, by name, as kept. `None` when one of them makes a page
/// start (`style:master-page-name`): WordCraft names its master pages afresh, so such markup is
/// written from the model instead.
pub(super) type Needed = (BTreeMap<String, Arc<str>>, BTreeMap<String, Arc<str>>, BTreeMap<String, Arc<str>>);

pub(super) fn needed(defs: &BTreeMap<String, Arc<str>>, start: BTreeSet<String>) -> Option<Needed> {
    let (mut auto, mut common, mut fonts) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut todo: Vec<String> = start.into_iter().collect();
    let mut seen = BTreeSet::new();
    while let Some(n) = todo.pop() {
        if !seen.insert(n.clone()) || seen.len() > MAX_DEFS {
            continue;
        }
        let (def, into) = if let Some(d) = defs.get(&format!("auto:{n}")) {
            (d, &mut auto)
        } else if let Some(d) = defs.get(&format!("common:{n}")) {
            (d, &mut common)
        } else if let Some(d) = defs.get(&format!("font:{n}")) {
            (d, &mut fonts)
        } else {
            continue;
        };
        if def.contains("style:master-page-name=\"") && !def.contains("style:master-page-name=\"\"") {
            return None;
        }
        into.insert(n, def.clone());
        todo.extend(names(def));
    }
    Some((auto, common, fonts))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENT: &str = r#"<office:document-content><office:font-face-decls><style:font-face style:name="Liberation Serif" svg:font-family="'Liberation Serif'"/></office:font-face-decls><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Body" style:list-style-name="L1"><style:text-properties style:font-name="Liberation Serif"/></style:style><text:list-style style:name="L1"><text:list-level-style-bullet text:level="1" text:style-name="Bullets"/></text:list-style><style:style style:name="P2" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body/></office:document-content>"#;

    #[test]
    fn definitions_are_kept_by_name_and_followed() {
        let d: BTreeMap<String, Arc<str>> = defs(CONTENT.as_bytes(), true).into_iter().collect();
        assert_eq!(d.keys().cloned().collect::<Vec<_>>(), ["auto:L1", "auto:P1", "auto:P2", "font:Liberation Serif"]);
        assert!(d["auto:P1"].starts_with("<style:style style:name=\"P1\"") && d["auto:P1"].ends_with("</style:style>"));
        let (auto, common, fonts) = needed(&d, names(r#"<text:p text:style-name="P1">x</text:p>"#)).unwrap();
        assert_eq!(auto.keys().cloned().collect::<Vec<_>>(), ["L1", "P1"]);
        // "Body" and "Bullets" weren't kept here (they're common styles in styles.xml).
        assert!(common.is_empty());
        assert_eq!(fonts.keys().cloned().collect::<Vec<_>>(), ["Liberation Serif"]);
        // A paragraph starting a page isn't written as kept.
        assert!(needed(&d, names(r#"<text:p text:style-name="P2"/>"#)).is_none());
    }

    #[test]
    fn hostile_markup_keeps_what_it_can() {
        assert!(defs(b"<a><office:styles><style:style style:name=\"x\"", false).is_empty());
        assert!(defs(b"", true).is_empty());
        // A definition naming itself is followed once.
        let d: BTreeMap<String, Arc<str>> =
            [("auto:A".to_string(), Arc::from(r#"<style:style style:name="A" style:parent-style-name="A"/>"#))].into();
        assert_eq!(needed(&d, ["A".to_string()].into()).unwrap().0.len(), 1);
    }
}
