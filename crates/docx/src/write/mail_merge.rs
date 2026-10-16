//! Preserve a mail-merge main document, never open its external sources or run its query.
use std::collections::BTreeMap;

use wordcraft_doc::Document;

use super::PartRels;
use crate::DocxError;
use crate::package::{MAIL_MERGE_RELS, MAX_EMBEDDED_RELS, MAX_EMBEDDED_XML, Rels, SETTINGS_ORIG};
use crate::xml::{self, Node, W};

/// Keep the settings part's relationship map with the original settings. Both survive native
/// saves/recovery through the existing passthrough store; unrelated relationships are not emitted.
pub(crate) fn keep_rels(rels: &Rels) -> Result<Vec<u8>, DocxError> {
    if rels.list.len() > MAX_EMBEDDED_RELS {
        return Err(DocxError::Limit("too many mail merge settings relationships".into()));
    }
    let mut w = W::new();
    w.open("Relationships", &[("xmlns", "http://schemas.openxmlformats.org/package/2006/relationships")]);
    for r in &rels.list {
        let mut attrs = vec![("Id", r.id.as_str()), ("Type", r.kind.as_str()), ("Target", r.target.as_str())];
        if r.external {
            attrs.push(("TargetMode", "External"));
        }
        w.empty("Relationship", &attrs);
    }
    w.close("Relationships");
    let bytes = w.into_bytes();
    if bytes.len() > MAX_EMBEDDED_XML {
        return Err(DocxError::Limit("mail merge settings relationships exceed 1 MiB".into()));
    }
    Ok(bytes)
}

/// Rebind only mail-merge external sources to newly allocated IDs. Missing, ambiguous, internal
/// or unsupported references refuse the save rather than silently turning a template into a letter.
pub(super) fn write(doc: &Document, out: &mut PartRels) -> Result<Option<String>, DocxError> {
    let Some(orig) = doc.passthrough.get(SETTINGS_ORIG) else { return Ok(None) };
    let settings = xml::parse(orig)?;
    let Some(original) = settings.child("w:mailMerge") else { return Ok(None) };
    if settings.children("w:mailMerge").count() != 1 {
        return Err(DocxError::MailMerge("multiple mailMerge elements".into()));
    }
    let rels = doc.passthrough.get(MAIL_MERGE_RELS).map(|bytes| xml::parse(bytes)).transpose()?;
    let mut merge = original.clone();
    let mut ids = BTreeMap::<String, String>::new();
    let mut pending = vec![&mut merge];
    while let Some(e) = pending.pop() {
        if e.name.starts_with("?:") || e.attrs.iter().any(|(k, _)| k.starts_with("?:")) {
            return Err(DocxError::MailMerge("unrecognised namespace in mailMerge".into()));
        }
        for (name, id) in &mut e.attrs {
            if name != "r:id" {
                continue;
            }
            if let Some(new) = ids.get(id) {
                *id = new.clone();
                continue;
            }
            let mut matches = rels.iter().flat_map(|r| r.els()).filter(|r| r.attr("Id") == Some(id.as_str()));
            let rel = matches.next().ok_or_else(|| DocxError::MailMerge(format!("missing source relationship {id}")))?;
            if matches.next().is_some() {
                return Err(DocxError::MailMerge(format!("ambiguous source relationship {id}")));
            }
            let kind = rel.attr("Type").unwrap_or("");
            let target = rel.attr("Target").unwrap_or("");
            let external = rel.attr("TargetMode").is_some_and(|v| v.eq_ignore_ascii_case("External"));
            let source = ["mailMergeSource", "mailMergeHeaderSource"].iter().any(|suffix| {
                kind == format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{suffix}")
                    || kind == format!("http://purl.oclc.org/ooxml/officeDocument/relationships/{suffix}")
            });
            if !external || !source || target.trim().is_empty() {
                return Err(DocxError::MailMerge(format!(
                    "unsupported or empty source relationship {id}; only external mail merge sources are preserved"
                )));
            }
            let new = out.add(kind, target, true);
            ids.insert(id.clone(), new.clone());
            *id = new;
        }
        pending.extend(e.kids.iter_mut().filter_map(|n| match n {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        }));
    }
    // Original producers may use other prefixes. The canonical subtree declares its own aliases.
    merge.attrs.push(("xmlns:w".into(), "http://schemas.openxmlformats.org/wordprocessingml/2006/main".into()));
    merge.attrs.push(("xmlns:r".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships".into()));
    Ok(Some(merge.to_xml()))
}
