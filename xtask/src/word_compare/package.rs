//! A minimal `.docx` writer for the synthetic probes: a stored (uncompressed) zip with the
//! parts a probe needs, and solid-colour PNGs for picture probes. Std only.

/// One probe document's parts beyond the fixed ones.
pub struct Parts<'a> {
    /// `w:body` content, without the final `w:sectPr` (added from `sect`).
    pub body: &'a str,
    /// The final section's `w:sectPr` children (page size, margins, columns, header references).
    pub sect: &'a str,
    /// `w:docDefaults` run properties (`w:rPr` children) and paragraph properties (`w:pPr` children).
    pub default_rpr: &'a str,
    pub default_ppr: &'a str,
    /// Extra `w:style` elements.
    pub styles: &'a str,
    /// `w:abstractNum` and `w:num` elements, if the probe has lists.
    pub numbering: &'a str,
    /// Footnote bodies (`w:footnote` elements with ids from 1), if any.
    pub footnotes: &'a str,
    /// Header bodies; header `i` is `header{i+1}.xml` with relationship id `rIdH{i+1}`.
    pub headers: &'a [String],
    /// Pictures as (width px, height px, RGB); picture `i` is `media/image{i+1}.png`, `rIdP{i+1}`.
    pub pictures: &'a [(u32, u32, [u8; 3])],
}

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;

/// The `.docx` bytes.
pub fn docx(p: &Parts<'_>) -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut overrides = String::new();
    let mut rels = String::new();
    let mut add_part = |files: &mut Vec<(String, Vec<u8>)>, name: &str, ct: &str, xml: String| {
        overrides.push_str(&format!(
            r#"<Override PartName="/word/{name}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.{ct}+xml"/>"#
        ));
        files.push((format!("word/{name}"), xml.into_bytes()));
    };
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles {NS}><w:docDefaults><w:rPrDefault><w:rPr>{}</w:rPr></w:rPrDefault><w:pPrDefault><w:pPr>{}</w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:semiHidden/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>{}</w:styles>"#,
        p.default_rpr, p.default_ppr, p.styles
    );
    add_part(&mut files, "styles.xml", "styles", styles);
    rels.push_str(&format!(r#"<Relationship Id="rIdS" Type="{REL}/styles" Target="styles.xml"/>"#));
    add_part(
        &mut files,
        "settings.xml",
        "settings",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings {NS}><w:defaultTabStop w:val="720"/><w:characterSpacingControl w:val="doNotCompress"/><w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#
        ),
    );
    rels.push_str(&format!(r#"<Relationship Id="rIdT" Type="{REL}/settings" Target="settings.xml"/>"#));
    if !p.numbering.is_empty() {
        add_part(
            &mut files,
            "numbering.xml",
            "numbering",
            format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering {NS}>{}</w:numbering>"#, p.numbering),
        );
        rels.push_str(&format!(r#"<Relationship Id="rIdN" Type="{REL}/numbering" Target="numbering.xml"/>"#));
    }
    if !p.footnotes.is_empty() {
        let sep = r#"<w:footnote w:type="separator" w:id="-1"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>"#;
        add_part(
            &mut files,
            "footnotes.xml",
            "footnotes",
            format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:footnotes {NS}>{sep}{}</w:footnotes>"#, p.footnotes),
        );
        rels.push_str(&format!(r#"<Relationship Id="rIdF" Type="{REL}/footnotes" Target="footnotes.xml"/>"#));
    }
    for (i, h) in p.headers.iter().enumerate() {
        add_part(
            &mut files,
            &format!("header{}.xml", i + 1),
            "header",
            format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr {NS}>{h}</w:hdr>"#),
        );
        rels.push_str(&format!(r#"<Relationship Id="rIdH{}" Type="{REL}/header" Target="header{}.xml"/>"#, i + 1, i + 1));
    }
    for (i, &(w, h, rgb)) in p.pictures.iter().enumerate() {
        files.push((format!("word/media/image{}.png", i + 1), png(w, h, rgb)));
        rels.push_str(&format!(r#"<Relationship Id="rIdP{}" Type="{REL}/image" Target="media/image{}.png"/>"#, i + 1, i + 1));
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{}<w:sectPr>{}</w:sectPr></w:body></w:document>"#,
        p.body, p.sect
    );
    files.insert(0, ("word/document.xml".into(), document.into_bytes()));
    files.push((
        "word/_rels/document.xml.rels".into(),
        format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{REL_PKG}">{rels}</Relationships>"#).into_bytes(),
    ));
    let types = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>{overrides}</Types>"#
    );
    files.insert(0, ("_rels/.rels".into(), format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{REL_PKG}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#).into_bytes()));
    files.insert(0, ("[Content_Types].xml".into(), types.into_bytes()));
    zip(&files)
}

const REL_PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// An inline picture run for picture `n` (1-based) at `w_pt` × `h_pt` points.
pub fn inline_picture(n: usize, w_pt: f64, h_pt: f64) -> String {
    let (cx, cy) = ((w_pt * 12700.0).round() as i64, (h_pt * 12700.0).round() as i64);
    format!(
        r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{n}" name="Picture {n}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{n}" name="image{n}.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdP{n}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

/// A stored zip archive (no compression; CRC-32 per entry).
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let crc = crc32(data);
        let offset = out.len() as u32;
        let (n, len) = (name.len() as u16, data.len() as u32);
        // Local header: signature, version 2.0, no flags, stored, a fixed 1980-01-01 time stamp.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        for v in [crc, len, len] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        for v in [crc, len, len] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&n.to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let (cd_offset, cd_len, count) = (out.len() as u32, central.len() as u32, files.len() as u16);
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A solid-colour 8-bit RGB PNG (zlib stream of stored deflate blocks).
pub fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    // Each row: filter type 0, then the pixels.
    let row: Vec<u8> = std::iter::once(0).chain((0..w).flat_map(|_| rgb)).collect();
    let raw = row.repeat(h as usize);
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(c) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        let len = c.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(&b"IHDR"[..], ihdr), (b"IDAT", z), (b"IEND", Vec::new())] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(&data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn zip_has_entries_and_an_end_record() {
        let z = zip(&[("a.txt".into(), b"hello".to_vec()), ("b/c.xml".into(), b"<x/>".to_vec())]);
        assert!(z.starts_with(&[0x50, 0x4b, 3, 4]));
        let end = z.len() - 22;
        assert_eq!(&z[end..end + 4], &[0x50, 0x4b, 5, 6]);
        assert_eq!(u16::from_le_bytes([z[end + 10], z[end + 11]]), 2);
        assert!(z.windows(5).any(|w| w == b"hello"));
    }

    #[test]
    fn png_has_signature_and_chunks() {
        let p = png(3, 2, [255, 0, 0]);
        assert!(p.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&p[12..16], b"IHDR");
        assert!(p.ends_with(&crc32(b"IEND").to_be_bytes()));
    }
}
