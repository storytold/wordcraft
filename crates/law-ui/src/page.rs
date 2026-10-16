//! File › Options › LawCraft: court profiles (list, edit, import, export, approve) and firm
//! details ([`crate::pleading::firm_section`]). English only:
//! the add-in's own words aren't in WordCraft's interface catalogs.

use std::collections::BTreeMap;

use egui::{RichText, Ui, vec2};
use serde_json::{Value, json};
use wordcraft_law::MODULE;
use wordcraft_ui_egui::file_dialogs::FileDialogRequest;
use wordcraft_ui_egui::theme::{Tokens, semibold};

use crate::Ctx;

/// Check kinds the profile schema and checks engine know.
const CHECK_KINDS: &[&str] = &["wordLimit", "minFontSize", "minMargins", "minLineSpacing", "noHyperlinks", "textSearchablePdf", "requiredSections"];

/// UI state for the LawCraft options page.
#[derive(Clone, Debug, Default)]
pub struct LawUi {
    /// Profile id selected in the list.
    pub selected: Option<String>,
    /// Document type being edited for the selected profile.
    pub doc_type: String,
    /// Waiting for the person to confirm approval.
    pub confirm_approve: bool,
    /// Last validation message from `law.profiles.validate` (every problem, one block).
    pub validation: Option<String>,
    /// Profile id a save dialog is exporting (`None` during import).
    pub export_id: Option<String>,
    /// Section display names being edited (`profile_id` + section `id`).
    pub section_name_drafts: BTreeMap<String, String>,
    /// Check source text being edited (`profile_id` + document type + check index). Kept between
    /// frames so typing is not lost before the field loses focus and saves.
    pub check_source_drafts: BTreeMap<String, String>,
    /// Style font name being edited (`profile_id` + document type + style index). Kept between
    /// frames so a name with spaces is not trimmed to `TimesNewRoman` while typing.
    pub style_font_drafts: BTreeMap<String, String>,
    /// Check font name being edited (`profile_id` + document type + check index). The same
    /// draft-until-commit rule as [`Self::style_font_drafts`].
    pub check_font_drafts: BTreeMap<String, String>,
    /// The New Pleading window, while it is open.
    pub pleading: Option<crate::pleading::PleadingForm>,
    /// Firm details as shown on the options page (`None`: read the saved ones again).
    pub firm_draft: Option<crate::pleading::Firm>,
    /// The options page's firm details have changes not saved yet.
    pub firm_dirty: bool,
    /// Why the options page's firm details couldn't be saved.
    pub firm_message: Option<String>,
}

/// After selecting or importing a profile, keep `doc_type` on a key that exists in `docs`.
fn reconcile_doc_type(law: &mut LawUi, profile: &Value) {
    let docs = profile["docs"].as_object();
    let first = docs.and_then(|d| d.keys().next()).map(String::from).unwrap_or_default();
    if law.doc_type.is_empty() || docs.is_none_or(|d| !d.contains_key(&law.doc_type)) {
        law.doc_type = first;
    }
}

fn section_draft_key(profile_id: &str, doc_key: &str, section_id: &str) -> String {
    format!("{profile_id}\0{doc_key}\0section\0{section_id}")
}

fn check_draft_key(profile_id: &str, doc_key: &str, index: usize) -> String {
    format!("{profile_id}\0{doc_key}\0{index}")
}

fn style_font_draft_key(profile_id: &str, doc_key: &str, index: usize) -> String {
    format!("{profile_id}\0{doc_key}\0style\0{index}")
}

/// Every draft key for one profile starts with its id and a NUL.
fn draft_prefix(profile_id: &str) -> String {
    format!("{profile_id}\0")
}

/// Forget one profile's text drafts (the ones just saved). Drafts for other profiles stay.
fn clear_drafts_for(law: &mut LawUi, profile_id: &str) {
    let prefix = draft_prefix(profile_id);
    law.section_name_drafts.retain(|k, _| !k.starts_with(&prefix));
    law.check_source_drafts.retain(|k, _| !k.starts_with(&prefix));
    law.check_font_drafts.retain(|k, _| !k.starts_with(&prefix));
    law.style_font_drafts.retain(|k, _| !k.starts_with(&prefix));
}

/// Whether the profile has text typed in a field that has not been committed yet.
fn has_drafts(law: &LawUi, profile_id: &str) -> bool {
    let prefix = draft_prefix(profile_id);
    law.section_name_drafts.keys().any(|k| k.starts_with(&prefix))
        || law.check_source_drafts.keys().any(|k| k.starts_with(&prefix))
        || law.check_font_drafts.keys().any(|k| k.starts_with(&prefix))
        || law.style_font_drafts.keys().any(|k| k.starts_with(&prefix))
}

/// Write every pending text draft of `profile_id` into `profile`, in place. The values are the
/// same trimmed strings the field's own blur handler would have committed.
fn apply_drafts(law: &LawUi, profile_id: &str, profile: &mut Value) {
    let Some(docs) = profile.get_mut("docs").and_then(Value::as_object_mut) else { return };
    for (doc_key, doc) in docs.iter_mut() {
        if let Some(sections) = doc.get_mut("sections").and_then(Value::as_array_mut) {
            for sec in sections.iter_mut() {
                let sid = sec.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                if let Some(name) = law.section_name_drafts.get(&section_draft_key(profile_id, doc_key, &sid))
                    && let Some(o) = sec.as_object_mut()
                {
                    o.insert("name".into(), json!(name.trim()));
                }
            }
        }
        if let Some(checks) = doc.get_mut("checks").and_then(Value::as_array_mut) {
            for (i, ch) in checks.iter_mut().enumerate() {
                let key = check_draft_key(profile_id, doc_key, i);
                let Some(o) = ch.as_object_mut() else { continue };
                if let Some(source) = law.check_source_drafts.get(&key) {
                    o.insert("source".into(), json!(source.trim()));
                }
                if let Some(font) = law.check_font_drafts.get(&key) {
                    set_font(o, font.trim());
                }
            }
        }
        if let Some(styles) = doc.get_mut("styles").and_then(Value::as_array_mut) {
            for (i, st) in styles.iter_mut().enumerate() {
                if let Some(font) = law.style_font_drafts.get(&style_font_draft_key(profile_id, doc_key, i))
                    && let Some(o) = st.as_object_mut()
                {
                    let chr = o.entry("chr").or_insert(json!({}));
                    if let Some(c) = chr.as_object_mut() {
                        set_font(c, font.trim());
                    }
                }
            }
        }
    }
}

/// Write a font draft into a character-props (or check) object consistently. A font is
/// `Option<String>`: the supported representation of "no font" is the key being absent, so an
/// empty draft removes it. Persisting `""` would fail validation (`font is empty`), so treating
/// blank as unset lets a cleared field commit instead of silently dropping the edit.
fn set_font(obj: &mut serde_json::Map<String, Value>, font: &str) {
    if font.is_empty() {
        obj.remove("font");
    } else {
        obj.insert("font".into(), json!(font));
    }
}

/// Profile ids that still have uncommitted text edits on the LawCraft page. The page has one draft
/// store; a draft belongs to the profile whose id prefixes its key.
fn profile_ids_with_drafts(law: &LawUi) -> Vec<String> {
    let mut ids = std::collections::BTreeSet::new();
    for key in
        law.section_name_drafts.keys().chain(law.check_source_drafts.keys()).chain(law.check_font_drafts.keys()).chain(law.style_font_drafts.keys())
    {
        if let Some(id) = key.split('\0').next().filter(|s| !s.is_empty()) {
            ids.insert(id.to_string());
        }
    }
    ids.into_iter().collect()
}

/// Save every pending LawCraft text edit for the whole page through `law.profiles.update`. Call
/// before any navigation (switch profile or document type, import, export, approve, leave Options)
/// and before the module toggle can hide the pane. Returns `false` — and leaves the failed draft in
/// place with `law.validation` set — if any profile's edit cannot be saved, so the caller can cancel.
pub fn commit_pending_drafts(app: &mut Ctx<'_>) -> bool {
    for id in profile_ids_with_drafts(app.law) {
        if !flush_drafts(app, &id) {
            return false;
        }
    }
    true
}

fn reload_profile(app: &mut Ctx<'_>, profile_id: &str, profile: &mut Value) {
    if let Ok(p) = app.run("law.profiles.get", json!({"id": profile_id})) {
        *profile = p;
    }
}

/// Save one profile's pending text edits through `law.profiles.update`, applying them with
/// [`apply_drafts`]. Its drafts are removed only when the update succeeds; on failure they stay, so
/// returning to the profile shows the text again and it can be retried.
fn flush_drafts(app: &mut Ctx<'_>, profile_id: &str) -> bool {
    if !has_drafts(app.law, profile_id) {
        return true;
    }
    let mut profile = match app.run("law.profiles.get", json!({"id": profile_id})) {
        Ok(p) => p,
        Err(e) => {
            app.law.validation = Some(e);
            return false;
        }
    };
    apply_drafts(app.law, profile_id, &mut profile);
    match save_profile(app, profile_id, profile) {
        Ok(_) => {
            clear_drafts_for(app.law, profile_id);
            true
        }
        Err(()) => false,
    }
}

/// The LawCraft settings page (only when the module is on).
pub fn page(app: &mut Ctx<'_>, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new("LawCraft").font(semibold(15.0)));
    ui.label(
        RichText::new(
            "Court profiles list the sections, styles and checks for each filing type. Edits save the profile as unverified until you approve it.",
        )
        .small()
        .weak(),
    );
    ui.add_space(8.0);

    let list = match app.run("law.profiles.list", json!({})) {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return;
        }
    };

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(240.0);
            ui.label(RichText::new("Court profiles").font(semibold(14.0)));
            egui::ScrollArea::vertical().id_salt("law_profiles").max_height(360.0).show(ui, |ui| {
                for row in &list {
                    let id = row["id"].as_str().unwrap_or("");
                    let name = row["name"].as_str().unwrap_or(id);
                    let jurisdiction = row["jurisdiction"].as_str().unwrap_or("");
                    let verified = row["verified"].as_bool() == Some(true);
                    let active = app.law.selected.as_deref() == Some(id);
                    let badge = if verified { "Verified" } else { "Unverified" };
                    let mut clicked = false;
                    ui.horizontal(|ui| {
                        clicked = ui.selectable_label(active, name).clicked();
                        ui.label(RichText::new(jurisdiction).small().weak());
                        ui.label(RichText::new(badge).small().color(if verified {
                            egui::Color32::from_rgb(0x10, 0x7c, 0x10)
                        } else {
                            ui.visuals().warn_fg_color
                        }));
                    });
                    if clicked {
                        // Every click in the list is a navigation: commit every pending draft for
                        // the whole page before the selection changes. A field whose blur never came
                        // (the click landed elsewhere first) still has its text in a draft, and a
                        // draft belonging to another profile must not be dropped either. If any
                        // commit fails the selection stays put and the error shows inline.
                        if commit_pending_drafts(app) {
                            app.law.selected = Some(id.to_string());
                            app.law.validation = None;
                            app.law.confirm_approve = false;
                            if let Ok(p) = app.run("law.profiles.get", json!({"id": id})) {
                                reconcile_doc_type(app.law, &p);
                            }
                        }
                    }
                    ui.label(RichText::new(id).small().weak());
                    ui.add_space(4.0);
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Import…").clicked() && commit_pending_drafts(app) {
                    app.addin_ask_file(MODULE, FileDialogRequest::Open { purpose: "json".into() }, "import");
                }
                let id = app.law.selected.clone();
                ui.add_enabled_ui(id.is_some(), |ui| {
                    if ui.button("Export…").clicked() && commit_pending_drafts(app) {
                        let profile_id = id.as_deref().unwrap_or("");
                        let name = format!("{}.json", profile_id);
                        app.law.export_id = id.clone();
                        app.addin_ask_file(MODULE, FileDialogRequest::Save { name }, "export");
                    }
                });
            });
        });
        ui.add_space(12.0);
        let (line, _) = ui.allocate_exact_size(vec2(1.0, 400.0), egui::Sense::hover());
        ui.painter().rect_filled(line, 0.0, t.border);
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_min_width(420.0);
            if let Some(id) = app.law.selected.clone() {
                profile_editor(app, ui, &id, &list);
            } else {
                ui.label("Select a court profile to view or edit it.");
            }
        });
    });
    ui.add_space(16.0);
    crate::pleading::firm_section(app, ui);
}

fn profile_editor(app: &mut Ctx<'_>, ui: &mut Ui, id: &str, list: &[Value]) {
    let summary = list.iter().find(|r| r["id"].as_str() == Some(id));
    let verified = summary.and_then(|r| r["verified"].as_bool()).unwrap_or(false);
    let mut profile = match app.run("law.profiles.get", json!({"id": id})) {
        Ok(p) => p,
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return;
        }
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new(profile["name"].as_str().unwrap_or(id)).font(semibold(16.0)));
        let badge = if verified { "Verified" } else { "Unverified" };
        let color = if verified { egui::Color32::from_rgb(0x10, 0x7c, 0x10) } else { ui.visuals().warn_fg_color };
        ui.label(RichText::new(badge).color(color));
    });

    if let Some(msg) = app.law.validation.as_ref().filter(|s| !s.is_empty()) {
        ui.add_space(4.0);
        ui.colored_label(ui.visuals().error_fg_color, msg);
    }

    ui.add_space(6.0);
    let docs: Vec<String> = profile["docs"].as_object().map(|d| d.keys().cloned().collect()).unwrap_or_default();
    if docs.is_empty() {
        ui.label("This profile has no document types.");
        return;
    }
    ui.horizontal(|ui| {
        ui.label("Document type:");
        // Switching document type is a navigation: commit every pending page draft first, so an
        // edit that never blurred is saved and any unsavable draft cancels the switch.
        for d in &docs {
            if ui.selectable_label(app.law.doc_type == *d, d.as_str()).clicked() && app.law.doc_type != *d && commit_pending_drafts(app) {
                app.law.doc_type = d.clone();
            }
        }
    });

    let doc_key = app.law.doc_type.clone();
    let doc = profile["docs"].get(&doc_key).cloned().unwrap_or(Value::Null);

    wordcraft_ui_egui::options::section(ui, "Sections");
    sections_editor(app, ui, id, &doc_key, &doc, &mut profile);

    wordcraft_ui_egui::options::section(ui, "Styles");
    styles_editor(app, ui, id, &doc_key, &doc, &mut profile);

    wordcraft_ui_egui::options::section(ui, "Checks");
    checks_editor(app, ui, id, &doc_key, &doc, &mut profile);

    ui.add_space(8.0);
    if !verified {
        approve_ui(app, ui, id);
    }
}

fn approve_ui(app: &mut Ctx<'_>, ui: &mut Ui, id: &str) {
    if app.law.confirm_approve {
        ui.label("Approve this court profile? You are confirming that its rules match the court's requirements.");
        ui.horizontal(|ui| {
            if ui.button("Approve").clicked() {
                // Approval is a navigation for the draft store: every pending page draft must save
                // first, or the approval is cancelled.
                if !commit_pending_drafts(app) {
                    return;
                }
                match app.run("law.profiles.approve", json!({"id": id})) {
                    Ok(_) => {
                        app.law.confirm_approve = false;
                        app.law.validation = None;
                        app.status("Court profile approved.");
                    }
                    Err(e) => app.status(e),
                }
            }
            if ui.button("Cancel").clicked() {
                app.law.confirm_approve = false;
            }
        });
    } else if ui.button("Approve profile…").clicked() && commit_pending_drafts(app) {
        app.law.confirm_approve = true;
    }
}

fn sections_editor(app: &mut Ctx<'_>, ui: &mut Ui, profile_id: &str, doc_key: &str, doc: &Value, profile: &mut Value) {
    let sections = doc.get("sections").and_then(Value::as_array).cloned().unwrap_or_default();
    if sections.is_empty() {
        ui.label("No sections defined for this document type.");
        return;
    }
    let required = required_section_ids(doc);
    for (i, sec) in sections.iter().enumerate() {
        let sid = sec["id"].as_str().unwrap_or("");
        let draft_key = section_draft_key(profile_id, doc_key, sid);
        let stored_name = sec["name"].as_str().unwrap_or("");
        let mut name = app.law.section_name_drafts.get(&draft_key).cloned().unwrap_or_else(|| stored_name.to_string());
        let is_req = required.iter().any(|r| r == sid);
        let mut req = is_req;
        ui.horizontal(|ui| {
            let label = ui.label(format!("{}.", i + 1));
            let resp = ui.text_edit_singleline(&mut name).labelled_by(label.id);
            if resp.changed() {
                app.law.section_name_drafts.insert(draft_key.clone(), name.clone());
            }
            ui.checkbox(&mut req, "Required");
            if ui.add_enabled(i > 0, egui::Button::new("↑")).clicked() {
                reorder_sections(app, profile_id, doc_key, profile, i, i - 1);
            }
            if ui.add_enabled(i + 1 < sections.len(), egui::Button::new("↓")).clicked() {
                reorder_sections(app, profile_id, doc_key, profile, i, i + 1);
            }
            let name_save = resp.lost_focus() && name.trim() != stored_name.trim();
            if name_save || req != is_req {
                // Commit the name by section id, not by position: an arrow click in this same frame
                // has already reordered the sections, so `i` may now point at a neighbour.
                if name_save {
                    let secs = patch_section_name(profile, doc_key, sid, name.trim());
                    if save_doc(app, profile_id, doc_key, profile, json!({"sections": secs})) {
                        app.law.section_name_drafts.remove(&draft_key);
                    }
                }
                if req != is_req {
                    set_section_required_in_profile(profile, doc_key, sid, req);
                    let checks = profile["docs"][doc_key]["checks"].clone();
                    save_doc(app, profile_id, doc_key, profile, json!({"checks": checks}));
                }
            }
        });
    }
}

/// Set one section's display name in the profile's *current* sections. The section is found by id,
/// so a reorder that already happened this frame cannot misdirect the rename to its neighbour.
fn patch_section_name(profile: &Value, doc_key: &str, section_id: &str, name: &str) -> Vec<Value> {
    let mut secs = profile["docs"][doc_key]["sections"].as_array().cloned().unwrap_or_default();
    for s in &mut secs {
        if s.get("id").and_then(Value::as_str) == Some(section_id)
            && let Some(o) = s.as_object_mut()
        {
            o.insert("name".into(), json!(name));
        }
    }
    secs
}

fn required_section_ids(doc: &Value) -> Vec<String> {
    doc.get("checks")
        .and_then(Value::as_array)
        .map(|checks| {
            checks
                .iter()
                .filter(|c| c["kind"].as_str() == Some("requiredSections"))
                .flat_map(|c| c["ids"].as_array().into_iter().flat_map(|a| a.iter().filter_map(|v| v.as_str().map(String::from))))
                .collect()
        })
        .unwrap_or_default()
}

fn set_section_required_in_profile(profile: &mut Value, doc_key: &str, section_id: &str, required: bool) {
    let doc = profile["docs"].get(doc_key).cloned().unwrap_or(Value::Null);
    let checks = doc.get("checks").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut checks = checks;
    let mut found = false;
    for c in &mut checks {
        if c.get("kind").and_then(Value::as_str) == Some("requiredSections") {
            found = true;
            let ids = c.get("ids").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut ids: Vec<Value> = ids;
            let present = ids.iter().any(|v| v.as_str() == Some(section_id));
            if required && !present {
                ids.push(json!(section_id));
            } else if !required && present {
                ids.retain(|v| v.as_str() != Some(section_id));
            }
            if let Some(o) = c.as_object_mut() {
                o.insert("ids".into(), json!(ids));
            }
        }
    }
    if required && !found {
        checks.push(json!({
            "kind": "requiredSections",
            "ids": [section_id],
            "source": "Edited in LawCraft settings"
        }));
    }
    if let Some(docs) = profile.get_mut("docs").and_then(Value::as_object_mut)
        && let Some(d) = docs.get_mut(doc_key).and_then(Value::as_object_mut)
    {
        d.insert("checks".into(), json!(checks));
    }
}

fn reorder_sections(app: &mut Ctx<'_>, profile_id: &str, doc_key: &str, profile: &mut Value, from: usize, to: usize) {
    let doc = profile["docs"].get(doc_key).cloned().unwrap_or(Value::Null);
    let mut sections = doc.get("sections").and_then(Value::as_array).cloned().unwrap_or_default();
    if from >= sections.len() || to >= sections.len() {
        return;
    }
    let item = sections.remove(from);
    sections.insert(to, item);
    save_doc(app, profile_id, doc_key, profile, json!({"sections": sections}));
}

/// The styles the profile currently holds for one document type (the up-to-date value, not the
/// snapshot a frame started with).
fn current_styles(profile: &Value, doc_key: &str) -> Vec<Value> {
    profile["docs"][doc_key]["styles"].as_array().cloned().unwrap_or_default()
}

/// Merge edits into one style. It reads the profile's *current* styles, so a commit later in the
/// same frame (the line-spacing field after the font field saved) cannot revert the earlier edit
/// by cloning the frame's starting snapshot.
fn patch_style(
    app: &mut Ctx<'_>,
    profile_id: &str,
    doc_key: &str,
    profile: &mut Value,
    index: usize,
    edit: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> bool {
    let mut styles = current_styles(profile, doc_key);
    if let Some(s) = styles.get_mut(index).and_then(Value::as_object_mut) {
        edit(s);
    }
    save_doc(app, profile_id, doc_key, profile, json!({"styles": styles}))
}

fn styles_editor(app: &mut Ctx<'_>, ui: &mut Ui, profile_id: &str, doc_key: &str, doc: &Value, profile: &mut Value) {
    let styles = doc.get("styles").and_then(Value::as_array).cloned().unwrap_or_default();
    if styles.is_empty() {
        ui.label("No styles defined for this document type.");
        return;
    }
    egui::ScrollArea::vertical().id_salt("law_styles").max_height(200.0).show(ui, |ui| {
        for (i, st) in styles.iter().enumerate() {
            let name = st["name"].as_str().unwrap_or("");
            ui.collapsing(name, |ui| {
                let draft_key = style_font_draft_key(profile_id, doc_key, i);
                let stored_font = st.pointer("/chr/font").and_then(Value::as_str).unwrap_or("").to_string();
                let mut font = app.law.style_font_drafts.get(&draft_key).cloned().unwrap_or_else(|| stored_font.clone());
                let mut size = st.pointer("/chr/size").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                ui.horizontal(|ui| {
                    let font_label = ui.label("Font:");
                    let font_resp = ui.text_edit_singleline(&mut font).labelled_by(font_label.id);
                    // Keep the untrimmed text between frames: trimming on every keystroke would
                    // drop the space in "Times New Roman" before the next character arrives.
                    if font_resp.changed() {
                        app.law.style_font_drafts.insert(draft_key.clone(), font.clone());
                    }
                    ui.label("Size (pt):");
                    let size_changed = ui.add(egui::DragValue::new(&mut size).speed(0.5).range(1.0..=4096.0)).changed();
                    let font_dirty = font.trim() != stored_font.trim();
                    if (size_changed || (font_resp.lost_focus() && font_dirty))
                        && patch_style(app, profile_id, doc_key, profile, i, |s| {
                            let chr = s.entry("chr").or_insert(json!({}));
                            if let Some(c) = chr.as_object_mut() {
                                // Blank means "no font", the supported unset value; the shared
                                // commit path ([`apply_drafts`]) applies the same rule so a
                                // cleared field is saved rather than silently discarded.
                                set_font(c, font.trim());
                                if size > 0.0 {
                                    c.insert("size".into(), json!(size));
                                }
                            }
                        })
                        && font_resp.lost_focus()
                        && font_dirty
                    {
                        app.law.style_font_drafts.remove(&draft_key);
                    }
                });
                let mut spacing = st.pointer("/para/lineSpacing/value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                ui.horizontal(|ui| {
                    ui.label("Line spacing (multiple):");
                    if ui.add(egui::DragValue::new(&mut spacing).speed(0.05).range(0.5..=4.0)).changed() {
                        patch_style(app, profile_id, doc_key, profile, i, |s| {
                            let para = s.entry("para").or_insert(json!({}));
                            if let Some(p) = para.as_object_mut() {
                                p.insert("lineSpacing".into(), json!({"rule": "multiple", "value": spacing}));
                            }
                        });
                    }
                });
            });
        }
    });
    ui.label(RichText::new("These are the profile's named styles. Applying the profile writes them into the document, where WordCraft's usual style tools can refine them.").small().weak());
}

fn checks_editor(app: &mut Ctx<'_>, ui: &mut Ui, profile_id: &str, doc_key: &str, doc: &Value, profile: &mut Value) {
    let checks = doc.get("checks").and_then(Value::as_array).cloned().unwrap_or_default();
    let section_ids: Vec<String> = doc
        .get("sections")
        .and_then(Value::as_array)
        .map(|secs| secs.iter().filter_map(|s| s["id"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    let style_names: Vec<String> = doc
        .get("styles")
        .and_then(Value::as_array)
        .map(|st| st.iter().filter_map(|s| s["name"].as_str().map(String::from)).collect())
        .unwrap_or_default();

    ui.horizontal(|ui| {
        ui.label(RichText::new("Checks for this document type").small().weak());
        if ui.button("Add check").clicked() && commit_pending_drafts(app) {
            reload_profile(app, profile_id, profile);
            let mut checks = current_checks(profile, doc_key);
            checks.push(json!({"kind": "wordLimit", "limit": 10000, "source": "Edited in LawCraft settings"}));
            save_doc(app, profile_id, doc_key, profile, json!({"checks": checks}));
        }
    });

    if checks.is_empty() {
        ui.label("No checks defined for this document type.");
        return;
    }

    for (i, ch) in checks.iter().enumerate() {
        let kind = ch["kind"].as_str().unwrap_or("");
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label("Kind:");
                let mut selected_kind = kind.to_string();
                egui::ComboBox::from_id_salt(("law_check_kind", profile_id, doc_key, i)).selected_text(&selected_kind).width(200.0).show_ui(
                    ui,
                    |ui| {
                        for k in CHECK_KINDS {
                            if ui.selectable_label(selected_kind == *k, *k).clicked() {
                                selected_kind = k.to_string();
                            }
                        }
                    },
                );
                if selected_kind != kind && commit_pending_drafts(app) {
                    reload_profile(app, profile_id, profile);
                    let mut checks = current_checks(profile, doc_key);
                    if let Some(c) = checks.get_mut(i) {
                        *c = default_check(&selected_kind, c);
                    }
                    save_doc(app, profile_id, doc_key, profile, json!({"checks": checks}));
                    return;
                }
                if ui.button("Remove").clicked() && commit_pending_drafts(app) {
                    reload_profile(app, profile_id, profile);
                    let mut checks = current_checks(profile, doc_key);
                    if i < checks.len() {
                        checks.remove(i);
                    }
                    save_doc(app, profile_id, doc_key, profile, json!({"checks": checks}));
                }
            });

            let kind = ch["kind"].as_str().unwrap_or("");
            match kind {
                "wordLimit" => {
                    let mut limit = ch["limit"].as_u64().unwrap_or(0) as u32;
                    ui.horizontal(|ui| {
                        ui.label("Word limit:");
                        if ui.add(egui::DragValue::new(&mut limit).speed(100.0).range(1..=1_000_000)).changed() {
                            patch_check(app, profile_id, doc_key, profile, i, json!({"limit": limit}));
                        }
                    });
                    check_id_list_editor(
                        app,
                        ui,
                        profile_id,
                        doc_key,
                        profile,
                        i,
                        "excludeSections",
                        &section_ids,
                        "Sections excluded from the count:",
                    );
                }
                "minFontSize" => {
                    let mut pt = ch["pt"].as_f64().unwrap_or(13.0) as f32;
                    let font_key = check_draft_key(profile_id, doc_key, i);
                    let stored_font = ch["font"].as_str().unwrap_or("").to_string();
                    let mut font = app.law.check_font_drafts.get(&font_key).cloned().unwrap_or_else(|| stored_font.clone());
                    ui.horizontal(|ui| {
                        ui.label("Minimum (pt):");
                        let pt_changed = ui.add(egui::DragValue::new(&mut pt).speed(0.5).range(1.0..=4096.0)).changed();
                        let font_label = ui.label("Font:");
                        let font_resp = ui.text_edit_singleline(&mut font).labelled_by(font_label.id);
                        // Untrimmed draft until commit: trimming each keystroke drops the spaces.
                        if font_resp.changed() {
                            app.law.check_font_drafts.insert(font_key.clone(), font.clone());
                        }
                        let font_dirty = font.trim() != stored_font.trim();
                        let font_value = if font.trim().is_empty() { Value::Null } else { json!(font.trim()) };
                        if (pt_changed || (font_resp.lost_focus() && font_dirty))
                            && patch_check(app, profile_id, doc_key, profile, i, json!({"pt": pt, "font": font_value}))
                            && font_resp.lost_focus()
                            && font_dirty
                        {
                            app.law.check_font_drafts.remove(&font_key);
                        }
                    });
                }
                "minMargins" => {
                    let mut pt = ch["pt"].as_f64().unwrap_or(72.0) as f32;
                    ui.horizontal(|ui| {
                        ui.label("Minimum margin (pt):");
                        if ui.add(egui::DragValue::new(&mut pt).speed(1.0).range(1.0..=4096.0)).changed() {
                            patch_check(app, profile_id, doc_key, profile, i, json!({"pt": pt}));
                        }
                    });
                }
                "minLineSpacing" => {
                    let mut spacing = ch["spacing"].as_f64().unwrap_or(1.5) as f32;
                    ui.horizontal(|ui| {
                        ui.label("Minimum line spacing (multiple):");
                        if ui.add(egui::DragValue::new(&mut spacing).speed(0.05).range(0.5..=4.0)).changed() {
                            patch_check(app, profile_id, doc_key, profile, i, json!({"spacing": spacing}));
                        }
                    });
                    check_id_list_editor(app, ui, profile_id, doc_key, profile, i, "exceptStyles", &style_names, "Styles that may be single-spaced:");
                }
                "requiredSections" => {
                    check_id_list_editor(app, ui, profile_id, doc_key, profile, i, "ids", &section_ids, "Required section ids:");
                }
                "noHyperlinks" | "textSearchablePdf" => {
                    ui.label(RichText::new("No parameters for this check.").small().weak());
                }
                _ => {}
            }

            // Keep what is typed between frames: the field saves on focus loss, so a buffer
            // rebuilt from the stored value every frame would lose the edit before it can save.
            let draft_key = check_draft_key(profile_id, doc_key, i);
            let stored_source = ch["source"].as_str().unwrap_or("").to_string();
            let mut source = app.law.check_source_drafts.get(&draft_key).cloned().unwrap_or_else(|| stored_source.clone());
            ui.horizontal(|ui| {
                let label = ui.label("Source:");
                let resp = ui.text_edit_singleline(&mut source).labelled_by(label.id);
                if resp.changed() {
                    app.law.check_source_drafts.insert(draft_key.clone(), source.clone());
                }
                if resp.lost_focus()
                    && source.trim() != stored_source.trim()
                    && patch_check(app, profile_id, doc_key, profile, i, json!({"source": source.trim()}))
                {
                    app.law.check_source_drafts.remove(&draft_key);
                }
            });
            if let Some(eff) = ch["effective"].as_str().filter(|s| !s.is_empty()) {
                ui.label(RichText::new(format!("{} {}", "Effective:", eff)).small().weak());
            }
        });
    }
}

fn default_check(kind: &str, previous: &Value) -> Value {
    let source = previous.get("source").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Edited in LawCraft settings");
    let effective = previous.get("effective").and_then(Value::as_str).unwrap_or("");
    match kind {
        "wordLimit" => json!({"kind": kind, "limit": 10000, "excludeSections": [], "source": source, "effective": effective}),
        "minFontSize" => json!({"kind": kind, "pt": 13.0, "font": "Times New Roman", "source": source, "effective": effective}),
        "minMargins" => json!({"kind": kind, "pt": 72.0, "source": source, "effective": effective}),
        "minLineSpacing" => json!({"kind": kind, "spacing": 1.5, "exceptStyles": [], "source": source, "effective": effective}),
        "requiredSections" => json!({"kind": kind, "ids": [], "source": source, "effective": effective}),
        "noHyperlinks" | "textSearchablePdf" => json!({"kind": kind, "source": source, "effective": effective}),
        _ => json!({"kind": kind, "source": source, "effective": effective}),
    }
}

/// The checks the profile currently holds for one document type (the up-to-date value, not the
/// snapshot a frame started with).
fn current_checks(profile: &Value, doc_key: &str) -> Vec<Value> {
    profile["docs"][doc_key]["checks"].as_array().cloned().unwrap_or_default()
}

/// Merge one patch into a check. It reads the profile's *current* checks, so a commit later in the
/// same frame (the Source field losing focus after a parameter changed) cannot revert the earlier
/// edit by cloning the frame's starting snapshot.
fn patch_check(app: &mut Ctx<'_>, profile_id: &str, doc_key: &str, profile: &mut Value, index: usize, patch: Value) -> bool {
    let mut checks = current_checks(profile, doc_key);
    if let Some(c) = checks.get_mut(index).and_then(Value::as_object_mut)
        && let Some(p) = patch.as_object()
    {
        for (k, v) in p {
            c.insert(k.clone(), v.clone());
        }
    }
    save_doc(app, profile_id, doc_key, profile, json!({"checks": checks}))
}

fn check_id_list_editor(
    app: &mut Ctx<'_>,
    ui: &mut Ui,
    profile_id: &str,
    doc_key: &str,
    profile: &mut Value,
    index: usize,
    field: &str,
    choices: &[String],
    label: &str,
) {
    let selected: Vec<String> = current_checks(profile, doc_key)
        .get(index)
        .and_then(|c| c.get(field))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    ui.label(label);
    if choices.is_empty() {
        ui.label(RichText::new("No sections or styles to choose from yet.").small().weak());
        return;
    }
    let mut changed = false;
    let mut next = selected.clone();
    ui.horizontal_wrapped(|ui| {
        for id in choices {
            let mut on = next.iter().any(|s| s == id);
            if ui.checkbox(&mut on, id.as_str()).changed() {
                changed = true;
                if on {
                    if !next.iter().any(|s| s == id) {
                        next.push(id.clone());
                    }
                } else {
                    next.retain(|s| s != id);
                }
            }
        }
    });
    if changed {
        let ids: Vec<Value> = next.iter().map(|s| json!(s)).collect();
        patch_check(app, profile_id, doc_key, profile, index, json!({field: ids}));
    }
}

fn save_doc(app: &mut Ctx<'_>, profile_id: &str, doc_key: &str, profile: &mut Value, patch: Value) -> bool {
    let doc = profile["docs"].get(doc_key).cloned().unwrap_or(Value::Null);
    let mut merged = doc.as_object().cloned().unwrap_or_default();
    if let Some(p) = patch.as_object() {
        for (k, v) in p {
            merged.insert(k.clone(), v.clone());
        }
    }
    if let Some(docs) = profile.get_mut("docs").and_then(Value::as_object_mut) {
        docs.insert(doc_key.to_string(), Value::Object(merged));
    }
    match save_profile(app, profile_id, profile.clone()) {
        Ok(updated) => {
            *profile = updated;
            true
        }
        Err(()) => false,
    }
}

/// Validate the full profile, then persist it. Drafts stay until this succeeds.
fn save_profile(app: &mut Ctx<'_>, profile_id: &str, profile: Value) -> Result<Value, ()> {
    if profile.get("id").and_then(Value::as_str) != Some(profile_id) {
        let mut p = profile;
        if let Some(o) = p.as_object_mut() {
            o.insert("id".into(), json!(profile_id));
        }
        return save_profile(app, profile_id, p);
    }
    match app.run("law.profiles.validate", profile.clone()) {
        Ok(_) => {}
        Err(e) => {
            app.law.validation = Some(e.clone());
            app.status(e);
            return Err(());
        }
    }
    match app.run("law.profiles.update", json!({"id": profile_id, "docs": profile["docs"].clone()})) {
        Ok(updated) => {
            validate_profile(app, profile_id);
            Ok(updated)
        }
        Err(e) => {
            app.law.validation = Some(e.clone());
            app.status(e);
            Err(())
        }
    }
}

fn validate_profile(app: &mut Ctx<'_>, profile_id: &str) {
    match app.run("law.profiles.get", json!({"id": profile_id})) {
        Ok(p) => match app.run("law.profiles.validate", p) {
            Ok(_) => app.law.validation = None,
            Err(e) => app.law.validation = Some(e),
        },
        Err(e) => app.law.validation = Some(e),
    }
}

/// Finish a file pick for LawCraft import or export.
pub(crate) fn finish_pick(app: &mut Ctx<'_>, tag: &str, picked: Option<String>) -> Result<Value, String> {
    match tag {
        "import" => {
            let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
            // Import is a navigation: commit every pending page draft first. Any unsavable draft
            // cancels the import and keeps its error inline.
            if !commit_pending_drafts(app) {
                return Ok(json!({"cancelled": true, "reason": "pending edits could not be saved"}));
            }
            let r = app.run("law.profiles.import", json!({"path": path}));
            if let Ok(v) = &r {
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    clear_drafts_for(app.law, id);
                    app.law.selected = Some(id.to_string());
                    app.law.validation = None;
                    if let Ok(p) = app.run("law.profiles.get", json!({"id": id})) {
                        reconcile_doc_type(app.law, &p);
                    }
                }
                app.status(format!("Imported court profile {path}"));
            }
            r
        }
        "export" => {
            let Some(path) = picked else {
                app.law.export_id = None;
                return Ok(json!({"cancelled": true}));
            };
            let id = app.law.export_id.clone().ok_or_else(|| "no profile selected for export".to_string())?;
            app.law.export_id = None;
            // Export is a navigation too: commit every pending page draft first.
            if !commit_pending_drafts(app) {
                return Ok(json!({"cancelled": true, "reason": "pending edits could not be saved"}));
            }
            let r = app.run("law.profiles.export", json!({"id": id, "path": path}));
            if r.is_ok() {
                app.status(format!("Exported court profile {path}"));
            }
            r
        }
        _ => Err("not a LawCraft file pick".into()),
    }
}
