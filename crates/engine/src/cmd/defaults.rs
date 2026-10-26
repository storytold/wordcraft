//! Defaults for new documents (File › Options › Defaults): adopting the current document's styles
//! or page setup as the starting point of every new document. The values are kept in the app's
//! preferences (`UiState::defaults` in `ui.json`) and used by `Session::blank_document`; an
//! existing document is never touched.

use serde_json::{Value, json};
use wordcraft_doc::defaults::PageDefaults;
use wordcraft_doc::props::Rgb;
use wordcraft_doc::section::SectionProps;

use super::page;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("defaults.state", "Defaults", "File › Options › Defaults", state)
            .params(r#"{} → {"styles": {"all": bool, "ids": [string]}, "page": {…} | null}"#)
            .pure(),
        CommandSpec::new("defaults.saveStyles", "Set as Default", "File › Options › Defaults", save_styles)
            .params(r#"{"what": "all" | [style id]}"#),
        CommandSpec::new("defaults.savePage", "Set as Default", "File › Options › Defaults", save_page)
            .params(r#"{}"#)
            .pure(),
        CommandSpec::new("defaults.savePageValues", "Set as Default", "File › Options › Defaults", save_page_values)
            .params(r#"{"pageW"?: pt, "pageH"?: pt, "landscape"?: bool, "marginTop"?: pt, "marginBottom"?: pt, "marginLeft"?: pt, "marginRight"?: pt, "mirror"?: bool, "color"?: "RRGGBB" | null}"#),
        CommandSpec::new("defaults.saveStyle", "Set as Default in the App", "File › Options › Defaults", save_style)
            .params(r#"{"style": string, "name"?: string, "next"?: string, "chr"?: CharProps, "para"?: ParaProps, "apply"?: bool (also apply it to the open document, for the Modify Style dialog's button)}"#),
        CommandSpec::new("defaults.resetStyle", "Reset", "File › Options › Defaults", reset_style).params(r#"{"style": string}"#),
        CommandSpec::new("defaults.reset", "Reset", "File › Options › Defaults", reset)
            .params(r#"{"what": "all" | "view" | "styles" | "page"}"#),
    ]
}

/// What the defaults hold, as agents and the Options dialog read it: the adopted styles (all, or
/// their ids) and the page setup.
fn state(s: &mut Session, _: &Value) -> CmdResult {
    Ok(describe(&s.defaults, &s.view))
}

fn describe(d: &wordcraft_doc::defaults::DocumentDefaults, view: &crate::ViewState) -> Value {
    let page = d.page.map(|p| {
        json!({
            "pageW": p.page_w,
            "pageH": p.page_h,
            "landscape": p.landscape,
            "marginTop": p.margin_top,
            "marginBottom": p.margin_bottom,
            "marginLeft": p.margin_left,
            "marginRight": p.margin_right,
            "mirror": p.mirror,
            "color": p.page_color.map(|c| c.hex()),
        })
    });
    json!({
        "styles": {"all": d.sheet.is_some(), "ids": d.style_ids()},
        "page": page,
        "view": {"ruler": view.ruler, "gridlines": view.gridlines, "tableGridlines": view.table_gridlines, "marks": view.marks},
    })
}

fn save_styles(s: &mut Session, v: &Value) -> CmdResult {
    let what = v.get("what").ok_or_else(|| CmdError::Params("`what` (\"all\" or style ids) is required".into()))?;
    match what {
        Value::String(name) if name == "all" => {
            // The whole sheet, as the document has it (built-in and custom styles).
            s.defaults.sheet = Some(s.doc.styles.clone());
            s.defaults.styles.clear();
        }
        Value::Array(ids) => {
            if ids.is_empty() {
                return Err(CmdError::Params("`what` lists no styles".into()));
            }
            if ids.len() > 500 {
                return Err(CmdError::Params("`what` lists more than 500 styles".into()));
            }
            // Validate the whole list first: a bad id must not leave the earlier ones adopted.
            let mut found = Vec::with_capacity(ids.len());
            for id in ids {
                let Some(id) = id.as_str() else {
                    return Err(CmdError::Params("`what` must hold style ids".into()));
                };
                let Some(style) = s.doc.styles.styles.iter().find(|st| st.id == id).cloned() else {
                    return Err(CmdError::Params(format!("unknown style `{id}`")));
                };
                found.push(style);
            }
            for style in found {
                let id = style.id.clone();
                // With a whole sheet adopted, the sheet is the source of truth: update it there,
                // so the two lists never disagree.
                if let Some(sheet) = &mut s.defaults.sheet {
                    match sheet.styles.iter_mut().find(|st| st.id == id) {
                        Some(slot) => *slot = style,
                        None => sheet.styles.push(style),
                    }
                } else {
                    match s.defaults.styles.iter_mut().find(|st| st.id == id) {
                        Some(slot) => *slot = style,
                        None => s.defaults.styles.push(style),
                    }
                }
            }
        }
        _ => return Err(CmdError::Params("`what` must be \"all\" or a list of style ids".into())),
    }
    s.defaults.sanitize();
    Ok(describe(&s.defaults, &s.view))
}

/// Adopt the current document's page setup: the caret section's size, orientation and margins,
/// with the document's mirror margins and page colour.
fn save_page(s: &mut Session, _: &Value) -> CmdResult {
    let sect = page::sect(s);
    s.defaults.page = Some(PageDefaults::from_section(&sect, s.doc.settings.mirror_margins, s.doc.settings.page_color));
    s.defaults.sanitize();
    Ok(describe(&s.defaults, &s.view))
}

/// Adopt page setup values that are not (yet) the document's: the Page Setup dialog's Set as
/// Default, which must not change the open document.
fn save_page_values(s: &mut Session, v: &Value) -> CmdResult {
    // Start from the defaults already adopted (what the editor shows), not from the open
    // document: fields the caller leaves out keep their defaults, not another document's page.
    let mut sect = match s.defaults.page {
        Some(p) => SectionProps {
            page_w: p.page_w,
            page_h: p.page_h,
            landscape: p.landscape,
            margin_top: p.margin_top,
            margin_bottom: p.margin_bottom,
            margin_left: p.margin_left,
            margin_right: p.margin_right,
            ..SectionProps::default()
        },
        None => page::sect(s),
    };
    for (key, slot) in [
        ("pageW", &mut sect.page_w),
        ("pageH", &mut sect.page_h),
        ("marginTop", &mut sect.margin_top),
        ("marginBottom", &mut sect.margin_bottom),
        ("marginLeft", &mut sect.margin_left),
        ("marginRight", &mut sect.margin_right),
    ] {
        if let Some(n) = v.get(key) {
            if n.is_null() {
                continue;
            }
            *slot = p::req_f32(v, key)?;
        }
    }
    if let Some(land) = v.get("landscape").filter(|b| !b.is_null()) {
        let Some(land) = land.as_bool() else { return Err(CmdError::Params("`landscape` must be true or false".into())) };
        sect.landscape = land;
    }
    let mirror = match v.get("mirror") {
        None | Some(Value::Null) => s.defaults.page.map(|p| p.mirror).unwrap_or(s.doc.settings.mirror_margins),
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(CmdError::Params("`mirror` must be true or false".into())),
    };
    let color = match v.get("color") {
        None => s.defaults.page.map(|p| p.page_color).unwrap_or(s.doc.settings.page_color),
        Some(Value::Null) => None,
        Some(Value::String(hex)) => Some(Rgb::parse(hex).ok_or_else(|| CmdError::Params("`color` is not a colour (RRGGBB)".into()))?),
        Some(_) => return Err(CmdError::Params("`color` must be a string or null".into())),
    };
    // The same rules as Layout › Page Setup: an impossible page is refused, not stored.
    sect.margin_top = sect.margin_top.max(0.0);
    sect.margin_bottom = sect.margin_bottom.max(0.0);
    sect.margin_left = sect.margin_left.max(0.0);
    sect.margin_right = sect.margin_right.max(0.0);
    page::validate_section(&mut sect)?;
    s.defaults.page = Some(PageDefaults::from_section(&sect, mirror, color));
    s.defaults.sanitize();
    Ok(describe(&s.defaults, &s.view))
}

/// Adopt the definition the Modify Style dialog shows for one style as the default for new
/// documents. With `apply`, the same definition goes to the open document's style too (the
/// dialog's "Set as Default in the App"), so the two never disagree.
fn save_style(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "style")?;
    let chr: Option<wordcraft_doc::props::CharProps> =
        v.get("chr").map(|c| serde_json::from_value(c.clone())).transpose().map_err(|e| CmdError::Params(e.to_string()))?;
    let para: Option<wordcraft_doc::props::ParaProps> =
        v.get("para").map(|c| serde_json::from_value(c.clone())).transpose().map_err(|e| CmdError::Params(e.to_string()))?;
    let name = p::str(v, "name");
    let next = p::str(v, "next");
    let apply = p::bool(v, "apply").unwrap_or(false);

    // The definition to adopt: the document's style when it has one (its `basedOn`, next style
    // and kind ride along), else the current default, else the built-in one.
    let base = s
        .doc
        .styles
        .get(id)
        .cloned()
        .or_else(|| s.defaults.styles_sheet().get(id).cloned())
        .ok_or_else(|| CmdError::Params(format!("unknown style `{id}`")))?;
    let mut adopted = base.clone();
    if let Some(c) = &chr {
        adopted.chr.overlay(c);
    }
    if let Some(pp) = &para {
        adopted.para.overlay(pp);
    }
    if let Some(n) = next {
        adopted.next = Some(n.to_string());
    }
    if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
        adopted.name = n.to_string();
    }
    adopted.id = id.to_string();

    // Apply to the open document first: it is the operation that can fail (an unknown style),
    // and the defaults then describe exactly what the document has.
    if apply {
        super::para::overlay_style(s, id, chr.as_ref(), para.as_ref(), name, next)?;
    }
    store_default_style(&mut s.defaults, adopted);
    s.defaults.sanitize();
    Ok(describe(&s.defaults, &s.view))
}

/// Put `style` into the defaults: in the adopted sheet when there is one (it is the source of
/// truth), otherwise among the individually adopted styles.
fn store_default_style(d: &mut wordcraft_doc::defaults::DocumentDefaults, style: wordcraft_doc::styles::Style) {
    if let Some(sheet) = &mut d.sheet {
        match sheet.styles.iter_mut().find(|s| s.id == style.id) {
            Some(slot) => *slot = style,
            None => sheet.styles.push(style),
        }
    } else {
        match d.styles.iter_mut().find(|s| s.id == style.id) {
            Some(slot) => *slot = style,
            None => d.styles.push(style),
        }
    }
}

/// Restore one style in the defaults to the application's own definition (a custom style is
/// simply removed from the defaults).
fn reset_style(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "style")?;
    let builtin = wordcraft_doc::styles::StyleSheet::builtin().get(id).cloned();
    if let Some(sheet) = &mut s.defaults.sheet {
        match builtin {
            Some(b) => match sheet.styles.iter_mut().find(|s| s.id == id) {
                Some(slot) => *slot = b,
                None => sheet.styles.push(b),
            },
            None => sheet.styles.retain(|s| s.id != id),
        }
    } else {
        s.defaults.styles.retain(|s| s.id != id);
    }
    Ok(describe(&s.defaults, &s.view))
}

fn reset(s: &mut Session, v: &Value) -> CmdResult {
    let what = p::str(v, "what").unwrap_or("all");
    match what {
        "all" | "view" => {
            let d = crate::ViewState::default();
            s.view.ruler = d.ruler;
            s.view.gridlines = d.gridlines;
            s.view.table_gridlines = d.table_gridlines;
            s.view.marks = d.marks;
        }
        "styles" | "page" => {}
        _ => return Err(CmdError::Params("`what` must be all, view, styles or page".into())),
    }
    match what {
        "all" => {
            s.defaults.sheet = None;
            s.defaults.styles.clear();
            s.defaults.page = None;
        }
        "styles" => {
            s.defaults.sheet = None;
            s.defaults.styles.clear();
        }
        "page" => s.defaults.page = None,
        _ => {}
    }
    Ok(describe(&s.defaults, &s.view))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new(wordcraft_doc::Document::from_text("one"))
    }

    #[test]
    fn adopting_styles_and_page_changes_only_the_defaults() {
        let mut s = session();
        let before = s.doc.clone();
        // One style, by id.
        s.run("defaults.saveStyles", &json!({"what": ["Normal"]})).unwrap();
        assert_eq!(s.defaults.style_ids(), ["Normal"]);
        assert!(s.defaults.sheet.is_none());
        // The whole sheet.
        s.run("defaults.saveStyles", &json!({"what": "all"})).unwrap();
        assert!(s.defaults.sheet.is_some());
        assert_eq!(s.defaults.style_ids().len(), s.doc.styles.styles.len());
        // The page setup, from the document.
        s.run("defaults.savePage", &json!({})).unwrap();
        assert!(s.defaults.page.is_some());
        // None of it touched the open document.
        assert_eq!(s.doc.styles, before.styles);
        assert_eq!(s.doc.last_section, before.last_section);
        // An unknown style is refused.
        assert!(s.run("defaults.saveStyles", &json!({"what": ["NoSuchStyle"]})).is_err());
    }

    #[test]
    fn page_values_from_the_dialog_are_checked() {
        let mut s = session();
        s.run(
            "defaults.savePageValues",
            &json!({"pageW": 600.0, "pageH": 800.0, "marginTop": 40.0, "marginBottom": 40.0, "marginLeft": 40.0, "marginRight": 40.0}),
        )
        .unwrap();
        let p = s.defaults.page.expect("page defaults");
        assert_eq!((p.page_w, p.page_h, p.margin_top), (600.0, 800.0, 40.0));
        // Margins that leave no text room are refused, and the defaults stay as they were.
        assert!(s.run("defaults.savePageValues", &json!({"marginLeft": 590.0, "marginRight": 590.0})).is_err());
        assert_eq!(s.defaults.page.map(|p| p.margin_left), Some(40.0));
        // A colour goes with the page setup.
        s.run("defaults.savePageValues", &json!({"color": "181818"})).unwrap();
        assert_eq!(s.defaults.page.unwrap().page_color, Some(Rgb(0x18, 0x18, 0x18)));
    }

    #[test]
    fn a_new_document_starts_from_the_defaults() {
        let mut s = session();
        // Change the Normal style and the page, adopt both, then make a new document.
        if let Some(normal) = s.doc.styles.styles.iter_mut().find(|st| st.id == "Normal") {
            normal.chr.size = Some(16.0);
        }
        s.run("defaults.saveStyles", &json!({"what": ["Normal"]})).unwrap();
        s.run(
            "defaults.savePageValues",
            &json!({"pageW": 500.0, "pageH": 700.0, "marginTop": 30.0, "marginBottom": 30.0, "marginLeft": 30.0, "marginRight": 30.0, "color": "181818"}),
        )
        .unwrap();
        s.run("file.new", &json!({})).unwrap();
        let normal = s.doc.styles.styles.iter().find(|st| st.id == "Normal").expect("Normal");
        assert_eq!(normal.chr.size, Some(16.0), "the adopted style starts the new document");
        assert_eq!((s.doc.last_section.page_w, s.doc.last_section.page_h), (500.0, 700.0));
        assert_eq!(s.doc.settings.page_color, Some(Rgb(0x18, 0x18, 0x18)));
        // A heading not adopted keeps the built-in definition.
        let h1 = s.doc.styles.styles.iter().find(|st| st.id == "Heading1").expect("Heading 1");
        let builtin = wordcraft_doc::Document::new();
        assert_eq!(h1.chr.size, builtin.styles.styles.iter().find(|st| st.id == "Heading1").unwrap().chr.size);
    }

    #[test]
    fn opening_a_document_keeps_its_own_styles_and_page() {
        let dir = std::env::temp_dir().join(format!("wc-defaults-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("own.docx");
        // A document with its own Normal size and its own page.
        let mut own = session();
        if let Some(normal) = own.doc.styles.styles.iter_mut().find(|st| st.id == "Normal") {
            normal.chr.size = Some(13.0);
        }
        own.doc.last_section.page_w = 612.0;
        own.run("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
        // The person now adopts different defaults.
        let mut s = session();
        s.run("defaults.saveStyles", &json!({"what": "all"})).unwrap();
        s.run(
            "defaults.savePageValues",
            &json!({"pageW": 400.0, "pageH": 400.0, "marginTop": 13.0, "marginBottom": 13.0, "marginLeft": 13.0, "marginRight": 13.0}),
        )
        .unwrap();
        s.run("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
        let normal = s.doc.styles.styles.iter().find(|st| st.id == "Normal").expect("Normal");
        assert_eq!(normal.chr.size, Some(13.0), "the file's own style, not the new defaults");
        assert_eq!(s.doc.last_section.page_w, 612.0, "the file's own page");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Modify Style dialog's definition for one style becomes the default: only that style
    /// is adopted, and a new document starts with it (the others keep the built-in sheet).
    #[test]
    fn a_style_saved_from_the_dialog_is_the_new_default() {
        let mut s = session();
        let chr = json!({"font": "Georgia", "size": 15.0, "bold": false, "italic": false, "color": {"Rgb": [0x11, 0x22, 0x33]}});
        s.run("defaults.saveStyle", &json!({"style": "Normal", "chr": chr, "para": {"spaceBefore": 6.0, "spaceAfter": 4.0}})).unwrap();
        assert_eq!(s.defaults.style_ids(), ["Normal"], "only the edited style is adopted");
        s.run("file.new", &json!({})).unwrap();
        let normal = s.doc.styles.get("Normal").expect("Normal");
        assert_eq!(normal.chr.font.as_deref(), Some("Georgia"));
        assert_eq!(normal.chr.size, Some(15.0));
        assert_eq!(normal.para.space_before, Some(6.0));
        assert_eq!(normal.para.space_after, Some(4.0));
        // Heading 1 keeps the application's own definition.
        let builtin = wordcraft_doc::styles::StyleSheet::builtin();
        assert_eq!(s.doc.styles.get("Heading1"), builtin.get("Heading1"));
        // Restoring the style drops the adoption.
        s.run("defaults.resetStyle", &json!({"style": "Normal"})).unwrap();
        assert!(s.defaults.styles.is_empty());
    }

    /// The dialog's "Set as Default in the App" applies the definition to the open document and
    /// adopts it for new documents, leaving the other styles alone.
    #[test]
    fn the_dialogs_button_applies_and_adopts_together() {
        let mut s = session();
        let other = s.doc.styles.get("Heading1").cloned();
        s.run("defaults.saveStyle", &json!({"style": "Normal", "apply": true, "chr": {"font": "Georgia", "size": 15.0}})).unwrap();
        assert_eq!(s.doc.styles.get("Normal").unwrap().chr.font.as_deref(), Some("Georgia"), "the document's style changed");
        assert_eq!(s.doc.styles.get("Heading1").cloned(), other, "the other styles didn't");
        assert_eq!(s.defaults.styles.len(), 1);
        assert_eq!(s.defaults.styles[0].chr.size, Some(15.0));
        // An unknown style is refused and nothing is adopted.
        let before = s.defaults.clone();
        assert!(s.run("defaults.saveStyle", &json!({"style": "NoSuchStyle", "apply": true})).is_err());
        assert_eq!(s.defaults, before);
        // A colour the model can't parse is refused the same way (the dialog only sends colours
        // it managed to parse).
        assert!(s.run("defaults.saveStyle", &json!({"style": "Normal", "chr": {"color": "not-a-colour"}})).is_err());
        assert_eq!(s.defaults, before);
    }

    #[test]
    fn reset_restores_the_app_defaults() {
        let mut s = session();
        s.view.ruler = false;
        s.view.gridlines = true;
        s.run("defaults.saveStyles", &json!({"what": "all"})).unwrap();
        s.run("defaults.savePage", &json!({})).unwrap();
        let d = s.run("defaults.reset", &json!({"what": "all"})).unwrap();
        assert!(s.defaults.is_empty());
        assert!(s.view.ruler && !s.view.gridlines && s.view.table_gridlines && !s.view.marks);
        assert_eq!(d["view"]["ruler"], true);
        // A fresh blank document again has the app's own styles and page.
        let doc = s.blank_document();
        assert_eq!(doc.styles, wordcraft_doc::Document::new().styles);
    }
}
