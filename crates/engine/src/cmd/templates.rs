//! Tools › Templates and Add-ins: the template a document is attached to, updating the
//! document's styles from it, and the Organizer (copy styles from another document or template,
//! delete and rename styles in this one).
//!
//! The attached template is kept as the file gives it (`w:attachedTemplate`, ECMA-376 Part 1
//! §17.15.1.6), and "Automatically update document styles" is `w:linkStyles` (§17.15.1.56).
//! The template path comes from the document, so it is never read on its own (not on open
//! either): only an explicit `tools.updateStyles` or `tools.attachTemplate` with `update` reads it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wordcraft_doc::Document;
use wordcraft_doc::numbering::{AbstractNum, Num};
use wordcraft_doc::styles::{Style, StyleKind};

use super::para::touch_all;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Longest template path or style name list we take from params.
const MAX_PATH: usize = 32 * 1024;
/// Most styles one copy takes (with the styles they are based on).
const MAX_COPY: usize = 10_000;
/// Longest style name (Word's limit).
const MAX_NAME: usize = 253;

pub fn specs() -> Vec<CommandSpec> {
    const LOC: &str = "View › Templates › Templates and Add-ins";
    vec![
        CommandSpec::new("tools.templates", "Templates and Add-ins", "View › Templates", |s, _| Ok(state(s)))
            .params(r#"{} → {"template": path|null, "linkStyles": bool} (from the ribbon, the app shows the dialog)"#)
        .pure(),
        CommandSpec::new("tools.attachTemplate", "Attach Template", LOC, attach)
            .params(r#"{"path": string, "linkStyles"?: bool, "update"?: bool (update the styles from it now), "data"?: base64 (the template's bytes, for "update")}"#),
        CommandSpec::new("tools.detachTemplate", "Detach Template", LOC, |s, _| {
            s.doc.settings.attached_template = None;
            s.doc.settings.link_styles = false;
            Ok(state(s))
        }),
        CommandSpec::new("tools.linkStyles", "Automatically Update Document Styles", LOC, |s, v| {
            s.doc.settings.link_styles = p::bool(v, "value").unwrap_or(!s.doc.settings.link_styles);
            Ok(state(s))
        })
        .params(r#"{"value"?: bool (default: toggle)}"#),
        CommandSpec::new("tools.updateStyles", "Update Styles from Template", LOC, update_styles)
            .params(r#"{"path"?: string (default: the attached template), "data"?: base64 (the file's bytes; `path` then only names it)} → {"added", "replaced"}"#),
        CommandSpec::new("styles.organizer", "Organizer", "View › Templates › Organizer", organizer)
            .params(r#"{"path"?: string, "data"?: base64} → {"document": [style], "file"?: {"path", "styles": [style]}}"#)
            .pure(),
        CommandSpec::new("styles.copyFrom", "Copy Styles", "View › Templates › Organizer", copy_from)
            .params(r#"{"path": string, "data"?: base64, "styles": [name or id]} → {"added", "replaced", "skipped"}"#),
        CommandSpec::new("styles.rename", "Rename Style", "View › Templates › Organizer", rename)
            .params(r#"{"style": string (name or id; custom styles only), "name": string}"#),
    ]
}

fn state(s: &Session) -> Value {
    json!({"template": s.doc.settings.attached_template, "linkStyles": s.doc.settings.link_styles})
}

fn attach(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?.trim();
    if path.is_empty() {
        return Err(CmdError::Params("`path` is empty".into()));
    }
    if path.len() > MAX_PATH {
        return Err(CmdError::Params("`path` is too long".into()));
    }
    let path = path.to_string();
    s.doc.settings.attached_template = Some(path.clone());
    if let Some(on) = p::bool(v, "linkStyles") {
        s.doc.settings.link_styles = on;
    }
    let mut out = state(s);
    if p::bool(v, "update").unwrap_or(false) {
        let mut params = json!({"path": path});
        if let (Some(d), Some(o)) = (p::str(v, "data"), params.as_object_mut()) {
            o.insert("data".into(), json!(d));
        }
        let r = update_styles(s, &params)?;
        if let Some(o) = out.as_object_mut() {
            o.insert("updated".into(), r);
        }
    }
    Ok(out)
}

/// Copy every style of the template (and its document defaults) into the document, replacing
/// styles of the same name.
fn update_styles(s: &mut Session, v: &Value) -> CmdResult {
    let attached = s.doc.settings.attached_template.clone();
    let path = p::str(v, "path").map(str::to_string).or(attached).ok_or_else(|| CmdError::Params("no template is attached; give `path`".into()))?;
    let base = s.path.clone();
    let src = load_other(&path, p::str(v, "data"), base.as_deref())?;
    let report = apply_template_styles(&mut s.doc, &src);
    touch_all(s);
    Ok(report)
}

/// What [`update_styles`] does to a document: all of `src`'s styles and its defaults.
pub fn apply_template_styles(doc: &mut Document, src: &Document) -> Value {
    doc.styles.default_chr = src.styles.default_chr.clone();
    doc.styles.default_para = src.styles.default_para.clone();
    let ids: Vec<String> = src.styles.styles.iter().map(|st| st.id.clone()).collect();
    copy_styles(doc, src, &ids)
}

fn organizer(s: &mut Session, v: &Value) -> CmdResult {
    let mut out = json!({"document": style_list(&s.doc)});
    if let Some(path) = p::str(v, "path") {
        let base = s.path.clone();
        let other = load_other(path, p::str(v, "data"), base.as_deref())?;
        if let Some(o) = out.as_object_mut() {
            o.insert("file".into(), json!({"path": path, "styles": style_list(&other)}));
        }
    }
    Ok(out)
}

/// A document's styles for the Organizer, by name: `[{id, name, type, builtIn}]`.
pub fn style_list(doc: &Document) -> Value {
    let mut list: Vec<&Style> = doc.styles.styles.iter().collect();
    list.sort_by_cached_key(|st| st.name.to_lowercase());
    Value::Array(
        list.iter()
            .map(|st| {
                let kind = match st.kind {
                    StyleKind::Paragraph => "paragraph",
                    StyleKind::Character => "character",
                    StyleKind::Table => "table",
                    StyleKind::Numbering => "numbering",
                };
                json!({"id": st.id, "name": st.name, "type": kind, "builtIn": st.builtin})
            })
            .collect(),
    )
}

fn copy_from(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let names: Vec<&str> = match v.get("styles") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).take(MAX_COPY).collect(),
        Some(Value::String(one)) => vec![one.as_str()],
        _ => return Err(CmdError::Params("`styles` (list of style names) is required".into())),
    };
    if names.is_empty() {
        return Err(CmdError::Params("`styles` is empty".into()));
    }
    let base = s.path.clone();
    let src = load_other(path, p::str(v, "data"), base.as_deref())?;
    let mut ids = Vec::new();
    for n in names {
        let st = src.styles.find(n).ok_or_else(|| CmdError::Params(format!("`{path}` has no style `{n}`")))?;
        ids.push(st.id.clone());
    }
    let report = copy_styles(&mut s.doc, &src, &ids);
    touch_all(s);
    Ok(report)
}

fn rename(s: &mut Session, v: &Value) -> CmdResult {
    let which = p::req_str(v, "style")?;
    let name = p::req_str(v, "name")?.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(CmdError::Params(format!("a style name has 1 to {MAX_NAME} characters")));
    }
    let st = s.doc.styles.find(which).cloned().ok_or_else(|| CmdError::Params(format!("no style `{which}`")))?;
    if st.builtin {
        return Err(CmdError::Failed(format!("`{}` is a built-in style and can't be renamed", st.name)));
    }
    if s.doc.styles.styles.iter().any(|o| o.id != st.id && o.name.eq_ignore_ascii_case(&name)) {
        return Err(CmdError::Failed(format!("a style named `{name}` already exists")));
    }
    if let Some(x) = s.doc.styles.get_mut(&st.id) {
        x.name = name.clone();
    }
    touch_all(s);
    Ok(json!({"id": st.id, "name": name}))
}

/// Read another document or template: from `data` (base64 bytes, `name` gives the format) or
/// from the file `name` names (a path or `file:` URL; relative to `base`'s folder).
fn load_other(name: &str, data: Option<&str>, base: Option<&Path>) -> Result<Document, CmdError> {
    if name.len() > MAX_PATH {
        return Err(CmdError::Params("`path` is too long".into()));
    }
    let local = local_path(name, base).ok_or_else(|| CmdError::Failed(format!("`{name}` is not a local file")))?;
    let ext = local.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "" | "txt" | "text") {
        return Err(CmdError::Params(format!("`{name}` is not a document or template with styles")));
    }
    match data {
        Some(d) => {
            let bytes = super::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64".into()))?;
            crate::io::open_bytes(&local.to_string_lossy(), &bytes).map_err(CmdError::Failed)
        }
        None => crate::io::open_path(&local).map_err(CmdError::Failed),
    }
}

/// A template reference as a local path: a `file:` URL (percent-encoded) becomes a path, a
/// relative path is taken from `base`'s folder. Other URLs (`http:`…) are `None`.
pub fn local_path(target: &str, base: Option<&Path>) -> Option<PathBuf> {
    let t = target.trim();
    let lower = t.to_ascii_lowercase();
    let path = if lower.starts_with("file:") {
        let rest = percent_decode(t.get(5..)?);
        let rest = rest.strip_prefix("//").map(|r| r.strip_prefix("localhost").unwrap_or(r)).unwrap_or(&rest).to_string();
        // `file:///C:/x` → `C:/x`.
        let b = rest.as_bytes();
        if b.first() == Some(&b'/') && b.get(1).is_some_and(u8::is_ascii_alphabetic) && b.get(2) == Some(&b':') {
            rest.get(1..)?.to_string()
        } else {
            rest
        }
    } else if t.contains("://") {
        return None;
    } else {
        t.to_string()
    };
    if path.is_empty() {
        return None;
    }
    let p = PathBuf::from(&path);
    if p.is_relative()
        && !is_windows_absolute(&path)
        && let Some(dir) = base.and_then(Path::parent)
    {
        return Some(dir.join(p));
    }
    Some(p)
}

fn is_windows_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with("\\\\") || (b.first().is_some_and(u8::is_ascii_alphabetic) && b.get(1) == Some(&b':'))
}

fn percent_decode(s: &str) -> String {
    let hex = |c: u8| (c as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(s.len());
    let mut it = s.bytes();
    while let Some(c) = it.next() {
        if c != b'%' {
            out.push(c);
            continue;
        }
        let rest = it.clone();
        match (it.next().and_then(hex), it.next().and_then(hex)) {
            (Some(h), Some(l)) => out.push((h << 4) | l),
            _ => {
                out.push(b'%');
                it = rest;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Copy styles `ids` of `src` into `doc`, Organizer-style: a style replaces the document's style
/// of the same name (and kind), or is added (under its own id when that is free). The styles they
/// are based on that the document lacks, and their linked halves, come too; `based on`, `next`
/// and `linked` are mapped to the document's ids, and lists a style numbers with are copied.
/// Returns `{"added": [name], "replaced": [name], "skipped": [name]}`.
pub fn copy_styles(doc: &mut Document, src: &Document, ids: &[String]) -> Value {
    let sheet = &src.styles;
    let same_name = |doc: &Document, st: &Style| doc.styles.styles.iter().find(|d| d.name.eq_ignore_ascii_case(&st.name)).map(|d| d.id.clone());
    // What to copy: the asked-for styles, their linked halves, and missing bases.
    let mut want: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = ids.iter().rev().cloned().collect();
    while let Some(id) = queue.pop() {
        if want.len() >= MAX_COPY {
            break;
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(st) = sheet.get(&id) else { continue };
        want.push(id.clone());
        if let Some(l) = st.linked.as_deref()
            && sheet.get(l).is_some_and(|x| x.linked.as_deref() == Some(st.id.as_str()))
        {
            queue.push(l.to_string());
        }
        if let Some(b) = st.based_on.as_deref()
            && let Some(base) = sheet.get(b)
            && same_name(doc, base).is_none()
        {
            queue.push(b.to_string());
        }
    }
    // Ids in the document, in `src` order.
    let mut map: HashMap<String, String> = HashMap::new();
    let mut order: Vec<(String, String)> = Vec::new();
    let (mut added, mut replaced, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    let want: HashSet<String> = want.into_iter().collect();
    for st in sheet.styles.iter().filter(|st| want.contains(&st.id)) {
        let target = match same_name(doc, st) {
            Some(existing) => {
                if doc.styles.get(&existing).is_some_and(|d| d.kind != st.kind) {
                    skipped.push(st.name.clone());
                    continue;
                }
                replaced.push(st.name.clone());
                existing
            }
            None => {
                added.push(st.name.clone());
                if doc.styles.get(&st.id).is_none() { st.id.clone() } else { doc.styles.new_id(&st.name) }
            }
        };
        let builtin = doc.styles.get(&target).is_some_and(|d| d.builtin) || st.builtin;
        doc.styles.upsert(Style { id: target.clone(), builtin, ..st.clone() });
        map.insert(st.id.clone(), target.clone());
        order.push((st.id.clone(), target));
    }
    // References to `src` ids → the document's ids (copied styles, or same-named ones).
    let resolve = |doc: &Document, map: &HashMap<String, String>, r: &str| -> Option<String> {
        map.get(r).cloned().or_else(|| sheet.get(r).and_then(|st| same_name(doc, st)))
    };
    let mut nums: HashMap<u32, u32> = HashMap::new();
    for (src_id, target) in order {
        let Some(st) = sheet.get(&src_id) else { continue };
        let based_on = st.based_on.as_deref().and_then(|r| resolve(doc, &map, r)).filter(|b| *b != target);
        let next = st.next.as_deref().and_then(|r| resolve(doc, &map, r));
        let linked = st.linked.as_deref().and_then(|r| map.get(r).cloned());
        let numbering = st.para.numbering.map(|mut n| {
            if n.num != 0 {
                n.num = import_num(doc, src, n.num, &map, &mut nums).unwrap_or(0);
            }
            n
        });
        if let Some(d) = doc.styles.get_mut(&target) {
            d.based_on = based_on;
            d.next = next;
            d.linked = linked;
            d.para.numbering = numbering.filter(|n| n.num != 0 || st.para.numbering.is_some_and(|o| o.num == 0));
        }
    }
    json!({"added": added, "replaced": replaced, "skipped": skipped})
}

/// Copy list `num` of `src` (and its abstract list) into `doc` under new ids; memoised in `done`.
fn import_num(doc: &mut Document, src: &Document, num: u32, map: &HashMap<String, String>, done: &mut HashMap<u32, u32>) -> Option<u32> {
    if let Some(n) = done.get(&num) {
        return Some(*n);
    }
    let n = src.numbering.nums.iter().find(|n| n.id == num)?;
    let abs = src.numbering.abstracts.iter().find(|a| a.id == n.abstract_id)?;
    let new_abs = doc.numbering.abstracts.iter().map(|a| a.id).max().map_or(Some(0), |m| m.checked_add(1))?;
    let new_num = doc.numbering.nums.iter().map(|n| n.id).max().map_or(Some(1), |m| m.checked_add(1))?;
    let mut levels = abs.levels.clone();
    for l in &mut levels {
        l.style = l.style.as_deref().and_then(|s| map.get(s).cloned().or_else(|| doc.styles.get(s).map(|x| x.id.clone())));
    }
    doc.numbering.abstracts.push(AbstractNum { id: new_abs, name: abs.name.clone(), levels });
    doc.numbering.nums.push(Num { id: new_num, abstract_id: new_abs, ..n.clone() });
    done.insert(num, new_num);
    Some(new_num)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::numbering::ListKind;
    use wordcraft_doc::props::{CharProps, NumRef};

    /// A template built in code: a changed Heading 1, a custom "Callout" based on a custom
    /// "Box" (missing in the document) and a numbered "Steps" style.
    fn template() -> Document {
        let mut t = Document::new();
        if let Some(h) = t.styles.get_mut("Heading1") {
            h.chr.size = Some(31.0);
        }
        t.styles.upsert(Style {
            id: "Box".into(),
            name: "Box".into(),
            kind: StyleKind::Paragraph,
            based_on: Some("Normal".into()),
            ..Default::default()
        });
        t.styles.upsert(Style {
            id: "Callout".into(),
            name: "Callout".into(),
            kind: StyleKind::Paragraph,
            based_on: Some("Box".into()),
            chr: CharProps { italic: Some(true), ..Default::default() },
            ..Default::default()
        });
        let num = t.numbering.add_list(ListKind::Numbered);
        t.styles.upsert(Style {
            id: "Steps".into(),
            name: "Steps".into(),
            kind: StyleKind::Paragraph,
            para: wordcraft_doc::props::ParaProps { numbering: Some(NumRef { num, level: 0 }), ..Default::default() },
            ..Default::default()
        });
        t
    }

    fn save(name: &str, doc: &Document) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-templates-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Report.dotx");
        crate::io::save_path(&path, doc).unwrap();
        path
    }

    #[test]
    fn update_styles_from_template_is_one_undo_step() {
        let path = save("update", &template());
        let mut s = Session::new(Document::new());
        s.run("tools.attachTemplate", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(s.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()));
        let before = s.doc.styles.clone();
        let r = s.run("tools.updateStyles", &json!({})).unwrap();
        assert!(r["replaced"].as_array().unwrap().iter().any(|n| n == "heading 1" || n == "Heading 1"), "{r}");
        assert_eq!(s.doc.styles.get("Heading1").unwrap().chr.size, Some(31.0));
        let callout = s.doc.styles.get("Callout").unwrap();
        assert_eq!(callout.based_on.as_deref(), Some("Box"), "the missing base came along");
        let steps = s.doc.styles.get("Steps").unwrap().para.numbering.unwrap();
        assert!(s.doc.numbering.nums.iter().any(|n| n.id == steps.num), "the style's list was copied");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.styles, before);
        // A missing template is an error, not a crash, and changes nothing.
        let err = s.run("tools.updateStyles", &json!({"path": "/nonexistent/x.dotx"}));
        assert!(err.is_err());
        assert_eq!(s.doc.styles, before);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn organizer_copies_renames_and_undoes() {
        let path = save("organizer", &template());
        let p = path.to_string_lossy().to_string();
        let mut s = Session::new(Document::new());
        let listed = s.run("styles.organizer", &json!({"path": p})).unwrap();
        assert!(listed["file"]["styles"].as_array().unwrap().iter().any(|x| x["name"] == "Callout"));
        let r = s.run("styles.copyFrom", &json!({"path": p, "styles": ["Callout"]})).unwrap();
        assert_eq!(r["added"], json!(["Box", "Callout"]));
        assert_eq!(s.doc.styles.get("Callout").unwrap().chr.italic, Some(true));
        s.run("styles.rename", &json!({"style": "Callout", "name": "Aside"})).unwrap();
        assert_eq!(s.doc.styles.find("Aside").map(|x| x.id.as_str()), Some("Callout"));
        assert!(s.run("styles.rename", &json!({"style": "Normal", "name": "Plain"})).is_err(), "built-in styles keep their names");
        assert!(s.run("styles.rename", &json!({"style": "Aside", "name": "Box"})).is_err(), "names stay unique");
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.styles.find("Callout").is_some());
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.styles.get("Callout").is_none() && s.doc.styles.get("Box").is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The template path comes from the document, so opening never reads it, even with
    /// "Automatically update document styles" on: only an explicit `tools.updateStyles` does.
    #[test]
    fn opening_a_linked_document_never_reads_its_template() {
        let path = save("open", &template());
        let mut d = Document::new();
        d.settings.attached_template = Some("Report.dotx".into());
        d.settings.link_styles = true;
        let doc_path = path.with_file_name("Letter.docx");
        crate::io::save_path(&doc_path, &d).unwrap();
        let mut s = Session::new(Document::new());
        s.run("file.open", &json!({"path": doc_path.to_string_lossy()})).unwrap();
        assert_eq!(s.doc.settings.attached_template.as_deref(), Some("Report.dotx"), "the reference is kept");
        assert!(s.doc.settings.link_styles);
        assert_ne!(s.doc.styles.get("Heading1").unwrap().chr.size, Some(31.0), "the template's styles were not read");
        assert!(s.doc.styles.get("Callout").is_none());
        assert!(!s.dirty);
        // The user's explicit update reads it (relative to the document's folder).
        s.run("tools.updateStyles", &json!({})).unwrap();
        assert_eq!(s.doc.styles.get("Heading1").unwrap().chr.size, Some(31.0));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn template_references_become_local_paths() {
        assert_eq!(local_path("file:///C:/Users/A%20B/T.dotx", None), Some(PathBuf::from("C:/Users/A B/T.dotx")));
        assert_eq!(local_path("file:///home/a/T.dotx", None), Some(PathBuf::from("/home/a/T.dotx")));
        assert_eq!(local_path("https://example.com/T.dotx", None), None);
        assert_eq!(local_path("T.dotx", Some(Path::new("/docs/Letter.docx"))), Some(PathBuf::from("/docs/T.dotx")));
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }
}
