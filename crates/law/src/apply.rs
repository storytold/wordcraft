//! Applying a profile to the open document.
//!
//! A profile never formats anything itself: it goes through WordCraft's own commands
//! (`layout.size`, `layout.margins`, `layout.lineNumbers`, `insert.pageNumber`,
//! `layout.pageNumberFormat`, `styles.create`, `styles.modify`), so what a profile does is
//! exactly what the ribbon, the CLI and the MCP server can do, with the same undo and the same
//! protection checks — and the whole application is one undo step.
//!
//! A style the document already has is modified in place, the way Modify Style does: the
//! profile's settings are laid over the style's, so a paragraph keeps the style it has (and its
//! formatting follows) and a document that already carries another profile's styles keeps
//! whatever the new profile leaves out.

use serde_json::{Value, json};
use wordcraft_engine::{CmdError, Session};

use crate::profile::{LineNumbersSpec, PageNumbersSpec, PageSpec, Profile, StyleSpec};
use crate::store::LawError;

/// Apply `doc_type`'s page setup, line numbers and styles to the document `s` has open.
/// Returns what was applied, for the command's result and for the tests.
pub fn apply(s: &mut Session, p: &Profile, doc_type: &str) -> Result<Value, LawError> {
    let Some(spec) = p.docs.get(doc_type) else {
        return Err(LawError::Failed(format!("profile `{}` has no document type `{doc_type}` (it has {})", p.id, p.doc_types().join(", "))));
    };
    let mut applied = Vec::new();
    page(s, &spec.page, &mut applied)?;
    if let Some(ln) = &spec.line_numbers {
        line_numbers(s, ln, &mut applied)?;
    }
    if let Some(pn) = &spec.page_numbers {
        page_numbers(s, pn, &mut applied)?;
    }
    let styles = styles(s, &spec.styles, &mut applied)?;
    crate::checks::remember_applied(s, &p.id, doc_type);
    Ok(json!({"profile": p.id, "doc": doc_type, "applied": applied, "styles": styles}))
}

/// Page size and margins (`layout.size`, `layout.margins`).
fn page(s: &mut Session, page: &PageSpec, applied: &mut Vec<String>) -> Result<(), LawError> {
    let mut size = json!({});
    if let Some(name) = &page.size {
        size["name"] = json!(name);
    } else if let (Some(w), Some(h)) = (page.width, page.height) {
        size["width"] = json!(w);
        size["height"] = json!(h);
    }
    if !size.as_object().is_some_and(|o| o.is_empty()) {
        run(s, "layout.size", size)?;
        applied.push("layout.size".into());
    }
    let mut margins = json!({});
    for (key, v) in [("top", page.top), ("bottom", page.bottom), ("left", page.left), ("right", page.right), ("gutter", page.gutter)] {
        if let Some(v) = v {
            margins[key] = json!(v);
        }
    }
    if !margins.as_object().is_some_and(|o| o.is_empty()) {
        run(s, "layout.margins", margins)?;
        applied.push("layout.margins".into());
    }
    Ok(())
}

/// Line numbers (`layout.lineNumbers`): off when the profile says so, otherwise the profile's
/// arguments as they are.
fn line_numbers(s: &mut Session, ln: &LineNumbersSpec, applied: &mut Vec<String>) -> Result<(), LawError> {
    let mut params = json!({});
    if !ln.enabled {
        params["value"] = json!("none");
    } else {
        if let Some(v) = &ln.value {
            params["value"] = json!(v);
        }
        for (key, v) in
            [("start", ln.start.map(|n| json!(n))), ("countBy", ln.count_by.map(|n| json!(n))), ("distance", ln.distance.map(|d| json!(d)))]
        {
            if let Some(v) = v {
                params[key] = v;
            }
        }
    }
    run(s, "layout.lineNumbers", params)?;
    applied.push("layout.lineNumbers".into());
    Ok(())
}

/// Page numbering (`insert.pageNumber`, `layout.pageNumberFormat`, `layout.differentFirstPage`).
fn page_numbers(s: &mut Session, pn: &PageNumbersSpec, applied: &mut Vec<String>) -> Result<(), LawError> {
    let mut params = json!({"position": pn.position, "align": pn.align});
    if pn.x_of_y {
        params["format"] = json!("x of y");
    }
    if let Some(prefix) = &pn.prefix {
        params["prefix"] = json!(prefix);
    }
    run(s, "insert.pageNumber", params)?;
    applied.push("insert.pageNumber".into());
    let mut fmt = json!({});
    if let Some(f) = &pn.format {
        fmt["format"] = json!(f);
    }
    if let Some(start) = pn.start {
        fmt["start"] = json!(start);
    }
    if let Some(prefix) = &pn.prefix {
        fmt["prefix"] = json!(prefix);
    }
    if !fmt.as_object().is_some_and(|o| o.is_empty()) {
        run(s, "layout.pageNumberFormat", fmt)?;
        applied.push("layout.pageNumberFormat".into());
    }
    match pn.cover_numbered {
        Some(false) => {
            run(s, "layout.differentFirstPage", json!({"value": true}))?;
            applied.push("layout.differentFirstPage".into());
        }
        Some(true) => {
            run(s, "layout.differentFirstPage", json!({"value": false}))?;
            applied.push("layout.differentFirstPage".into());
        }
        None => {}
    }
    Ok(())
}

/// The profile's named styles. Every style is created or updated first; `basedOn` and `next` are
/// wired in a second pass so forward references (Caption before Body) resolve.
fn styles(s: &mut Session, want: &[StyleSpec], applied: &mut Vec<String>) -> Result<Vec<String>, LawError> {
    let mut ids: Vec<String> = Vec::new();
    for i in create_order(want) {
        let Some(st) = want.get(i) else { continue };
        let id = ensure_style(s, st, applied)?;
        modify_formatting(s, st, &id)?;
        ids.push(id);
    }
    for st in want {
        let Some(id) = style_id(s, &st.name) else { continue };
        modify_relationships(s, st, &id)?;
        if st.based_on.is_some() || st.next.is_some() {
            applied.push(format!("styles.modify {} (links)", st.name));
        }
    }
    Ok(ids)
}

/// Create the style when it is missing; return its document id.
fn ensure_style(s: &mut Session, st: &StyleSpec, applied: &mut Vec<String>) -> Result<String, LawError> {
    if let Some(id) = style_id(s, &st.name) {
        applied.push(format!("styles.modify {}", st.name));
        return Ok(id);
    }
    let mut params = json!({"name": st.name, "fromSelection": false, "apply": false});
    if let Some(base) = st.based_on.as_deref().and_then(|b| style_id(s, b)) {
        params["basedOn"] = json!(base);
    }
    run(s, "styles.create", params)?;
    applied.push(format!("styles.create {}", st.name));
    style_id(s, &st.name).ok_or_else(|| LawError::Failed(format!("styles.create `{}` did not add the style", st.name)))
}

/// Character and paragraph formatting from the profile.
fn modify_formatting(s: &mut Session, st: &StyleSpec, id: &str) -> Result<(), LawError> {
    if st.chr.is_none() && st.para.is_none() {
        return Ok(());
    }
    let mut params = json!({"style": id});
    if let Some(chr) = &st.chr {
        params["chr"] = serde_json::to_value(chr).map_err(|e| LawError::Failed(e.to_string()))?;
    }
    if let Some(para) = &st.para {
        params["para"] = serde_json::to_value(para).map_err(|e| LawError::Failed(e.to_string()))?;
    }
    run(s, "styles.modify", params).map(|_| ())
}

/// `basedOn` and `next` after every named style exists in the document.
fn modify_relationships(s: &mut Session, st: &StyleSpec, id: &str) -> Result<(), LawError> {
    let based = st.based_on.as_deref().and_then(|b| style_id(s, b));
    let next = st.next.as_deref().and_then(|n| style_id(s, n));
    if based.is_none() && next.is_none() {
        return Ok(());
    }
    let mut params = json!({"style": id});
    if let Some(b) = based {
        params["basedOn"] = json!(b);
    }
    if let Some(n) = next {
        params["next"] = json!(n);
    }
    run(s, "styles.modify", params).map(|_| ())
}

/// The order to define missing styles in: a style based on another style of the profile comes
/// after it (a circular base is what [`crate::profile::problems`] reports).
fn create_order(want: &[StyleSpec]) -> Vec<usize> {
    let base_in_profile = |b: &str| want.iter().any(|st| st.name.eq_ignore_ascii_case(b));
    let mut order: Vec<usize> = Vec::new();
    let mut defined: Vec<String> = Vec::new();
    while order.len() < want.len() {
        let ready = want.iter().enumerate().find(|(i, st)| {
            !order.contains(i) && st.based_on.as_deref().is_none_or(|b| !base_in_profile(b) || defined.iter().any(|d| d.eq_ignore_ascii_case(b)))
        });
        let Some((i, st)) = ready.or_else(|| want.iter().enumerate().find(|(i, _)| !order.contains(i))) else { break };
        defined.push(st.name.clone());
        order.push(i);
    }
    order
}

/// A style's id in the document, by id or by name.
fn style_id(s: &Session, name: &str) -> Option<String> {
    s.doc.styles.find(name).map(|st| st.id.clone())
}

/// Run one of WordCraft's own commands. Every one of them joins the undo step
/// `law.profiles.apply` opened, so applying a profile is a single Undo.
fn run(s: &mut Session, id: &str, params: Value) -> Result<Value, LawError> {
    s.join_next_undo();
    s.run(id, &params).map_err(|e| match e {
        CmdError::Unknown(_) => LawError::Failed(format!("{id}: WordCraft no longer has this command")),
        other => LawError::Failed(format!("{id}: {other}")),
    })
}
