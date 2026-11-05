//! Synthetic document fixtures shared by WordCraft's tests.
//!
//! Everything here is **our own markup, written from the public ECMA-376 / ISO 29500 spec** —
//! clean-room: nothing is exported from Microsoft Word, Mendeley, Elsevier, Zotero, Clarivate
//! or any other product. The payloads are deliberately synthetic (ids like `wc-synthetic-1`)
//! while keeping the shapes the spec allows: an instruction split across `w:instrText` runs,
//! `w:fldData` nested in the begin `w:fldChar`, `w:fldLock`, `w:dirty`, fields spanning
//! paragraphs, inside footnotes and inside tracked-insert ranges, and `docProps/custom.xml`.
//!
//! The crate exists so the DOCX reader/writer tests (`wordcraft-docx`) and the engine tests
//! (`wordcraft-engine`) exercise the **same** documents. It may only be used as a
//! dev-dependency (see `xtask` layering).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::Write;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties" Target="docProps/custom.xml"/></Relationships>"#;

const DOC_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/></Relationships>"#;

/// The Mendeley citation's field code, split across two `w:instrText` runs (Word splits long
/// codes; a formatting-only run sits between the pieces).
pub const MENDELEY_CITE_A: &str = r#" ADDIN MENDELEY_Citation {"citationId":"WC-SYNTH-1","#;
pub const MENDELEY_CITE_B: &str = r#""mendeleyIds":["wc-synthetic-1"]} "#;
/// The Mendeley citation's joined instruction, as it must read back.
pub const MENDELEY_CITE: &str = r#"ADDIN MENDELEY_Citation {"citationId":"WC-SYNTH-1","mendeleyIds":["wc-synthetic-1"]}"#;

/// The bibliography field code; its result spans two paragraphs.
pub const MENDELEY_BIBL_INSTR: &str = r#"ADDIN MENDELEY_Bibliography {"seed":"wc-synthetic"}"#;

/// A Zotero-style citation (CSL JSON payload) on a begin flagged `w:dirty`. The dirty begin is
/// forward-looking: the reader has no dirty handling today; it is asserted once #877 lands.
pub const ZOTERO_CITE_INSTR: &str = r#"ADDIN ZOTERO_ITEM CSL_CITATION {"citationID":"wc2","citationItems":[{"id":9,"uris":["wc://synthetic/2"]}],"properties":{"formattedCitation":"(Doe, 2020)"}}"#;

/// An EndNote-style citation carrying XML in its code, inside a tracked-insert range.
pub const EN_CITE_INSTR: &str = r#"ADDIN EN.CITE <EndNote><Cite><Author>Roe</Author><Year>2019</Year></Cite></EndNote>"#;

/// The citation field's `w:fldData` (base64 of `WC-SYN`, synthetic).
pub const FLD_DATA: &str = "V0MtU1lO";
/// The footnote citation's field code.
pub const NOTE_CITE_INSTR: &str = r#"ADDIN MENDELEY_Citation {"citationId":"WC-SYNTH-FN"}"#;
/// The footnote citation's `w:fldData` (base64 of `WC-SYN2`, synthetic).
pub const NOTE_FLD_DATA: &str = "V0MtU1lOMg==";

/// `docProps/custom.xml`: synthetic custom properties (a manager's settings block and a plain
/// document property), written from the OPC custom-properties schema.
fn custom_xml() -> String {
    concat!(
        r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties""#,
        r#" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">"#,
        r#"<property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="WC_PREF_1"><vt:lpwstr>&lt;settings synthetic="1"/&gt;</vt:lpwstr></property>"#,
        r#"<property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Citations"><vt:i4>4</vt:i4></property>"#,
        r#"</Properties>"#
    )
    .to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn fld(kind: &str) -> String {
    format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#)
}

/// The begin marker, with extra attributes (`w:fldLock`, `w:dirty`) and the spec's placement
/// of `w:fldData`: nested inside the `w:fldChar` begin element.
fn fld_begin(extra: &[&str], data: Option<&str>) -> String {
    let attrs: String = extra.iter().map(|a| format!(" {a}")).collect();
    match data {
        Some(d) => format!(r#"<w:r><w:fldChar w:fldCharType="begin"{attrs}><w:fldData xml:space="preserve">{d}</w:fldData></w:fldChar></w:r>"#),
        None => format!(r#"<w:r><w:fldChar w:fldCharType="begin"{attrs}/></w:r>"#),
    }
}

fn instr(s: &str) -> String {
    format!(r#"<w:r><w:instrText xml:space="preserve">{}</w:instrText></w:r>"#, esc(s))
}

/// An instruction run with formatting (Word writes codes with the result's formatting).
fn instr_fmt(s: &str) -> String {
    format!(r#"<w:r><w:rPr><w:b/><w:sz w:val="18"/></w:rPr><w:instrText xml:space="preserve">{}</w:instrText></w:r>"#, esc(s))
}

fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#, esc(s))
}

fn ti(s: &str) -> String {
    format!(r#"<w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r>"#, esc(s))
}

fn p(content: &str) -> String {
    format!("<w:p>{content}</w:p>")
}

fn cat(parts: &[&str]) -> String {
    parts.concat()
}

/// `word/footnotes.xml`: the separator plus one footnote whose text is a whole citation field
/// (locked, carrying its own `w:fldData`).
fn footnotes() -> String {
    let note = p(&cat(&[
        r#"<w:r><w:footnoteRef/></w:r>"#,
        &fld_begin(&[], Some(NOTE_FLD_DATA)),
        &instr(&format!(" {NOTE_CITE_INSTR} ")),
        &fld("separate"),
        &t(" (Foot, 2025)"),
        &fld("end"),
    ]));
    format!(
        r#"<w:footnotes {W_NS}><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1">{note}</w:footnote></w:footnotes>"#
    )
}

/// The document body: one citation field per manager variant plus the hard shapes. Paragraph
/// by paragraph — a Mendeley citation (locked, `w:fldData`, code split across two instruction
/// runs around a formatting-only run); a Zotero citation on a `w:dirty` begin; a Mendeley
/// bibliography opening in one paragraph and closing in the next; a footnote reference (the
/// note itself holds a citation, see [`footnotes`]); an EndNote citation inside a
/// tracked-insert range; a plain `PAGE` field; a trailing paragraph for edit-pass tests.
fn body() -> String {
    let mendeley = cat(&[
        &t("See "),
        &fld_begin(&[r#"w:fldLock="1""#], Some(FLD_DATA)),
        &instr(MENDELEY_CITE_A),
        // A formatting-only run between the code pieces: no text, still part of the code.
        r#"<w:r><w:rPr><w:b/><w:sz w:val="18"/></w:rPr></w:r>"#,
        &instr_fmt(MENDELEY_CITE_B),
        &fld("separate"),
        &t("(Austen, "),
        &ti("1813"),
        &t(")"),
        &fld("end"),
        &t("."),
    ]);
    let zotero = cat(&[
        &t("Also "),
        // Flagged dirty by its producer: the cached result is stale.
        &fld_begin(&[r#"w:dirty="1""#], None),
        &instr(&format!(" {ZOTERO_CITE_INSTR} ")),
        &fld("separate"),
        &t("(Doe, 2020)"),
        &fld("end"),
        &t("."),
    ]);
    let bibl_a = cat(&[
        &fld_begin(&[], None),
        &instr(&format!(" {MENDELEY_BIBL_INSTR} ")),
        &fld("separate"),
        &t("Austen, J. (1813). Pride and Prejudice. London."),
    ]);
    let bibl_b = cat(&[&t("Doe, J. (2020). A Study. Somewhere."), &fld("end")]);
    let note_ref = cat(&[&t("Cf. note"), r#"<w:r><w:footnoteReference w:id="1"/></w:r>"#, &t(".")]);
    let endnote = cat(&[
        &t("Tracked "),
        r#"<w:ins w:id="7" w:author="WC" w:date="2026-10-01T00:00:00Z">"#,
        &fld_begin(&[], None),
        &instr(&format!(" {EN_CITE_INSTR} ")),
        &fld("separate"),
        &t("(Roe, 2019)"),
        &fld("end"),
        "</w:ins>",
        &t("."),
    ]);
    let page = cat(&[&t("Page "), &fld_begin(&[], None), &instr(" PAGE "), &fld("separate"), &t("1"), &fld("end"), &t(".")]);
    [p(&mendeley), p(&zotero), p(&bibl_a), p(&bibl_b), p(&note_ref), p(&endnote), p(&page), p(&t("After."))].concat()
}

/// A whole synthetic package holding every citation-manager field variant and hard shape
/// (see [`body`]). Only I/O errors can fail.
pub fn manager_fields_docx() -> std::io::Result<Vec<u8>> {
    let doc = format!(r#"<w:document {W_NS}><w:body>{}</w:body></w:document>"#, body());
    zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()),
        ("word/footnotes.xml", footnotes().as_bytes()),
        ("docProps/custom.xml", custom_xml().as_bytes()),
    ])
}

/// A package of hostile field constructs: a truncated field (begin + code + `w:fldData`,
/// never closed), `w:instrText` outside any field, an unterminated JSON payload, `w:fldData`
/// that is not base64, `w:fldData` after the separator, and a single `w:instrText` run of
/// `instr_pad` payload bytes (over the reader's instruction cap when large). None of these
/// may panic the reader; each paragraph is checked in the tests.
pub fn hostile_fields_docx(instr_pad: usize) -> std::io::Result<Vec<u8>> {
    let instr_huge = format!(r#" ADDIN MENDELEY_Citation {{"pad":"{}"}}"#, "A".repeat(instr_pad));
    let paras = [
        // Truncated: no separate, no end.
        p(&cat(&[&fld_begin(&[r#"w:fldLock="1""#], Some(FLD_DATA)), &instr(r#" ADDIN MENDELEY_Citation {"citationId":"trunc"#)])),
        // Instruction text outside any field.
        p(&instr(" ADDIN ORPHAN_Citation {} ")),
        // Unterminated JSON payload, whole field.
        p(&cat(&[
            &fld_begin(&[], None),
            &instr(r#" ADDIN MENDELEY_Citation {"citationId":"unterminated "#),
            &fld("separate"),
            &t("(Un, 1)"),
            &fld("end"),
        ])),
        // Not base64 in the begin, and a fldData after the separator (Result phase: ignored).
        p(&cat(&[
            &fld_begin(&[], Some("%%%not base64%%%")),
            &instr(r#" ADDIN MENDELEY_Citation {"k":1} "#),
            &fld("separate"),
            r#"<w:r><w:fldData>aGVsbG8=</w:fldData></w:r>"#,
            &t("(Weird, 2)"),
            &fld("end"),
        ])),
        // One instruction run bigger than the cap, and a `w:fldData` just as big.
        p(&cat(&[&fld_begin(&[], Some(&"A".repeat(instr_pad))), &instr(&instr_huge), &fld("separate"), &t("(Big, 3)"), &fld("end")])),
        p(&t("After.")),
    ];
    let doc = format!(r#"<w:document {W_NS}><w:body>{}</w:body></w:document>"#, paras.concat());
    zip(&[
        ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
    ])
}

fn zip(entries: &[(&str, &[u8])]) -> std::io::Result<Vec<u8>> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default())?;
        zw.write_all(bytes)?;
    }
    Ok(zw.finish()?.into_inner())
}
