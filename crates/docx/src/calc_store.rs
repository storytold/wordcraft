//! The calculation store in a DOCX custom XML part: each calculation's source, keyed by the id in
//! its content control's tag, and the document's calculation settings (see
//! `wordcraft_doc::calc::CalcStore`). Word shows the equations; this part is what makes them
//! calculations again when WordCraft opens the file.
//!
//! Namespaced like the Fit One Page store (`fit.rs`) so it can never be mistaken for a user's
//! data. Read back by its content, not its name: Word may rename custom XML parts it keeps.

use craft_docx::DocxError;
use wordcraft_doc::calc::{CalcStore, MAX_STORE_BYTES, MAX_STORED_REGIONS};

/// The part this crate writes.
pub(crate) const PART: &str = "customxml/wordcraftcalc.xml";
/// Root element of the store.
pub(crate) const ROOT: &str = "wordcraftCalc";
/// Namespace of the store.
pub(crate) const NS: &str = "urn:wordcraft:calc";
/// Marker attribute the parser keeps (namespace declarations are stripped from `El`).
pub(crate) const KIND: &str = "wordcraft-calc-v1";
/// The store's data store id (`ds:itemID` of its properties part): the same for every document.
pub(crate) const ITEM_ID: &str = "{6F1D3A52-9C1B-4E7A-8B2D-5C0A7E3F9D41}";
/// The element holding the JSON payload.
const PAYLOAD: &str = "calcStore";

/// Serialize `store` as the custom XML store.
pub(crate) fn write(store: &CalcStore) -> Result<Vec<u8>, DocxError> {
    let json = serde_json::to_string(store).map_err(|e| DocxError::Xml(e.to_string()))?;
    let esc_json = quick_xml::escape::escape(&json);
    Ok(format!(r#"<?xml version="1.0" encoding="UTF-8"?><{ROOT} xmlns="{NS}" kind="{KIND}"><{PAYLOAD}>{esc_json}</{PAYLOAD}></{ROOT}>"#).into_bytes())
}

/// Read the store from a custom XML part's bytes, if it is ours and sound: `None` for another
/// part, malformed XML or JSON, a part over [`MAX_STORE_BYTES`] or a store of more than
/// [`MAX_STORED_REGIONS`] entries (regions and tables together).
pub(crate) fn read_from_bytes(bytes: &[u8]) -> Option<CalcStore> {
    if bytes.len() > MAX_STORE_BYTES {
        return None;
    }
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buf = Vec::new();
    let mut is_our_kind = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                let name = e.local_name();
                if name.as_ref() == ROOT.as_bytes() {
                    is_our_kind = e.attributes().flatten().any(|a| a.key.local_name().as_ref() == b"kind" && a.value.as_ref() == KIND.as_bytes());
                    if !is_our_kind {
                        return None;
                    }
                } else if name.as_ref() == PAYLOAD.as_bytes() && is_our_kind {
                    let text = reader.read_text(e.to_end().name()).ok()?;
                    let s = std::str::from_utf8(text.as_ref()).ok()?;
                    let unescaped = quick_xml::escape::unescape(s).ok()?;
                    let store: CalcStore = serde_json::from_str(&unescaped).ok()?;
                    return (store.regions.len().saturating_add(store.tables.len()).saturating_add(store.plots.len()) <= MAX_STORED_REGIONS)
                        .then_some(store);
                } else if !is_our_kind {
                    // The root is something else.
                    return None;
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
    use wordcraft_doc::calc::{StoredRegion, StoredTable};

    fn store() -> CalcStore {
        CalcStore {
            settings: Default::default(),
            regions: vec![StoredRegion {
                result_format: None,
                worksheet_order: None,
                id: 7,
                source: "σ := P/A_b = ? ksi <&>".into(),
                unit: None,
                precision: Some(2),
                numbered: true,
                fingerprint: u64::MAX,
            }],
            tables: vec![StoredTable { id: 2, fingerprint: 7 }],
            plots: Vec::new(),
        }
    }

    #[test]
    fn store_round_trips_through_its_part() {
        assert_eq!(read_from_bytes(&write(&store()).unwrap()), Some(store()));
    }

    #[test]
    fn other_parts_and_bad_payloads_are_not_a_store() {
        assert_eq!(read_from_bytes(b"<x kind=\"wordcraft-calc-v1\"><calcStore>{}</calcStore></x>"), None);
        assert_eq!(read_from_bytes(b"<wordcraftCalc kind=\"other\"><calcStore>{}</calcStore></wordcraftCalc>"), None);
        assert_eq!(read_from_bytes(b"<wordcraftCalc kind=\"wordcraft-calc-v1\"><calcStore>{oops</calcStore></wordcraftCalc>"), None);
        assert_eq!(read_from_bytes(b"\xff\xfe<<<"), None);
        assert_eq!(read_from_bytes(&vec![b' '; MAX_STORE_BYTES + 1]), None);
    }
}
