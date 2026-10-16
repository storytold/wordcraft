//! The single opaque inclusion/exclusion part of a mail-merge template.
use crate::DocxError;
use crate::package::MAX_EMBEDDED_XML;
use crate::xml;

pub(crate) fn recipient_kind(kind: &str) -> bool {
    // MS-OI29500 §2.1.378 documents Word's spelling alongside the ISO spelling.
    matches!(
        kind,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/recipientData"
            | "http://schemas.openxmlformats.org/officeDocument/2006/mailMergeRecipientData"
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
    let root = xml::parse_ns(bytes, &[("wne", "http://schemas.microsoft.com/office/word/2006/wordml")])
        .map_err(|e| DocxError::MailMerge(format!("invalid recipient data: {e}")))?;
    let valid = match content_type {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.mailMergeRecipientData+xml" => root.name == "w:recipients",
        "application/vnd.ms-word.mailMergeRecipientData+xml" => root.name == "wne:recipients",
        _ => false,
    };
    if !valid {
        return Err(DocxError::MailMerge("unsupported recipient data content type or root".into()));
    }
    Ok(())
}
