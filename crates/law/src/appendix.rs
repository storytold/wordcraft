//! Appendix documents: separate filings numbered A1, A2… (or federal addenda as Add. 1…).

use serde_json::{Value, json};
use wordcraft_engine::{CmdError, Session, p};

use crate::apply;
use crate::checks::{PROP_DOC_TYPE, PROP_PROFILE};
use crate::cmd::store;
use crate::profile::Profile;
use crate::store::LawError;

const LOC: &str = "Law › Appendix";

/// Document type key for a profile's separate exhibit volume (`appendix` or `addendum`).
pub fn exhibit_doc_type(profile: &Profile) -> Result<&str, LawError> {
    if profile.docs.contains_key("appendix") {
        Ok("appendix")
    } else if profile.docs.contains_key("addendum") {
        Ok("addendum")
    } else {
        Err(LawError::Failed(format!(
            "profile `{}` has no appendix or addendum document type (it has {})",
            profile.id,
            profile.doc_types().join(", ")
        )))
    }
}

/// Apply appendix/addendum styles and numbering to the open document.
pub fn apply_appendix(s: &mut Session, v: &Value) -> Result<Value, LawError> {
    let profile_id = profile_id(s, v)?;
    let Some(p) = store().get(&profile_id).cloned() else {
        return Err(LawError::Failed(format!("no court profile `{profile_id}`")));
    };
    let doc_type = match p::str(v, "doc") {
        Some(d) => {
            if d != "appendix" && d != "addendum" {
                return Err(LawError::Failed(format!("`doc` must be `appendix` or `addendum` (got `{d}`)")));
            }
            if !p.docs.contains_key(d) {
                return Err(LawError::Failed(format!("profile `{}` has no `{d}` document type", profile_id)));
            }
            d.to_string()
        }
        None => exhibit_doc_type(&p)?.to_string(),
    };
    apply::apply(s, &p, &doc_type).map_err(|e| LawError::Failed(e.to_string()))
}

/// New appendix document from the brief on screen: blank document, profile appendix styles, skeleton and TOC.
pub fn create_appendix(s: &mut Session, v: &Value) -> Result<Value, LawError> {
    let profile_id = profile_id(s, v)?;
    let Some(p) = store().get(&profile_id).cloned() else {
        return Err(LawError::Failed(format!("no court profile `{profile_id}`")));
    };
    let exhibit_type = exhibit_doc_type(&p)?.to_string();
    if s.doc.custom_prop(PROP_DOC_TYPE) == Some(exhibit_type.as_str()) {
        return Err(LawError::Failed("the open document is already an appendix".into()));
    }
    run(s, "file.new", json!({}))?;
    let applied = apply::apply(s, &p, &exhibit_type)?;
    write_skeleton(s, &p, &exhibit_type)?;
    Ok(json!({
        "profile": profile_id,
        "doc": exhibit_type,
        "applied": applied["applied"],
        "styles": applied["styles"],
    }))
}

fn profile_id(s: &Session, v: &Value) -> Result<String, LawError> {
    if let Some(id) = p::str(v, "id") {
        return Ok(id.to_string());
    }
    s.doc
        .custom_prop(PROP_PROFILE)
        .map(|s| s.to_string())
        .ok_or_else(|| LawError::Failed("`id` (profile id) is required when no court profile is applied".into()))
}

fn write_skeleton(s: &mut Session, profile: &Profile, doc_type: &str) -> Result<(), LawError> {
    let spec = profile.docs.get(doc_type).ok_or_else(|| LawError::Failed(format!("profile has no `{doc_type}`")))?;
    let body_style = if spec.styles.iter().any(|st| st.name.eq_ignore_ascii_case("Body")) {
        "Body"
    } else {
        spec.styles.first().map(|st| st.name.as_str()).filter(|n| !n.is_empty()).unwrap_or("Normal")
    };
    for sec in &spec.sections {
        run(s, "text.insert", json!({"text": sec.name}))?;
        if let Some(st) = &sec.style {
            run(s, "para.style", json!({"style": st}))?;
        }
        run(s, "text.insert", json!({"text": "\r"}))?;
        if sec.id == "contents" {
            let mut levels = json!({});
            for st in &spec.styles {
                if st.name.contains("Heading") {
                    levels[st.name.clone()] = json!(1);
                }
            }
            run(
                s,
                "references.toc",
                json!({
                    "title": sec.name,
                    "levels": 1,
                    "styleLevels": levels,
                    "hyperlinks": false,
                }),
            )?;
        } else {
            run(s, "styles.apply", json!({"style": body_style}))?;
            run(s, "text.insert", json!({"text": "\r"}))?;
        }
    }
    // The TOC is inserted before the sections below it, so its entries are only complete once the
    // whole skeleton exists: refresh it here, the way Word builds a table as the document is made.
    // A profile without a `contents` section has no TOC and this is a no-op.
    run(s, "references.updateToc", json!({}))?;
    Ok(())
}

fn run(s: &mut Session, id: &str, params: Value) -> Result<Value, LawError> {
    s.run(id, &params).map_err(|e| match e {
        CmdError::Unknown(_) => LawError::Failed(format!("{id}: WordCraft no longer has this command")),
        other => LawError::Failed(format!("{id}: {other}")),
    })
}

pub fn specs() -> Vec<wordcraft_engine::CommandSpec> {
    vec![
        wordcraft_engine::CommandSpec::new("law.appendix.create", "Create Appendix", LOC, |s, v| {
            create_appendix(s, v).map_err(|e| CmdError::Failed(e.to_string()))
        })
        // It opens the appendix in place of the brief: the interface asks to save the brief first.
        .replaces_document()
        .params(
            r#"{"id"?: string (court profile; default: the profile applied to the open brief)} → {"profile", "doc", "applied", "styles"}; opens a new appendix document"#,
        ),
        wordcraft_engine::CommandSpec::new("law.appendix.apply", "Apply Appendix Formatting", LOC, |s, v| {
            apply_appendix(s, v).map_err(|e| CmdError::Failed(e.to_string()))
        })
        .params(
            r#"{"id"?: string, "doc"?: string (appendix or addendum; default: the profile's exhibit document type)} → as law.profiles.apply"#,
        ),
    ]
}
