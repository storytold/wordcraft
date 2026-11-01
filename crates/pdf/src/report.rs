//! The record of one export: what the writer knows directly, for callers that need a
//! machine-readable account of a conversion (the CLI's `convert --report`). Nothing
//! here is inferred after the fact — each field is read off the work as it finished.

/// A record of one PDF export.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// Pages written (after any page selection in the options).
    pub pages: usize,
    /// The PDF version, read from the finished file's header. krilla does not expose
    /// the version it was configured with to this crate, so the header of the bytes
    /// actually written is the honest source.
    pub pdf_version: String,
    /// Whether the export wrote a structure tree (the `tagged` option).
    pub tagged: bool,
    /// Whether the file claims PDF/UA conformance. Always `false` today: tagging is
    /// best effort and WordCraft makes no UA claim. The field exists so a report can
    /// state that plainly instead of letting a tagged file be mistaken for a
    /// conforming one — the in-product conformance signal LibreOffice's export-report
    /// request (tdf#174248) asks for.
    pub pdf_ua: bool,
    /// The `SOURCE_DATE_EPOCH` timestamp the export's dates were pinned to, when one
    /// was in effect (see the metadata module).
    pub source_date_epoch: Option<i64>,
    /// Family names of the fonts embedded in the file, sorted and deduplicated. A face
    /// krilla could not read never enters the exporter's font table and so is absent;
    /// a face whose text was drawn as outlines after a subsetting failure is absent
    /// too — its glyphs are paths in the file, not a font.
    pub embedded_fonts: Vec<String>,
}

/// The version from a finished file's `%PDF-` header ("1.7"), or "unknown".
pub(crate) fn pdf_version(bytes: &[u8]) -> String {
    let Some(rest) = bytes.strip_prefix(b"%PDF-") else { return "unknown".into() };
    let version: String = rest.iter().take_while(|b| b.is_ascii_digit() || **b == b'.').map(|b| *b as char).collect();
    if version.is_empty() { "unknown".into() } else { version }
}
