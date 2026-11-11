//! A field nested in another field's code (`{ IF { REF amount } = 0 … }`) survives a round trip
//! with its own begin, code, result and end, and the outer field keeps its cached result (#1671).
//! All markup here is synthetic, written from ECMA-376 Part 1 §17.16 (complex fields nest via
//! `w:fldChar` begin / separate / end).

use std::io::Write;

use std::sync::Arc;

use wordcraft_doc::{Block, Document, Revision, RevisionKind, StoryRef};
use wordcraft_docx::{read, write};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

fn package(body: &str) -> Vec<u8> {
    let doc = format!(r#"<w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn document_xml(docx: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut s).unwrap();
    s
}

fn resaved(body: &str) -> String {
    document_xml(&write(&read(&package(body)).unwrap()).unwrap())
}

fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}
fn fld(kind: &str) -> String {
    format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#)
}
fn instr(s: &str) -> String {
    format!(r#"<w:r><w:instrText xml:space="preserve">{s}</w:instrText></w:r>"#)
}

/// `{ REF amount }` with cached result `0`.
fn ref_amount() -> String {
    format!("{}{}{}{}{}", fld("begin"), instr(" REF amount "), fld("separate"), t("0"), fld("end"))
}

/// The issue's fixture: bookmark `amount` = 0; `{ IF { REF amount } = 0 "…" "…" }`;
/// `{ = { REF amount } + 5 }`.
fn fixture() -> String {
    format!(
        concat!(
            r#"<w:p><w:bookmarkStart w:id="0" w:name="amount"/>{}<w:bookmarkEnd w:id="0"/></w:p>"#,
            "<w:p>{}{}{}{}{}{}{}</w:p>",
            "<w:p>{}{}{}{}{}{}{}</w:p>",
        ),
        t("0"),
        fld("begin"),
        instr(" IF "),
        ref_amount(),
        instr(r#" = 0 "amount is zero" "amount is not zero" "#),
        fld("separate"),
        t("amount is zero"),
        fld("end"),
        fld("begin"),
        instr(" = "),
        ref_amount(),
        instr(" + 5 "),
        fld("separate"),
        t("5"),
        fld("end"),
    )
}

/// The structure of every field in `xml`, in document order: `[` for begin, `|` for separate,
/// `]` for end, and each `w:instrText`'s text in between.
fn field_marks(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    loop {
        let next = ["<w:fldChar ", "<w:instrText", "<w:delInstrText"].iter().filter_map(|m| rest.find(m).map(|i| (i, *m))).min();
        let Some((at, m)) = next else { break };
        rest = &rest[at..];
        if m == "<w:fldChar " {
            let end = rest.find('>').unwrap();
            let tag = &rest[..end];
            out.push(if tag.contains("\"begin\"") {
                '['
            } else if tag.contains("\"separate\"") {
                '|'
            } else {
                ']'
            });
            rest = &rest[end..];
        } else {
            let open = rest.find('>').unwrap() + 1;
            let close = rest.find("</w:instrText>").or_else(|| rest.find("</w:delInstrText>")).unwrap();
            out.push_str(&rest[open..close].trim().replace("&quot;", "\""));
            rest = &rest[close..];
        }
    }
    out
}

#[test]
fn nested_field_codes_survive_a_round_trip() {
    let xml = resaved(&fixture());
    assert_eq!(xml.matches(r#"w:fldCharType="begin""#).count(), 4, "{xml}");
    assert_eq!(field_marks(&xml), concat!(r#"[IF[REF amount|]= 0 "amount is zero" "amount is not zero"|]"#, "[=[REF amount|]+ 5|]"), "{xml}");
    // The nested REF fields keep their cached results.
    assert_eq!(xml.matches(r#"<w:t xml:space="preserve">0</w:t>"#).count(), 3, "{xml}");
}

#[test]
fn nested_field_outer_keeps_its_cached_result_as_displayed_text() {
    let doc = read(&package(&fixture())).unwrap();
    assert_eq!(doc.plain_text(StoryRef::Body), "0\namount is zero\n5");
    // And the result is still there after saving and reading back.
    let again = read(&write(&doc).unwrap()).unwrap();
    assert_eq!(again.plain_text(StoryRef::Body), "0\namount is zero\n5");
    // A second round trip keeps the nesting too.
    let xml = document_xml(&write(&again).unwrap());
    assert_eq!(xml.matches(r#"w:fldCharType="begin""#).count(), 4, "{xml}");
}

#[test]
fn nested_fields_three_deep_survive() {
    let inner = format!("{}{}{}{}{}", fld("begin"), instr(" PAGE "), fld("separate"), t("1"), fld("end"));
    let mid = format!("{}{}{}{}{}{}", fld("begin"), instr(" = "), inner, instr(" + 1 "), fld("separate"), t("2"));
    let body = format!(
        "<w:p>{}{}{}{}{}{}{}{}</w:p>",
        fld("begin"),
        instr(" IF "),
        mid,
        fld("end"),
        instr(r#" = 2 "two" "other" "#),
        fld("separate"),
        t("two"),
        fld("end")
    );
    let xml = resaved(&body);
    assert_eq!(field_marks(&xml), r#"[IF[=[PAGE|]+ 1|]= 2 "two" "other"|]"#, "{xml}");
}

#[test]
fn a_field_without_nesting_is_written_as_before() {
    let body = format!("<w:p>{}{}{}{}{}</w:p>", fld("begin"), instr(" REF amount "), fld("separate"), t("0"), fld("end"));
    let xml = resaved(&body);
    assert_eq!(field_marks(&xml), "[REF amount|]", "{xml}");
    // The writer's own form for a field: a space-padded code in one instrText.
    assert!(xml.contains(r#"<w:instrText xml:space="preserve"> REF amount </w:instrText>"#), "{xml}");
}

#[test]
fn hostile_nested_fields_never_panic() {
    // Deeply nested begins in a code, never ended.
    let deep: String = (0..200).map(|_| format!("{}{}", fld("begin"), instr(" IF "))).collect();
    let _ = read(&package(&format!("<w:p>{deep}</w:p>"))).map(|d| write(&d));
    // Deeply nested and properly ended.
    let mut nested = format!("{}{}{}", fld("begin"), instr(" PAGE "), fld("end"));
    for _ in 0..100 {
        nested = format!("{}{}{}{}{}", fld("begin"), instr(" = "), nested, fld("separate"), fld("end"));
    }
    let doc = read(&package(&format!("<w:p>{nested}</w:p>"))).unwrap();
    let xml = document_xml(&write(&doc).unwrap());
    let opens = xml.matches(r#"w:fldCharType="begin""#).count();
    let ends = xml.matches(r#"w:fldCharType="end""#).count();
    assert_eq!(opens, ends, "{xml}");
    // A nested field that ends after a separate and an unmatched end in the code.
    let odd = format!("<w:p>{}{}{}{}{}{}{}</w:p>", fld("begin"), instr(" IF "), fld("begin"), fld("separate"), fld("end"), fld("end"), fld("end"));
    let _ = read(&package(&odd)).map(|d| write(&d));
    // Special characters in nested codes are escaped.
    let body = format!(
        "<w:p>{}{}{}{}{}{}{}{}</w:p>",
        fld("begin"),
        instr(" IF "),
        fld("begin"),
        instr(" REF a&amp;b "),
        fld("separate"),
        t("&lt;x&gt;"),
        fld("end"),
        fld("end")
    );
    let xml = resaved(&body);
    assert!(xml.contains("REF a&amp;b"), "{xml}");
    assert!(xml.contains("&lt;x&gt;"), "{xml}");
}

/// The `n`th paragraph of `xml` (`<w:p>` … `</w:p>`).
fn nth_para(xml: &str, n: usize) -> &str {
    let at = xml.match_indices("<w:p>").chain(xml.match_indices("<w:p ")).map(|(i, _)| i).collect::<std::collections::BTreeSet<_>>();
    let start = *at.iter().nth(n).unwrap();
    let end = start + xml[start..].find("</w:p>").unwrap();
    &xml[start..end]
}

/// The fixture with its second paragraph (the nested IF field) given `rev` (a tracked insertion
/// or deletion) and bold, as Track Changes and Ctrl+B would.
fn tracked_if(kind: RevisionKind) -> Document {
    let mut doc = read(&package(&fixture())).unwrap();
    doc.revisions.push(Revision { kind, author: "Ada".into(), date: "2026-10-15T10:00:00Z".into(), move_name: None });
    let rev = Some(u32::try_from(doc.revisions.len() - 1).unwrap());
    let Block::Para(p) = Arc::make_mut(&mut doc.body[1]) else { panic!("paragraph") };
    for r in &mut p.runs {
        r.props.bold = Some(true);
        if kind == RevisionKind::Delete {
            r.props.del = rev;
        } else {
            r.props.ins = rev;
        }
    }
    doc
}

const IF_MARKS: &str = r#"[IF[REF amount|]= 0 "amount is zero" "amount is not zero"|]"#;

#[test]
fn a_tracked_deleted_bold_nested_field_saves_deleted_bold_and_nested() {
    let xml = document_xml(&write(&tracked_if(RevisionKind::Delete)).unwrap());
    let p = nth_para(&xml, 1);
    assert_eq!(field_marks(p), IF_MARKS, "{p}");
    assert!(p.starts_with("<w:p") && p.contains("<w:del ") && p.contains("<w:b/>"), "{p}");
    // Deleted text is `w:delText` / `w:delInstrText` throughout, the nested field's too.
    assert!(!p.contains("<w:instrText") && !p.contains("<w:t ") && !p.contains("<w:t>"), "{p}");
    assert!(p.contains(r#"<w:delText xml:space="preserve">0</w:delText>"#), "{p}");
    // Read back: still deleted, still nested.
    let again = read(&write(&tracked_if(RevisionKind::Delete)).unwrap()).unwrap();
    let xml = document_xml(&write(&again).unwrap());
    let p = nth_para(&xml, 1);
    assert_eq!(field_marks(p), IF_MARKS, "{p}");
    assert!(p.contains("<w:del "), "{p}");
}

#[test]
fn a_tracked_inserted_nested_field_saves_inserted_and_nested() {
    let xml = document_xml(&write(&tracked_if(RevisionKind::Insert)).unwrap());
    let p = nth_para(&xml, 1);
    assert_eq!(field_marks(p), IF_MARKS, "{p}");
    assert!(p.contains("<w:ins ") && p.contains("<w:b/>") && p.contains("<w:instrText"), "{p}");
    // Everything in the paragraph sits inside the insertion.
    let (open, close) = (p.find("<w:ins ").unwrap(), p.rfind("</w:ins>").unwrap());
    assert!(open < p.find(r#"w:fldCharType="begin""#).unwrap() && close > p.rfind(r#"w:fldCharType="end""#).unwrap(), "{p}");
}

// Where the nesting can't be kept, the inner field's cached result stands in for it in the code,
// as before #1671 (documented in docs/file-format-parity.md, Fields row).

#[test]
fn fallback_a_tracked_change_inside_the_code_flattens_the_nested_field() {
    let ins = r#"<w:ins w:id="1" w:author="Ada" w:date="2026-10-15T10:00:00Z"><w:r><w:instrText xml:space="preserve"> = 0 "z" "nz" </w:instrText></w:r></w:ins>"#;
    let body = format!("<w:p>{}{}{}{ins}{}{}{}</w:p>", fld("begin"), instr(" IF "), ref_amount(), fld("separate"), t("z"), fld("end"));
    let xml = resaved(&body);
    assert_eq!(field_marks(&xml), r#"[IF 0 = 0 "z" "nz"|]"#, "{xml}");
}

#[test]
fn fallback_a_code_over_the_size_cap_flattens_the_nested_field() {
    let long = "x".repeat(2 * 1024 * 1024 + 16);
    let inner = format!("{}{}{}{}{}", fld("begin"), instr(&format!(" QUOTE {long} ")), fld("separate"), t("q"), fld("end"));
    let body = format!("<w:p>{}{}{inner}{}{}{}</w:p>", fld("begin"), instr(" IF "), fld("separate"), t("r"), fld("end"));
    let xml = resaved(&body);
    assert_eq!(xml.matches(r#"w:fldCharType="begin""#).count(), 1);
    assert!(field_marks(&xml).starts_with("[IF q"), "{}", &field_marks(&xml)[..40]);
}

#[test]
fn fallback_a_result_spanning_paragraphs_flattens_the_nested_field() {
    let body = format!(
        "<w:p>{}{}{}{}{}{}</w:p><w:p>{}{}</w:p>",
        fld("begin"),
        instr(" IF "),
        ref_amount(),
        instr(r#" = 0 "a b" "c" "#),
        fld("separate"),
        t("a"),
        t("b"),
        fld("end")
    );
    let xml = resaved(&body);
    assert_eq!(xml.matches(r#"w:fldCharType="begin""#).count(), 1, "{xml}");
    assert!(field_marks(&xml).starts_with(r#"[IF 0 = 0 "a b" "c"|]"#), "{xml}");
}

#[test]
fn fallback_a_result_kept_as_a_range_flattens_the_nested_field() {
    // A tracked insertion inside the result (as a cached picture would) keeps the field as a range.
    let ins = format!(r#"<w:ins w:id="1" w:author="Ada" w:date="2026-10-15T10:00:00Z">{}</w:ins>"#, t("new"));
    let body = format!(
        "<w:p>{}{}{}{}{}{}{ins}{}</w:p>",
        fld("begin"),
        instr(" IF "),
        ref_amount(),
        instr(r#" = 0 "z" "nz" "#),
        fld("separate"),
        t("z"),
        fld("end")
    );
    let xml = resaved(&body);
    assert_eq!(field_marks(&xml), r#"[IF 0 = 0 "z" "nz"|]"#, "{xml}");
    assert!(xml.contains("<w:ins "), "{xml}");
}

#[test]
fn page_and_column_breaks_in_a_cached_result_are_kept() {
    let brk = |ty: &str| format!(r#"<w:r><w:br w:type="{ty}"/></w:r>"#);
    let body = format!(
        "<w:p>{}{}{}{}{}{}{}{}</w:p>",
        fld("begin"),
        instr(" QUOTE x "),
        fld("separate"),
        t("a"),
        brk("page"),
        t("b"),
        brk("column"),
        fld("end")
    );
    let xml = resaved(&body);
    assert!(xml.contains(r#"<w:br w:type="page"/>"#) && xml.contains(r#"<w:br w:type="column"/>"#), "{xml}");
}

/// An `IF` field whose `code_xml` is `code` (as a hostile `.json` or recovery file could hold).
fn crafted(code: &str) -> String {
    let mut p = wordcraft_doc::Paragraph::with_text("", wordcraft_doc::props::CharProps::default());
    let obj = wordcraft_doc::InlineObject::Field {
        instr: r#"IF 1 = 1 "a" "b""#.into(),
        result: "a".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
        code_xml: code.into(),
    };
    p.insert_object(0, obj, &wordcraft_doc::props::CharProps::default()).unwrap();
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(p)];
    document_xml(&write(&d).unwrap())
}

#[test]
fn a_malformed_or_unbalanced_nested_code_is_written_as_the_flat_code() {
    let ok = format!("{}{}{}{}{}", instr(" IF "), ref_amount(), instr(" = 0 "), "", "");
    assert_eq!(field_marks(&crafted(&ok)), "[IF[REF amount|]= 0|]", "a well-formed code is kept");
    let flat = r#"[IF 1 = 1 "a" "b"|]"#;
    for bad in [
        // Elements a field code's runs never hold.
        r#"<w:r><w:drawing><w:inline r:id="rId99"/></w:drawing></w:r>"#.to_string(),
        "<w:r><w:p/></w:r>".to_string(),
        r#"<w:r><w:rPr><w:p/></w:rPr><w:instrText>IF</w:instrText></w:r>"#.to_string(),
        "<w:r>loose text</w:r>".to_string(),
        "<w:p><w:r><w:instrText>IF</w:instrText></w:r></w:p>".to_string(),
        // Unbalanced nested fields.
        format!("{}{}{}", instr(" IF "), fld("begin"), instr(" PAGE ")),
        format!("{}{}{}", instr(" IF "), fld("end"), fld("begin")),
        format!("{}{}{}{}{}", instr(" IF "), fld("begin"), fld("separate"), fld("separate"), fld("end")),
        format!("{}{}", instr(" IF "), fld("separate")),
    ] {
        let xml = crafted(&bad);
        assert_eq!(field_marks(&xml), flat, "{bad}\n{xml}");
        assert!(!xml.contains("<w:p/>") && !xml.contains("rId99") && !xml.contains("loose"), "{xml}");
    }
}

#[test]
fn fallback_a_tracked_change_on_the_code_before_a_nested_field_flattens_it() {
    let ins =
        r#"<w:ins w:id="1" w:author="Ada" w:date="2026-10-15T10:00:00Z"><w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r></w:ins>"#;
    let body = format!("<w:p>{}{ins}{}{}{}{}{}</w:p>", fld("begin"), ref_amount(), instr(r#" = 0 "z" "nz" "#), fld("separate"), t("z"), fld("end"));
    assert_eq!(field_marks(&resaved(&body)), r#"[IF 0 = 0 "z" "nz"|]"#);
}

#[test]
fn fallback_a_tracked_change_in_a_nested_fields_own_code_flattens_it() {
    let ins = r#"<w:ins w:id="1" w:author="Ada" w:date="2026-10-15T10:00:00Z"><w:r><w:instrText xml:space="preserve"> REF amount </w:instrText></w:r></w:ins>"#;
    let inner = format!("{}{ins}{}{}{}", fld("begin"), fld("separate"), t("0"), fld("end"));
    let body =
        format!("<w:p>{}{}{inner}{}{}{}{}</w:p>", fld("begin"), instr(" IF "), instr(r#" = 0 "z" "nz" "#), fld("separate"), t("z"), fld("end"));
    assert_eq!(field_marks(&resaved(&body)), r#"[IF 0 = 0 "z" "nz"|]"#);
}

#[test]
fn fallback_a_tracked_formatting_change_in_the_code_flattens_it() {
    let changed = r#"<w:r><w:rPr><w:b/><w:rPrChange w:id="1" w:author="Ada" w:date="2026-10-15T10:00:00Z"><w:rPr/></w:rPrChange></w:rPr><w:instrText xml:space="preserve"> = 0 "z" "nz" </w:instrText></w:r>"#;
    let body = format!("<w:p>{}{}{}{changed}{}{}{}</w:p>", fld("begin"), instr(" IF "), ref_amount(), fld("separate"), t("z"), fld("end"));
    assert_eq!(field_marks(&resaved(&body)), r#"[IF 0 = 0 "z" "nz"|]"#);
}

#[test]
fn a_dirty_flag_on_a_nested_field_survives() {
    // `w:dirty` (#877) on the outer field and on a nested field's begin both survive a round trip.
    let dirty = r#"<w:r><w:fldChar w:fldCharType="begin" w:dirty="true"/></w:r>"#;
    let inner = format!("{dirty}{}{}{}{}", instr(" REF amount "), fld("separate"), t("0"), fld("end"));
    let body = format!("<w:p>{dirty}{}{inner}{}{}{}{}</w:p>", instr(" IF "), instr(r#" = 0 "z" "nz" "#), fld("separate"), t("z"), fld("end"));
    let xml = resaved(&body);
    assert_eq!(field_marks(&xml), r#"[IF[REF amount|]= 0 "z" "nz"|]"#, "{xml}");
    assert_eq!(xml.matches(r#"w:dirty="true""#).count(), 2, "{xml}");
}
