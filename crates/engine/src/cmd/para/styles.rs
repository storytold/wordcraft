use super::format::fmt;
use super::lists::touch_all;
use super::{CharProps, CmdError, CmdResult, ParaProps, Session, Style, StyleKind, Value, json, p, sel_result};

pub(super) fn style_id(s: &Session, name: &str) -> Result<String, CmdError> {
    s.doc.styles.find(name).map(|x| x.id.clone()).ok_or_else(|| CmdError::Params(format!("no style `{name}`")))
}

/// Apply Styles. Pure itself: with a `style` it runs `para.style` as its own command (so undo,
/// protection, macro recording and Repeat see a real edit); without one it only shows the
/// styles pane, which is view state and never an edit.
pub(super) fn apply_styles(s: &mut Session, v: &Value) -> CmdResult {
    match v.get("style") {
        None => {
            s.view.styles_pane = true;
            Ok(json!({"pane": true}))
        }
        Some(Value::String(name)) if !name.trim().is_empty() => s.run("para.style", &json!({"style": name})),
        Some(Value::String(_)) => Err(CmdError::Params("`style` must not be empty".into())),
        Some(_) => Err(CmdError::Params("`style` must be a string (name or id)".into())),
    }
}

pub(super) fn apply_style(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "style")?;
    let id = style_id(s, name)?;
    let kind = s.doc.styles.get(&id).map(|x| x.kind).unwrap_or_default();
    if kind == StyleKind::Character {
        return crate::cmd::format::apply(s, &|c| c.style = Some(id.clone()));
    }
    let (a, b) = s.sel.ordered();
    // A style linked to a list level (a heading in a numbered-headings list) brings its number.
    let numbered = s.doc.styles.resolve_para(&ParaProps { style: Some(id.clone()), ..Default::default() }).numbering.is_some();
    fmt(s, &|p| {
        p.style = Some(id.clone());
        // Applying a heading or Normal removes direct list numbering (except list styles); a
        // numbered style's own number replaces any direct numbering, even numbering switched off.
        if numbered || ((id.starts_with("Heading") || id == "Title" || id == "Normal") && p.numbering.is_some_and(|n| n.num != 0)) {
            p.numbering = None;
        }
    })?;
    // A paragraph style resets direct character formatting that matches nothing (Word keeps
    // direct formatting unless it covers most of the paragraph; we clear font/size/colour).
    if a.path == b.path || a == b {
        let paths = s.doc.paths_between(&a, &b);
        for path in paths {
            let para = s.doc.para_mut(a.story, &path)?;
            let len = para.len();
            para.format(0, len, &|c| {
                c.font = None;
                c.font_cs = None;
                c.font_ea = None;
                c.size = None;
                c.size_cs = None;
                c.color = None;
            })?;
            para.mark.font = None;
            para.mark.font_cs = None;
            para.mark.font_ea = None;
            para.mark.size = None;
            para.mark.size_cs = None;
            para.mark.color = None;
        }
    }
    s.pending = None;
    sel_result(s)
}

pub(super) fn create_style(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if name.is_empty() {
        return Err(CmdError::Params("name is empty".into()));
    }
    if s.doc.styles.find(&name).is_some() {
        return Err(CmdError::Failed(format!("a style named `{name}` already exists")));
    }
    let id = s.doc.styles.new_id(&name);
    let based = p::str(v, "basedOn").and_then(|b| s.doc.styles.find(b).map(|x| x.id.clone())).or(Some("Normal".into()));
    let mut st = Style {
        id: id.clone(),
        name,
        kind: StyleKind::Paragraph,
        based_on: based,
        next: Some(id.clone()),
        quick: true,
        priority: Some(50),
        ..Default::default()
    };
    if p::bool(v, "fromSelection").unwrap_or(true)
        && let Some(para) = s.doc.para_at(&s.sel.focus)
    {
        st.para = para.props.clone();
        st.para.style = None;
        st.para.numbering = None;
        // A symbol's own font is the symbol's, not the style's.
        st.chr = para.props_at(s.sel.focus.off).clone().without_symbol();
        st.chr.style = None;
        st.chr.link = None;
        st.chr.ins = None;
        st.chr.del = None;
        st.chr.fmt_change = None;
        st.para = st.para.formatting();
    }
    s.doc.styles.upsert(st);
    apply_style(s, &json!({"style": id}))?;
    Ok(json!({"id": id}))
}

pub(super) fn modify_style(s: &mut Session, v: &Value) -> CmdResult {
    let id = style_id(s, p::req_str(v, "style")?)?;
    let chr: Option<CharProps> = v.get("chr").map(|c| serde_json::from_value(c.clone())).transpose().map_err(|e| CmdError::Params(e.to_string()))?;
    let para: Option<ParaProps> =
        v.get("para").map(|c| serde_json::from_value(c.clone())).transpose().map_err(|e| CmdError::Params(e.to_string()))?;
    let next = p::str(v, "next").map(str::to_string);
    let name = p::str(v, "name").map(str::to_string);
    let st = s.doc.styles.get_mut(&id).ok_or_else(|| CmdError::Params("no such style".into()))?;
    if let Some(c) = chr {
        st.chr.overlay(&c);
    }
    if let Some(pp) = para {
        st.para.overlay(&pp);
    }
    if let Some(n) = next {
        st.next = Some(n);
    }
    if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
        st.name = n;
    }
    // Every paragraph's layout depends on styles: bump all revisions.
    touch_all(s);
    sel_result(s)
}

/// Update Style to Match Selection: the style takes on how the selection looks — its paragraph
/// style, character style and direct formatting, read at the start of the selection — so every
/// paragraph using the style changes. A paragraph style is then applied to the selected
/// paragraphs, whose direct formatting the style now carries is dropped (other direct formatting,
/// a bold word say, stays). The style keeps its outline level and list numbering.
pub(super) fn update_to_match(s: &mut Session, v: &Value) -> CmdResult {
    let (a, b) = s.sel.ordered();
    let Some(para) = s.doc.para_at(&a).cloned() else { return sel_result(s) };
    let id = match p::str(v, "style") {
        Some(n) => style_id(s, n)?,
        None => para.props.style.clone().unwrap_or_else(|| "Normal".into()),
    };
    let sheet = &s.doc.styles;
    let Some(target) = sheet.get(&id).cloned() else { return Err(CmdError::Params(format!("no style `{id}`"))) };
    if !matches!(target.kind, StyleKind::Paragraph | StyleKind::Character) {
        return Err(CmdError::Failed(format!("`{}` can't be updated to match the selection", target.name)));
    }
    // A symbol's own font is the symbol's, not the style's.
    let direct = if s.sel.is_collapsed() { para.props_at(a.off) } else { para.props_of_char(a.off) }.clone().without_symbol();
    let para_style = para.props.style.clone().unwrap_or_else(|| "Normal".into());
    // How the selection looks: document defaults, its paragraph style chain, its character
    // style chain, direct formatting (the resolve order, before defaults are filled in).
    let (mut want_p, mut want_c) = sheet.para_style_props(&para_style);
    let char_chain = |c: &mut CharProps, style: Option<&str>| {
        for st in style.map(|cs| sheet.chain(cs)).unwrap_or_default() {
            if matches!(st.kind, StyleKind::Character | StyleKind::Paragraph) {
                c.overlay(&st.chr);
            }
        }
    };
    char_chain(&mut want_c, direct.style.as_deref());
    want_c.overlay(&CharProps { style: None, ..direct.formatting() });
    want_p.overlay(&ParaProps { style: None, numbering: None, ..para.props.formatting() });
    // How the style looks now, in the same terms.
    let (have_p, have_c) = match target.kind {
        StyleKind::Character => {
            let (pp, mut cc) = sheet.para_style_props(&para_style);
            char_chain(&mut cc, Some(&id));
            (pp, cc)
        }
        _ => sheet.para_style_props(&id),
    };
    let chr = retarget(&target.chr, &have_c, &want_c, CHAR_KEEP)?;
    let pp = if target.kind == StyleKind::Paragraph { retarget(&target.para, &have_p, &want_p, PARA_KEEP)? } else { target.para.clone() };
    if let Some(st) = s.doc.styles.get_mut(&id) {
        st.chr = chr;
        st.para = pp;
    }
    if target.kind == StyleKind::Paragraph {
        // The selected paragraphs take the style; what it now gives them needn't stay direct.
        let paths = s.doc.paths_between(&a, &b);
        for path in paths {
            let p = s.doc.para_mut(a.story, &path)?;
            p.props.style = Some(id.clone());
            p.props = strip_matching(&p.props, &want_p, PARA_KEEP);
            let len = p.len();
            p.format(0, len, &|c| *c = strip_matching(c, &want_c, CHAR_KEEP))?;
            p.mark = strip_matching(&p.mark, &want_c, CHAR_KEEP);
        }
    }
    s.pending = None;
    touch_all(s);
    sel_result(s)
}

/// Character properties Update to Match leaves alone: the character style, links and revisions.
const CHAR_KEEP: &[&str] = &["style", "link", "ins", "del", "fmtChange"];

/// Paragraph properties Update to Match leaves alone: the style, its outline level (a heading
/// stays a heading), list numbering and revisions.
const PARA_KEEP: &[&str] = &["style", "outlineLevel", "numbering", "numChange", "fmtChange"];

fn props_map<T: serde::Serialize>(t: &T) -> serde_json::Map<String, Value> {
    match serde_json::to_value(t) {
        Ok(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    }
}

/// A style's own properties `own`, changed so that its chain gives `want` where it gave `have`:
/// a property that differs is set to the wanted value, or dropped (inherited again) when the
/// selection doesn't set it at all.
fn retarget<T: serde::Serialize + serde::de::DeserializeOwned>(own: &T, have: &T, want: &T, keep: &[&str]) -> Result<T, CmdError> {
    let (mut out, have, want) = (props_map(own), props_map(have), props_map(want));
    let keys: std::collections::BTreeSet<&String> = have.keys().chain(want.keys()).collect();
    for k in keys {
        if keep.contains(&k.as_str()) || have.get(k) == want.get(k) {
            continue;
        }
        match want.get(k) {
            Some(v) => out.insert(k.clone(), v.clone()),
            None => out.remove(k),
        };
    }
    serde_json::from_value(Value::Object(out)).map_err(|e| CmdError::Failed(e.to_string()))
}

/// `direct` without the properties that have the same value in `inherited` (outside `keep`).
fn strip_matching<T: serde::Serialize + serde::de::DeserializeOwned + Clone>(direct: &T, inherited: &T, keep: &[&str]) -> T {
    let (mut out, inherited) = (props_map(direct), props_map(inherited));
    out.retain(|k, v| keep.contains(&k.as_str()) || inherited.get(k) != Some(v));
    serde_json::from_value(Value::Object(out)).unwrap_or_else(|_| direct.clone())
}

/// Delete a custom style. Text that uses it falls back to the style it was based on (or the
/// default style), styles based on it are rebased, and a custom character style linked to it
/// goes with it. Built-in styles are refused.
pub(super) fn delete_style(s: &mut Session, v: &Value) -> CmdResult {
    let id = style_id(s, p::req_str(v, "style")?)?;
    let Some(st) = s.doc.styles.get(&id).cloned() else { return Err(CmdError::Params(format!("no style `{id}`"))) };
    // Table styles: their tables take Table Grid and styles based on them keep their look.
    if st.kind == StyleKind::Table {
        let mut out = crate::cmd::table_style::delete(s, &id)?;
        if let Some(o) = out.as_object_mut() {
            let tables = o.get("tables").cloned().unwrap_or(json!(0));
            o.insert("deleted".into(), json!([id]));
            o.insert("restyled".into(), tables);
        }
        return Ok(out);
    }
    if st.builtin {
        return Err(CmdError::Failed(format!("`{}` is a built-in style and can't be deleted", st.name)));
    }
    let mut gone = vec![st.clone()];
    if let Some(partner) = st.linked.as_deref().and_then(|l| s.doc.styles.get(l))
        && !partner.builtin
        && partner.linked.as_deref() == Some(id.as_str())
    {
        gone.push(partner.clone());
    }
    let mut changed = 0;
    for g in &gone {
        s.doc.styles.remove(&g.id);
        // Fall back to the base only while it is a style of the same kind that still exists.
        let base = g.based_on.clone().filter(|b| s.doc.styles.get(b).is_some_and(|x| x.kind == g.kind));
        changed += s.doc.restyle(&g.id, base);
    }
    touch_all(s);
    Ok(json!({"deleted": gone.iter().map(|g| g.id.clone()).collect::<Vec<_>>(), "restyled": changed}))
}

/// Styles gallery and style list visibility (`w:qFormat` / `w:semiHidden`).
pub(super) fn set_visibility(s: &mut Session, v: &Value) -> CmdResult {
    let id = style_id(s, p::req_str(v, "style")?)?;
    let (gallery, hidden) = (p::bool(v, "gallery"), p::bool(v, "hidden"));
    if gallery.is_none() && hidden.is_none() {
        return Err(CmdError::Params("give `gallery` and/or `hidden`".into()));
    }
    let st = s.doc.styles.get_mut(&id).ok_or_else(|| CmdError::Params(format!("no style `{id}`")))?;
    if let Some(h) = hidden {
        st.hidden = h;
    }
    if let Some(g) = gallery {
        st.quick = g;
        // Showing a style in the gallery unhides it.
        if g {
            st.hidden = false;
        }
    }
    let out = json!({"style": st.id, "inGallery": st.quick && !st.hidden && st.kind != StyleKind::Table, "hidden": st.hidden});
    touch_all(s);
    Ok(out)
}
