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

/// The definitions in `xml` (a `content.xml` or `styles.xml`), each by the key it is kept
/// under: a child of `office:automatic-styles` as `auto:<name>` in `content.xml` (`content`
/// true), `sauto:<name>` in `styles.xml`; of `office:styles` as `common:<name>`; of
/// `office:font-face-decls` as `font:<name>`; of `office:master-styles` as `master:<name>`; the
/// namespaces the root declares as `ns:<prefix>` (the URI); and in `content.xml`, the document's
/// `office:scripts` as `top:office:scripts` and the declarations opening the text
/// (`text:variable-decls`…) as `decls:<element>`.
pub(super) fn defs(xml: &[u8], content: bool) -> Vec<(String, Arc<str>)> {
    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut open: Option<(String, usize, usize)> = None;
    let mut before_1_3 = false;
    events_at(xml, |ev, at| match ev {
        Ev::Start(q, a) => {
            path.push(q.clone());
            let depth = path.len();
            // The namespaces the root declares, which kept markup may use.
            if depth == 1 {
                // ODF 1.2 and before measured some angles in tenths of a degree (see `to_1_3`).
                before_1_3 = get(&a, "office:version").is_none_or(|v| v.trim() < "1.3");
                for (k, v) in &a {
                    if let Some(p) = k.strip_prefix("xmlns:").filter(|p| !p.is_empty() && p.len() <= 64)
                        && out.len() < MAX_DEFS
                    {
                        out.push((format!("ns:{p}"), Arc::from(v.as_str())));
                    }
                }
                return;
            }
            if open.is_some() || out.len() >= MAX_DEFS {
                return;
            }
            let parent = path.get(depth.wrapping_sub(2)).map(String::as_str).unwrap_or("");
            let key = match (depth, parent) {
                (2, _) if content && q == "office:scripts" => Some("top:office:scripts".to_string()),
                (3, "office:automatic-styles") => child_key(if content { "auto" } else { "sauto" }, &q, &a),
                (3, "office:styles") => child_key("common", &q, &a),
                (3, "office:font-face-decls") => child_key("font", &q, &a),
                (3, "office:master-styles") => child_key("master", &q, &a),
                (4, "office:text") if content && q.starts_with("text:") && q.ends_with("-decls") => Some(format!("decls:{q}")),
                _ => None,
            };
            if let Some(key) = key {
                open = Some((key, at.0, depth));
            }
        }
        Ev::End(_) => {
            if let Some((_, from, d)) = &open
                && *d == path.len()
            {
                let from = *from;
                if let Some((key, ..)) = open.take()
                    && let Some(s) = xml.get(from..at.1).and_then(|b| std::str::from_utf8(b).ok())
                {
                    let s = if before_1_3 { to_1_3(s) } else { s.to_string() };
                    out.push((key, Arc::from(s)));
                }
            }
            path.pop();
        }
        Ev::Text(_) => {}
    });
    out
}

/// Markup kept from an ODF 1.2 (or older) file, written as ODF 1.3: a gradient's,
/// transparency gradient's or hatch's angle without a unit was tenths of a degree there and is
/// degrees in 1.3 (§19.112 `draw:angle`, §19.209 `draw:rotation`), so it is given its unit.
fn to_1_3(xml: &str) -> String {
    if !(xml.contains("<draw:gradient") || xml.contains("<draw:opacity") || xml.contains("<draw:hatch")) {
        return xml.to_string();
    }
    let mut out = String::with_capacity(xml.len() + 16);
    let mut rest = xml;
    while let Some(i) = rest.find(['<']) {
        let (head, tail) = rest.split_at(i);
        out.push_str(head);
        let end = tail.find('>').map_or(tail.len(), |e| e + 1);
        let (tag, after) = tail.split_at(end);
        if tag.starts_with("<draw:gradient") || tag.starts_with("<draw:opacity") || tag.starts_with("<draw:hatch") {
            let mut t = tag.to_string();
            for attr in ["draw:angle=\"", "draw:rotation=\""] {
                if let Some(a) = t.find(attr).map(|a| a + attr.len())
                    && let Some(len) = t.get(a..).and_then(|v| v.find('"'))
                    && let Some(v) = t.get(a..a + len).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
                {
                    t.replace_range(a..a + len, &format!("{}deg", v / 10.0));
                }
            }
            out.push_str(&t);
        } else {
            out.push_str(tag);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The key a definition (element `q` with attributes `a`, a child of group `g`) is kept under:
/// `<g>:<name>` (its `style:name`, or a drawing definition's `draw:name`); an unnamed one (a
/// default style, the outline style, a note or line numbering configuration, a layer set) as
/// `<g>~<element>~<its family or note class>`.
fn child_key(g: &str, q: &str, a: &super::xml::Attrs) -> Option<String> {
    // Drawing definitions (dashes, gradients, arrowheads…) are named by `draw:name`.
    match get(a, "style:name").or_else(|| get(a, "draw:name")).filter(|n| !n.is_empty()) {
        Some(name) => Some(format!("{g}:{name}")),
        None => {
            let which = get(a, "style:family").or_else(|| get(a, "text:note-class")).unwrap_or("");
            Some(format!("{g}~{q}~{which}"))
        }
    }
}

/// The children of `container` (a child of the root) in `xml` (a part WordCraft wrote): each
/// one's key in `group` (as [`defs`] gives it) and its markup's byte range; and the range of
/// the container's content (`None` when there is no such container).
fn children(xml: &str, container: &str, group: &str) -> (Vec<(String, (usize, usize))>, Option<(usize, usize)>) {
    let mut out = Vec::new();
    let (mut depth, mut inside) = (0usize, false);
    let mut open: Option<(String, usize)> = None;
    let mut content: Option<(usize, usize)> = None;
    events_at(xml.as_bytes(), |ev, at| match ev {
        Ev::Start(q, a) => {
            depth += 1;
            if depth == 2 {
                inside = q == container;
                if inside {
                    content = Some((at.1, at.1));
                }
            } else if depth == 3
                && inside
                && let Some(key) = child_key(group, &q, &a)
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
            if depth == 2 && inside {
                // An empty element (`<x/>`) has no content to replace: none.
                content = content.filter(|c| c.0 < at.1).map(|c| (c.0, at.0.max(c.0)));
                inside = false;
            }
            depth = depth.saturating_sub(1);
        }
        Ev::Text(_) => {}
    });
    (out, content)
}

/// `xml` (a part WordCraft wrote) with `container`'s children as read: the file's (those of
/// `group` in `defs`, in `order`; `pick` may choose WordCraft's instead, or neither), then
/// WordCraft's own the file had none of by that key.
fn with_as_read(
    xml: &str,
    container: &str,
    group: &str,
    defs: &BTreeMap<String, Arc<str>>,
    order: &[String],
    pick: &dyn Fn(&str, &str, Option<&str>) -> Option<String>,
) -> String {
    let (kids, content) = children(xml, container, group);
    let Some((from, to)) = content else { return xml.to_string() };
    let ours = |k: &str| kids.iter().find(|(key, _)| key == k).and_then(|(_, (a, b))| xml.get(*a..*b));
    let prefix = (format!("{group}:"), format!("{group}~"));
    let mut body = String::new();
    for k in order.iter().filter(|k| k.starts_with(&prefix.0) || k.starts_with(&prefix.1)) {
        if let Some(d) = defs.get(k)
            && let Some(chosen) = pick(k, d, ours(k))
        {
            body.push_str(&chosen);
        }
    }
    for (k, (a, b)) in &kids {
        if !defs.contains_key(k)
            && let Some(x) = xml.get(*a..*b)
        {
            body.push_str(x);
        }
    }
    let (Some(head), Some(tail)) = (xml.get(..from), xml.get(to..)) else { return xml.to_string() };
    format!("{head}{body}{tail}")
}

/// Whether a definition (its markup) is a drawing's: a graphic style or default style, or a
/// dash, gradient, marker or fill picture. Drawings are always written from the model, so
/// their definitions are WordCraft's where both have one by that name.
fn drawing_def(xml: &str) -> bool {
    let head = xml.split('>').next().unwrap_or("");
    head.starts_with("<draw:") || head.contains(r#"style:family="graphic""#)
}

/// `styles` (a `styles.xml` WordCraft wrote) with its `office:styles` as read: every kept
/// child of the file's, then WordCraft's own children the file had no such of. A drawing's
/// definition both have stays WordCraft's (see [`drawing_def`]). For a document whose style
/// definitions are as read.
pub(super) fn with_common_as_read(styles: &str, defs: &BTreeMap<String, Arc<str>>, order: &[String]) -> String {
    with_as_read(styles, "office:styles", "common", defs, order, &|_, d, ours| match ours {
        Some(o) if drawing_def(o) => Some(o.to_string()),
        // (The default graphic style is the file's: WordCraft's drawings say what they need.)
        _ => Some(d.to_string()),
    })
}

/// `styles` (a `styles.xml` WordCraft wrote) with its page layouts, master pages and their
/// automatic styles as read, WordCraft's own added where the file had none by that name. For a
/// document whose page setup is as read.
pub(super) fn with_pages_as_read(styles: &str, defs: &BTreeMap<String, Arc<str>>, order: &[String]) -> String {
    let keep_file = |_: &str, d: &str, _: Option<&str>| Some(d.to_string());
    let s = with_as_read(styles, "office:automatic-styles", "sauto", defs, order, &keep_file);
    with_as_read(&s, "office:master-styles", "master", defs, order, &keep_file)
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
/// fill picture; to a folder, as an embedded object in a header: the files in it), read with
/// `fetch` (`entries`: the package's paths): a definition written as read needs them.
pub(super) fn files(defs: &[(String, Arc<str>)], entries: &[String], fetch: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Vec<(String, Vec<u8>)> {
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
    let mut out = Vec::new();
    for p in paths {
        if entries.contains(&p) {
            out.extend(fetch(&p).map(|b| (p, b)));
        } else {
            // A folder (an object): the files in it.
            let dir = format!("{}/", p.trim_end_matches('/'));
            for e in entries.iter().filter(|e| e.starts_with(&dir) && !e.ends_with('/')).take(MAX_FILES) {
                out.extend(fetch(e).map(|b| (e.clone(), b)));
            }
        }
    }
    out
}

/// Whether link `h` names a file inside the package (a relative path, not a URL or a fragment).
fn inside_package(h: &str) -> bool {
    !h.is_empty() && !h.contains(':') && !h.starts_with(['#', '/']) && !h.contains("..")
}

/// The document properties WordCraft writes from its model (§4.3); the file's others are kept.
const META_MODELLED: [&str; 11] = [
    "meta:generator",
    "dc:title",
    "dc:subject",
    "dc:description",
    "meta:initial-creator",
    "dc:creator",
    "dc:date",
    "meta:editing-cycles",
    "meta:keyword",
    "meta:creation-date",
    "meta:user-defined",
];

/// The children of `meta.xml`'s `office:meta` WordCraft doesn't model (the template, the
/// statistics, the editing time, the printing…), as read, in order.
pub(super) fn meta_defs(xml: &[u8]) -> Vec<(String, Arc<str>)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut open: Option<(String, usize)> = None;
    events_at(xml, |ev, at| match ev {
        Ev::Start(q, _) => {
            depth += 1;
            if depth == 3 && open.is_none() && !META_MODELLED.contains(&q.as_str()) && out.len() < MAX_DEFS {
                open = Some((format!("meta~{}~{q}", out.len()), at.0));
            }
        }
        Ev::End(_) => {
            if depth == 3
                && let Some((key, from)) = open.take()
                && let Some(s) = xml.get(from..at.1).and_then(|b| std::str::from_utf8(b).ok())
            {
                out.push((key, Arc::from(s)));
            }
            depth = depth.saturating_sub(1);
        }
        Ev::Text(_) => {}
    });
    out
}

/// `meta` (a `meta.xml` WordCraft wrote) with the file's document properties WordCraft doesn't
/// model (`defs`' `meta~` ones) added, in their order.
pub(super) fn with_kept_meta(meta: String, defs: &BTreeMap<String, Arc<str>>, order: &[String]) -> String {
    let kept: String = order.iter().filter(|k| k.starts_with("meta~")).filter_map(|k| defs.get(k)).map(|d| &**d).collect();
    match meta.rfind("</office:meta>") {
        Some(at) if !kept.is_empty() => format!("{}{kept}{}", meta.get(..at).unwrap_or(""), meta.get(at..).unwrap_or("")),
        _ => meta,
    }
}

/// A list of declarations as the file had it (`orig`, a `text:variable-decls` or the like),
/// with those of WordCraft's (`ours`, the same list) it doesn't have by name; `ours` alone when
/// the file had none.
pub(super) fn merge_decls(orig: Option<&str>, ours: String) -> String {
    let Some(orig) = orig else { return ours };
    let named = |xml: &str| -> Vec<(String, (usize, usize))> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        events_at(xml.as_bytes(), |ev, at| match ev {
            Ev::Start(_, a) => {
                depth += 1;
                if depth == 2
                    && let Some(n) = get(&a, "text:name")
                {
                    out.push((n.to_string(), at));
                }
            }
            Ev::End(_) => {
                if depth == 2
                    && let Some(last) = out.last_mut()
                {
                    last.1.1 = at.1;
                }
                depth = depth.saturating_sub(1);
            }
            Ev::Text(_) => {}
        });
        out
    };
    let have: BTreeSet<String> = named(orig).into_iter().map(|(n, _)| n).collect();
    let extra: String = named(&ours).into_iter().filter(|(n, _)| !have.contains(n)).filter_map(|(_, (a, b))| ours.get(a..b)).collect();
    if extra.is_empty() {
        return orig.to_string();
    }
    match orig.rfind("</") {
        Some(at) if orig.ends_with('>') && !orig.trim_end().ends_with("/>") => {
            format!("{}{extra}{}", orig.get(..at).unwrap_or(""), orig.get(at..).unwrap_or(""))
        }
        // `<text:variable-decls/>`: opened and closed around them.
        _ => {
            let el = orig.trim_start_matches('<').split([' ', '/', '>']).next().unwrap_or("");
            format!("{}>{extra}</{el}>", orig.trim_end().trim_end_matches("/>"))
        }
    }
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

pub(super) fn needed(defs: &BTreeMap<String, Arc<str>>, start: BTreeSet<String>, masters: bool) -> Option<Needed> {
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
        if !masters && def.contains("style:master-page-name=\"") && !def.contains("style:master-page-name=\"\"") {
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
        let (auto, common, fonts) = needed(&d, names(r#"<text:p text:style-name="P1">x</text:p>"#), false).unwrap();
        assert_eq!(auto.keys().cloned().collect::<Vec<_>>(), ["L1", "P1"]);
        // "Body" and "Bullets" weren't kept here (they're common styles in styles.xml).
        assert!(common.is_empty());
        assert_eq!(fonts.keys().cloned().collect::<Vec<_>>(), ["Liberation Serif"]);
        // A paragraph starting a page isn't written as kept.
        assert!(needed(&d, names(r#"<text:p text:style-name="P2"/>"#), false).is_none());
    }

    #[test]
    fn angles_from_an_older_file_get_their_unit() {
        let xml = r#"<office:document-styles office:version="1.2"><office:styles><draw:gradient draw:name="G" draw:angle="300" draw:style="linear"/><draw:hatch draw:name="H" draw:rotation="450"/><draw:gradient draw:name="D" draw:angle="45deg"/></office:styles></office:document-styles>"#;
        let d: BTreeMap<String, Arc<str>> = defs(xml.as_bytes(), false).into_iter().collect();
        assert!(d["common:G"].contains(r#"draw:angle="30deg""#), "{}", d["common:G"]);
        assert!(d["common:H"].contains(r#"draw:rotation="45deg""#));
        assert!(d["common:D"].contains(r#"draw:angle="45deg""#));
        // From a 1.3 file: as it is.
        let d: BTreeMap<String, Arc<str>> = defs(xml.replace("1.2", "1.3").as_bytes(), false).into_iter().collect();
        assert!(d["common:G"].contains(r#"draw:angle="300""#));
    }

    #[test]
    fn hostile_markup_keeps_what_it_can() {
        assert!(defs(b"<a><office:styles><style:style style:name=\"x\"", false).is_empty());
        assert!(defs(b"", true).is_empty());
        // A definition naming itself is followed once.
        let d: BTreeMap<String, Arc<str>> =
            [("auto:A".to_string(), Arc::from(r#"<style:style style:name="A" style:parent-style-name="A"/>"#))].into();
        assert_eq!(needed(&d, ["A".to_string()].into(), false).unwrap().0.len(), 1);
    }
}
