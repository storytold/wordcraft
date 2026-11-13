//! The Fit One Page budget in a DOCX custom XML part.
//!
//! Word keeps custom XML stores as ordinary package parts; our own store is namespaced so it can
//! never be mistaken for a user's data. It is not a document property, so it stays out of
//! `docProps/custom.xml`.

use wordcraft_doc::FitState;

use crate::DocxError;
use crate::xml::{El, esc};

/// The part this crate writes and reads back.
pub(crate) const PART: &str = "customxml/wordcraftfit.xml";
/// Root element of the store.
pub(crate) const ROOT: &str = "wordcraftFit";
/// Namespace of the store.
pub(crate) const NS: &str = "urn:wordcraft:fit";
/// Marker attribute the parser keeps (namespace declarations are stripped from `El`).
const KIND: &str = "wordcraft-fit-v1";

/// Whether `root` is our store (and not a user's custom XML part with a coincidental name).
pub(crate) fn is_state(root: &El) -> bool {
    root.local() == ROOT && root.attr("kind") == Some(KIND)
}

/// Serialize `fit` as the custom XML store.
pub(crate) fn write(fit: &FitState) -> Result<Vec<u8>, DocxError> {
    let json = serde_json::to_string(fit).map_err(|e| DocxError::Xml(e.to_string()))?;
    Ok(format!(r#"<?xml version="1.0" encoding="UTF-8"?><{ROOT} xmlns="{NS}" kind="{KIND}"><fitState>{}</fitState></{ROOT}>"#, esc(&json))
        .into_bytes())
}

/// Read the state from a parsed custom XML root, if it is ours.
pub(crate) fn read(root: &El) -> Option<FitState> {
    if !is_state(root) {
        return None;
    }
    let state = root.els().find(|e| e.local() == "fitState")?;
    serde_json::from_str(&state.text()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::props::LineSpacing;

    fn state() -> FitState {
        FitState {
            cum_delta: 1.0,
            cum_space: 0.94,
            reduced_font_sizes: vec![11.0],
            reduced_line_spacings: vec![LineSpacing::Multiple(0.94)],
            reduced_paragraph_spacings: vec![18.8],
            original_font_sizes: vec![12.0],
            original_line_spacings: vec![LineSpacing::Multiple(1.0)],
            original_paragraph_spacings: vec![20.0],
            ..Default::default()
        }
    }

    /// The cumulative budget survives a real DOCX write and read, not just `serde_json`.
    #[test]
    fn fit_state_round_trips_through_docx() {
        let mut doc = wordcraft_doc::Document::from_text("Alpha\nBeta");
        doc.fit_state = Some(state());
        let bytes = crate::write(&doc).expect("write docx");
        let back = crate::read(&bytes).expect("read docx");
        assert_eq!(back.fit_state, doc.fit_state);
    }
}
