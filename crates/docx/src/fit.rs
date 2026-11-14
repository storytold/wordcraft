//! The Fit One Page budget in a DOCX custom XML part.
//!
//! Word keeps custom XML stores as ordinary package parts; our own store is namespaced so it can
//! never be mistaken for a user's data. It is not a document property, so it stays out of
//! `docProps/custom.xml`.

use craft_docx::DocxError;
use wordcraft_doc::FitState;

/// The part this crate writes and reads back.
pub(crate) const PART: &str = "customxml/wordcraftfit.xml";
/// Root element of the store.
pub(crate) const ROOT: &str = "wordcraftFit";
/// Namespace of the store.
pub(crate) const NS: &str = "urn:wordcraft:fit";
/// Marker attribute the parser keeps (namespace declarations are stripped from `El`).
pub(crate) const KIND: &str = "wordcraft-fit-v1";

/// Serialize `fit` as the custom XML store.
pub(crate) fn write(fit: &FitState) -> Result<Vec<u8>, DocxError> {
    let json = serde_json::to_string(fit).map_err(|e| DocxError::Xml(e.to_string()))?;
    let esc_json = quick_xml::escape::escape(&json);
    Ok(format!(r#"<?xml version="1.0" encoding="UTF-8"?><{ROOT} xmlns="{NS}" kind="{KIND}"><fitState>{}</fitState></{ROOT}>"#, esc_json).into_bytes())
}

/// Read the state from a custom XML part's bytes, if it is ours.
pub(crate) fn read_from_bytes(bytes: &[u8]) -> Option<FitState> {
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buf = Vec::new();
    let mut is_our_kind = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                let name = e.local_name();
                if name.as_ref() == ROOT.as_bytes() {
                    for attr in e.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"kind" && attr.value.as_ref() == KIND.as_bytes() {
                            is_our_kind = true;
                        }
                    }
                } else if name.as_ref() == b"fitState" && is_our_kind {
                    let text = reader.read_text(e.to_end().name()).ok()?;
                    let s = std::str::from_utf8(text.as_ref()).ok()?;
                    let unescaped = quick_xml::escape::unescape(s).ok()?;
                    return serde_json::from_str(&unescaped).ok();
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
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
