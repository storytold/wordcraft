//! The document's ribbon part (`customUI/customUI14.xml`, Office 2010 namespace, or the Office
//! 2007 `customUI/customUI.xml`), from Microsoft's published RibbonX (custom UI) schema.
//!
//! [`parse_exact`] models a ribbon only when the model can write all of it back (tabs, groups,
//! macro buttons); anything richer stays in the file as a kept package part, and
//! [`parse_buttons`] still finds its macro buttons so they show and run.

use std::fmt::Write as _;

use wordcraft_doc::ribbon::MAX_RIBBON_ITEMS;
use wordcraft_doc::{Document, Ribbon, RibbonButton, RibbonGroup, RibbonTab};

use crate::xml::{self, El, esc};

/// The Office 2010 custom UI namespace (written), and the Office 2007 one (read).
pub const NS_CUSTOM_UI14: &str = "http://schemas.microsoft.com/office/2009/07/customui";
pub const NS_CUSTOM_UI12: &str = "http://schemas.microsoft.com/office/2006/01/customui";
/// The package relationship to an Office 2010 ribbon part (an Office 2007 one ends the same way,
/// under `2006/relationships`).
pub const REL_CUSTOM_UI14: &str = "http://schemas.microsoft.com/office/2007/relationships/ui/extensibility";
pub const PART_CUSTOM_UI14: &str = "customUI/customUI14.xml";

/// Whether a relationship type is a ribbon customisation (either Office version).
pub fn is_ribbon_rel(kind: &str) -> bool {
    kind.ends_with("/ui/extensibility")
}

/// Whether the part is the Office 2010 flavour (preferred when a file holds both).
pub fn is_v14(bytes: &[u8]) -> bool {
    bytes.windows(b"2009/07/customui".len()).any(|w| w == b"2009/07/customui")
}

fn parse(bytes: &[u8]) -> Option<El> {
    xml::parse_ns(bytes, &[("ui", NS_CUSTOM_UI14), ("ui", NS_CUSTOM_UI12)]).ok()
}

/// The element's name, when it is in the custom UI namespace (or none).
fn name(e: &El) -> &str {
    match e.name.split_once(':') {
        Some(("ui", local)) => local,
        Some(_) => "",
        None => &e.name,
    }
}

fn els(e: &El) -> impl Iterator<Item = &El> {
    e.els()
}

/// No text, and no attributes but `allowed` (`startFromScratch="false"` is the default anyway).
fn only(e: &El, allowed: &[&str]) -> bool {
    e.text().trim().is_empty() && e.attrs.iter().all(|(k, v)| allowed.contains(&k.as_str()) || (k == "startFromScratch" && v == "false"))
}

/// The ribbon, when it is made only of what the model holds; `None` otherwise (or when it isn't
/// a ribbon part at all).
pub fn parse_exact(bytes: &[u8]) -> Option<Ribbon> {
    let root = parse(bytes)?;
    if name(&root) != "customUI" || !only(&root, &[]) {
        return None;
    }
    let mut out = Ribbon::default();
    for ribbon in els(&root) {
        if name(ribbon) != "ribbon" || !only(ribbon, &[]) {
            return None;
        }
        for tabs in els(ribbon) {
            if name(tabs) != "tabs" || !only(tabs, &[]) {
                return None;
            }
            for tab in els(tabs) {
                if name(tab) != "tab" || !only(tab, &["id", "idMso", "label"]) || els(tab).count() > MAX_RIBBON_ITEMS {
                    return None;
                }
                let builtin = tab.attr("idMso").map(str::to_string);
                let id = tab.attr("id").unwrap_or("").to_string();
                if builtin.is_some() == !id.is_empty() {
                    return None;
                }
                let mut t = RibbonTab { id, builtin, label: tab.attr("label").unwrap_or("").to_string(), groups: Vec::new() };
                for group in els(tab) {
                    if name(group) != "group" || !only(group, &["id", "label"]) || els(group).count() > MAX_RIBBON_ITEMS {
                        return None;
                    }
                    let mut g =
                        RibbonGroup { id: group.attr("id")?.to_string(), label: group.attr("label").unwrap_or("").to_string(), buttons: Vec::new() };
                    for b in els(group) {
                        let allowed = ["id", "label", "onAction", "imageMso", "size", "screentip", "supertip", "tag"];
                        if name(b) != "button" || !only(b, &allowed) || els(b).next().is_some() {
                            return None;
                        }
                        let large = match b.attr("size") {
                            None | Some("normal") => false,
                            Some("large") => true,
                            Some(_) => return None,
                        };
                        g.buttons.push(button(b, b.attr("id")?, large)?);
                    }
                    t.groups.push(g);
                }
                out.tabs.push(t);
            }
        }
    }
    (out.tabs.len() <= MAX_RIBBON_ITEMS).then_some(out)
}

fn button(b: &El, id: &str, large: bool) -> Option<RibbonButton> {
    let s = |k: &str| b.attr(k).map(str::to_string);
    Some(RibbonButton {
        id: id.to_string(),
        label: s("label").unwrap_or_default(),
        on_action: s("onAction")?,
        image: s("imageMso"),
        large,
        screentip: s("screentip"),
        supertip: s("supertip"),
        tag: s("tag"),
    })
}

/// Whether an element is shown (`visible`, default true).
fn visible(e: &El) -> bool {
    !matches!(e.attr("visible"), Some("false" | "0"))
}

/// The macro buttons of any ribbon part, wherever they sit in a group (menus and split buttons
/// included), for showing a ribbon that isn't modelled. Buttons without a fixed label are named
/// by their id; built-in commands (`idMso`) and buttons without `onAction` are left out.
pub fn parse_buttons(bytes: &[u8]) -> Ribbon {
    let mut out = Ribbon::default();
    let Some(root) = parse(bytes) else { return out };
    let Some(tabs) = els(&root).find(|e| name(e) == "ribbon").and_then(|r| els(r).find(|e| name(e) == "tabs")) else { return out };
    for tab in els(tabs).filter(|t| name(t) == "tab").take(MAX_RIBBON_ITEMS) {
        if !visible(tab) {
            continue;
        }
        let mut t = RibbonTab {
            id: tab.attr("id").or(tab.attr("idQ")).unwrap_or("").to_string(),
            builtin: tab.attr("idMso").map(str::to_string),
            label: tab.attr("label").unwrap_or("").to_string(),
            groups: Vec::new(),
        };
        for group in els(tab).filter(|g| name(g) == "group").take(MAX_RIBBON_ITEMS) {
            let mut g = RibbonGroup {
                id: group.attr("id").or(group.attr("idQ")).unwrap_or("").to_string(),
                label: group.attr("label").unwrap_or("").to_string(),
                buttons: Vec::new(),
            };
            collect(group, &mut g.buttons, 0);
            if !g.buttons.is_empty() {
                t.groups.push(g);
            }
        }
        if !t.groups.is_empty() {
            out.tabs.push(t);
        }
    }
    out
}

fn collect(e: &El, out: &mut Vec<RibbonButton>, depth: usize) {
    if depth > 8 {
        return;
    }
    for c in els(e) {
        if out.len() >= MAX_RIBBON_ITEMS {
            return;
        }
        if name(c) == "button" && c.attr("idMso").is_none() && visible(c) {
            let large = c.attr("size") == Some("large");
            if let Some(mut b) = button(c, c.attr("id").or(c.attr("idQ")).unwrap_or(""), large) {
                if b.label.is_empty() {
                    b.label = b.id.clone();
                }
                out.push(b);
            }
        } else {
            collect(c, out, depth + 1);
        }
    }
}

/// The ribbon as a `customUI14.xml` part.
pub fn write(r: &Ribbon) -> String {
    let mut x = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<customUI xmlns=\"{NS_CUSTOM_UI14}\"><ribbon><tabs>");
    for t in &r.tabs {
        match &t.builtin {
            Some(mso) => {
                let _ = write!(x, "<tab idMso=\"{}\"", esc(mso));
            }
            None => {
                let _ = write!(x, "<tab id=\"{}\"", esc(&t.id));
            }
        }
        if !t.label.is_empty() {
            let _ = write!(x, " label=\"{}\"", esc(&t.label));
        }
        x.push('>');
        for g in &t.groups {
            let _ = write!(x, "<group id=\"{}\" label=\"{}\">", esc(&g.id), esc(&g.label));
            for b in &g.buttons {
                let _ = write!(x, "<button id=\"{}\" label=\"{}\"", esc(&b.id), esc(&b.label));
                if b.large {
                    x.push_str(" size=\"large\"");
                }
                for (k, v) in [("imageMso", &b.image), ("screentip", &b.screentip), ("supertip", &b.supertip), ("tag", &b.tag)] {
                    if let Some(v) = v {
                        let _ = write!(x, " {k}=\"{}\"", esc(v));
                    }
                }
                let _ = write!(x, " onAction=\"{}\"/>", esc(&b.on_action));
            }
            x.push_str("</group>");
        }
        x.push_str("</tab>");
    }
    x.push_str("</tabs></ribbon></customUI>");
    x
}

/// A richer ribbon kept from the file (not modelled), as its part's bytes: the Office 2010 one
/// when the file has both.
pub fn kept_part(doc: &Document) -> Option<&[u8]> {
    let roots = doc.passthrough.get(crate::package::UNMODELLED_ROOTS)?;
    let parts: Vec<&[u8]> = String::from_utf8_lossy(roots)
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["P", kind, target] if is_ribbon_rel(kind) => doc.passthrough.get(*target).map(|b| b.as_slice()),
                _ => None,
            }
        })
        .collect();
    parts.iter().copied().find(|d| is_v14(d)).or(parts.first().copied())
}

/// Whether the file brought a richer ribbon of its own (kept unchanged; its buttons can't be edited).
pub fn has_kept_ribbon(doc: &Document) -> bool {
    doc.passthrough.get(crate::package::UNMODELLED_ROOTS).is_some_and(|roots| {
        String::from_utf8_lossy(roots).lines().any(|line| {
            let mut f = line.split('\t');
            f.next() == Some("P") && f.next().is_some_and(is_ribbon_rel)
        })
    })
}
