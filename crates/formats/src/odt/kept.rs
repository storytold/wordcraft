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
    let mut prefixes = std::collections::BTreeSet::new();
    events_at(xml, |ev, at| match ev {
        Ev::Start(q, a) => {
            path.push(q.clone());
            let depth = path.len();
            // The namespaces the root declares, which kept markup may use.
            if depth == 1 {
                // Older files' unitless gradient angles are tenths of a degree (see `to_1_3`).
                before_1_3 = super::strokes::legacy_angles(get(&a, "office:version"));
                for (k, v) in &a {
                    if let Some(p) = k.strip_prefix("xmlns:").filter(|p| !p.is_empty() && p.len() <= 64)
                        && out.len() < MAX_DEFS
                    {
                        prefixes.insert(p.to_string());
                        out.push((format!("ns:{p}"), Arc::from(v.as_str())));
                    }
                }
                return;
            }
            // Those a part's container declares too: what is kept of it leaves the container behind.
            if depth == 2 {
                for (k, v) in &a {
                    if let Some(p) = k.strip_prefix("xmlns:").filter(|p| !p.is_empty() && p.len() <= 64)
                        && out.len() < MAX_DEFS
                        && prefixes.insert(p.to_string())
                    {
                        out.push((format!("ns:{p}"), Arc::from(v.as_str())));
                    }
                }
            }
            // A reference mark (§6.6.2), which `text:reference-ref` names: WordCraft reads it as a
            // bookmark, and writes it as both.
            if content
                && matches!(q.as_str(), "text:reference-mark" | "text:reference-mark-start")
                && let Some(n) = get(&a, "text:name").filter(|n| !n.is_empty() && n.len() <= 256)
                && out.len() < MAX_DEFS
            {
                out.push((format!("refmark:{n}"), Arc::from("")));
            }
            if open.is_some() || out.len() >= MAX_DEFS {
                return;
            }
            let parent = path.get(depth.wrapping_sub(2)).map(String::as_str).unwrap_or("");
            let key = match (depth, parent) {
                (2, _) if content && q == "office:scripts" => Some("top:office:scripts".to_string()),
                // Whether the text holds soft page breaks (§19.857), which markup kept may.
                (3, "office:body")
                    if content
                        && q == "office:text"
                        && get(&a, "text:use-soft-page-breaks").is_some_and(|v| v.trim() == "true")
                        && out.len() < MAX_DEFS =>
                {
                    out.push(("soft-page-breaks".to_string(), Arc::from("true")));
                    None
                }
                (3, "office:automatic-styles") => child_key(if content { "auto" } else { "sauto" }, &q, &a),
                (3, "office:styles") => child_key("common", &q, &a),
                (3, "office:font-face-decls") => child_key("font", &q, &a),
                (3, "office:master-styles") => child_key("master", &q, &a),
                (4, "office:text") if content && q.starts_with("text:") && q.ends_with("-decls") => Some(format!("decls:{q}")),
                (4, "office:text") if content && q == "office:forms" => Some("forms".to_string()),
                // A tracked change (§5.5.2), by its id: a paragraph written as read names it.
                (5, "text:tracked-changes") if content && q == "text:changed-region" => {
                    get(&a, "text:id").or_else(|| get(&a, "xml:id")).filter(|i| !i.is_empty() && i.len() <= 256).map(|i| format!("change:{i}"))
                }
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
                    let mut s = if before_1_3 { to_1_3(s) } else { s.to_string() };
                    // ODF 1.3 names a changed region by `xml:id` (§5.5.2); an older file's by
                    // `text:id` alone.
                    if let Some(id) = key.strip_prefix("change:")
                        && s.split('>').next().is_some_and(|h| !h.contains("xml:id="))
                    {
                        s.insert_str("<text:changed-region".len(), &format!(" xml:id=\"{}\"", super::writer::x(id)));
                    }
                    out.push((key, Arc::from(s)));
                }
            }
            path.pop();
        }
        Ev::Text(_) => {}
    });
    out
}

/// Markup kept from a file older than ODF 1.3, written as ODF 1.3: a gradient's, transparency
/// gradient's or hatch's angle without a unit is given its unit, as the file meant it — tenths
/// of a degree (`300` → `30deg`). That reading deviates from the standard out of necessity
/// (§18.3.1 says degrees): see `strokes::legacy_angles`.
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
        Some(name) => Some(format!("{g}:{name}{}", space(q, get(a, "style:family").unwrap_or("")))),
        None => {
            let which = get(a, "style:family").or_else(|| get(a, "text:note-class")).unwrap_or("");
            Some(format!("{g}~{q}~{which}"))
        }
    }
}

/// The name space a definition (element `q`, of style family `family`) belongs to, as a key's
/// suffix: names are unique only within one (§16.2: a style's within its family; a list style,
/// a data style, a gradient may share a paragraph style's name, as a graphic style may:
/// LibreOffice's "Marginalia" is both). Paragraph styles, font faces and master pages have none;
/// another family's style is `|<family>`, a list style `|list`, a data style `|data`, another
/// definition `|<element>`.
fn space(q: &str, family: &str) -> String {
    match q {
        "style:style" if family.trim() == "paragraph" || family.is_empty() => String::new(),
        "style:style" => format!("|{}", family.trim()),
        "style:font-face" | "style:master-page" => String::new(),
        "text:list-style" => "|list".to_string(),
        q if q.starts_with("number:") => "|data".to_string(),
        q => format!("|{}", q.split_once(':').map_or(q, |(_, l)| l)),
    }
}

/// Whether `xml` (a part WordCraft wrote) already defines what `def` (a kept definition) does:
/// an element of its kind by its name (a list style and a paragraph style may share one).
pub(super) fn defined(xml: &str, def: &str) -> bool {
    let head = def.split('>').next().unwrap_or("");
    let el = head.trim_start_matches('<').split([' ', '/', '\t', '\n', '\r']).next().unwrap_or("");
    let name = ["style:name", "draw:name"].into_iter().find_map(|a| Some((a, raw_attr(head, a)?)));
    let (Some((attr, name)), false) = (name, el.is_empty()) else { return false };
    let open = format!("<{el} ");
    xml.match_indices(&open).any(|(i, _)| {
        xml.get(i..).and_then(|t| t.split('>').next()).is_some_and(|tag| raw_attr(tag, attr).is_some_and(|n| attr_text(n) == attr_text(name)))
    })
}

/// An attribute's value as it reads, however XML spells it (`O'Brien`, `O&apos;Brien`).
pub(super) fn attr_text(raw: &str) -> std::borrow::Cow<'_, str> {
    quick_xml::escape::unescape(raw).unwrap_or(std::borrow::Cow::Borrowed(raw))
}

/// Attribute `name`'s value as `tag` (a start tag) writes it, spaced and quoted however XML allows.
pub(super) fn raw_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let past_space = |at: usize| Some(at + tag.get(at..)?.find(|c: char| !c.is_ascii_whitespace())?);
    let mut at = tag.find(|c: char| c.is_ascii_whitespace())?;
    loop {
        let name_at = past_space(at)?;
        let eq = name_at + tag.get(name_at..)?.find('=')?;
        let value_at = past_space(eq + 1)?;
        let rest = tag.get(value_at..)?;
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let len = rest.get(1..)?.find(quote)?;
        if tag.get(name_at..eq)?.trim_end() == name {
            return rest.get(1..1 + len);
        }
        at = value_at + 1 + len + 1;
    }
}

/// The children of `container` (a child of the root) in `xml` (a part WordCraft wrote): each
/// one's key in `group` (as [`defs`] gives it) and its markup's byte range; and the range of
/// the container's content (`None` when there is no such container).
pub(super) fn children(xml: &str, container: &str, group: &str) -> (Vec<(String, (usize, usize))>, Option<(usize, usize)>) {
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

/// Whether a definition (its markup) is a dash, gradient, marker or fill picture: WordCraft's
/// where both have one by that name, as the drawings it writes from the model name theirs. A
/// graphic style (the `Graphics` and `Frame` styles drawings derive from) is the file's: its
/// drawings written as read derive from it, and those written from the model derived from it
/// when read (what WordCraft models of it is in theirs).
fn drawing_def(xml: &str) -> bool {
    xml.split('>').next().unwrap_or("").starts_with("<draw:")
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
/// Also the paths linked to that are in the package but couldn't be read (over the picture
/// budget, unreadable): markup linking to one can't be written as read.
pub(super) fn files(
    defs: &[(String, Arc<str>)],
    entries: &[String],
    fetch: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> (Vec<(String, Vec<u8>)>, BTreeSet<String>) {
    let mut paths = BTreeSet::new();
    for (_, xml) in defs {
        hrefs(xml, &mut paths);
    }
    let mut out = Vec::new();
    let mut missing = BTreeSet::new();
    for p in paths {
        if entries.contains(&p) {
            match fetch(&p) {
                Some(b) => out.push((p, b)),
                None => {
                    missing.insert(p);
                }
            }
        } else {
            // A folder (an object): the files in it.
            let dir = format!("{}/", p.trim_end_matches('/'));
            for e in entries.iter().filter(|e| e.starts_with(&dir) && !e.ends_with('/')).take(MAX_FILES) {
                match fetch(e) {
                    Some(b) => out.push((e.clone(), b)),
                    None => {
                        missing.insert(p.clone());
                    }
                }
            }
        }
    }
    (out, missing)
}

/// Whether markup `xml` links to one of the package files in `missing` (see [`files`]).
pub(super) fn links_to(xml: &str, missing: &BTreeSet<String>) -> bool {
    if missing.is_empty() {
        return false;
    }
    let mut paths = BTreeSet::new();
    hrefs(xml, &mut paths);
    paths.iter().any(|p| missing.contains(p))
}

/// The package files `xml` links to (`xlink:href` to a path inside the package), added to
/// `paths` (at most [`MAX_FILES`]).
pub(super) fn hrefs(xml: &str, paths: &mut BTreeSet<String>) {
    if !xml.contains("xlink:href") {
        return;
    }
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

/// Whether a child of a `form:form` is a form control (else the form's own properties or
/// event listeners).
fn is_control(q: &str) -> bool {
    q.starts_with("form:") && !matches!(q, "form:properties" | "form:form" | "form:connection-resource")
}

/// The children `depth` deep in `xml` (the root is 1): each one's element, `form:name` and
/// markup.
fn children_at(xml: &str, at_depth: usize) -> Vec<(String, Option<String>, &str)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut open: Option<(String, Option<String>, usize)> = None;
    events_at(xml.as_bytes(), |ev, at| match ev {
        Ev::Start(q, a) => {
            depth += 1;
            if depth == at_depth && open.is_none() {
                open = Some((q, get(&a, "form:name").map(str::to_string), at.0));
            }
        }
        Ev::End(_) => {
            if depth == at_depth
                && let Some((q, name, from)) = open.take()
                && let Some(x) = xml.get(from..at.1)
            {
                out.push((q, name, x));
            }
            depth = depth.saturating_sub(1);
        }
        Ev::Text(_) => {}
    });
    out
}

/// Control `ours` (as WordCraft writes it) with what the file's control `theirs` had that
/// WordCraft doesn't model and so can't have changed: its other attributes (not its ids) and its
/// properties and event listeners.
fn with_unmodelled(ours: &str, theirs: &str) -> String {
    let attrs_of = |xml: &str| {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut first = true;
        super::xml::events(xml.as_bytes(), |ev| {
            if let Ev::Start(_, a) = ev
                && std::mem::take(&mut first)
            {
                out = a;
            }
        });
        out
    };
    let have: BTreeSet<String> = attrs_of(ours).into_iter().map(|(k, _)| k).collect();
    let extra: String = attrs_of(theirs)
        .into_iter()
        .filter(|(k, _)| !have.contains(k) && !matches!(k.as_str(), "form:id" | "xml:id" | "form:name") && !k.starts_with("xmlns"))
        .map(|(k, v)| format!(" {k}=\"{}\"", super::writer::x(&v)))
        .collect();
    let kids: String = children_at(theirs, 2)
        .into_iter()
        .filter(|(q, ..)| matches!(q.as_str(), "form:properties" | "office:event-listeners"))
        .filter(|(q, ..)| !ours.contains(&format!("<{q}")))
        .map(|(.., x)| x)
        .collect();
    // Into its start tag, and before its end (an empty one opened for them).
    let Some(end) = ours.find('>') else { return ours.to_string() };
    let (head, rest) = ours.split_at(end);
    let empty = head.ends_with('/');
    let head = head.trim_end_matches('/');
    let name = head.trim_start_matches('<').split(char::is_whitespace).next().unwrap_or("");
    if empty {
        if kids.is_empty() { format!("{head}{extra}/>") } else { format!("{head}{extra}>{kids}</{name}>") }
    } else {
        let close = format!("</{name}>");
        let body = rest.get(1..).unwrap_or("");
        match body.rfind(&close) {
            Some(at) => format!("{head}{extra}>{}{kids}{}", body.get(..at).unwrap_or(""), body.get(at..).unwrap_or("")),
            None => format!("{head}{extra}>{body}"),
        }
    }
}

/// The document's forms (`office:forms`, §13.2) as the file had them (`orig`), with the
/// controls WordCraft writes (`ours`, its own `office:forms`): the file's forms element and each
/// form's own start tag, properties and event listeners as read; the controls the document has
/// in the first form (the file's controls are the document's, read into it and written back);
/// `ours` alone when the file had none, its own form when the file had none.
pub(super) fn merge_forms(orig: Option<&str>, ours: String) -> String {
    let Some(orig) = orig else { return ours };
    // `ours`: the controls (children of its one form).
    let ours_controls = children_at(&ours, 3);
    let orig_controls: Vec<(String, Option<String>, &str)> = children_at(orig, 3).into_iter().filter(|(q, ..)| is_control(q)).collect();
    // Each of ours with what the file's control by that name (else in that place) had and
    // WordCraft doesn't model.
    let mut used = vec![false; orig_controls.len()];
    let mut controls = String::new();
    for (i, (q, name, xml)) in ours_controls.iter().enumerate() {
        let same = |(k, (oq, oname, _)): &(usize, &(String, Option<String>, &str))| !used[*k] && oq == q && oname == name;
        let found = orig_controls.iter().enumerate().find(|c| same(c)).map(|(k, _)| k).or_else(|| {
            let nth = ours_controls.iter().take(i).filter(|(oq, ..)| oq == q).count();
            orig_controls.iter().enumerate().filter(|(k, (oq, ..))| !used[*k] && oq == q).nth(nth).map(|(k, _)| k)
        });
        match found.and_then(|k| orig_controls.get(k).map(|c| (k, c.2))) {
            Some((k, theirs)) => {
                used[k] = true;
                controls.push_str(&with_unmodelled(xml, theirs));
            }
            None => controls.push_str(xml),
        }
    }
    // `orig`: its forms element, each form's start tag and non-control children.
    let mut head: Option<String> = None;
    let mut forms: Vec<(String, String)> = Vec::new();
    let mut child: Option<(usize, bool)> = None;
    let mut depth = 0usize;
    events_at(orig.as_bytes(), |ev, at| match ev {
        Ev::Start(q, _) => {
            depth += 1;
            let tag = orig.get(at.0..at.1).unwrap_or("");
            let opened = || tag.strip_suffix("/>").map_or_else(|| tag.to_string(), |t| format!("{}>", t.trim_end()));
            match depth {
                1 => head = Some(opened()),
                2 if q == "form:form" => forms.push((opened(), String::new())),
                3 if child.is_none() => child = Some((at.0, is_control(&q))),
                _ => {}
            }
        }
        Ev::End(_) => {
            if depth == 3
                && let Some((from, control)) = child.take()
                && !control
                && let (Some(f), Some(x)) = (forms.last_mut(), orig.get(from..at.1))
            {
                f.1.push_str(x);
            }
            depth = depth.saturating_sub(1);
        }
        Ev::Text(_) => {}
    });
    let Some(head) = head else { return ours };
    let mut out = head;
    if forms.is_empty() && !controls.is_empty() {
        out.push_str(&format!("<form:form form:name=\"Form\">{controls}</form:form>"));
    }
    for (i, (start, kids)) in forms.iter().enumerate() {
        out.push_str(start);
        out.push_str(kids);
        if i == 0 {
            out.push_str(&controls);
        }
        out.push_str("</form:form>");
    }
    out.push_str("</office:forms>");
    out
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

/// The tracked changes (§5.5.7 `text:change-id`) `xml` marks, by id.
pub(super) fn changes(xml: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if xml.contains("text:change-id") {
        super::xml::events(xml.as_bytes(), |ev| {
            if let Ev::Start(_, a) = ev
                && let Some(id) = get(&a, "text:change-id").filter(|i| !i.is_empty() && i.len() <= 256)
                && out.len() < MAX_DEFS
            {
                out.insert(id.to_string());
            }
        });
    }
    out
}

/// The names of the bookmarks (§6.2.1) `xml` begins (at a point too) and ends.
pub(super) fn bookmarks(xml: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut starts, mut ends) = (BTreeSet::new(), BTreeSet::new());
    if xml.contains("<text:bookmark") {
        super::xml::events(xml.as_bytes(), |ev| {
            if let Ev::Start(q, a) = ev
                && let Some(n) = get(&a, "text:name")
                && starts.len() + ends.len() < MAX_DEFS
            {
                match q.as_str() {
                    "text:bookmark" => {
                        starts.insert(n.to_string());
                        ends.insert(n.to_string());
                    }
                    "text:bookmark-start" => {
                        starts.insert(n.to_string());
                    }
                    "text:bookmark-end" => {
                        ends.insert(n.to_string());
                    }
                    _ => {}
                }
            }
        });
    }
    (starts, ends)
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
/// `(automatic, common, font faces)`, by name (with its name space's suffix, see `space`), as kept. `None` when one of them makes a page
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
        // Every definition by that name (in any name space: see `space`), from the first group
        // that has one: automatic styles, then common ones, then font faces.
        let mut found = false;
        for (group, into) in [("auto", &mut auto), ("common", &mut common), ("font", &mut fonts)] {
            let base = format!("{group}:{n}");
            for (k, def) in defs.range(base.clone()..).take_while(|(k, _)| k.starts_with(&base)) {
                let Some(rest) = k.strip_prefix(&base).filter(|r| r.is_empty() || r.starts_with('|')) else { continue };
                if !masters && def.contains("style:master-page-name=\"") && !def.contains("style:master-page-name=\"\"") {
                    return None;
                }
                into.insert(format!("{n}{rest}"), def.clone());
                todo.extend(names(def));
                found = true;
            }
            if found {
                break;
            }
        }
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
        assert_eq!(d.keys().cloned().collect::<Vec<_>>(), ["auto:L1|list", "auto:P1", "auto:P2", "font:Liberation Serif"]);
        assert!(d["auto:P1"].starts_with("<style:style style:name=\"P1\"") && d["auto:P1"].ends_with("</style:style>"));
        let (auto, common, fonts) = needed(&d, names(r#"<text:p text:style-name="P1">x</text:p>"#), false).unwrap();
        assert_eq!(auto.keys().cloned().collect::<Vec<_>>(), ["L1|list", "P1"]);
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
        assert!(d["common:G|gradient"].contains(r#"draw:angle="30deg""#), "{}", d["common:G|gradient"]);
        assert!(d["common:H|hatch"].contains(r#"draw:rotation="45deg""#));
        assert!(d["common:D|gradient"].contains(r#"draw:angle="45deg""#));
        // From a 1.3 file: as it is.
        let d: BTreeMap<String, Arc<str>> = defs(xml.replace("1.2", "1.3").as_bytes(), false).into_iter().collect();
        assert!(d["common:G|gradient"].contains(r#"draw:angle="300""#));
    }

    #[test]
    fn definitions_sharing_a_name_are_each_kept() {
        // A list style may have a paragraph style's name (§16.2: names are unique within a kind
        // of definition), as LibreOffice's "Numbering 1" both are; each is kept, and markup
        // naming it needs both.
        let xml = r#"<office:document-styles><office:styles><style:style style:name="N" style:family="paragraph"/><text:list-style style:name="N"><text:list-level-style-number text:level="1"/></text:list-style></office:styles></office:document-styles>"#;
        let d: BTreeMap<String, Arc<str>> = defs(xml.as_bytes(), false).into_iter().collect();
        assert!(d["common:N"].starts_with("<style:style ") && d["common:N|list"].starts_with("<text:list-style "));
        let (_, common, _) = needed(&d, names(r#"<text:p text:style-name="N"/>"#), false).unwrap();
        assert_eq!(common.keys().cloned().collect::<Vec<_>>(), ["N", "N|list"]);
        // One is defined where an element of its kind has the name, not the other kind.
        let ours = r#"<office:styles><style:style style:name="N" style:family="paragraph"/></office:styles>"#;
        assert!(defined(ours, &d["common:N"]) && !defined(ours, &d["common:N|list"]));
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
