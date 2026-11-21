//! Style-definition revision helpers that extend `craft-docx` with WordCraft-specific state:
//! - Preserving level-only list inheritance provenance (`numbering_from_base`, `old_numbering_from_base`)
//!   across `word/styles.xml` read and write.
//! - Re-resolving pre-change level-only list references along the pre-change based-on chain.
//! - Overlaying `w:tblStylePr[@w:type="wholeTable"]` paragraph/run properties onto `ch.old` so
//!   rejecting a table style's tracked property change preserves `wholeTable` defaults.

use std::collections::HashMap;
use std::sync::Arc;

use craft_docx::DocxError;
use craft_xml::{DECL_STANDALONE, El, Limits as XmlLimits, Node, Options as XmlOptions};
use craft_zip::{Archive, Compression, Limits as ZipLimits, Writer as ZipWriter};
use wordcraft_doc::Document;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::styles::{StyleKind, StyleSheet};

use crate::{ReadOptions, is_encrypted};

/// Maximum `word/styles.xml` size processed for style revision metadata (32 MiB).
const MAX_STYLES_XML: u64 = 32 << 20;
/// Maximum package size buffered when rewriting or scanning style revisions (256 MiB).
const MAX_PKG_BYTES: u64 = 256 << 20;
/// Maximum entries scanned in a package ZIP archive.
const MAX_PKG_ENTRIES: u64 = 100_000;
/// Maximum `w:style` elements scanned per stylesheet.
const MAX_STYLES: usize = 10_000;
/// Maximum `based_on` chain traversal steps.
const MAX_CHAIN_DEPTH: usize = 32;

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn zip_limits() -> ZipLimits {
    ZipLimits { max_entries: MAX_PKG_ENTRIES, max_total_bytes: MAX_PKG_BYTES }
}

fn xml_limits() -> XmlLimits {
    XmlLimits { max_input: MAX_STYLES_XML as usize, ..XmlLimits::DEFAULT }
}

#[derive(Clone, Copy, Default)]
struct NumPrScan {
    has_ilvl: bool,
    ilvl: u8,
    has_num_id: bool,
}

impl NumPrScan {
    fn is_level_only(self) -> bool {
        self.has_ilvl && !self.has_num_id
    }
}

#[derive(Clone, Default)]
struct StyleXmlInfo {
    style_id: String,
    kind: StyleKind,
    is_default: bool,
    name: Option<String>,
    live_num: Option<NumPrScan>,
    old_num: Option<NumPrScan>,
    whole_ppr: Vec<u8>,
    whole_rpr: Vec<u8>,
}

/// Rewrites `word/styles.xml` inside serialized DOCX ZIP bytes when any style carries
/// `numbering_from_base` or `old_numbering_from_base`, omitting `w:numId` so reload keeps
/// the level-only inheritance provenance.
pub(crate) fn postprocess_written_docx(doc: &Document, zip_bytes: Vec<u8>) -> Result<Vec<u8>, DocxError> {
    let sheet = &doc.styles;
    let needs_rewrite =
        sheet.styles.iter().any(|s| s.numbering_from_base || (s.old_numbering_from_base && (s.para_change.is_some() || s.para.fmt_change.is_some())));
    if !needs_rewrite || !zip_bytes.starts_with(b"PK") {
        return Ok(zip_bytes);
    }

    let Ok(mut archive) = Archive::open(&zip_bytes, zip_limits()) else {
        return Ok(zip_bytes);
    };
    if !archive.contains("word/styles.xml") || archive.declared_size("word/styles.xml").is_some_and(|n| n > MAX_STYLES_XML) {
        return Ok(zip_bytes);
    }

    let Some(raw_styles) = archive.read("word/styles.xml", MAX_STYLES_XML).map_err(|e| DocxError::Xml(format!("read styles.xml: {e}")))? else {
        return Ok(zip_bytes);
    };

    let rewritten = strip_inherited_num_ids(&raw_styles, sheet)?;
    let names: Vec<String> = archive.names().map(|n| n.into_owned()).collect();
    let mut writer = ZipWriter::new();
    for name in names {
        let compression = if archive.is_stored(&name) == Some(true) { Compression::Stored } else { Compression::Deflated };
        if name == "word/styles.xml" {
            writer.add(&name, &rewritten, compression).map_err(|e| DocxError::Xml(format!("write styles.xml entry: {e}")))?;
        } else {
            let data = archive.read(&name, MAX_PKG_BYTES).map_err(|e| DocxError::Xml(format!("copy zip entry {name}: {e}")))?.unwrap_or_default();
            writer.add(&name, &data, compression).map_err(|e| DocxError::Xml(format!("write zip entry {name}: {e}")))?;
        }
    }
    writer.finish().map_err(|e| DocxError::Xml(format!("finish docx zip: {e}")))
}

/// Restores `numbering_from_base`, `old_numbering_from_base`, pre-change level-only numbering,
/// and `wholeTable` overlays on style change snapshots from `word/styles.xml`.
pub(crate) fn postprocess_read_docx(bytes: &[u8], opts: &ReadOptions<'_>, doc: &mut Document) -> Result<(), DocxError> {
    if doc.styles.styles.is_empty() {
        return Ok(());
    }
    let decrypted;
    let pkg_bytes = if is_encrypted(bytes) {
        decrypted = crate::decrypt(bytes, opts.password)?;
        decrypted.as_slice()
    } else {
        bytes
    };
    let Ok(mut archive) = Archive::open(pkg_bytes, zip_limits()) else {
        return Ok(());
    };
    let Ok(Some(xml)) = archive.read("word/styles.xml", MAX_STYLES_XML) else {
        return Ok(());
    };

    let scanned = scan_styles_xml(&xml);
    if scanned.is_empty() {
        return Ok(());
    }
    let aliases = compute_style_aliases(&scanned);
    let mut by_id: HashMap<String, StyleXmlInfo> = HashMap::with_capacity(scanned.len());
    for info in scanned {
        let canonical = aliases.get(&info.style_id).cloned().unwrap_or_else(|| info.style_id.clone());
        by_id.insert(canonical, info);
    }

    let sheet = Arc::make_mut(&mut doc.styles);
    for st in &mut sheet.styles {
        if let Some(info) = by_id.get(&st.id) {
            st.numbering_from_base = info.live_num.is_some_and(NumPrScan::is_level_only);
            st.old_numbering_from_base = info.old_num.is_some_and(NumPrScan::is_level_only);
        }
    }

    // Re-resolve old level-only numbering against the pre-change based-on chain so intermediate
    // unchanged styles with live inherited numbering don't shadow the base style's old list.
    let resolved_old: Vec<(usize, Option<NumRef>)> = sheet
        .styles
        .iter()
        .enumerate()
        .filter(|(_, s)| s.old_numbering_from_base)
        .filter_map(|(i, s)| {
            let info = by_id.get(&s.id)?;
            let level = info
                .old_num
                .map(|n| n.ilvl)
                .or_else(|| s.para_change.as_ref().or(s.para.fmt_change.as_ref()).and_then(|c| c.old.numbering.map(|n| n.level)))?;
            let ch = s.para_change.as_ref().or(s.para.fmt_change.as_ref())?;
            let base = ch.old.style.as_deref().or(s.based_on.as_deref())?;
            let num_ref = chain_list_before(sheet, base).map(|num| NumRef { num, level });
            Some((i, num_ref))
        })
        .collect();
    for (idx, num_ref) in resolved_old {
        if let Some(st) = sheet.styles.get_mut(idx) {
            if let Some(ch) = st.para_change.as_mut() {
                ch.old.numbering = num_ref;
            }
            if let Some(ch) = st.para.fmt_change.as_mut() {
                ch.old.numbering = num_ref;
            }
        }
    }

    // Overlay `wholeTable` paragraph/run properties onto `ch.old` for table styles with changes.
    for st in &mut sheet.styles {
        if st.kind != StyleKind::Table || (st.para_change.is_none() && st.chr_change.is_none()) {
            continue;
        }
        let Some(info) = by_id.get(&st.id) else { continue };
        if info.whole_ppr.is_empty() && info.whole_rpr.is_empty() {
            continue;
        }
        if let Some((wt_para, wt_chr)) = parse_whole_table_props(&info.whole_ppr, &info.whole_rpr) {
            if let Some(ch) = st.para_change.as_mut() {
                ch.old.overlay(&wt_para);
            }
            if let Some(ch) = st.para.fmt_change.as_mut() {
                ch.old.overlay(&wt_para);
            }
            if let Some(ch) = st.chr_change.as_mut() {
                ch.old.overlay(&wt_chr);
            }
            if let Some(ch) = st.chr.fmt_change.as_mut() {
                ch.old.overlay(&wt_chr);
            }
        }
    }

    Ok(())
}

/// Nearest explicit list ID in `style`'s based-on chain in its pre-change state.
fn chain_list_before(sheet: &StyleSheet, style: &str) -> Option<u32> {
    let mut cur = Some(style);
    for _ in 0..MAX_CHAIN_DEPTH {
        let Some(id) = cur else { break };
        let st = sheet.styles.iter().find(|s| s.id == id)?;
        let inherited_here =
            if st.para_change.is_some() || st.para.fmt_change.is_some() { st.old_numbering_from_base } else { st.numbering_from_base };
        if !inherited_here {
            let num = st.para_change.as_ref().or(st.para.fmt_change.as_ref()).map_or(st.para.numbering, |c| c.old.numbering).map(|n| n.num);
            if let Some(n) = num {
                return Some(n);
            }
        }
        cur = st.based_on.as_deref();
    }
    None
}

fn strip_num_id(num_pr: &mut El) {
    num_pr.children.retain(|n| !matches!(n, Node::El(e) if e.local() == "numId"));
}

fn strip_inherited_num_ids(xml: &[u8], sheet: &StyleSheet) -> Result<Vec<u8>, DocxError> {
    let mut root = XmlOptions::default().with_limits(xml_limits()).parse(xml).map_err(|e| DocxError::Xml(format!("styles.xml parse: {e}")))?;

    for style_el in root.elements_mut().filter(|e| e.local() == "style") {
        let Some(id) = style_el.attr_local("styleId") else { continue };
        let Some(st) = sheet.get(id) else { continue };
        let (strip_live, strip_old) = (st.numbering_from_base, st.old_numbering_from_base);
        if !strip_live && !strip_old {
            continue;
        }
        for ppr in style_el.elements_mut().filter(|e| e.local() == "pPr") {
            if strip_live {
                for num_pr in ppr.elements_mut().filter(|e| e.local() == "numPr") {
                    strip_num_id(num_pr);
                }
            }
            if strip_old {
                for ch in ppr.elements_mut().filter(|e| e.local() == "pPrChange") {
                    for old_ppr in ch.elements_mut().filter(|e| e.local() == "pPr") {
                        for num_pr in old_ppr.elements_mut().filter(|e| e.local() == "numPr") {
                            strip_num_id(num_pr);
                        }
                    }
                }
            }
        }
    }

    let mut out = String::from(DECL_STANDALONE);
    root.write_xml(&mut out);
    Ok(out.into_bytes())
}

fn scan_styles_xml(xml: &[u8]) -> Vec<StyleXmlInfo> {
    let Ok(root) = XmlOptions::default().with_limits(xml_limits()).parse(xml) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for style_el in root.elements().filter(|e| e.local() == "style").take(MAX_STYLES) {
        let Some(id) = style_el.attr_local("styleId").filter(|s| !s.is_empty()) else {
            continue;
        };
        let kind = match style_el.attr_local("type") {
            Some("character") => StyleKind::Character,
            Some("table") => StyleKind::Table,
            Some("numbering") => StyleKind::Numbering,
            _ => StyleKind::Paragraph,
        };
        let is_default = style_el.attr_local("default").is_some_and(|v| v == "1" || v == "true");
        let name = style_el.elements().find(|e| e.local() == "name").and_then(|e| e.attr_local("val").map(str::to_owned));

        let mut live_num = None;
        let mut old_num = None;
        if let Some(ppr) = style_el.elements().find(|e| e.local() == "pPr") {
            if let Some(num_pr) = ppr.elements().find(|e| e.local() == "numPr") {
                live_num = Some(scan_num_pr(num_pr));
            }
            if let Some(ppr_ch) = ppr.elements().find(|e| e.local() == "pPrChange")
                && let Some(old_ppr) = ppr_ch.elements().find(|e| e.local() == "pPr")
                && let Some(old_num_pr) = old_ppr.elements().find(|e| e.local() == "numPr")
            {
                old_num = Some(scan_num_pr(old_num_pr));
            }
        }

        let mut whole_ppr = Vec::new();
        let mut whole_rpr = Vec::new();
        if kind == StyleKind::Table
            && let Some(wt) = style_el.elements().find(|e| e.local() == "tblStylePr" && e.attr_local("type") == Some("wholeTable"))
        {
            if let Some(ppr) = wt.elements().find(|e| e.local() == "pPr") {
                let mut s = String::new();
                ppr.write_xml(&mut s);
                whole_ppr = s.into_bytes();
            }
            if let Some(rpr) = wt.elements().find(|e| e.local() == "rPr") {
                let mut s = String::new();
                rpr.write_xml(&mut s);
                whole_rpr = s.into_bytes();
            }
        }

        out.push(StyleXmlInfo { style_id: id.to_owned(), kind, is_default, name, live_num, old_num, whole_ppr, whole_rpr });
    }
    out
}

fn scan_num_pr(num_pr: &El) -> NumPrScan {
    let mut scan = NumPrScan::default();
    for child in num_pr.elements() {
        match child.local() {
            "ilvl" => {
                scan.has_ilvl = true;
                if let Some(v) = child.attr_local("val").and_then(|s| s.trim().parse::<u8>().ok()) {
                    scan.ilvl = v.min(8);
                }
            }
            "numId" => scan.has_num_id = true,
            _ => {}
        }
    }
    scan
}

fn compute_style_aliases(styles: &[StyleXmlInfo]) -> HashMap<String, String> {
    let mut alias = HashMap::new();
    let has_normal = styles.iter().any(|s| s.style_id == "Normal");
    if !has_normal && let Some(def) = styles.iter().find(|s| s.kind == StyleKind::Paragraph && s.is_default) {
        alias.insert(def.style_id.clone(), "Normal".to_string());
    }
    let mut taken: Vec<String> = styles.iter().map(|s| s.style_id.clone()).collect();
    for s in styles.iter().filter(|s| s.kind == StyleKind::Paragraph) {
        let Some(name) = s.name.as_deref() else { continue };
        let canonical = match name.trim().to_ascii_lowercase().as_str() {
            "toc heading" => "TOCHeading".to_string(),
            n => match n.strip_prefix("toc ").and_then(|d| d.parse::<u8>().ok()).filter(|d| (1..=9).contains(d)) {
                Some(d) => format!("TOC{d}"),
                None => continue,
            },
        };
        if !taken.contains(&canonical) && !alias.contains_key(&s.style_id) {
            taken.push(canonical.clone());
            alias.insert(s.style_id.clone(), canonical);
        }
    }
    alias
}

fn parse_whole_table_props(ppr_xml: &[u8], rpr_xml: &[u8]) -> Option<(wordcraft_doc::props::ParaProps, wordcraft_doc::props::CharProps)> {
    const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    const CT: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-officedocument.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;

    let ppr = String::from_utf8_lossy(ppr_xml);
    let rpr = String::from_utf8_lossy(rpr_xml);
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="_WT">{ppr}{rpr}</w:style></w:styles>"#
    );
    let doc_xml =
        format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W_NS}"><w:body><w:p/></w:body></w:document>"#);

    let mut zw = ZipWriter::new();
    for (name, bytes) in [
        ("[Content_Types].xml", CT.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
        ("word/document.xml", doc_xml.as_bytes()),
    ] {
        zw.add(name, bytes, Compression::Stored).ok()?;
    }
    let zip_bytes = zw.finish().ok()?;
    let shared = craft_docx::read(&zip_bytes).ok()?;
    let doc = crate::from_shared_model(shared).ok()?;
    let st = doc.styles.get("_WT")?;
    Some((st.para.clone(), st.chr.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_table_overlay_test() {
        let styles = br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
         <w:style w:type="table" w:styleId="Whole"><w:name w:val="Whole"/>
          <w:pPr><w:jc w:val="left"/><w:pPrChange w:id="1" w:author="Editor" w:date="2026-01-01T00:00:00Z"><w:pPr><w:jc w:val="right"/></w:pPr></w:pPrChange></w:pPr>
          <w:rPr><w:i/><w:rPrChange w:id="2" w:author="Editor" w:date="2026-01-01T00:00:00Z"><w:rPr><w:b/></w:rPr></w:rPrChange></w:rPr>
          <w:tblStylePr w:type="wholeTable"><w:pPr><w:jc w:val="center"/></w:pPr><w:rPr><w:color w:val="1F4E79"/></w:rPr></w:tblStylePr>
         </w:style>
        </w:styles>"#;
        let scanned = scan_styles_xml(styles);
        assert_eq!(scanned.len(), 1);
        let ppr = String::from_utf8_lossy(&scanned[0].whole_ppr);
        let rpr = String::from_utf8_lossy(&scanned[0].whole_rpr);
        let parsed = parse_whole_table_props(&scanned[0].whole_ppr, &scanned[0].whole_rpr);
        assert!(parsed.is_some(), "ppr={ppr:?} rpr={rpr:?} — read_xml failed");
        let (p, c) = parsed.unwrap();
        assert_eq!(p.align, Some(wordcraft_doc::props::Align::Center), "ppr={ppr:?}");
        assert_eq!(c.color, Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::Rgb(0x1F, 0x4E, 0x79))), "rpr={rpr:?}");
    }
}
