//! The Compare header/footer flag record in a DOCX custom XML part.
//!
//! Compare turns on Different first page and Different odd & even pages so the First and Even
//! stories it inserts can be shown, and records the values those flags had so Reject All can put
//! them back. That record belongs to the document: a result that is saved, reopened and then
//! rejected must restore them too (#5837). It is not a document property, so it stays out of
//! `docProps/custom.xml`, like the Fit One Page budget (`fit.rs`).

use craft_docx::DocxError;
use wordcraft_doc::CompareHfFlags;

/// The part this crate writes and reads back.
pub(crate) const PART: &str = "customxml/wordcraftcomparehf.xml";
/// Root element of the store.
pub(crate) const ROOT: &str = "wordcraftCompareHf";
/// Namespace of the store.
pub(crate) const NS: &str = "urn:wordcraft:compare-hf";
/// Marker attribute the parser keeps (namespace declarations are stripped from `El`).
const KIND: &str = "wordcraft-compare-hf-v1";

/// Serialize `flags` as the custom XML store.
pub(crate) fn write(flags: &CompareHfFlags) -> Result<Vec<u8>, DocxError> {
    let json = serde_json::to_string(flags).map_err(|e| DocxError::Xml(e.to_string()))?;
    let esc_json = craft_xml::esc(&json);
    Ok(format!(r#"<?xml version="1.0" encoding="UTF-8"?><{ROOT} xmlns="{NS}" kind="{KIND}"><compareHf>{}</compareHf></{ROOT}>"#, esc_json)
        .into_bytes())
}

/// Read the flags from a custom XML part's bytes, if it is ours.
pub(crate) fn read_from_bytes(bytes: &[u8]) -> Option<CompareHfFlags> {
    let root = craft_xml::parse(bytes).ok()?;
    if root.local() != ROOT || root.attr("kind") != Some(KIND) {
        return None;
    }
    let text = root.child_text("compareHf")?;
    serde_json::from_str(text.as_ref()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags() -> CompareHfFlags {
        CompareHfFlags { title_page: vec![(Some(3), false), (None, false)], even_odd: Some(false) }
    }

    /// The record survives a real DOCX write and read, not just `serde_json`, so rejecting after
    /// a reopen can still restore the flags.
    #[test]
    fn compare_hf_flags_round_trip_through_docx() {
        let mut doc = wordcraft_doc::Document::from_text("Alpha\nBeta");
        doc.compare_hf = Some(flags());
        let bytes = crate::write(&doc).expect("write docx");
        let back = crate::read(&bytes).expect("read docx");
        assert_eq!(back.compare_hf, doc.compare_hf);
    }
}
