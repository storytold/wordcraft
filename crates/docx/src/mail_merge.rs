//! The single opaque inclusion/exclusion part of a mail-merge template.
use std::borrow::Cow;

use crate::DocxError;
use crate::package::MAX_EMBEDDED_XML;
use crate::xml;

pub(crate) fn recipient_kind(kind: &str) -> bool {
    // ECMA-376 §17.14.28 and its Transitional form, plus both legacy spellings.
    matches!(
        kind,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/recipientData"
            | "http://schemas.openxmlformats.org/officeDocument/2006/mailMergeRecipientData"
            | "http://schemas.openxmlformats.org/officeDocument/2006/relationships/mailMergeRecipientData"
            | "http://purl.oclc.org/ooxml/officeDocument/relationships/mailMergeRecipientData"
    )
}

/// Resolve only a package-local URI, refusing traversal above the package and ambiguous URI
/// syntax. Unsupported encodings refuse saving rather than guessing which ZIP entry they mean.
pub(crate) fn recipient_target(source: &str, target: &str) -> Option<String> {
    if target.is_empty()
        || target.trim() != target
        || target.starts_with("//")
        || target.chars().any(|c| c.is_control() || matches!(c, ':' | '\\' | '?' | '#' | '%'))
    {
        return None;
    }
    let dir = source.rsplit_once('/').map_or("", |(d, _)| d);
    let mut segments: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|s| !s.is_empty()).collect() };
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "" => return None,
            "." => {}
            ".." => {
                segments.pop()?;
            }
            s => segments.push(s),
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

/// Check the bounded root vocabulary, but never interpret, regenerate or evaluate record keys.
/// The original bytes (including namespace aliases and exclusion choices) are copied unchanged.
pub(crate) fn validate_recipient(bytes: &[u8], content_type: &str) -> Result<(), DocxError> {
    if bytes.len() > MAX_EMBEDDED_XML {
        return Err(DocxError::MailMerge("recipient data exceeds 1 MiB".into()));
    }
    // Import parsing repairs some malformed XML. A retained leaf instead needs validation of
    // the complete bytes we will copy, including namespaces, encoding and document grammar.
    let text = recipient_text(bytes)?;
    check_recipient_limits(&text, bytes)?;
    let parsed =
        roxmltree::Document::parse_with_options(&text, roxmltree::ParsingOptions { nodes_limit: MAX_EMBEDDED_XML as u32, ..Default::default() })
            .map_err(|e| DocxError::MailMerge(format!("invalid recipient data: {e}")))?;
    let root = parsed.root_element().tag_name();
    let valid = match content_type {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.mailMergeRecipientData+xml" => {
            root.name() == "recipients"
                && matches!(
                    root.namespace(),
                    Some("http://schemas.openxmlformats.org/wordprocessingml/2006/main" | "http://purl.oclc.org/ooxml/wordprocessingml/main")
                )
        }
        "application/vnd.ms-word.mailMergeRecipientData+xml" => {
            root.name() == "recipients" && root.namespace() == Some("http://schemas.microsoft.com/office/word/2006/wordml")
        }
        _ => false,
    };
    if !valid {
        return Err(DocxError::MailMerge("unsupported recipient data content type or root".into()));
    }
    Ok(())
}

/// Bound work before the validating parser compares attributes or builds a tree.
fn check_recipient_limits(text: &str, bytes: &[u8]) -> Result<(), DocxError> {
    use quick_xml::events::Event;

    // A plain reader avoids namespace resolution and duplicate-attribute comparisons here.
    // The subsequent parser validates the complete XML and namespace grammar.
    let mut reader = quick_xml::Reader::from_str(text);
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| DocxError::MailMerge(format!("invalid recipient data: {e}")))?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                if depth >= xml::MAX_DEPTH || element.attributes().with_checks(false).take(xml::MAX_ATTRS + 1).count() > xml::MAX_ATTRS {
                    return Err(DocxError::MailMerge("recipient XML exceeds structural limits".into()));
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::Decl(declaration) => {
                if let Some(encoding) =
                    declaration.encoding().transpose().map_err(|e| DocxError::MailMerge(format!("invalid recipient data: {e}")))?
                {
                    // The string parser ignores declarations. Check the original byte encoding
                    // as required by XML 1.0 section 4.3.3 before copying those bytes unchanged.
                    let matches = match bytes.get(..2) {
                        Some([0xFF, 0xFE] | [0xFE, 0xFF]) => encoding.eq_ignore_ascii_case(b"UTF-16"),
                        _ => encoding.eq_ignore_ascii_case(b"UTF-8"),
                    };
                    if !matches {
                        return Err(DocxError::MailMerge("unsupported or mismatched recipient XML encoding".into()));
                    }
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// Decode only supported Unicode input, without repairing bytes that will later be copied.
fn recipient_text(bytes: &[u8]) -> Result<Cow<'_, str>, DocxError> {
    let little_endian = match bytes.get(..2) {
        Some([0xFF, 0xFE]) => Some(true),
        Some([0xFE, 0xFF]) => Some(false),
        _ => None,
    };
    if let Some(le) = little_endian {
        let rest = bytes.get(2..).unwrap_or_default();
        let (pairs, remainder) = rest.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(DocxError::MailMerge("incomplete UTF-16 recipient data".into()));
        }
        let units = pairs.iter().map(|pair| if le { u16::from_le_bytes(*pair) } else { u16::from_be_bytes(*pair) });
        let text = char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|e| DocxError::MailMerge(format!("invalid UTF-16 recipient data: {e}")))?;
        Ok(Cow::Owned(text))
    } else {
        std::str::from_utf8(bytes).map(Cow::Borrowed).map_err(|e| DocxError::MailMerge(format!("invalid UTF-8 recipient data: {e}")))
    }
}
