//! Strict export (#6005): the report of what a PDF or PNG would leave out silently (characters no
//! face can draw, pictures that are missing or can't be decoded), and the refusal built from it.
//!
//! The check runs on the layout and display options the export itself uses, so what it reports is
//! what the output lacks.

use std::ops::RangeInclusive;

use serde_json::{Value, json};
use wordcraft_doc::{Document, MAX_MISSING_PICTURES, MissingPicture};
use wordcraft_layout::display::DisplayOptions;
use wordcraft_layout::{DocLayout, LayoutCache};

use crate::io::ExportOptions;

/// The report's JSON schema version; fields may be added without raising it.
pub const SCHEMA: u32 = 1;

/// What a strict export found missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StrictReport {
    /// (code point, times drawn), sorted by code point.
    pub glyphs: Vec<(char, u64)>,
    pub pictures: Vec<MissingPicture>,
    /// A cap was hit: more were found than are listed.
    pub truncated: bool,
}

impl StrictReport {
    /// Nothing is missing: the strict export passes.
    pub fn passed(&self) -> bool {
        self.glyphs.is_empty() && self.pictures.is_empty() && !self.truncated
    }

    /// The schema-1 JSON of the report.
    pub fn to_json(&self) -> Value {
        json!({
            "schema": SCHEMA,
            "strict": if self.passed() { "passed" } else { "failed" },
            "missing_glyphs": self.glyphs.iter().map(|(c, n)| json!({"cp": code_point(*c), "count": n})).collect::<Vec<_>>(),
            "missing_pictures": self.pictures.iter().map(|p| json!({"part": p.part, "in": story_label(&p.story), "reason": p.reason.label()})).collect::<Vec<_>>(),
            "truncated": self.truncated,
        })
    }

    /// The human lines (without the `strict: ` prefix): one for the characters, one for the
    /// pictures; none when nothing is missing.
    fn parts(&self) -> Vec<String> {
        let glyphs: Vec<(String, u64)> = self.glyphs.iter().map(|(c, n)| (code_point(*c), *n)).collect();
        let pictures: Vec<(String, String, String)> =
            self.pictures.iter().map(|p| (p.part.clone(), story_label(&p.story).to_string(), p.reason.label().to_string())).collect();
        human_parts(&glyphs, &pictures, self.truncated)
    }

    /// The human lines, each `strict: …`.
    pub fn lines(&self) -> Vec<String> {
        self.parts().into_iter().map(|p| format!("strict: {p}")).collect()
    }

    /// The refusal's message: `strict: …`, the lines joined.
    pub fn refusal(&self) -> String {
        format!("strict: {}", self.parts().join("; "))
    }
}

/// The human text of a report: (code point text, count) and (part, in, reason) as the JSON has them.
fn human_parts(glyphs: &[(String, u64)], pictures: &[(String, String, String)], truncated: bool) -> Vec<String> {
    let mut out = Vec::new();
    if !glyphs.is_empty() {
        let list: Vec<String> = glyphs.iter().map(|(cp, n)| format!("{cp} (×{n})")).collect();
        let more = if truncated { ", …" } else { "" };
        let n = glyphs.len();
        out.push(format!("{n} character{} cannot be drawn: {}{more}", if n == 1 { "" } else { "s" }, list.join(", ")));
    }
    if !pictures.is_empty() {
        let list: Vec<String> = pictures
            .iter()
            .map(|(part, at, reason)| if at.is_empty() { format!("{part}: {reason}") } else { format!("{part} ({at}): {reason}") })
            .collect();
        let more = if truncated { "; …" } else { "" };
        out.push(format!("{} picture(s) left out: {}{more}", pictures.len(), list.join("; ")));
    }
    if out.is_empty() && truncated {
        out.push("more is missing than the report lists".to_string());
    }
    out
}

/// The `strict: …` human lines of a report in its JSON form (what `file.strictReport` returns).
/// Anything the JSON lacks is left out.
pub fn lines_from_json(report: &Value) -> Vec<String> {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let glyphs: Vec<(String, u64)> = report
        .get("missing_glyphs")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|g| (text(g, "cp"), g.get("count").and_then(Value::as_u64).unwrap_or(0))).collect())
        .unwrap_or_default();
    let pictures: Vec<(String, String, String)> = report
        .get("missing_pictures")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|p| (text(p, "part"), text(p, "in"), text(p, "reason"))).collect())
        .unwrap_or_default();
    let truncated = report.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    human_parts(&glyphs, &pictures, truncated).into_iter().map(|p| format!("strict: {p}")).collect()
}

/// `U+2D30`: upper-case hex, at least four digits.
fn code_point(c: char) -> String {
    format!("U+{:04X}", c as u32)
}

/// The part a picture is in, as the report names it: `header2.xml` for `word/header2.xml`.
fn story_label(story: &str) -> &str {
    story.strip_prefix("word/").unwrap_or(story)
}

/// The report of `doc` as drawn by `lay` with `display`, on `pages` (0-based; `None` is every
/// page). The pictures dropped when the document was opened are added to those that can't be
/// decoded now, capped like each.
pub fn strict_report(doc: &Document, lay: &DocLayout, display: &DisplayOptions, pages: Option<RangeInclusive<usize>>) -> StrictReport {
    let missing = lay.missing_glyphs(doc, display, pages.clone());
    let (undecodable, pictures_truncated) = wordcraft_layout::undecodable_pictures(doc, lay, pages, display);
    let mut pictures: Vec<MissingPicture> = Vec::new();
    let mut truncated = missing.truncated || pictures_truncated || doc.missing_pictures.truncated;
    for p in doc.missing_pictures.list.iter().cloned().chain(undecodable) {
        if pictures.contains(&p) {
            continue;
        }
        if pictures.len() >= MAX_MISSING_PICTURES {
            truncated = true;
            break;
        }
        pictures.push(p);
    }
    StrictReport { glyphs: missing.chars, pictures, truncated }
}

/// The PDF export's options for `opts`: the one place they are built, shared by the export and
/// its strict check.
pub(crate) fn pdf_options(opts: &ExportOptions) -> wordcraft_pdf::PdfOptions {
    wordcraft_pdf::PdfOptions { include_markup: opts.markup, comments: opts.markup && opts.comments, numeral: opts.numeral, ..Default::default() }
}

/// What a page image shows: the display options `file.exportPng` renders with.
pub(crate) fn png_display() -> DisplayOptions {
    // A page image is the printed page: headers and footers at full strength.
    let mut opts = wordcraft_render::RenderOptions::default();
    opts.display.dim_header = false;
    opts.display
}

/// The strict report of a PDF export of `doc`: every page, laid out as the export lays it out.
pub(crate) fn pdf_report(doc: &Document, opts: &ExportOptions) -> StrictReport {
    let pdf = pdf_options(opts);
    let lay = wordcraft_layout::layout(doc, &mut LayoutCache::new(), &pdf.layout_options());
    strict_report(doc, &lay, &pdf.display_options(), None)
}

/// The PDF of `doc`; with `opts.strict`, only if nothing is missing, laid out once for the check
/// and the export.
pub(crate) fn export_pdf(doc: &Document, opts: &ExportOptions) -> Result<Vec<u8>, String> {
    let pdf = pdf_options(opts);
    if !opts.strict {
        return wordcraft_pdf::export(doc, &pdf).map_err(|e| e.to_string());
    }
    let lay = wordcraft_layout::layout(doc, &mut LayoutCache::new(), &pdf.layout_options());
    let report = strict_report(doc, &lay, &pdf.display_options(), None);
    if !report.passed() {
        return Err(report.refusal());
    }
    wordcraft_pdf::export_layout(doc, &lay, &pdf).map_err(|e| e.to_string())
}

/// Is `path` a PDF or PNG output (the only outputs a strict export applies to)?
pub(crate) fn is_strict_output(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".pdf") || lower.ends_with(".png")
}
