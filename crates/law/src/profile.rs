//! Court profiles: the data model, its JSON schema and validation.
//!
//! A profile is a data file, one per court, with three parts per document type (original
//! petition, later pleading, motion, brief, appendix…): the document **sections** in order, the
//! **styles** (page settings, named WordCraft styles, line numbers) and the **checks**. Styles
//! are ordinary WordCraft style objects ([`wordcraft_doc::props::CharProps`] /
//! [`ParaProps`]) and page settings, so `law.profiles.apply` applies them through the existing
//! `styles.*` and `layout.*` commands — a profile never formats a document itself.
//!
//! Everything here is hostile input: the JSON comes from files, commands, MCP and the control
//! channel. Parsing never panics, every number is range-checked, and an unknown schema version
//! is an error rather than a guess.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::props::{CharProps, LineSpacing, ParaProps};

/// The schema version this crate reads and writes. A file that says anything else is rejected.
pub const SCHEMA: u32 = 1;

// Caps: a profile is data a person (or their agent) wrote by hand, so anything past these is a
// mistake or an attack. Each one keeps parsing, layout and the style sheet bounded.
const MAX_DOCS: usize = 64;
const MAX_STYLES: usize = 200;
const MAX_SECTIONS: usize = 400;
const MAX_CHECKS: usize = 100;
const MAX_NAME: usize = 200;
const MAX_ID: usize = 64;
const MAX_LISTED: usize = 64;
const MAX_CERT_TEMPLATE: usize = 16_384;
/// Points (ECMA-376's `w:spacing` and friends are 16-bit twips: 1584 pt is the largest).
const MAX_PT: f32 = 1584.0;
/// Word's largest type size, in points.
const MAX_TYPE: f32 = 4096.0;

/// A court profile: one court's formatting rules per document type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Profile {
    /// Schema version; must be [`SCHEMA`].
    pub schema: u32,
    /// Stable id, also the file name (`<id>.json`): letters, digits, `-`, `_` and `.`.
    pub id: String,
    /// Display name (`Missouri appellate brief`).
    pub name: String,
    /// Court or jurisdiction (`Missouri`, `Federal (FRAP)`).
    pub jurisdiction: String,
    /// A person approved this profile's rules. Created and updated through commands it is
    /// always false; only `law.profiles.approve` (a person's action) sets it.
    #[serde(default)]
    pub verified: bool,
    /// Citation pack this profile's citations follow, by pack id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_pack: Option<String>,
    /// The three parts per document type, keyed by document type.
    #[serde(default)]
    pub docs: BTreeMap<String, DocSpec>,
}

/// One document type's three parts, and the rules they came from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct DocSpec {
    /// Display name for the document type (`Appellant's initial brief`).
    pub label: String,
    /// Page settings: paper size and margins.
    pub page: PageSpec,
    /// Line numbers, as `layout.lineNumbers` takes them; `None` leaves the document's alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<LineNumbersSpec>,
    /// Page numbering in the header or footer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_numbers: Option<PageNumbersSpec>,
    /// The named styles the document uses (Body, Caption, Block Quote…), including the ones it
    /// changes (Normal).
    pub styles: Vec<StyleSpec>,
    /// The document skeleton in order.
    pub sections: Vec<SectionSpec>,
    /// Rules that are not styles (word limits, minimum type size…).
    pub checks: Vec<CheckSpec>,
    /// Table of authorities: print `passim` for an authority cited on five or more pages (TOA
    /// `\p`); `law.toa.build` uses it when the call doesn't say (default true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toa_passim: Option<bool>,
    /// Certificate of compliance text for [`crate::certificate`] (`{wordCount}`, `{wordLimit}`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_of_compliance: Option<ComplianceCertificateSpec>,
    /// The rule these formatting rules come from (`Mo. Sup. Ct. R. 84.03`).
    pub source: String,
    /// When the rule took effect (`2025-01-01`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effective: String,
}

/// Page settings: paper size and margins, in points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct PageSpec {
    /// A paper name WordCraft knows (`Letter`, `Legal`, `A4`…), instead of `width`/`height`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// Page width in points (instead of `size`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Page height in points (instead of `size`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<f32>,
    /// Gutter width (the bound edge of a printed document), points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gutter: Option<f32>,
}

/// Line numbers, exactly as `layout.lineNumbers` takes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LineNumbersSpec {
    /// Off: no line numbers (`value: "none"`). Default: on.
    #[serde(default = "on")]
    pub enabled: bool,
    /// `continuous`, `restartPage` or `restartSection`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// First number of the numbering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    /// Count every n-th line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count_by: Option<u32>,
    /// Distance between the numbers and the text, points (0 = automatic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
}

/// Page numbering in the header or footer, applied with `insert.pageNumber` and
/// `layout.pageNumberFormat`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct PageNumbersSpec {
    /// `top` or `bottom`.
    pub position: String,
    /// `left`, `center` or `right`.
    pub align: String,
    /// "Page X of Y" rather than just the number.
    pub x_of_y: bool,
    /// Numbering format (`decimal`, `lowerRoman`, `upperLetter`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Number the first page of the document starts from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    /// The cover page is numbered too; `false` turns on a different first page (its own header
    /// and footer, which the document's author fills in).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_numbered: Option<bool>,
    /// Literal text before the page number (`A` for appendix pages numbered A1, A2…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

impl Default for PageNumbersSpec {
    fn default() -> Self {
        PageNumbersSpec {
            position: "bottom".into(),
            align: "center".into(),
            x_of_y: false,
            format: None,
            start: None,
            cover_numbered: None,
            prefix: None,
        }
    }
}

/// A named style the document uses: standard WordCraft style objects.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct StyleSpec {
    /// Style name as the document shows it (`Block Quote`).
    pub name: String,
    /// The style it builds on; another style of this profile, or `Normal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    /// The style the paragraph after one of these takes (Enter at the end).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chr: Option<CharProps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub para: Option<ParaProps>,
}

/// Template for the certificate of compliance section (placeholders filled by
/// `law.certificate.compliance`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ComplianceCertificateSpec {
    /// Paragraphs joined with `\n`; `{wordCount}` becomes a live field.
    pub template: String,
    /// The rule this certificate comes from (`Mo. Sup. Ct. R. 84.06(c)`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// One part of the document skeleton, in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct SectionSpec {
    /// Stable id (`jurisdictionalStatement`).
    pub id: String,
    /// Heading the skeleton writes (`Statement of Facts`).
    pub name: String,
    /// The style these paragraphs use; one of this document type's styles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// How the checks find this section: its first paragraph in this style (one of this document
    /// type's styles). Without it, a paragraph whose whole text is the section's `name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_style: Option<String>,
    /// Caption fields this section holds (the caption section only): `court`, `county`,
    /// `partyNames`, `partyRoles`, `partyAddresses`, `caseNumber`, `attorneyName`, `barNumber`,
    /// `attorneyAddress`, `telephone`, `email`, `signature`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// The rule that requires this section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A rule that is not a style: checked on demand and before export (the checks engine is a
/// separate piece; this records the rule).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct CheckSpec {
    /// `wordLimit`, `minFontSize`, `minMargins`, `minLineSpacing`, `noHyperlinks`,
    /// `textSearchablePdf` or `requiredSections`.
    pub kind: String,
    /// `wordLimit`: the most words the document may have.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// `minFontSize` / `minMargins`: the smallest type size or margin in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pt: Option<f32>,
    /// `minFontSize`: required typeface family (`Times New Roman`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// `minLineSpacing`: the smallest body line spacing as a multiple (for example 1.5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f32>,
    /// `wordLimit`: section ids left out of the count (cover, tables, certificates…).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude_sections: Vec<String>,
    /// `minLineSpacing`: named styles that may be single-spaced (caption, headings…).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub except_styles: Vec<String>,
    /// `requiredSections`: the section ids that must be present.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// The rule this check comes from (`Mo. Sup. Ct. R. 84.04(b)(1)`).
    pub source: String,
    /// When the rule took effect (`2025-01-01`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effective: String,
}

fn on() -> bool {
    true
}

/// The fields of a [`Profile`] that may be changed one at a time (`law.profiles.update`).
/// Fields left out, or given as `null`, stay as they are.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Patch {
    /// The schema version, when the caller sends one: it has to be [`SCHEMA`].
    pub schema: Option<u32>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub jurisdiction: Option<String>,
    pub citation_pack: Option<String>,
    pub docs: Option<BTreeMap<String, DocSpec>>,
}

impl Patch {
    /// Read a patch out of a JSON value (command parameters, an MCP call). An agent may send a
    /// whole profile: the fields it doesn't name keep what the stored one has.
    pub fn from_value(value: Value) -> Result<Patch, Vec<String>> {
        if let Some(v) = value.get("schema").and_then(Value::as_u64)
            && v != u64::from(SCHEMA)
        {
            return Err(vec![format!("unsupported schema version {v} (this WordCraft reads version {SCHEMA})")]);
        }
        serde_json::from_value(value).map_err(|e| vec![format!("not a court profile: {e}")])
    }

    /// The stored profile with the patch's fields replaced.
    pub fn apply(&self, base: &Profile) -> Profile {
        Profile {
            schema: SCHEMA,
            id: self.id.clone().unwrap_or_else(|| base.id.clone()),
            name: self.name.clone().unwrap_or_else(|| base.name.clone()),
            jurisdiction: self.jurisdiction.clone().unwrap_or_else(|| base.jurisdiction.clone()),
            verified: false,
            citation_pack: self.citation_pack.clone().or_else(|| base.citation_pack.clone()),
            docs: self.docs.clone().unwrap_or_else(|| base.docs.clone()),
        }
    }
}

impl Profile {
    /// Parse and validate a profile from JSON. Returns every problem found, so an agent fixing
    /// a file sees all of them at once.
    pub fn from_json(text: &str) -> Result<Profile, Vec<String>> {
        let value: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => return Err(vec![format!("not valid JSON: {e}")]),
        };
        Self::from_value(value)
    }

    /// Parse and validate a profile from a JSON value (command parameters, an MCP call).
    pub fn from_value(value: Value) -> Result<Profile, Vec<String>> {
        // The schema version is read first, so a file from a newer WordCraft says that rather
        // than complaining about the fields it doesn't have.
        if let Some(v) = value.get("schema").and_then(Value::as_u64)
            && v != u64::from(SCHEMA)
        {
            return Err(vec![format!("unsupported schema version {v} (this WordCraft reads version {SCHEMA})")]);
        }
        let p: Profile = match serde_json::from_value(value) {
            Ok(p) => p,
            Err(e) => return Err(vec![format!("not a court profile: {e}")]),
        };
        let bad = problems(&p);
        if bad.is_empty() { Ok(p) } else { Err(bad) }
    }

    /// The profile as JSON.
    pub fn to_json(&self) -> Result<String, Vec<String>> {
        serde_json::to_string_pretty(self).map_err(|e| vec![format!("can't write the profile: {e}")])
    }

    /// The profile as a JSON value (for command results).
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// The document types this profile covers, in alphabetical order.
    pub fn doc_types(&self) -> Vec<&str> {
        self.docs.keys().map(String::as_str).collect()
    }

    /// A short description for `law.profiles.list`.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "jurisdiction": self.jurisdiction,
            "verified": self.verified,
            "citationPack": self.citation_pack,
            "docs": self.docs.keys().collect::<Vec<_>>(),
        })
    }
}

/// Every problem with `p`, most fundamental first; empty when the profile is valid.
pub fn problems(p: &Profile) -> Vec<String> {
    let mut out = Vec::new();
    if p.schema != SCHEMA {
        out.push(format!("unsupported schema version {} (this WordCraft reads version {SCHEMA})", p.schema));
        return out;
    }
    check_id(&mut out, "`id`", &p.id);
    check_text(&mut out, "`name`", &p.name);
    check_text(&mut out, "`jurisdiction`", &p.jurisdiction);
    if let Some(pack) = &p.citation_pack {
        check_id(&mut out, "`citationPack`", pack);
    }
    if p.docs.is_empty() {
        out.push("`docs` is empty: a profile needs at least one document type".into());
    }
    if p.docs.len() > MAX_DOCS {
        out.push(format!("`docs` has {} document types (at most {MAX_DOCS})", p.docs.len()));
        return out;
    }
    for (doc_type, spec) in &p.docs {
        let at = format!("docs.{doc_type}");
        check_id(&mut out, &format!("{at}: document type"), doc_type);
        check_doc(&mut out, &at, spec);
    }
    out
}

fn check_doc(out: &mut Vec<String>, at: &str, d: &DocSpec) {
    if !d.label.is_empty() {
        check_text(out, &format!("{at}.label"), &d.label);
    }
    check_text(out, &format!("{at}.source"), &d.source);
    check_date(out, &format!("{at}.effective"), &d.effective);
    check_page(out, &format!("{at}.page"), &d.page);
    if let Some(ln) = &d.line_numbers {
        check_line_numbers(out, &format!("{at}.lineNumbers"), ln);
    }
    if let Some(pn) = &d.page_numbers {
        check_page_numbers(out, &format!("{at}.pageNumbers"), pn);
    }
    if d.styles.len() > MAX_STYLES {
        out.push(format!("{at}.styles has {} styles (at most {MAX_STYLES})", d.styles.len()));
        return;
    }
    // Names are checked against every style of this document type: `next` may name one that
    // comes later, and a section may use one defined after it.
    let names: Vec<&str> = d.styles.iter().map(|st| st.name.as_str()).collect();
    let known = |n: &str| n.eq_ignore_ascii_case("Normal") || names.iter().any(|have| have.eq_ignore_ascii_case(n));
    for (i, st) in d.styles.iter().enumerate() {
        let at = format!("{at}.styles[{i}]");
        check_text(out, &format!("{at}.name"), &st.name);
        if names[..i].iter().any(|n| n.eq_ignore_ascii_case(&st.name)) {
            out.push(format!("{at}.name `{}` is listed twice", st.name));
        }
        if let Some(base) = &st.based_on
            && !known(base)
        {
            out.push(format!("{at}.basedOn `{base}` is not a style of this profile (or `Normal`)"));
        }
        if let Some(next) = &st.next
            && !known(next)
        {
            out.push(format!("{at}.next `{next}` is not a style of this profile"));
        }
        check_chr(out, &format!("{at}.chr"), st.chr.as_ref());
        check_para(out, &format!("{at}.para"), st.para.as_ref());
    }
    // A style that (through its based-on chain) ends up based on itself would never resolve.
    for st in &d.styles {
        let mut at = st.name.as_str();
        let mut steps = 0;
        while let Some(base) = d.styles.iter().find(|s| s.name.eq_ignore_ascii_case(at)).and_then(|s| s.based_on.as_deref()) {
            steps += 1;
            if steps > names.len() {
                out.push(format!("{at}.styles: `{}` is based on itself (a circular basedOn)", st.name));
                break;
            }
            at = base;
        }
    }
    if d.sections.len() > MAX_SECTIONS {
        out.push(format!("{at}.sections has {} sections (at most {MAX_SECTIONS})", d.sections.len()));
        return;
    }
    let mut ids: Vec<&str> = Vec::new();
    for (i, sec) in d.sections.iter().enumerate() {
        let at = format!("{at}.sections[{i}]");
        check_id(out, &format!("{at}.id"), &sec.id);
        check_text(out, &format!("{at}.name"), &sec.name);
        if ids.contains(&sec.id.as_str()) {
            out.push(format!("{at}.id `{}` is listed twice", sec.id));
        }
        ids.push(&sec.id);
        for (key, style) in [("style", &sec.style), ("matchStyle", &sec.match_style)] {
            if let Some(style) = style
                && !names.iter().any(|n| n.eq_ignore_ascii_case(style))
            {
                out.push(format!("{at}.{key} `{style}` is not a style of this document type"));
            }
        }
        for f in &sec.fields {
            check_id(out, &format!("{at}.fields"), f);
        }
    }
    if d.checks.len() > MAX_CHECKS {
        out.push(format!("{at}.checks has {} checks (at most {MAX_CHECKS})", d.checks.len()));
        return;
    }
    for (i, c) in d.checks.iter().enumerate() {
        let at = format!("{at}.checks[{i}]");
        check_check(out, &at, c, &ids);
    }
    if let Some(cert) = &d.certificate_of_compliance {
        let cat = format!("{at}.certificateOfCompliance");
        if cert.template.is_empty() {
            out.push(format!("{cat}.template must not be empty"));
        } else if cert.template.len() > MAX_CERT_TEMPLATE {
            out.push(format!("{cat}.template is too long (at most {MAX_CERT_TEMPLATE} characters)"));
        }
        if !cert.source.is_empty() {
            check_text(out, &format!("{cat}.source"), &cert.source);
        }
    }
}

fn check_page(out: &mut Vec<String>, at: &str, p: &PageSpec) {
    match (&p.size, (&p.width, &p.height)) {
        (Some(_), (Some(_), _)) | (Some(_), (_, Some(_))) => out.push(format!("{at}: give `size` or `width`/`height`, not both")),
        // Nothing given: the page size is left as the document has it.
        (None, (None, None)) => {}
        (None, (w, h)) => {
            if w.is_none() || h.is_none() {
                out.push(format!("{at}: `width` and `height` go together"));
            }
            if let Some(v) = w {
                check_points(out, &format!("{at}.width"), *v, 72.0, MAX_PT);
            }
            if let Some(v) = h {
                check_points(out, &format!("{at}.height"), *v, 72.0, MAX_PT);
            }
        }
        (Some(name), (None, None)) => {
            if name.trim().is_empty() {
                out.push(format!("{at}.size is empty"));
            } else if !wordcraft_geom::PAPER_SIZES.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
                // A name WordCraft doesn't know would only fail later in `layout.size`, so it is
                // reported here, where an agent can still fix the profile.
                let known: Vec<&str> = wordcraft_geom::PAPER_SIZES.iter().map(|(n, _, _)| *n).collect();
                out.push(format!("{at}.size `{name}` is not a paper WordCraft knows ({})", known.join(", ")));
            }
        }
    }
    for (what, v) in [("top", p.top), ("bottom", p.bottom), ("left", p.left), ("right", p.right), ("gutter", p.gutter)] {
        if let Some(v) = v {
            check_points(out, &format!("{at}.{what}"), v, 0.0, MAX_PT);
        }
    }
}

fn check_line_numbers(out: &mut Vec<String>, at: &str, ln: &LineNumbersSpec) {
    if let Some(v) = &ln.value
        && !matches!(v.as_str(), "continuous" | "restartPage" | "restartSection")
    {
        out.push(format!("{at}.value `{v}` is not continuous, restartPage or restartSection"));
    }
    if let Some(n) = ln.start {
        check_count(out, &format!("{at}.start"), n, 1, 32_767);
    }
    if let Some(n) = ln.count_by {
        check_count(out, &format!("{at}.countBy"), n, 1, 100);
    }
    if let Some(d) = ln.distance {
        check_points(out, &format!("{at}.distance"), d, 0.0, MAX_PT);
    }
}

fn check_page_numbers(out: &mut Vec<String>, at: &str, pn: &PageNumbersSpec) {
    if !matches!(pn.position.as_str(), "top" | "bottom") {
        out.push(format!("{at}.position `{}` is not top or bottom", pn.position));
    }
    if !matches!(pn.align.as_str(), "left" | "center" | "right") {
        out.push(format!("{at}.align `{}` is not left, center or right", pn.align));
    }
    if let Some(f) = &pn.format {
        const FORMATS: [&str; 5] = ["decimal", "lowerRoman", "upperRoman", "lowerLetter", "upperLetter"];
        if !FORMATS.contains(&f.as_str()) {
            out.push(format!("{at}.format `{f}` is not one of {}", FORMATS.join(", ")));
        }
    }
    if let Some(n) = pn.start {
        check_count(out, &format!("{at}.start"), n, 0, 100_000);
    }
    if let Some(prefix) = &pn.prefix {
        check_text(out, &format!("{at}.prefix"), prefix);
        if prefix.chars().count() > 16 {
            out.push(format!("{at}.prefix is {} characters (at most 16)", prefix.chars().count()));
        }
    }
}

fn check_check(out: &mut Vec<String>, at: &str, c: &CheckSpec, section_ids: &[&str]) {
    check_text(out, &format!("{at}.source"), &c.source);
    check_date(out, &format!("{at}.effective"), &c.effective);
    match c.kind.as_str() {
        "wordLimit" => {
            let Some(limit) = c.limit else {
                out.push(format!("{at}: a wordLimit check needs `limit`"));
                return;
            };
            check_count(out, &format!("{at}.limit"), limit, 1, 10_000_000);
            if c.exclude_sections.len() > MAX_LISTED {
                out.push(format!("{at}.excludeSections has {} sections (at most {MAX_LISTED})", c.exclude_sections.len()));
            }
            for id in &c.exclude_sections {
                check_id(out, &format!("{at}.excludeSections"), id);
                if !id.is_empty() && !section_ids.contains(&id.as_str()) {
                    out.push(format!("{at}.excludeSections `{id}` is not a section of this document type"));
                }
            }
        }
        "minFontSize" => {
            let Some(pt) = c.pt else {
                out.push(format!("{at}: a minFontSize check needs `pt`"));
                return;
            };
            check_points(out, &format!("{at}.pt"), pt, 1.0, MAX_TYPE);
        }
        "noHyperlinks" | "textSearchablePdf" => {}
        "minMargins" => {
            if let Some(pt) = c.pt {
                check_points(out, &format!("{at}.pt"), pt, 0.0, MAX_PT);
            }
        }
        "minLineSpacing" => {
            let Some(sp) = c.spacing else {
                out.push(format!("{at}: a minLineSpacing check needs `spacing`"));
                return;
            };
            check_points(out, &format!("{at}.spacing"), sp, 0.05, 132.0);
            if c.except_styles.len() > MAX_LISTED {
                out.push(format!("{at}.exceptStyles has {} styles (at most {MAX_LISTED})", c.except_styles.len()));
            }
            for name in &c.except_styles {
                check_text(out, &format!("{at}.exceptStyles"), name);
            }
        }
        "requiredSections" => {
            if c.ids.is_empty() {
                out.push(format!("{at}: a requiredSections check needs `ids`"));
            }
            if c.ids.len() > MAX_LISTED {
                out.push(format!("{at}.ids has {} sections (at most {MAX_LISTED})", c.ids.len()));
            }
            for id in &c.ids {
                check_id(out, &format!("{at}.ids"), id);
                if !id.is_empty() && !section_ids.contains(&id.as_str()) {
                    out.push(format!("{at}.ids `{id}` is not a section of this document type"));
                }
            }
        }
        other => {
            out.push(format!(
                "{at}.kind `{other}` is not a check this WordCraft knows (wordLimit, minFontSize, minMargins, minLineSpacing, noHyperlinks, textSearchablePdf, requiredSections)"
            ));
        }
    }
}

fn check_chr(out: &mut Vec<String>, at: &str, c: Option<&CharProps>) {
    let Some(c) = c else { return };
    if let Some(size) = c.size {
        check_points(out, &format!("{at}.size"), size, 1.0, MAX_TYPE);
    }
    if let Some(font) = &c.font {
        check_text(out, &format!("{at}.font"), font);
    }
}

fn check_para(out: &mut Vec<String>, at: &str, p: Option<&ParaProps>) {
    let Some(p) = p else { return };
    for (what, v) in [
        ("indentLeft", p.indent_left),
        ("indentRight", p.indent_right),
        ("indentFirst", p.indent_first),
        ("spaceBefore", p.space_before),
        ("spaceAfter", p.space_after),
    ] {
        if let Some(v) = v {
            check_points(out, &format!("{at}.{what}"), v, -MAX_PT, MAX_PT);
        }
    }
    if let Some(ls) = p.line_spacing {
        let (what, v) = match ls {
            LineSpacing::Multiple(v) => ("multiple", v),
            LineSpacing::AtLeast(v) => ("atLeast", v),
            LineSpacing::Exactly(v) => ("exactly", v),
        };
        let max = if what == "multiple" { 132.0 } else { MAX_PT };
        check_points(out, &format!("{at}.lineSpacing.{what}"), v, 0.05, max);
    }
}

/// An id (profile id, document type, section id): short, and safe as a file name.
fn check_id(out: &mut Vec<String>, what: &str, id: &str) {
    if id.is_empty() {
        out.push(format!("{what} is empty"));
        return;
    }
    if id.chars().count() > MAX_ID {
        out.push(format!("{what} is {} characters (at most {MAX_ID})", id.chars().count()));
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        out.push(format!("{what} `{id}` may hold only letters, digits, `-`, `_` and `.`"));
    }
}

fn check_text(out: &mut Vec<String>, what: &str, text: &str) {
    if text.trim().is_empty() {
        out.push(format!("{what} is empty"));
    }
    if text.chars().count() > MAX_NAME {
        out.push(format!("{what} is {} characters (at most {MAX_NAME})", text.chars().count()));
    }
}

/// A date as `YYYY-MM-DD`, when the rule took effect. Optional everywhere.
fn check_date(out: &mut Vec<String>, what: &str, date: &str) {
    if date.is_empty() {
        return;
    }
    let ok = date.len() == 10
        && date.as_bytes().get(4) == Some(&b'-')
        && date.as_bytes().get(7) == Some(&b'-')
        && date[..4].chars().all(|c| c.is_ascii_digit())
        && date[5..7].chars().all(|c| c.is_ascii_digit())
        && date[8..].chars().all(|c| c.is_ascii_digit());
    if !ok {
        out.push(format!("{what} `{date}` is not a date (YYYY-MM-DD)"));
    }
}

fn check_count(out: &mut Vec<String>, what: &str, n: u32, min: u32, max: u32) {
    if n < min || n > max {
        out.push(format!("{what} {n} is outside {min}–{max}"));
    }
}

fn check_points(out: &mut Vec<String>, what: &str, v: f32, min: f32, max: f32) {
    if !v.is_finite() {
        out.push(format!("{what} is not a finite number"));
    } else if v < min || v > max {
        out.push(format!("{what} {v} is outside {min}–{max}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> Profile {
        serde_json::from_value(json!({
            "schema": 1,
            "id": "test-court",
            "name": "Test court",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "page": {"size": "Letter", "top": 72},
                    "styles": [{"name": "Normal", "chr": {"size": 13}}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "source": "Rule 1",
                }
            }
        }))
        .expect("a minimal profile parses")
    }

    #[test]
    fn minimal_profile_is_valid() {
        assert!(problems(&minimal()).is_empty(), "{:?}", problems(&minimal()));
    }

    #[test]
    fn unknown_schema_version_is_rejected_with_a_clear_error() {
        let text = r#"{"schema": 2, "id": "x", "name": "X", "jurisdiction": "Y", "docs": {}}"#;
        let err = Profile::from_json(text).expect_err("version 2 is not ours");
        assert_eq!(err.len(), 1);
        assert!(err[0].contains("unsupported schema version 2"), "{}", err[0]);
        assert!(err[0].contains("version 1"), "{}", err[0]);
    }

    #[test]
    fn references_between_parts_are_checked() {
        let mut p = minimal();
        let d = p.docs.get_mut("brief").expect("brief");
        d.styles.push(StyleSpec { name: "Body".into(), based_on: Some("Nowhere".into()), ..Default::default() });
        d.sections.push(SectionSpec { id: "body".to_string(), name: "Body".to_string(), style: Some("Missing".into()), ..Default::default() });
        d.checks.push(CheckSpec {
            kind: "requiredSections".into(),
            ids: vec!["cover".into(), "gone".into()],
            source: "Rule 2".into(),
            ..Default::default()
        });
        let errs = problems(&p).join("; ");
        assert!(errs.contains("basedOn `Nowhere` is not a style"), "{errs}");
        assert!(errs.contains("style `Missing` is not a style"), "{errs}");
        assert!(errs.contains("ids `gone` is not a section"), "{errs}");
    }

    #[test]
    fn unknown_paper_sizes_are_reported_at_validation_time() {
        let mut p = minimal();
        p.docs.get_mut("brief").expect("brief").page.size = Some("Letterr".into());
        let errs = problems(&p).join("; ");
        assert!(errs.contains("Letterr"), "{errs}");
        assert!(errs.contains("not a paper WordCraft knows"), "{errs}");
        // A name WordCraft does know, whatever its case, is accepted.
        let mut p = minimal();
        p.docs.get_mut("brief").expect("brief").page.size = Some("legal".into());
        assert!(problems(&p).is_empty(), "{:?}", problems(&p));
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let text = r#"{"schema": 1, "id": "no/slash", "name": "", "jurisdiction": "",
            "docs": {"brief": {"styles": [{"name": "Body", "chr": {"size": 0}}], "checks": [{"kind": "nope"}], "sections": []}}}"#;
        let errs = Profile::from_json(text).expect_err("invalid");
        let all = errs.join("\n");
        for want in ["may hold only letters, digits", "`name` is empty", "`jurisdiction` is empty", "size 0 is outside", "kind `nope`"] {
            assert!(all.contains(want), "{want} not in:\n{all}");
        }
    }

    #[test]
    fn hostile_json_never_panics() {
        let bad = [
            "",
            "[]",
            "null",
            "{}",
            "{\"schema\":1}",
            "42",
            "\"x\"",
            r#"{"schema":1,"id":"a","name":"a","jurisdiction":"a","docs":{"brief":{"page":{"size":1e400,"top":NaN}}}}"#,
            r#"{"schema":1,"id":"a","name":"a","jurisdiction":"a","docs":{"brief":{"styles":[{"name":"N","para":{"lineSpacing":{"rule":"multiple","value":1e400}}}]}}}"#,
            r#"{"schema":1,"id":"a","name":"a","jurisdiction":"a","docs":{"../etc":"{}"}}"#,
            r#"{"schema":1,"id":"a","name":"a","jurisdiction":"a","docs":{"brief":{"sections":[{"id":"","name":""}]}}}"#,
        ];
        for text in bad {
            // Whatever it says, it is either a profile or a list of problems — never a panic.
            let _ = Profile::from_json(text);
            let _ = problems(&minimal());
        }
    }

    #[test]
    fn serialises_back_to_the_same_profile() {
        let p = minimal();
        let text = p.to_json().expect("write");
        let back = Profile::from_json(&text).expect("read back");
        assert_eq!(p, back);
    }

    #[test]
    fn every_section_is_validated_even_in_a_large_profile() {
        let mut p = minimal();
        let d = p.docs.get_mut("brief").expect("brief");
        d.sections.clear();
        for i in 0..65 {
            d.sections.push(SectionSpec { id: format!("sec{i}"), name: format!("Section {i}"), ..Default::default() });
        }
        d.sections.push(SectionSpec { id: "bad/id".into(), name: "Bad".into(), ..Default::default() });
        let errs = problems(&p).join("; ");
        assert!(errs.contains("bad/id"), "the invalid section at the end is caught: {errs}");
    }
}
