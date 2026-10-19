//! Certificate of compliance: profile-driven text with a live word-count field.

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, Path, StoryRef};
use wordcraft_engine::{CmdError, Session};

use crate::checks::{CERT_SECTION_ID, PROP_DOC_TYPE, PROP_PROFILE, word_count, word_limit_finding};
use crate::cmd::store;
use crate::profile::{DocSpec, Profile};
use crate::store::LawError;

/// Field instruction for the live word count (updated on field rebuild and before export).
pub const FIELD_WORDCOUNT: &str = "LAWWORDCOUNT";

const SOFTWARE: &str = "WordCraft";

/// Missouri appellate brief certificate (Rule 84.06(c)); placeholders filled at insert time.
const MO_TEMPLATE: &str = "I certify that this brief contains {wordCount} words, excluding the portions of the brief exempted by Rule 84.04, does not exceed the word limit of {wordLimit} words, and complies with the type-volume requirements of Supreme Court Rule 55.03. The brief is set in {font} {fontSize}-point type and was prepared with {software}.";

/// Federal appellate brief certificate (FRAP 32(g)).
const FEDERAL_TEMPLATE: &str = "I certify that this brief complies with the type-volume limitation of the Federal Rules of Appellate Procedure and contains {wordCount} words, excluding the portions of the document required by Rule 32(f) to be excluded. The word limit for this document is {wordLimit} words. The brief is set in {font} {fontSize}-point type and was prepared with {software}.";

struct TemplateValues {
    word_count: u32,
    word_limit: u32,
    rule: String,
    font: String,
    font_size: String,
}

/// Insert or refresh the certificate of compliance for `doc_type` in `profile`.
pub fn insert_or_refresh(session: &mut Session, profile: &Profile, doc_type: &str) -> Result<Value, LawError> {
    ensure_matching_applied_metadata(session, &profile.id, doc_type)?;
    let spec = profile.docs.get(doc_type).ok_or_else(|| missing_doc(profile, doc_type))?;
    let sec = spec
        .sections
        .iter()
        .find(|s| s.id == CERT_SECTION_ID)
        .ok_or_else(|| LawError::Failed(format!("profile `{}` / `{doc_type}` has no `{CERT_SECTION_ID}` section", profile.id)))?;
    let template = resolve_template(profile, spec)?;
    let values = template_values(&session.doc, profile, doc_type, spec, &template)?;
    write_certificate(session, spec, sec, &template, &values)?;
    // Persist the profile/document binding the certificate was built for, even when the command
    // was given explicit `id`/`doc` on a document that had no profile applied. Field rebuilds and
    // the pre-export refresh read this document metadata, so the count can never go stale; this
    // also (re-)registers the pre-export refresh hook. Nothing is mutated before this point.
    crate::checks::remember_applied(session, &profile.id, doc_type);
    refresh_wordcount_fields_for(session, Some(&profile.id), Some(doc_type)).map_err(|e| LawError::Failed(e.to_string()))?;
    let mut out = json!({
        "profile": profile.id,
        "doc": doc_type,
        "wordCount": values.word_count,
        "wordLimit": values.word_limit,
        "section": CERT_SECTION_ID,
    });
    if let Some(f) = word_limit_finding(&session.doc, profile, doc_type)? {
        out["findings"] = json!(crate::checks::findings_json(&[f]));
    }
    Ok(out)
}

/// Refresh every [`FIELD_WORDCOUNT`] result in the document (export hook and `law.certificate.refresh`).
pub fn refresh_wordcount_fields(session: &mut Session) -> Result<(), CmdError> {
    refresh_wordcount_fields_for(session, None, None)
}

fn refresh_wordcount_fields_for(session: &mut Session, profile_id: Option<&str>, doc_type: Option<&str>) -> Result<(), CmdError> {
    if !has_lawwordcount_fields(&session.doc) {
        return Ok(());
    }
    let profile_id = profile_id
        .or_else(|| session.doc.custom_prop(PROP_PROFILE))
        .ok_or_else(|| CmdError::Params("no court profile is applied to this document (apply a profile first)".into()))?;
    let doc_type = doc_type
        .or_else(|| session.doc.custom_prop(PROP_DOC_TYPE))
        .ok_or_else(|| CmdError::Params("no court document type is set on this document".into()))?;
    let profile = store().get(profile_id).cloned().ok_or_else(|| CmdError::Failed(format!("no court profile `{profile_id}`")))?;
    let count = word_count(&session.doc, &profile, doc_type).map_err(|e| CmdError::Failed(e.to_string()))?;
    let want = field_name_instr(FIELD_WORDCOUNT);
    let mut touched = false;
    for story in session.doc.counted_stories() {
        for path in session.doc.para_paths(story) {
            let Ok(para) = session.doc.para_mut(story, &path) else { continue };
            let mut changed = false;
            for obj in &mut para.objects {
                if let InlineObject::Field { instr, result, locked: false, .. } = obj
                    && field_name_instr(instr.as_str()) == want
                {
                    let text = count.to_string();
                    if *result != text {
                        *result = text;
                        changed = true;
                    }
                }
            }
            if changed {
                para.touch();
                touched = true;
            }
        }
    }
    if touched {
        session.touch();
    }
    Ok(())
}

/// Hook for [`wordcraft_engine::hooks::register_on_update_fields`].
pub fn on_update_fields(session: &mut Session) -> Result<(), CmdError> {
    // The hook stays registered for the process; with LawCraft off it does nothing.
    if !session.module_enabled(crate::MODULE) {
        return Ok(());
    }
    if session.doc.custom_prop(PROP_PROFILE).is_some() && session.doc.custom_prop(PROP_DOC_TYPE).is_some() {
        refresh_wordcount_fields(session)?;
    }
    Ok(())
}

fn missing_doc(profile: &Profile, doc_type: &str) -> LawError {
    LawError::Failed(format!("profile `{}` has no document type `{doc_type}` (it has {})", profile.id, profile.doc_types().join(", ")))
}

fn ensure_matching_applied_metadata(session: &Session, profile_id: &str, doc_type: &str) -> Result<(), LawError> {
    if let Some(applied) = session.doc.custom_prop(PROP_PROFILE)
        && applied != profile_id
    {
        return Err(LawError::Failed(format!("document has profile `{applied}` applied but the command requested `{profile_id}`")));
    }
    if let Some(applied) = session.doc.custom_prop(PROP_DOC_TYPE)
        && applied != doc_type
    {
        return Err(LawError::Failed(format!("document has document type `{applied}` applied but the command requested `{doc_type}`")));
    }
    Ok(())
}

fn has_lawwordcount_fields(doc: &Document) -> bool {
    let want = field_name_instr(FIELD_WORDCOUNT);
    for story in doc.counted_stories() {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            for obj in &para.objects {
                if let InlineObject::Field { instr, .. } = obj
                    && field_name_instr(instr.as_str()) == want
                {
                    return true;
                }
            }
        }
    }
    false
}

fn top_level_block_index(path: &Path) -> usize {
    path.0.first().copied().unwrap_or(0) as usize
}

fn top_level_block_count(doc: &Document) -> usize {
    doc.story(StoryRef::Body).map(|b| b.len()).unwrap_or(0)
}

fn resolve_template(profile: &Profile, spec: &DocSpec) -> Result<String, LawError> {
    if let Some(c) = &spec.certificate_of_compliance {
        return Ok(c.template.clone());
    }
    match profile.id.as_str() {
        "missouri-appellate-brief" => Ok(MO_TEMPLATE.to_string()),
        "federal-appellate-brief" => Ok(FEDERAL_TEMPLATE.to_string()),
        _ => Err(LawError::Failed(format!(
            "profile `{}` has no certificateOfCompliance template for this document type (add one to the profile file)",
            profile.id
        ))),
    }
}

fn template_values(doc: &Document, profile: &Profile, doc_type: &str, spec: &DocSpec, template: &str) -> Result<TemplateValues, LawError> {
    let word_count = word_count(doc, profile, doc_type)?;
    let wl = spec
        .checks
        .iter()
        .find(|c| c.kind == "wordLimit")
        .ok_or_else(|| LawError::Failed(format!("profile `{}` / `{doc_type}` has no wordLimit check", profile.id)))?;
    let word_limit = wl.limit.ok_or_else(|| LawError::Failed(format!("profile `{}` wordLimit has no limit", profile.id)))?;
    let rule = cert_rule(spec, wl);
    let (font, font_size) = typeface(spec);
    let _ = template;
    Ok(TemplateValues { word_count, word_limit, rule, font, font_size })
}

fn cert_rule(spec: &DocSpec, wl: &crate::profile::CheckSpec) -> String {
    spec.certificate_of_compliance
        .as_ref()
        .and_then(|c| (!c.source.is_empty()).then(|| c.source.clone()))
        .or_else(|| (!wl.source.is_empty()).then(|| wl.source.clone()))
        .unwrap_or_else(|| spec.source.clone())
}

fn typeface(spec: &DocSpec) -> (String, String) {
    if let Some(c) = spec.checks.iter().find(|x| x.kind == "minFontSize") {
        let font = c.font.clone().or_else(|| {
            spec.styles.iter().find(|s| s.name.eq_ignore_ascii_case("Normal")).and_then(|s| s.chr.as_ref()).and_then(|chr| chr.font.clone())
        });
        if let (Some(font), Some(pt)) = (font, c.pt) {
            let size = if pt.fract().abs() < 0.01 { format!("{:.0}", pt) } else { format!("{pt}") };
            return (font, size);
        }
    }
    if let Some(st) = spec.styles.iter().find(|s| s.name.eq_ignore_ascii_case("Normal"))
        && let Some(chr) = &st.chr
    {
        let font = chr.font.clone().unwrap_or_else(|| "Times New Roman".into());
        let pt = chr.size.unwrap_or(12.0);
        let size = if pt.fract().abs() < 0.01 { format!("{:.0}", pt) } else { format!("{pt}") };
        return (font, size);
    }
    ("Times New Roman".into(), "12".into())
}

fn write_certificate(
    session: &mut Session,
    spec: &DocSpec,
    sec: &crate::profile::SectionSpec,
    template: &str,
    values: &TemplateValues,
) -> Result<(), LawError> {
    let heading = sec.name.trim();
    let body_paths = session.doc.para_paths(StoryRef::Body);
    let heading_i = find_heading_index(&session.doc, &body_paths, heading);
    let cert_style = sec.style.as_deref().or(Some("Certificate"));
    let style_id = cert_style.and_then(|name| session.doc.styles.find(name).map(|s| s.id.clone()));

    let Some(hi) = heading_i else {
        append_certificate(session, heading, style_id.as_deref(), template, values)?;
        return Ok(());
    };
    // A section heading inside a table cell has no top-level place to put the certificate: taking
    // only its first path component would insert after the whole table and, on a re-run, leave the
    // previous certificate outside the searched section. Refuse before changing anything.
    ensure_top_level_heading(&body_paths, hi, heading)?;
    let next_para_i = next_section_index(&session.doc, spec, &body_paths, heading_i);
    if next_para_i < body_paths.len() {
        let next_heading = session.doc.para(StoryRef::Body, &body_paths[next_para_i]).map(|p| p.plain_text().trim().to_string());
        ensure_top_level_heading(&body_paths, next_para_i, next_heading.as_deref().unwrap_or(""))?;
    }
    let heading_block = top_level_block_index(&body_paths[hi]);
    let next_block =
        if next_para_i < body_paths.len() { top_level_block_index(&body_paths[next_para_i]) } else { top_level_block_count(&session.doc) };
    // One replacement of the certificate's paragraphs: a tracked change while Track Changes is on,
    // so Reject All brings the old certificate back.
    let blocks: Vec<Block> = paragraphs_from_template(template, values, style_id.as_deref()).into_iter().map(Block::Para).collect();
    wordcraft_engine::cmd::replace_blocks(session, StoryRef::Body, heading_block + 1, next_block, blocks)
        .map_err(|e| LawError::Failed(e.to_string()))?;
    session.touch();
    Ok(())
}

/// A section heading a certificate can be placed under must sit at the top level of the body.
fn ensure_top_level_heading(paths: &[Path], index: usize, heading: &str) -> Result<(), LawError> {
    if paths.get(index).is_some_and(|p| p.depth() == 0) {
        return Ok(());
    }
    Err(LawError::Failed(format!(
        "the `{heading}` section heading is inside a table; a certificate can only be placed under a top-level section heading"
    )))
}

fn append_certificate(session: &mut Session, heading: &str, style_id: Option<&str>, template: &str, values: &TemplateValues) -> Result<(), LawError> {
    let at = top_level_block_count(&session.doc);
    let mut head = Paragraph::with_text(heading, CharProps::default());
    if let Some(id) = style_id {
        head.props.style = Some(id.to_string());
    }
    // Added as a tracked insertion while Track Changes is on.
    let blocks: Vec<Block> = std::iter::once(head).chain(paragraphs_from_template(template, values, style_id)).map(Block::Para).collect();
    wordcraft_engine::cmd::replace_blocks(session, StoryRef::Body, at, at, blocks).map_err(|e| LawError::Failed(e.to_string()))?;
    session.touch();
    Ok(())
}

fn find_heading_index(doc: &Document, paths: &[Path], heading: &str) -> Option<usize> {
    paths.iter().position(|path| doc.para(StoryRef::Body, path).is_some_and(|p| p.plain_text().trim().eq_ignore_ascii_case(heading)))
}

fn next_section_index(doc: &Document, spec: &DocSpec, paths: &[Path], heading_i: Option<usize>) -> usize {
    let Some(hi) = heading_i else { return paths.len() };
    // Bound the replacement at the next recognized profile section, whatever its position in the
    // profile. A document whose sections are out of order (an earlier section such as `Argument`
    // sitting after the certificate) must not have that section deleted along with the old
    // certificate: any profile section heading ends the certificate's block range.
    for (offset, path) in paths.iter().enumerate().skip(hi + 1) {
        let Some(para) = doc.para(StoryRef::Body, path) else { continue };
        let text = para.plain_text();
        let text = text.trim();
        if spec.sections.iter().any(|s| text.eq_ignore_ascii_case(s.name.trim())) {
            return offset;
        }
    }
    paths.len()
}

fn paragraphs_from_template(template: &str, values: &TemplateValues, style_id: Option<&str>) -> Vec<Paragraph> {
    template.split('\n').map(|line| build_paragraph(substitute_line(line, values), values.word_count, style_id)).collect()
}

fn field_name_instr(instr: &str) -> String {
    instr.split_whitespace().next().unwrap_or("").trim_start_matches('=').to_ascii_uppercase()
}

fn substitute_line(line: &str, values: &TemplateValues) -> String {
    let mut s = line.to_string();
    for (from, to) in [
        ("{wordLimit}", values.word_limit.to_string()),
        ("{rule}", values.rule.clone()),
        ("{font}", values.font.clone()),
        ("{fontSize}", values.font_size.clone()),
        ("{software}", SOFTWARE.to_string()),
    ] {
        s = s.replace(from, &to);
    }
    s
}

fn build_paragraph(text: String, word_count: u32, style_id: Option<&str>) -> Paragraph {
    let mut para = Paragraph::new();
    if let Some(id) = style_id {
        para.props.style = Some(id.to_string());
    }
    let props = CharProps::default();
    let marker = "{wordCount}";
    let mut off = 0usize;
    let mut rest = text.as_str();
    while let Some(i) = rest.find(marker) {
        let before = &rest[..i];
        if !before.is_empty() {
            let _ = para.insert_text(off, before, &props);
            off = off.saturating_add(before.len());
        }
        let result = word_count.to_string();
        let _ = para.insert_object(
            off,
            InlineObject::Field { instr: FIELD_WORDCOUNT.into(), result, locked: false, dirty: false, code: false, form: String::new() },
            &props,
        );
        off = off.saturating_add(OBJ.len_utf8());
        rest = &rest[i + marker.len()..];
    }
    if !rest.is_empty() {
        let _ = para.insert_text(off, rest, &props);
    }
    para
}
