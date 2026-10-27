use super::*;

/// Set Proofing Language: the selection's language and "do not check spelling or grammar",
/// or (`default`) the document's default and the saved language for future blank documents;
/// `detect` is the per-user detection preference.
/// Nothing at all sets US English, as before.
pub(super) fn language(s: &mut Session, v: &Value) -> CmdResult {
    let lang = match p::str(v, "lang") {
        Some(t) => Some(
            wordcraft_proof::lang::normalize_tag(t)
                .ok_or_else(|| CmdError::Params(format!("`lang` must be a language tag such as pl-PL or en-US, not {t:?}")))?,
        ),
        None => None,
    };
    let no_proof = p::bool(v, "noProof");
    let detect = p::bool(v, "detect");
    if let Some(d) = detect {
        s.prefs.detect_language = d;
    }
    if p::bool(v, "default") == Some(true) {
        let l = lang.clone().ok_or_else(|| CmdError::Params("`default` needs `lang`".into()))?;
        s.doc.styles.default_chr.lang = Some(l.clone());
        // A Normal style that names its own language follows the new default.
        if let Some(n) = s.doc.styles.styles.iter_mut().find(|x| x.id == "Normal")
            && n.chr.lang.is_some()
        {
            n.chr.lang = Some(l);
        }
    }
    let lang = match (lang, no_proof, detect) {
        (None, None, None) => Some("en-US".to_string()),
        (l, ..) => l,
    };
    if lang.is_none() && no_proof.is_none() {
        return Ok(json!({"detect": s.prefs.detect_language}));
    }
    let result = crate::cmd::format::apply(s, &|c| {
        if let Some(l) = &lang {
            c.lang = Some(l.clone());
        }
        if let Some(np) = no_proof {
            c.no_proof = Some(np);
        }
    })?;
    if p::bool(v, "default") == Some(true) {
        s.prefs.default_language = lang;
    }
    Ok(result)
}

pub(super) fn macros(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(d) = v.get("define") {
        let name = d.get("name").and_then(Value::as_str).unwrap_or("Macro").to_string();
        let steps: Vec<(String, Value)> = d
            .get("steps")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter().filter_map(|st| Some((st.get("command")?.as_str()?.to_string(), st.get("params").cloned().unwrap_or(json!({}))))).collect()
            })
            .unwrap_or_default();
        s.macros.insert(name, steps);
    }
    if let Some(n) = p::str(v, "delete") {
        s.macros.remove(n);
    }
    if let Some(n) = p::str(v, "run") {
        let steps = s.macros.get(n).cloned().ok_or_else(|| CmdError::Params(format!("no macro `{n}`")))?;
        for (id, params) in steps.iter().take(10_000) {
            if id == "tools.macros" {
                continue;
            }
            s.run(id, params)?;
        }
        return Ok(json!({"ran": n, "steps": steps.len()}));
    }
    Ok(Value::Object(
        s.macros.iter().map(|(k, v)| (k.clone(), json!(v.iter().map(|(c, p)| json!({"command": c, "params": p})).collect::<Vec<_>>()))).collect(),
    ))
}

/// Insert › Quick Parts and AutoText: this session's building blocks, and the document's own
/// (`Document::glossary`, kept in its file) in `gallery`. A name is looked up in the session's
/// first, then in the document's (`gallery` first).
pub(super) fn quick_parts(s: &mut Session, v: &Value, gallery: &str) -> CmdResult {
    let in_document = p::str(v, "in") == Some("document");
    if let Some(name) = p::str(v, "save") {
        let (a, b) = s.sel.ordered();
        if a == b {
            return Err(CmdError::Params("select the content to save".into()));
        }
        let frag = s.doc.copy_range(&a, &b);
        if in_document {
            if name.trim().is_empty() {
                return Err(CmdError::Params("a building block needs a name".into()));
            }
            let blocks: wordcraft_doc::Blocks = frag.blocks.into_iter().map(Arc::new).collect();
            match s.doc.glossary.iter_mut().find(|e| e.name == name && e.gallery == gallery) {
                Some(e) => e.blocks = blocks,
                None => s.doc.glossary.push(wordcraft_doc::BuildingBlock {
                    name: name.to_string(),
                    gallery: gallery.to_string(),
                    category: "General".into(),
                    blocks,
                    ..Default::default()
                }),
            }
        } else {
            s.building_blocks.insert(name.to_string(), frag);
        }
    }
    if let Some(name) = p::str(v, "delete")
        && (in_document || s.building_blocks.remove(name).is_none())
        && let Some(i) = document_entry(&s.doc, name, gallery)
    {
        s.doc.glossary.remove(i);
    }
    if let Some(name) = p::str(v, "insert") {
        let frag = match s.building_blocks.get(name) {
            Some(f) => f.clone(),
            None => {
                let e = document_entry(&s.doc, name, gallery).and_then(|i| s.doc.glossary.get(i));
                let e = e.ok_or_else(|| CmdError::Params(format!("no Quick Part `{name}`")))?;
                wordcraft_doc::edit::Fragment { blocks: e.blocks.iter().map(|b| (**b).clone()).collect(), parts: Default::default() }
            }
        };
        let at = delete_selection(s)?;
        let end = s.doc.insert_fragment(&at, &frag)?;
        s.sel = Selection::caret(end);
    }
    let document: Vec<&str> = s.doc.glossary.iter().filter(|e| e.gallery == gallery).map(|e| e.name.as_str()).collect();
    let mut parts: Vec<&str> = s.building_blocks.keys().map(String::as_str).collect();
    let mut seen: std::collections::HashSet<&str> = parts.iter().copied().collect();
    parts.extend(document.iter().copied().filter(|n| seen.insert(n)));
    Ok(json!({"parts": parts, "document": document}))
}

/// The index of the document's building block `name`, in `gallery` if there is one there.
fn document_entry(doc: &Document, name: &str, gallery: &str) -> Option<usize> {
    let g = &doc.glossary;
    g.iter().position(|e| e.name == name && e.gallery == gallery).or_else(|| g.iter().position(|e| e.name == name))
}

pub(super) fn hf_nav(s: &mut Session, dir: i32) -> CmdResult {
    let StoryRef::Part(cur) = s.sel.focus.story else { return Err(CmdError::Disabled("edit a header or footer first".into())) };
    let mut ids: Vec<u32> = Vec::new();
    for (_, sect) in s.doc.sections() {
        for id in [sect.headers.default, sect.headers.first, sect.headers.even, sect.footers.default, sect.footers.first, sect.footers.even]
            .into_iter()
            .flatten()
        {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let i = ids.iter().position(|x| *x == cur).unwrap_or(0) as i32;
    let n = ids.len().max(1) as i32;
    let next = ids.get(((i + dir).rem_euclid(n)) as usize).copied().unwrap_or(cur);
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Part(next)));
    sel_result(s)
}

pub(super) fn select_similar(s: &mut Session, _: &Value) -> CmdResult {
    let f = s.sel.ordered().0;
    let target = s.doc.para_at(&f).map(|p| p.props_of_char(f.off).clone()).unwrap_or_default();
    let mut first: Option<Pos> = None;
    let mut last: Option<Pos> = None;
    let mut count = 0;
    for path in s.doc.para_paths(f.story) {
        let Some(p) = s.doc.para(f.story, &path) else { continue };
        for (r, c) in p.run_ranges() {
            if *c == target {
                count += 1;
                if first.is_none() {
                    first = Some(Pos { story: f.story, path: path.clone(), off: r.start });
                }
                last = Some(Pos { story: f.story, path: path.clone(), off: r.end });
            }
        }
    }
    if let (Some(a), Some(b)) = (first, last) {
        s.sel = Selection { anchor: a, focus: b };
    }
    Ok(json!({"runs": count}))
}

/// Tools › Customize Keyboard: read and change the custom keyboard shortcuts ([`crate::KeyMap`]).
/// Without params it opens the dialog; with any, it only does what they ask (no dialog).
pub(super) fn customize_keyboard(s: &mut Session, v: &Value) -> CmdResult {
    let reg = s.registry.clone();
    let pair = |x: &Value| -> Result<(String, String), CmdError> { Ok((p::req_str(x, "command")?.to_string(), p::req_str(x, "key")?.to_string())) };
    let mut out = serde_json::Map::new();
    let mut asked = false;
    if p::bool(v, "reset") == Some(true) {
        asked = true;
        s.keymap.reset();
    }
    if let Some(a) = v.get("assign") {
        asked = true;
        let (id, key) = pair(a)?;
        let previous = s.keymap.assign(&reg, &id, &key).map_err(CmdError::Params)?;
        out.insert("replaced".into(), json!(previous));
    }
    if let Some(r) = v.get("remove") {
        asked = true;
        let (id, key) = pair(r)?;
        out.insert("removed".into(), json!(s.keymap.remove(&reg, &id, &key).map_err(CmdError::Params)?));
    }
    if let Some(id) = p::str(v, "command") {
        asked = true;
        if reg.get(id).is_none() {
            return Err(CmdError::Params(format!("unknown command `{id}`")));
        }
        out.insert("keys".into(), json!(s.keymap.keys_for(&reg, id)));
    }
    if let Some(key) = p::str(v, "key") {
        asked = true;
        let key = crate::keymap::normalize(key).ok_or_else(|| CmdError::Params(format!("`{key}` isn't a key (e.g. Mod+Shift+K, Alt+F7, F2)")))?;
        out.insert("assignedTo".into(), json!(s.keymap.resolve(&reg, &key).map(|c| c.id)));
        out.insert("key".into(), json!(key));
    }
    out.insert("custom".into(), serde_json::to_value(&s.keymap).unwrap_or(Value::Null));
    if !asked {
        s.push_ui_request(json!({"open": "customizeKeyboard"}));
    }
    Ok(Value::Object(out))
}

/// `tools.search`: the commands whose label, location or id hold every word of `query`, or,
/// with no query, the Search box (`ui_requests` → the UI's command search).
pub(super) fn search_commands(s: &mut Session, v: &Value) -> CmdResult {
    let Some(query) = p::str(v, "query") else {
        s.push_ui_request(json!({"open": "commands"}));
        return Ok(json!({"matches": []}));
    };
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let matches: Vec<Value> = if words.is_empty() {
        Vec::new()
    } else {
        s.registry
            .all()
            .iter()
            .filter(|c| {
                let hay = format!("{} {} {}", c.label, c.location, c.id).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .take(50)
            .map(|c| json!({"id": c.id, "label": c.label, "location": c.location, "shortcut": c.shortcut}))
            .collect()
    };
    Ok(json!({"matches": matches}))
}

/// Tools › Customize Ribbon: read and change the ribbon's tabs, custom groups and the Quick Access
/// Toolbar ([`crate::RibbonLayout`]). Without params it opens the dialog; with any, it only does
/// what they ask (no dialog).
pub(super) fn customize_ribbon(s: &mut Session, v: &Value) -> CmdResult {
    let reg = s.registry.clone();
    let l = &mut s.ribbon;
    let e = CmdError::Params;
    let str_of = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).map(str::to_string);
    let req = |x: &Value, k: &str| p::req_str(x, k).map(str::to_string);
    let by = |x: &Value| x.get("by").and_then(Value::as_i64).ok_or_else(|| e("`by` (integer, e.g. -1 or 1) is required".into()));
    let mut out = serde_json::Map::new();
    let mut asked = false;
    if let Some(x) = v.get("layout") {
        asked = true;
        *l = crate::RibbonLayout::from_value(x);
        l.retain_known(&reg);
    }
    if let Some(r) = v.get("reset") {
        asked = true;
        match (r.as_bool(), r.as_str()) {
            (Some(true), _) => *l = crate::RibbonLayout::default(),
            (_, Some("ribbon")) => l.reset_ribbon(),
            (_, Some("qat")) => l.reset_qat(),
            _ => return Err(e("`reset` is true, \"ribbon\" or \"qat\"".into())),
        }
    }
    for (key, hidden) in [("hide", true), ("show", false)] {
        if let Some(tab) = p::str(v, key) {
            asked = true;
            l.set_hidden(tab, hidden).map_err(e)?;
        }
    }
    if let Some(x) = v.get("newTab") {
        asked = true;
        let name = l.new_tab(str_of(x, "name").as_deref(), str_of(x, "after").as_deref()).map_err(e)?;
        out.insert("tab".into(), json!(name));
    }
    if let Some(x) = v.get("newGroup") {
        asked = true;
        let name = l.new_group(&req(x, "tab")?, str_of(x, "name").as_deref()).map_err(e)?;
        out.insert("group".into(), json!(name));
    }
    if let Some(x) = v.get("add") {
        asked = true;
        let added = l.add(&reg, &req(x, "tab")?, &req(x, "group")?, &req(x, "command")?, str_of(x, "after").as_deref()).map_err(e)?;
        out.insert("added".into(), json!(added));
    }
    if let Some(x) = v.get("remove") {
        asked = true;
        l.remove(&req(x, "tab")?, str_of(x, "group").as_deref(), str_of(x, "command").as_deref()).map_err(e)?;
    }
    if let Some(x) = v.get("rename") {
        asked = true;
        let name = l.rename(&req(x, "tab")?, str_of(x, "group").as_deref(), &req(x, "to")?).map_err(e)?;
        out.insert("name".into(), json!(name));
    }
    if let Some(x) = v.get("move") {
        asked = true;
        l.move_by(&req(x, "tab")?, str_of(x, "group").as_deref(), str_of(x, "command").as_deref(), by(x)?).map_err(e)?;
    }
    if let Some(id) = p::str(v, "qatAdd") {
        asked = true;
        out.insert("added".into(), json!(l.qat_add(&reg, id, None).map_err(e)?));
    }
    if let Some(id) = p::str(v, "qatRemove") {
        asked = true;
        out.insert("removed".into(), json!(l.qat_remove(id)));
    }
    if let Some(x) = v.get("qatMove") {
        asked = true;
        l.qat_move(&req(x, "command")?, by(x)?).map_err(e)?;
    }
    out.insert("tabs".into(), serde_json::to_value(l.tabs().as_ref()).unwrap_or(Value::Null));
    out.insert("qat".into(), json!(l.qat()));
    if !asked {
        s.push_ui_request(json!({"open": "customizeRibbon"}));
    }
    Ok(Value::Object(out))
}
