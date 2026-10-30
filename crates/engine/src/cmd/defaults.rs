//! Set As Default and the Normal template (#4237, ADR 0001): the commands that save formatting
//! for new documents (Design › Set as Default, the Font, Paragraph and Page Setup dialogs' Set As
//! Default, Modify Style › New documents based on this template), Reset, and
//! [`Session::normal_document`], the one function every new blank document comes from.
//!
//! Each Set As Default takes `"scope": "document" | "new"`. `"document"` changes the open
//! document only; `"new"` (the default) changes the open document too, in the same undo step,
//! then saves the change into the Normal template. The template write itself isn't undone, and
//! a session without a template store (headless, tests) refuses `"new"`.

use serde_json::{Value, json};
use wordcraft_doc::Document;
use wordcraft_doc::props::{CharProps, ParaProps};

use super::sel_result;
use crate::normal_template::{self as nt, Part};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("format.setDefault", "Set As Default (Font)", "Home › Font › Font", font_default)
            .params(r#"{"props": CharProps (as format.set; text language is left out), "scope"?: "document" (this document only) | "new" (this document and all new documents based on the Normal template; the default)} — the document's default character formatting and the Normal style's"#),
        CommandSpec::new("para.setDefault", "Set As Default (Paragraph)", "Home › Paragraph › Paragraph", para_default)
            .params(r#"{"props": ParaProps (as para.set), "scope"?: "document" (this document only) | "new" (this document and all new documents based on the Normal template; the default)} — the document's default paragraph formatting and the Normal style's"#),
        CommandSpec::new("layout.setDefault", "Set As Default (Page Setup)", "Layout › Page Setup", page_default)
            .params(r#"{"scope"?: "document" (this document only) | "new" (this document and all new documents based on the Normal template; the default)} — the caret's section's paper, orientation, margins, header and footer distance, vertical alignment and different first page, and the document's different odd and even pages and mirrored margins, for new documents ("document" changes nothing: Page Setup's OK already did)"#)
            .pure(),
        CommandSpec::new("styles.setDefault", "New Documents Based on This Template", "Home › Styles › Modify Style", style_default)
            .params(r#"{"style": string (id or name), "scope"?: "document" (this document only) | "new" (this document and all new documents based on the Normal template; the default)} — the style, with the styles it is based on, its next and linked styles and the lists it uses, saved for new documents ("document" changes nothing)"#)
            .pure(),
        CommandSpec::new("defaults.reset", "Reset Default Formatting", "File › Options › Advanced", reset)
            .params(r#"{} — deletes the Normal template: new documents use WordCraft's built-in formatting again. Can't be undone. Open documents don't change."#)
            .pure(),
    ]
}

/// Design › Set as Default: the document's style set, document defaults, theme fonts and
/// colours and paragraph spacing, for new documents.
pub(crate) fn design_default(s: &mut Session, v: &Value) -> CmdResult {
    if scope_is_new(v)? {
        let doc = Box::new(s.doc.clone());
        queue(s, TemplateEdit::Design(doc))?;
        s.status = "These settings will be used for new documents.".into();
    }
    done(s, v)
}

/// `scope`: true for all new documents (the default), false for this document only.
fn scope_is_new(v: &Value) -> Result<bool, CmdError> {
    match v.get("scope") {
        None | Some(Value::Null) => Ok(true),
        Some(Value::String(x)) if x == "new" => Ok(true),
        Some(Value::String(x)) if x == "document" => Ok(false),
        Some(_) => Err(CmdError::Params(r#"`scope` must be "document" or "new""#.into())),
    }
}

fn done(s: &Session, v: &Value) -> CmdResult {
    let mut r = sel_result(s)?;
    if let Some(o) = r.as_object_mut() {
        o.insert("scope".into(), json!(if scope_is_new(v)? { "new" } else { "document" }));
    }
    Ok(r)
}

/// A change waiting for the Normal template. A Set As Default queues it; [`Session::run`] writes
/// the queue once the command has succeeded and passed Restrict Editing's post-check, so a
/// refused or failed command never leaves its change in the template (#4237 review).
pub(crate) enum TemplateEdit {
    Char(CharProps),
    Para(ParaProps),
    /// The page setup on this document's last section and settings.
    Page(Box<Document>),
    /// The whole Design state of this document.
    Design(Box<Document>),
    /// The style with this id, from this document.
    Style(Box<Document>, String),
    /// Reset: back to the built-in formatting. Queued like the others, so a macro's edits keep
    /// their order around it and a failed macro leaves the saved template alone (#4237 review).
    Reset,
}

/// Queue `edit` for the template; refused when the session has none.
fn queue(s: &mut Session, edit: TemplateEdit) -> Result<(), CmdError> {
    if s.normal_store.is_none() {
        return Err(CmdError::Failed("this session has no Normal template (headless or without preferences): use \"scope\": \"document\"".into()));
    }
    s.pending_template.push(edit);
    Ok(())
}

/// Write the queued `edits` into the template, in one read-modify-write; ending on a Reset deletes
/// it instead.
pub(crate) fn commit_template(s: &mut Session, edits: Vec<TemplateEdit>) -> Result<(), CmdError> {
    if matches!(edits.last(), Some(TemplateEdit::Reset)) {
        let store =
            s.normal_store.clone().ok_or_else(|| CmdError::Failed("this session has no Normal template (headless or without preferences)".into()))?;
        return store.reset().map_err(|e| CmdError::Failed(format!("the Normal template couldn't be deleted: {e}")));
    }
    update_template(s, |t| {
        for e in edits {
            match e {
                TemplateEdit::Char(c) => {
                    apply_char(t, &c);
                    nt::mark(t, Part::Styles);
                }
                TemplateEdit::Para(pp) => {
                    apply_para(t, &pp);
                    nt::mark(t, Part::Styles);
                }
                TemplateEdit::Page(from) => {
                    nt::copy_page(t, &from);
                    nt::mark(t, Part::Page);
                }
                TemplateEdit::Design(doc) => {
                    crate::cmd::templates::apply_template_styles(t, &doc);
                    nt::copy_theme(&mut t.settings, &doc.settings);
                    nt::mark(t, Part::Styles);
                    nt::mark(t, Part::Theme);
                }
                TemplateEdit::Style(doc, id) => {
                    crate::cmd::templates::copy_styles(t, &doc, std::slice::from_ref(&id));
                    nt::mark(t, Part::Styles);
                }
                TemplateEdit::Reset => *t = nt::empty(),
            }
        }
    })
}

/// Re-read the template, change it with `f`, write it back. A missing or damaged template starts
/// from the built-in formatting (a damaged one is replaced).
fn update_template(s: &mut Session, f: impl FnOnce(&mut Document)) -> Result<(), CmdError> {
    let store = s.normal_store.clone().ok_or_else(|| {
        CmdError::Failed("this session has no Normal template (headless or without preferences): use \"scope\": \"document\"".into())
    })?;
    let mut t = match store.load() {
        Ok(Some(bytes)) => nt::parse(&bytes).unwrap_or_else(|e| {
            log::warn!("Normal template unreadable ({e}); replacing it");
            nt::empty()
        }),
        Ok(None) => nt::empty(),
        Err(e) => {
            log::warn!("Normal template unreadable ({e}); replacing it");
            nt::empty()
        }
    };
    f(&mut t);
    nt::drop_unused_lists(&mut t);
    let bytes = nt::to_bytes(&t).map_err(CmdError::Failed)?;
    store.save(&bytes).map_err(|e| CmdError::Failed(format!("the Normal template couldn't be saved: {e}")))?;
    Ok(())
}

/// The template, if there is one and it reads; a damaged one is reported in the status bar.
fn load_template(s: &mut Session) -> Option<Document> {
    let store = s.normal_store.clone()?;
    let read = match store.load() {
        Ok(None) => return None,
        Ok(Some(bytes)) => nt::parse(&bytes),
        Err(e) => Err(e),
    };
    match read {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("Normal template unreadable: {e}");
            s.status = "The Normal template couldn't be read, so this document uses WordCraft's built-in formatting.".into();
            None
        }
    }
}

impl Session {
    /// A new blank document: the built-in one on the region's paper (#838), with the Normal
    /// template's saved formatting on it, written in the saved proofing language (else the
    /// editing language). Every blank document comes from here: start-up (desktop and web), File
    /// › New › Blank, closing the last document, opening an empty file.
    pub fn normal_document(&mut self) -> Document {
        let mut d = self.blank_document();
        if let Some(t) = load_template(self) {
            nt::apply_to_blank(&mut d, &t);
        }
        if let Some(lang) = self.prefs.default_language.as_ref().or(self.editing_lang.as_ref()) {
            d.styles.default_chr.lang = Some(lang.clone());
        }
        d
    }

    /// Give a document read from plain text or Markdown the Normal template's formatting
    /// (they carry none of their own) and the saved proofing language.
    pub(crate) fn apply_normal_to_import(&mut self, d: &mut Document) {
        if let Some(t) = load_template(self) {
            nt::apply_to_import(d, &t);
        }
        if let Some(lang) = self.prefs.default_language.as_ref().or(self.editing_lang.as_ref()) {
            d.styles.default_chr.lang = Some(lang.clone());
        }
    }
}

/// The character formatting a default may carry: no style, link, revision or language.
fn char_patch(v: &Value) -> Result<CharProps, CmdError> {
    let mut c: CharProps = serde_json::from_value(v.get("props").cloned().unwrap_or(Value::Null)).map_err(|e| CmdError::Params(e.to_string()))?;
    c.style = None;
    c.link = None;
    c.ins = None;
    c.del = None;
    c.fmt_change = None;
    c.lang = None;
    c.lang_ea = None;
    c.lang_bidi = None;
    let size = |x: f32| if x.is_finite() { x.clamp(1.0, 1638.0) } else { wordcraft_doc::props::DEFAULT_FONT_SIZE };
    c.size = c.size.map(size);
    c.size_cs = c.size_cs.map(size).or(c.size);
    c.bold_cs = c.bold_cs.or(c.bold);
    c.italic_cs = c.italic_cs.or(c.italic);
    Ok(c)
}

fn apply_char(d: &mut Document, c: &CharProps) {
    d.styles.default_chr.overlay(c);
    if let Some(n) = d.styles.get_mut("Normal") {
        n.chr.overlay(c);
    }
}

/// Font › Set As Default.
fn font_default(s: &mut Session, v: &Value) -> CmdResult {
    let all = scope_is_new(v)?;
    let c = char_patch(v)?;
    apply_char(&mut s.doc, &c);
    crate::cmd::para::touch_all(s);
    if all {
        queue(s, TemplateEdit::Char(c))?;
        s.status = "New documents will use this font.".into();
    }
    done(s, v)
}

/// The paragraph formatting a default may carry: no style, list, frame or revision (formatting
/// or numbering change).
fn para_patch(v: &Value) -> Result<ParaProps, CmdError> {
    let mut pp: ParaProps = serde_json::from_value(v.get("props").cloned().unwrap_or(Value::Null)).map_err(|e| CmdError::Params(e.to_string()))?;
    pp.style = None;
    pp.numbering = None;
    pp.frame = None;
    pp.fmt_change = None;
    pp.num_change = None;
    pp.drop_cap = None;
    Ok(pp)
}

fn apply_para(d: &mut Document, pp: &ParaProps) {
    d.styles.default_para.overlay(pp);
    if let Some(n) = d.styles.get_mut("Normal") {
        n.para.overlay(pp);
    }
}

/// Paragraph › Set As Default.
fn para_default(s: &mut Session, v: &Value) -> CmdResult {
    let all = scope_is_new(v)?;
    let pp = para_patch(v)?;
    apply_para(&mut s.doc, &pp);
    crate::cmd::para::touch_all(s);
    if all {
        queue(s, TemplateEdit::Para(pp))?;
        s.status = "New documents will use this paragraph formatting.".into();
    }
    done(s, v)
}

/// Page Setup › Set As Default: the caret's section's page setup.
fn page_default(s: &mut Session, v: &Value) -> CmdResult {
    if scope_is_new(v)? {
        // The caret's section from any story: a header, footnote or text box finds the section
        // it belongs to, as the Page Setup dialog does.
        let mut from = Document::new();
        nt::page_setup_onto(&mut from.last_section, &crate::cmd::page::sect(s));
        from.settings.even_odd_headers = s.doc.settings.even_odd_headers;
        from.settings.mirror_margins = s.doc.settings.mirror_margins;
        queue(s, TemplateEdit::Page(Box::new(from)))?;
        s.status = "New documents will use this page setup.".into();
    }
    done(s, v)
}

/// Modify Style › New documents based on this template.
fn style_default(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "style")?;
    let st = s
        .doc
        .styles
        .get(name)
        .or_else(|| s.doc.styles.styles.iter().find(|st| st.name.eq_ignore_ascii_case(name)))
        .ok_or_else(|| CmdError::Params(format!("no style `{name}`")))?;
    let id = st.id.clone();
    if scope_is_new(v)? {
        let doc = Box::new(s.doc.clone());
        queue(s, TemplateEdit::Style(doc, id))?;
        s.status = "New documents will use this style.".into();
    }
    done(s, v)
}

/// File › Options › Advanced › Reset: delete the template, once the command (or the macro it is
/// in) has succeeded.
fn reset(s: &mut Session, _: &Value) -> CmdResult {
    if s.normal_store.is_none() {
        return Err(CmdError::Failed("this session has no Normal template (headless or without preferences)".into()));
    }
    s.pending_template.push(TemplateEdit::Reset);
    s.status = "New documents will use WordCraft's built-in formatting.".into();
    Ok(json!({"reset": true}))
}
