//! Court profile checks: evaluate a profile's `checks` against the open document.

use std::collections::HashMap;
use std::sync::Once;

use serde_json::{Value, json};
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{Document, Paragraph, PartKind, Path, Pos, StoryRef};
use wordcraft_engine::lint::{Finding, MAX_FINDINGS};
use wordcraft_engine::{Session, io};
use wordcraft_pdf::TextSearchability;
#[cfg(not(target_arch = "wasm32"))]
use wordcraft_pdf::{Verdict, extract_verdict_detail};

use crate::profile::{CheckSpec, DocSpec, Profile, SectionSpec};
use crate::store::LawError;

/// Custom document properties written by [`crate::apply::apply`].
pub const PROP_PROFILE: &str = "LAW_PROFILE_ID";
pub const PROP_DOC_TYPE: &str = "LAW_DOC_TYPE";

/// Section id for the certificate of compliance in a profile skeleton.
pub const CERT_SECTION_ID: &str = "certificateOfCompliance";

/// Run every check for `doc_type` in `profile` against `doc`. `for_export` runs export-only
/// checks such as `textSearchablePdf`, but `textSearchablePdf` itself only runs when the output
/// being exported is a PDF (`pdf_output`), since other formats cannot be verified as PDFs.
/// `export_opts` mirrors the export when present.
pub fn evaluate(
    doc: &Document,
    profile: &Profile,
    doc_type: &str,
    for_export: bool,
    pdf_output: bool,
    export_opts: Option<&io::ExportOptions>,
) -> Result<Vec<Finding>, LawError> {
    let Some(spec) = profile.docs.get(doc_type) else {
        return Err(LawError::Failed(format!(
            "profile `{}` has no document type `{doc_type}` (it has {})",
            profile.id,
            profile.doc_types().join(", ")
        )));
    };
    let sections = locate_sections(doc, spec);
    let mut out = Vec::new();
    for (i, check) in spec.checks.iter().enumerate() {
        let id = check_id(check, i);
        run_one(doc, spec, &sections, check, &id, for_export, pdf_output, export_opts, &mut out);
        if out.len() >= MAX_FINDINGS {
            break;
        }
    }
    Ok(out)
}

fn check_id(c: &CheckSpec, index: usize) -> String {
    if c.kind.is_empty() { format!("check{index}") } else { c.kind.clone() }
}

fn run_one(
    doc: &Document,
    spec: &DocSpec,
    sections: &[LocatedSection],
    check: &CheckSpec,
    id: &str,
    for_export: bool,
    pdf_output: bool,
    export_opts: Option<&io::ExportOptions>,
    out: &mut Vec<Finding>,
) {
    match check.kind.as_str() {
        "wordLimit" => word_limit(doc, spec, sections, check, id, out),
        "minFontSize" => min_font_size(doc, check, id, out),
        "minMargins" => min_margins(doc, spec, check, id, out),
        "minLineSpacing" => min_line_spacing(doc, spec, check, id, out),
        "noHyperlinks" => no_hyperlinks(doc, id, out),
        "textSearchablePdf" => {
            // Only a PDF export can be verified as a PDF; a `.docx` or `.png` copy is not one.
            if for_export && pdf_output {
                text_searchable_pdf(doc, id, export_opts, out);
            }
        }
        "requiredSections" => required_sections(sections, check, id, out),
        _ => {}
    }
}

struct LocatedSection {
    id: String,
    path: Path,
}

/// Body block index of a profile section's heading when it is present.
pub fn section_body_index(doc: &Document, spec: &DocSpec, section_id: &str) -> Option<usize> {
    locate_sections(doc, spec).iter().find(|s| s.id == section_id).and_then(|s| (s.path.depth() == 0).then_some(s.path.last()))
}

/// Section headings found in document order (by matching profile section names, or the
/// paragraph style for a section with `matchStyle`).
fn locate_sections(doc: &Document, spec: &DocSpec) -> Vec<LocatedSection> {
    let mut found = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        for sec in &spec.sections {
            if section_heading_matches(doc, para, sec) && !found.iter().any(|f: &LocatedSection| f.id == sec.id) {
                found.push(LocatedSection { id: sec.id.clone(), path: path.clone() });
            }
        }
    }
    found
}

fn section_heading_matches(doc: &Document, para: &Paragraph, sec: &SectionSpec) -> bool {
    match &sec.match_style {
        Some(style) => para_style_name(doc, para).is_some_and(|n| n.eq_ignore_ascii_case(style)),
        None => para.plain_text().trim().eq_ignore_ascii_case(sec.name.trim()),
    }
}

fn para_style_name(doc: &Document, para: &Paragraph) -> Option<String> {
    let id = para.props.style.as_deref()?;
    doc.styles.get(id).map(|s| s.name.clone()).or_else(|| doc.styles.find(id).map(|s| s.name.clone()))
}

fn required_sections(found: &[LocatedSection], check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    if check.ids.is_empty() {
        return;
    }
    let have: Vec<&str> = found.iter().map(|s| s.id.as_str()).collect();
    let mut hi = 0;
    for want in &check.ids {
        while hi < have.len() && have[hi] != want {
            hi += 1;
        }
        if hi >= have.len() {
            let source = &check.source;
            out.push(Finding::error(id, format!("Required section `{want}` is missing or out of order (from {source})")));
            return;
        }
        hi += 1;
    }
}

fn word_limit(doc: &Document, spec: &DocSpec, sections: &[LocatedSection], check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let Some(limit) = check.limit else { return };
    let words = word_count_with_check(doc, spec, sections, check);
    if words > limit {
        let source = &check.source;
        out.push(Finding::error(id, format!("Word count {words} exceeds the limit of {limit} ({source})")));
    }
}

/// Word count for the profile's `wordLimit` check (same exclusions the check uses).
pub fn word_count(doc: &Document, profile: &Profile, doc_type: &str) -> Result<u32, LawError> {
    let spec = profile.docs.get(doc_type).ok_or_else(|| {
        LawError::Failed(format!("profile `{}` has no document type `{doc_type}` (it has {})", profile.id, profile.doc_types().join(", ")))
    })?;
    let check = spec
        .checks
        .iter()
        .find(|c| c.kind == "wordLimit")
        .ok_or_else(|| LawError::Failed(format!("profile `{}` / `{doc_type}` has no wordLimit check", profile.id)))?;
    let sections = locate_sections(doc, spec);
    Ok(word_count_with_check(doc, spec, &sections, check))
}

/// The word-limit finding for `doc`, if any (`None` when at or under the limit).
pub fn word_limit_finding(doc: &Document, profile: &Profile, doc_type: &str) -> Result<Option<Finding>, LawError> {
    let spec = profile.docs.get(doc_type).ok_or_else(|| {
        LawError::Failed(format!("profile `{}` has no document type `{doc_type}` (it has {})", profile.id, profile.doc_types().join(", ")))
    })?;
    let check = spec.checks.iter().find(|c| c.kind == "wordLimit");
    let Some(check) = check else { return Ok(None) };
    let Some(limit) = check.limit else { return Ok(None) };
    let sections = locate_sections(doc, spec);
    let words = word_count_with_check(doc, spec, &sections, check);
    if words > limit {
        let source = &check.source;
        let id = if check.kind.is_empty() { "wordLimit".into() } else { check.kind.clone() };
        Ok(Some(Finding::error(&id, format!("Word count {words} exceeds the limit of {limit} ({source})"))))
    } else {
        Ok(None)
    }
}

fn word_count_with_check(doc: &Document, spec: &DocSpec, sections: &[LocatedSection], check: &CheckSpec) -> u32 {
    let exclude: Vec<&str> = check.exclude_sections.iter().map(String::as_str).collect();
    counted_words(doc, spec, sections, &exclude)
}

fn counted_words(doc: &Document, _spec: &DocSpec, sections: &[LocatedSection], exclude: &[&str]) -> u32 {
    let body_paths = doc.para_paths(StoryRef::Body);
    let section_by_para = section_ids_per_paragraph(&body_paths, sections);
    let mut total = 0u32;
    for (pi, path) in body_paths.iter().enumerate() {
        if section_by_para.get(pi).and_then(|s| s.as_deref()).is_some_and(|sec| exclude.contains(&sec)) {
            continue;
        }
        if let Some(p) = doc.para(StoryRef::Body, path) {
            total = total.saturating_add(para_words(p) as u32);
        }
    }
    // Which section owns each text box, precomputed in one pass so counting stays linear: a box's
    // section is the one that holds the paragraph anchoring it. A box under an excluded section (a
    // label on the cover, say) is part of that section and is left out with its other words; a box
    // counted with its section is counted. Notes have no text-box anchor and are counted, as FRAP's
    // word count includes footnotes and endnotes.
    let owner: BoxOwnership = box_ownership(doc, &body_paths, &section_by_para, exclude);
    for story in doc.counted_stories() {
        if story == StoryRef::Body {
            continue;
        }
        if let StoryRef::Part(id) = story
            && owner.excluded(id)
        {
            continue;
        }
        for path in doc.para_paths(story) {
            if let Some(p) = doc.para(story, &path) {
                total = total.saturating_add(para_words(p) as u32);
            }
        }
    }
    total
}

/// Which text-box story belongs to an excluded section. Built in a single body pass so no per-story
/// anchor search is needed; [`Document::text_box_anchor`] would rescan the whole document each time.
struct BoxOwnership {
    excluded: HashMap<u32, bool>,
}

impl BoxOwnership {
    fn excluded(&self, part: u32) -> bool {
        self.excluded.get(&part).copied().unwrap_or(false)
    }
}

fn box_ownership(doc: &Document, body_paths: &[Path], section_by_para: &[Option<String>], exclude: &[&str]) -> BoxOwnership {
    let mut excluded = HashMap::new();
    for (pi, path) in body_paths.iter().enumerate() {
        let skip = section_by_para.get(pi).and_then(|s| s.as_deref()).is_some_and(|sec| exclude.contains(&sec));
        let Some(p) = doc.para(StoryRef::Body, path) else { continue };
        for off in p.object_offsets() {
            let Some(obj) = p.object_at(off) else { continue };
            for part in obj.text_boxes() {
                // The first anchor wins, matching the body-first scan of `text_box_anchor`.
                excluded.entry(part).or_insert(skip);
            }
        }
    }
    BoxOwnership { excluded }
}

/// For each body paragraph index, the section id of the last located section heading at or before it.
fn section_ids_per_paragraph(body_paths: &[Path], sections: &[LocatedSection]) -> Vec<Option<String>> {
    let path_index: HashMap<&Path, usize> = body_paths.iter().enumerate().map(|(i, p)| (p, i)).collect();
    let mut boundaries: Vec<(usize, String)> = Vec::new();
    for loc in sections {
        if let Some(&i) = path_index.get(&loc.path) {
            boundaries.push((i, loc.id.clone()));
        }
    }
    boundaries.sort_by_key(|(i, _)| *i);
    let mut out = Vec::with_capacity(body_paths.len());
    let mut bi = 0usize;
    let mut current: Option<String> = None;
    for pi in 0..body_paths.len() {
        while bi < boundaries.len() && boundaries[bi].0 <= pi {
            current = Some(boundaries[bi].1.clone());
            bi += 1;
        }
        out.push(current.clone());
    }
    out
}

fn para_words(p: &Paragraph) -> usize {
    p.plain_text().split_whitespace().filter(|w| !w.is_empty()).count()
}

fn min_font_size(doc: &Document, check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let min_pt = check.pt;
    let want_font = check.font.as_deref().filter(|f| !f.is_empty());
    if min_pt.is_none() && want_font.is_none() {
        return;
    }
    for story in doc.counted_stories() {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            let para_style = para.props.style.as_deref();
            let mut off = 0usize;
            for run in &para.runs {
                let chr = doc.styles.resolve_char(para_style, &run.props);
                if let Some(min_pt) = min_pt
                    && chr.size + 0.01 < min_pt
                {
                    let size = chr.size;
                    let start = Pos { story, path: path.clone(), off };
                    let end = Pos { story, path: path.clone(), off: off.saturating_add(run.len) };
                    let source = &check.source;
                    out.push(Finding::at(id, "error", format!("Type size {size} pt is below the minimum {min_pt} pt ({source})"), start, end));
                    if out.len() >= MAX_FINDINGS {
                        return;
                    }
                }
                if let Some(want) = want_font {
                    let family = chr.font.trim();
                    if !family.is_empty() && !family.eq_ignore_ascii_case(want) {
                        let start = Pos { story, path: path.clone(), off };
                        let end = Pos { story, path: path.clone(), off: off.saturating_add(run.len) };
                        let source = &check.source;
                        out.push(Finding::at(
                            id,
                            "error",
                            format!("Typeface `{family}` does not match the required `{want}` ({source})"),
                            start,
                            end,
                        ));
                        if out.len() >= MAX_FINDINGS {
                            return;
                        }
                    }
                }
                off = off.saturating_add(run.len);
            }
        }
    }
}

fn min_margins(doc: &Document, spec: &DocSpec, check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let min_pt = check.pt.or_else(|| {
        let p = &spec.page;
        [p.top, p.bottom, p.left, p.right].into_iter().flatten().reduce(f32::min)
    });
    let Some(min_pt) = min_pt else { return };
    let multi = doc.sections().len() > 1;
    for (index, sect) in doc.sections() {
        for (name, val) in [("top", sect.margin_top), ("bottom", sect.margin_bottom), ("left", sect.margin_left), ("right", sect.margin_right)] {
            if val + 0.01 < min_pt {
                let source = &check.source;
                let where_at = if multi { format!(" (section {index})") } else { String::new() };
                out.push(Finding::error(id, format!("{name} margin {val} pt is below the minimum {min_pt} pt{where_at} ({source})")));
                if out.len() >= MAX_FINDINGS {
                    return;
                }
            }
        }
    }
}

/// The tightest a single-spaced exception (caption, heading, block quote, signature, certificate)
/// may be. Exceptions may be single-spaced, but not tighter than single spacing.
const SINGLE_SPACING: f32 = 1.0;

fn min_line_spacing(doc: &Document, spec: &DocSpec, check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let min_mult = check.spacing.or_else(|| {
        spec.styles
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("Normal"))
            .and_then(|s| s.para.as_ref())
            .and_then(|p| p.line_spacing)
            .and_then(|ls| effective_line_spacing_multiple(ls, 12.0 * 1.15))
    });
    let Some(min_mult) = min_mult else { return };
    let except: Vec<String> = if check.except_styles.is_empty() {
        spec.styles
            .iter()
            .filter(|s| {
                s.para
                    .as_ref()
                    .and_then(|p| p.line_spacing)
                    .and_then(|ls| effective_line_spacing_multiple(ls, 12.0 * 1.15))
                    .is_some_and(|m| m + 0.01 < min_mult)
            })
            .map(|s| s.name.clone())
            .collect()
    } else {
        check.except_styles.clone()
    };
    // The body and the text boxes it shows: body-flow text, wherever the document keeps it.
    // Footnotes and endnotes have their own spacing conventions and are not brief body text.
    for story in body_flow_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            // A single-spaced exception may be tighter than the body's minimum, but it still may
            // not go below single spacing: a Heading at 0.5 is reported, one at 1.0 is not.
            let is_exception = except.iter().any(|n| para_style_name(doc, para).is_some_and(|s| s.eq_ignore_ascii_case(n)));
            let floor = if is_exception { SINGLE_SPACING } else { min_mult };
            let rp = doc.styles.resolve_para(&para.props);
            let natural = para_natural_line_height(doc, para);
            let mult = effective_line_spacing_multiple(rp.line_spacing, natural);
            match mult {
                None => {
                    let start = Pos { story, path: path.clone(), off: 0 };
                    let end = Pos { story, path: path.clone(), off: para.text.len() };
                    let source = &check.source;
                    out.push(Finding::at(
                        id,
                        "error",
                        format!("Line spacing could not be verified against the minimum {floor} ({source})"),
                        start,
                        end,
                    ));
                }
                Some(m) if m + 0.01 < floor => {
                    let start = Pos { story, path: path.clone(), off: 0 };
                    let end = Pos { story, path: path.clone(), off: para.text.len() };
                    let source = &check.source;
                    out.push(Finding::at(id, "error", format!("Line spacing {m} is below the minimum {floor} ({source})"), start, end));
                }
                _ => {}
            }
            if out.len() >= MAX_FINDINGS {
                return;
            }
        }
    }
}

/// The stories that carry the document's own body text: the body plus every text box it shows.
/// Headers, footers, footnotes, endnotes and comments are not brief body text (and a story nothing
/// points at is not shown at all).
fn body_flow_stories(doc: &Document) -> Vec<StoryRef> {
    doc.counted_stories()
        .into_iter()
        .filter(|s| match s {
            StoryRef::Body => true,
            StoryRef::Part(id) => doc.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox),
        })
        .collect()
}

/// Approximate single-spaced line height (ascender + descender) from the paragraph's runs.
fn para_natural_line_height(doc: &Document, para: &Paragraph) -> f32 {
    let para_style = para.props.style.as_deref();
    let mut size = 12.0f32;
    for run in &para.runs {
        let chr = doc.styles.resolve_char(para_style, &run.props);
        if chr.size.is_finite() && chr.size > size {
            size = chr.size;
        }
    }
    size * 1.15
}

fn effective_line_spacing_multiple(ls: LineSpacing, natural: f32) -> Option<f32> {
    if !natural.is_finite() || natural <= 0.01 {
        return None;
    }
    match ls {
        LineSpacing::Multiple(m) if m.is_finite() => Some(m),
        LineSpacing::Exactly(v) if v.is_finite() => Some(v / natural),
        LineSpacing::AtLeast(v) if v.is_finite() => Some(natural.max(v) / natural),
        _ => None,
    }
}

/// Every story a reader of the filing sees: the body, notes, text boxes, and headers and
/// footers (which the word count leaves out but a hyperlink check must not). Comments are not
/// part of the filing.
fn printed_stories(doc: &Document) -> Vec<StoryRef> {
    let mut stories = doc.counted_stories();
    for (id, part) in &doc.parts {
        let story = StoryRef::Part(*id);
        if part.kind != PartKind::Comment && !stories.contains(&story) {
            stories.push(story);
        }
    }
    stories
}

fn no_hyperlinks(doc: &Document, id: &str, out: &mut Vec<Finding>) {
    for story in printed_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            let para_style = para.props.style.as_deref();
            let mut off = 0usize;
            for run in &para.runs {
                let chr = doc.styles.resolve_char(para_style, &run.props);
                if chr.link.is_some() {
                    let start = Pos { story, path: path.clone(), off };
                    let end = Pos { story, path: path.clone(), off: off.saturating_add(run.len) };
                    out.push(Finding::at(id, "error", "Active hyperlinks are not allowed in this filing", start, end));
                    if out.len() >= MAX_FINDINGS {
                        return;
                    }
                }
                off = off.saturating_add(run.len);
            }
        }
    }
}

fn text_searchable_pdf(doc: &Document, id: &str, export_opts: Option<&io::ExportOptions>, out: &mut Vec<Finding>) {
    let opts = export_opts.copied().unwrap_or_default();
    match io::save_bytes_opts("export.pdf", doc, &opts) {
        Ok(bytes) => {
            if let Some(finding) = searchability_finding(&bytes, id) {
                out.push(finding);
            }
        }
        Err(e) => out.push(Finding::error(id, format!("PDF export failed: {e}"))),
    }
}

/// The finding for exported PDF `bytes` against `checkId`, or `None` when the PDF is
/// text-searchable. Split out so tests can hand it bytes no exporter would produce (a font-only
/// PDF, say) and see the matching finding.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn searchability_finding(bytes: &[u8], check_id: &str) -> Option<Finding> {
    let detail = extract_verdict_detail(bytes);
    let verdict = match detail.verdict {
        Verdict::Searchable => TextSearchability::Searchable,
        Verdict::NotSearchable => TextSearchability::NotSearchable,
        Verdict::Uncertain => TextSearchability::Uncertain,
    };
    finding_for_verdict(verdict, check_id, &detail.unverifiable_pages)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn searchability_finding(_bytes: &[u8], check_id: &str) -> Option<Finding> {
    finding_for_verdict(TextSearchability::Unavailable, check_id, &[])
}

fn finding_for_verdict(verdict: TextSearchability, check_id: &str, unverifiable_pages: &[usize]) -> Option<Finding> {
    match verdict {
        TextSearchability::Searchable => None,
        TextSearchability::NotSearchable => Some(Finding::error(check_id, "PDF export is not text-searchable")),
        TextSearchability::Uncertain => Some(Finding::warning(check_id, unverifiable_scan_message(unverifiable_pages))),
        TextSearchability::Unavailable => {
            Some(Finding::warning(check_id, "Text-searchability could not be verified on this build (no PDF text extractor available)"))
        }
    }
}

fn unverifiable_scan_message(pages: &[usize]) -> String {
    match pages {
        [] => "A page has scanned content that could not be verified as searchable.".into(),
        [n] => format!("Page {n} has scanned content that could not be verified as searchable."),
        _ => {
            let list = pages.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
            format!("Pages {list} have scanned content that could not be verified as searchable.")
        }
    }
}

/// Findings as JSON for `law.checks.run`.
pub fn findings_json(findings: &[Finding]) -> Vec<Value> {
    findings.iter().map(|f| wordcraft_engine::lint::finding_json(f, wordcraft_engine::cmd::pos_json)).collect()
}

/// The lint source court checks store their findings under ([`Session::set_lint_findings`]).
/// Every LawCraft source starts with `law.`, so turning LawCraft off clears them together.
pub const FINDINGS_SOURCE: &str = "law.checks";

/// Store applied profile metadata on the document and register export checks on the session.
pub fn remember_applied(session: &mut Session, profile_id: &str, doc_type: &str) {
    session.doc.set_custom_prop(PROP_PROFILE, profile_id);
    session.doc.set_custom_prop(PROP_DOC_TYPE, doc_type);
    restore_export_registration(session);
}

/// Re-register export checks from document metadata (after open / `set_document`). The check
/// names no profile: it reads the one the document has applied when it runs, so undoing a
/// profile change (the document's metadata goes back with it) also changes which rules apply.
pub fn restore_export_registration(session: &mut Session) {
    if !session.module_enabled(crate::MODULE) {
        return;
    }
    if session.doc.custom_prop(PROP_PROFILE).is_some() {
        session.set_export_check_commands(vec![("law.certificate.refresh".into(), json!({})), ("law.checks.run".into(), json!({"forExport": true}))]);
    }
}

static HOOK_ONCE: Once = Once::new();

/// Register the document-loaded hook once (called from [`crate::install`]).
pub fn register_document_hook() {
    HOOK_ONCE.call_once(|| {
        wordcraft_engine::hooks::register_on_document_loaded(restore_export_registration);
        wordcraft_engine::hooks::register_on_update_fields(crate::certificate::on_update_fields);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An uncertain text layer (a page dominated by a picture) is reported as a warning, never as
    /// a silent pass; a definite verdict keeps its error/None.
    #[test]
    fn unverifiable_pdf_coverage_is_reported_as_a_warning() {
        let uncertain = finding_for_verdict(TextSearchability::Uncertain, "textSearchablePdf", &[2]).expect("a finding");
        assert_eq!(uncertain.check_id, "textSearchablePdf");
        assert_eq!(uncertain.severity, "warning");
        assert!(uncertain.message.contains("Page 2"));
        assert!(finding_for_verdict(TextSearchability::Searchable, "textSearchablePdf", &[]).is_none());
        assert_eq!(finding_for_verdict(TextSearchability::NotSearchable, "x", &[]).map(|f| f.severity), Some("error".into()));
        assert_eq!(finding_for_verdict(TextSearchability::Unavailable, "x", &[]).map(|f| f.severity), Some("warning".into()));
    }

    #[test]
    fn line_spacing_effective_multiple() {
        let natural = 13.0 * 1.15;
        assert!((effective_line_spacing_multiple(LineSpacing::Multiple(1.5), natural).unwrap() - 1.5).abs() < 0.001);
        assert!(effective_line_spacing_multiple(LineSpacing::Exactly(12.0), natural).unwrap() + 0.01 < 1.5);
        assert!(effective_line_spacing_multiple(LineSpacing::AtLeast(24.0), natural).unwrap() >= 1.5);
    }
}
