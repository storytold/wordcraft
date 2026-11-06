//! Synthetic numbered-reference regression adapted from the patch by @Alyosha-dot-shop in #4533.
//! Switch semantics: Microsoft MS-OI29500, Part 1 Section 17.16.5.51 (REF).
//! All OOXML packages and document text are constructed in memory; no external documents.

use std::io::Write;

use serde_json::json;
use wordcraft_doc::{InlineObject, StoryRef};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// Build an OOXML ZIP entirely in memory.
fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(body.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// One synthetic numbering level.
fn lvl(ilvl: u8, fmt: &str, text: &str) -> String {
    format!(r#"<w:lvl w:ilvl="{ilvl}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/><w:lvlText w:val="{text}"/></w:lvl>"#)
}

/// Five independent list schemes for context and restart coverage.
fn numbering() -> String {
    let schemes: [&[(&str, &str)]; 5] = [
        &[("decimal", "%1."), ("decimal", "%1.%2"), ("lowerLetter", "(%3)")],
        &[("decimal", "%1"), ("decimal", "%1.%2"), ("decimal", "%1.%2.%3")],
        &[("decimal", "%1"), ("decimal", "%1.%2"), ("lowerLetter", "(%3)")],
        &[("decimal", "%1."), ("lowerLetter", "(%2)")],
        // Scheme 5: as scheme 1, a list of its own whose first two levels are never used.
        &[("decimal", "%1."), ("decimal", "%1.%2"), ("lowerLetter", "(%3)")],
    ];
    let mut out = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering {W}>"#);
    for (i, levels) in schemes.iter().enumerate() {
        out.push_str(&format!(r#"<w:abstractNum w:abstractNumId="{i}"><w:multiLevelType w:val="multilevel"/>"#));
        for (k, (fmt, text)) in levels.iter().enumerate() {
            out.push_str(&lvl(k as u8, fmt, text));
        }
        out.push_str("</w:abstractNum>");
    }
    for i in 0..schemes.len() {
        out.push_str(&format!(r#"<w:num w:numId="{}"><w:abstractNumId w:val="{i}"/></w:num>"#, i + 1));
    }
    // A second instance of scheme 4.
    out.push_str(r#"<w:num w:numId="7"><w:abstractNumId w:val="3"/></w:num>"#);
    out + "</w:numbering>"
}

/// A paragraph, numbered `(numId, level)` or not, holding `content`.
fn para(num: Option<(u32, u8)>, content: &str) -> String {
    let ppr = num.map(|(n, l)| format!(r#"<w:pPr><w:numPr><w:ilvl w:val="{l}"/><w:numId w:val="{n}"/></w:numPr></w:pPr>"#)).unwrap_or_default();
    format!("<w:p>{ppr}{content}</w:p>")
}

/// One synthetic text run.
fn text(t: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{t}</w:t></w:r>"#)
}

/// A bookmark surrounding a synthetic text run.
fn marked(name: &str, id: u32, t: &str) -> String {
    format!(r#"<w:bookmarkStart w:id="{id}" w:name="{name}"/>{}<w:bookmarkEnd w:id="{id}"/>"#, text(t))
}

/// A complex REF field with an intentionally stale result.
fn field(instr: &str, cached: &str) -> String {
    format!(
        concat!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r>"#,
            r#"<w:r><w:instrText xml:space="preserve"> {} </w:instrText></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r>"#,
            r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        ),
        instr, cached
    )
}

/// Open a synthetic OOXML package through the real document reader.
fn open(body: &str) -> wordcraft_engine::Session {
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#;
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
    let numbering = numbering();
    let bytes =
        zip(&[("_rels/.rels", rels), ("word/_rels/document.xml.rels", doc_rels), ("word/document.xml", &doc), ("word/numbering.xml", &numbering)]);
    wordcraft_engine::Session::new(wordcraft_engine::io::open_bytes("d.docx", &bytes).expect("opens"))
}

/// Every body field, as (code, result), in document order.
fn fields(s: &wordcraft_engine::Session) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for o in &p.objects {
            if let InlineObject::Field { instr, result, .. } = o {
                out.push((instr.trim().to_string(), result.clone()));
            }
        }
    }
    out
}

/// Numeric REF switches replace cached values with list labels, while plain REF and position switches retain their behavior.
/// Every expected result is what Word for Windows shows for this document after F9.
#[test]
fn ref_number_switches_show_the_paragraph_number_as_word_does() {
    let body = [
        // Scheme 1: %1. / %1.%2 / (%3) — clause (a) of 1.1.
        para(Some((1, 0)), &text("Definitions")),
        para(Some((1, 1)), &text("Defined terms")),
        para(Some((1, 2)), &marked("_RefA", 1, "Synthetic target alpha.")),
        // Scheme 2: the stock chapter scheme %1 / %1.%2 / %1.%2.%3.
        para(Some((2, 0)), &text("Chapter")),
        para(Some((2, 1)), &text("Section")),
        para(Some((2, 2)), &marked("_RefB", 2, "Point")),
        // Scheme 3: %1 / %1.%2 / (%3).
        para(Some((3, 0)), &text("Part")),
        para(Some((3, 1)), &text("Part point")),
        para(Some((3, 2)), &marked("_RefC", 3, "Sub-point")),
        // Scheme 4: %1. / (%2), clauses and sub-clauses as in most contracts.
        para(Some((4, 0)), &text("Hire")),
        para(Some((4, 1)), &text("first")),
        para(Some((4, 0)), &marked("_RefD", 4, "Payment")),
        para(Some((4, 1)), &text("a")),
        para(Some((4, 1)), &marked("_RefE", 5, "b")),
        // 2(c), inside clause 2: relative references drop the shared clause number.
        para(Some((4, 1)), &format!("{}{}{}", text("See "), field(r"REF _RefE \r \h", "STALE"), field(r"REF _RefD \r \h", "STALE"))),
        para(Some((4, 0)), &text("Third")),
        // 3(a), another clause: the full number. _RefF's list has no clauses above its items,
        // whose levels show the number they start at, and nothing of that list comes before.
        para(Some((4, 1)), &format!("{}{}", field(r"REF _RefE \r \h", "STALE"), field(r"REF _RefF \r \h", "STALE"))),
        // Scheme 5 items in a table whose second row's cell is merged into the first: the empty
        // item in the merged-away cell takes no number, so the next item is (b), not (c).
        format!(
            concat!(
                r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>"#,
                r#"<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{}</w:tc></w:tr>"#,
                r#"<w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr>{}</w:tc></w:tr></w:tbl>"#,
            ),
            para(Some((5, 2)), &marked("_RefF", 6, "Target F")),
            para(Some((5, 2)), ""),
        ),
        para(Some((5, 2)), &marked("_RefG", 7, "Target G")),
        // An unnumbered paragraph: `\r` takes the context of each target's list as it stands here.
        para(
            None,
            &[
                field(r"REF _RefA \w \h", "STALE"),
                field(r"REF _RefA \n \h", "STALE"),
                field(r"REF _RefA \r \h", "STALE"),
                field(r"REF _RefB \w \h", "STALE"),
                field(r"REF _RefC \w \h", "STALE"),
                field(r"REF _RefE \r \h", "STALE"),
                field(r"REF _RefD \n \h", "STALE"),
                field(r"REF _RefG \r \h", "STALE"),
                // No number switch shows the text; \p adds where the target is.
                field(r"REF _RefB \h", "STALE"),
                field(r"REF _RefA \r \p", "STALE"),
            ]
            .concat(),
        ),
    ]
    .concat();
    let mut s = open(&body);
    s.run("references.updateFields", &json!({"all": true})).expect("updates");
    let got: Vec<(String, String)> = fields(&s);
    let want = [
        (r"REF _RefE \r \h", "(b)"),
        (r"REF _RefD \r \h", "2"),
        (r"REF _RefE \r \h", "2(b)"),
        (r"REF _RefF \r \h", "1.1(a)"),
        (r"REF _RefA \w \h", "1.1(a)"),
        (r"REF _RefA \n \h", "(a)"),
        (r"REF _RefA \r \h", "(a)"),
        (r"REF _RefB \w \h", "1.1.1"),
        (r"REF _RefC \w \h", "1.1(a)"),
        (r"REF _RefE \r \h", "2(b)"),
        (r"REF _RefD \n \h", "2"),
        (r"REF _RefG \r \h", "(b)"),
        (r"REF _RefB \h", "Point"),
        (r"REF _RefA \r \p", "(a) above"),
    ];
    let want: Vec<(String, String)> = want.iter().map(|(i, r)| (i.to_string(), r.to_string())).collect();
    assert_eq!(got, want);
}

/// Word retains context already rendered by the target's own label for relative REF.
#[test]
fn relative_number_keeps_context_already_in_the_target_label() {
    let body = [
        para(Some((2, 0)), &text("Root")),
        para(Some((2, 1)), &text("First section")),
        para(Some((2, 2)), &marked("_Target", 20, "Synthetic target")),
        para(Some((2, 2)), &field(r"REF _Target \r", "STALE")),
        para(Some((2, 1)), &text("Other section")),
        para(Some((2, 2)), &field(r"REF _Target \r", "STALE")),
        para(None, &field(r"REF _Target \w", "STALE")),
    ]
    .concat();
    let mut s = open(&body);
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["1.1.1", "1.1.1", "1.1.1"]);
}

/// Unsupported switches (`\d`, `\t`) and locked fields keep their cached values, never the
/// paragraph's text. As in Word for Windows, a missing bookmark gives Word's error and a
/// paragraph with no number shows 0.
#[test]
fn unsupported_numeric_refs_keep_their_cache_and_unresolvable_ones_show_what_word_does() {
    let body = [
        para(Some((1, 0)), &marked("_Root", 30, "Synthetic root")),
        para(
            None,
            &[
                field(r"REF _Root \w \d /", "KEEP DELIMITER"),
                field(r"REF _Root \n \t", "KEEP TEXT"),
                field(r"REF _Root \p \d /", "KEEP POSITION"),
                field(r"REF _Missing \w", "STALE"),
                field(r"REF _Missing \p", "STALE"),
                field(r"REF _Plain \n", "STALE"),
                field(r"REF _Root \n", "KEEP LOCKED").replace(r#"w:fldCharType="begin""#, r#"w:fldCharType="begin" w:fldLock="true""#),
            ]
            .concat(),
        ),
        para(None, &marked("_Plain", 31, "Synthetic unnumbered target")),
    ]
    .concat();
    let mut s = open(&body);
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    let error = wordcraft_engine::cmd::references::REFERENCE_NOT_FOUND;
    assert_eq!(got, ["KEEP DELIMITER", "KEEP TEXT", "KEEP POSITION", error, error, "0", "KEEP LOCKED"]);
}

/// `\p` says where the bookmark is from the field: "above" or "below", also within one
/// paragraph, after the number when a number switch comes with it, in the case a case switch
/// asks for. A bullet item's number is its bullet, and a paragraph with no number, or numbered
/// at a level whose format is none, is 0. Every value measured in Word for Windows.
#[test]
fn position_bullets_and_unnumbered_targets_show_what_word_does() {
    let refs = |cached: &str| {
        [
            field(r"REF _T \p", cached),
            field(r"REF _T \r \p", cached),
            field(r"REF _T \w \p", cached),
            field(r"REF _U \p", cached),
            field(r"REF _U \r \p", cached),
            field(r"REF _T \p \* Upper", cached),
        ]
        .concat()
    };
    let mut s = open(
        &[
            para(None, &refs("before")),
            para(Some((4, 0)), &text("Synthetic one")),
            para(Some((4, 1)), &[field(r"REF _T \p", "own"), marked("_T", 100, "Synthetic target"), field(r"REF _T \r \p", "own")].concat()),
            para(None, &marked("_U", 101, "Synthetic plain")),
            para(None, &refs("after")),
        ]
        .concat(),
    );
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    assert_eq!(
        got,
        [
            "below",
            "1(a) below",
            "1(a) below",
            "below",
            "0 below",
            "BELOW", //
            "below",
            "(a) above", //
            "above",
            "(a) above",
            "1(a) above",
            "above",
            "0 above",
            "ABOVE",
        ]
    );
}

/// A bullet item shows its bullet, a Symbol-font bullet as its Unicode character; a level whose
/// format is none shows 0 (Word for Windows).
#[test]
fn references_to_bullets_and_none_format_levels() {
    let mut s = open(
        &[
            para(Some((11, 0)), &marked("_Bullet", 110, "Synthetic bullet item")),
            para(Some((12, 0)), &marked("_None", 111, "Synthetic none-format item")),
            para(None, &[field(r"REF _Bullet \r", "STALE"), field(r"REF _Bullet \w \p", "STALE"), field(r"REF _None \w", "STALE")].concat()),
        ]
        .concat(),
    );
    let numbering = std::sync::Arc::make_mut(&mut s.doc.numbering);
    for (id, format, text, font) in
        [(11u32, wordcraft_doc::section::NumFormat::Bullet, "\u{F0B7}", Some("Symbol")), (12, wordcraft_doc::section::NumFormat::None, "", None)]
    {
        let mut n = numbering.nums.iter().find(|n| n.id == 1).unwrap().clone();
        n.id = id;
        let mut lvl = wordcraft_doc::numbering::Level { format, text: text.into(), ..Default::default() };
        lvl.chr.font = font.map(Into::into);
        n.level_overrides = vec![(0, lvl)];
        numbering.nums.push(n);
    }
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["\u{2022}", "\u{2022} above", "0"]);
}

/// Updating, undoing and saving keep the field instructions and their numeric results intact.
#[test]
fn numeric_refs_survive_undo_redo_and_docx_round_trip() {
    let body = [
        para(Some((4, 0)), &text("Root")),
        para(Some((4, 1)), &marked("_Item", 40, "Synthetic item șțăîâ")),
        para(None, &field(r"REF _Item \w \h", "STALE")),
    ]
    .concat();
    let mut s = open(&body);
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    assert_eq!(fields(&s), [(r"REF _Item \w \h".into(), "1(a)".into())]);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(fields(&s)[0].1, "STALE");
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(fields(&s)[0].1, "1(a)");
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let back = wordcraft_engine::Session::new(wordcraft_engine::io::open_bytes("roundtrip.docx", &bytes).unwrap());
    assert_eq!(fields(&back), fields(&s));
}

/// Context already displayed by a needed parent label is retained as Word measured it.
#[test]
fn relative_number_from_the_root_keeps_the_parents_rendered_context() {
    let body = [
        para(Some((1, 0)), &field(r"REF _Target \r", "STALE")),
        para(Some((1, 1)), &text("Synthetic section")),
        para(Some((1, 2)), &marked("_Target", 50, "Synthetic target")),
    ]
    .concat();
    let mut s = open(&body);
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    assert_eq!(fields(&s)[0].1, "1.1(a)");
}

/// A header can reference a body list without inheriting the body caret's relative context.
#[test]
fn numeric_refs_in_headers_use_the_body_target_number() {
    use wordcraft_doc::{Paragraph, PartKind, para_block};
    let mut s = open(&[para(Some((4, 0)), &text("Synthetic root")), para(Some((4, 1)), &marked("_Target", 60, "Synthetic target"))].concat());
    let mut header = Paragraph::new();
    header
        .insert_object(
            0,
            InlineObject::Field {
                instr: r"REF _Target \r \h".into(),
                result: "STALE".into(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &Default::default(),
        )
        .unwrap();
    let id = s.doc.add_part(PartKind::Header, vec![para_block(header)]);
    s.doc.last_section.headers.default = Some(id);
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let p = s.doc.para(StoryRef::Part(id), &wordcraft_doc::Path::top(0)).unwrap();
    let InlineObject::Field { result, .. } = &p.objects[0] else { panic!() };
    assert_eq!(result, "1(a)");
}

/// Separate bare numeric level labels cannot be concatenated without inventing a delimiter.
#[test]
fn ambiguous_numeric_label_join_keeps_the_cached_full_context() {
    let mut s = open(
        &[
            para(Some((2, 0)), &text("Root")),
            para(Some((2, 1)), &text("Section")),
            para(Some((2, 2)), &marked("_Target", 70, "Synthetic target")),
            para(None, &field(r"REF _Target \w", "1.1.1")),
            para(None, &field(r"REF _Target \n", "STALE")),
        ]
        .concat(),
    );
    let abs = std::sync::Arc::make_mut(&mut s.doc.numbering).abstracts.iter_mut().find(|a| a.id == 1).unwrap();
    abs.levels[1].text = "%2".into();
    abs.levels[2].text = "%3".into();
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, s)| s).collect();
    assert_eq!(got, ["1.1.1", "1"]);
}

/// Reusing the lookup keeps the first body bookmark even with duplicate names and many updates.
#[test]
fn repeated_numeric_refs_keep_the_first_body_bookmark() {
    use wordcraft_doc::{PartKind, Path};
    let mut body = [
        para(Some((4, 0)), &marked("_Duplicate", 80, "First synthetic target")),
        para(Some((4, 0)), &marked("_Duplicate", 81, "Second synthetic target")),
    ]
    .concat();
    for _ in 0..64 {
        body.push_str(&para(None, &field(r"REF _Duplicate \w \h", "STALE")));
    }
    body.push_str(&para(None, &field(r"REF _Duplicate \h", "STALE")));
    let mut s = open(&body);
    assert_eq!(s.doc.bookmarks().iter().filter(|(name, _)| name == "_Duplicate").count(), 2);
    // A same-named bookmark outside the body cannot replace the body target.
    let header = open(&para(None, &marked("_Duplicate", 82, "Header synthetic target"))).doc.body[0].clone();
    let id = s.doc.add_part(PartKind::Header, vec![header]);
    s.doc.last_section.headers.default = Some(id);
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    let got = fields(&s);
    assert_eq!(got.len(), 65);
    assert!(got[..64].iter().all(|(_, value)| value == "1"));
    assert_eq!(got[64].1, "First synthetic target");
    assert_eq!(s.doc.bookmarks().iter().find(|(name, _)| name == "_Duplicate").unwrap().1.path, Path::top(0));
}

/// A target's instance must not invent a different label for its actual ancestor paragraph.
#[test]
fn full_context_keeps_the_actual_ancestor_instance_label() {
    let mut s = open(
        &[
            para(Some((1, 0)), &text("Synthetic ancestor")),
            para(Some((6, 1)), &marked("_ActualAncestor", 80, "Synthetic child")),
            para(None, &field(r"REF _ActualAncestor \w", "STALE")),
        ]
        .concat(),
    );
    let numbering = std::sync::Arc::make_mut(&mut s.doc.numbering);
    numbering.abstracts.iter_mut().find(|a| a.id == 0).unwrap().levels[0].text = "Clause %1.".into();
    let mut child = numbering.nums.iter().find(|n| n.id == 1).unwrap().clone();
    child.id = 6;
    child.level_overrides = vec![
        (0, wordcraft_doc::numbering::Level { text: "Part %1.%2.".into(), ..Default::default() }),
        (1, wordcraft_doc::numbering::Level { format: wordcraft_doc::section::NumFormat::LowerLetter, text: "(%2)".into(), ..Default::default() }),
    ];
    numbering.nums.push(child);
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    assert_eq!(fields(&s).into_iter().map(|(_, value)| value).collect::<Vec<_>>(), ["Clause 1(a)"]);
}

/// The six legal-list cases measured in Word for Windows by @pakiat:
/// https://github.com/storytold/wordcraft/pull/4609#issuecomment-6557062456
#[test]
fn relative_refs_keep_the_legal_template_in_the_six_measured_contexts() {
    let refs = || [field(r"REF _Leaf \r", "STALE"), field(r"REF _Mid \r", "STALE")].concat();
    let body = [
        para(Some((2, 0)), &text("Synthetic first root")),
        para(Some((2, 0)), &refs()),
        para(Some((2, 1)), &text("Synthetic first section")),
        para(Some((2, 1)), &marked("_Mid", 90, "Synthetic target section")),
        para(Some((2, 2)), &text("Synthetic first leaf")),
        para(Some((2, 2)), &marked("_Leaf", 91, "Synthetic target leaf")),
        para(Some((2, 2)), &refs()),
        para(Some((2, 1)), &refs()),
    ]
    .concat();
    let mut s = open(&body);
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["2.2.2", "2.2", "2.2.2", "2.2", "2.2.2", "2.2"]);
}

/// Equal counter values in another list do not provide shared relative context: before any
/// paragraph of the target's list, the reference is in full.
#[test]
fn relative_context_is_not_shared_by_different_lists() {
    let mut s = open(
        &[
            para(Some((1, 0)), &text("Synthetic separate root")),
            para(Some((1, 1)), &field(r"REF _OtherList \r", "STALE")),
            para(Some((4, 0)), &text("Synthetic target root")),
            para(Some((4, 1)), &marked("_OtherList", 92, "Synthetic target")),
        ]
        .concat(),
    );
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    assert_eq!(fields(&s)[0].1, "1(a)");
}

/// `\r` outside the target's list takes its context from where that list stands at the field:
/// its last paragraph at or before it, whether the field's paragraph is unnumbered or numbered in
/// another list. Measured in Word for Windows: after clause 1's (a), "(a)" from an unnumbered
/// paragraph, from another list's clause, and between 1. and its (a); "1(a)" once clause 2 has
/// begun, and before the list.
#[test]
fn relative_context_is_where_the_target_list_stands_at_the_field() {
    let r = |cached| field(r"REF _Ctx \r \h", cached);
    let mut s = open(
        &[
            para(None, &r("before")),
            para(Some((4, 0)), &text("Synthetic one")),
            para(None, &r("between")),
            para(Some((4, 1)), &marked("_Ctx", 93, "Synthetic target")),
            para(None, &r("after")),
            para(Some((1, 0)), &r("other list")),
            para(Some((4, 0)), &r("own list, clause 2")),
            para(None, &r("end")),
        ]
        .concat(),
    );
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["1(a)", "(a)", "(a)", "(a)", "1(a)", "1(a)"]);
}

/// Another numbering instance of the same abstract list (numId 7 of scheme 4) continues its
/// counters but is another list for `\r`, as Word shows it: 2(a) from clause 2 of the first
/// instance, (a) after the item.
#[test]
fn relative_context_is_not_shared_by_another_instance_of_the_abstract_list() {
    let mut s = open(
        &[
            para(Some((4, 0)), &text("Synthetic one")),
            para(Some((4, 0)), &field(r"REF _Inst \r \h", "STALE")),
            para(Some((7, 1)), &marked("_Inst", 94, "Synthetic item")),
            para(None, &field(r"REF _Inst \r \h", "STALE")),
        ]
        .concat(),
    );
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["2(a)", "(a)"]);
}

/// A level above the target that its list never used shows its start number in `\w`, and in
/// `\r` with no context, as Word shows it; after the target, `\r` is the item's own label.
#[test]
fn unused_levels_above_the_target_show_their_start_number() {
    let refs = || [field(r"REF _Unused \r \h", "STALE"), field(r"REF _Unused \w \h", "STALE")].concat();
    let mut s = open(
        &[
            para(None, &refs()),
            para(Some((1, 0)), &text("Synthetic other clause")),
            para(Some((1, 0)), &refs()),
            para(Some((4, 1)), &marked("_Unused", 95, "Synthetic item")),
            para(None, &refs()),
        ]
        .concat(),
    );
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    let got: Vec<_> = fields(&s).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["1(a)", "1(a)", "1(a)", "1(a)", "(a)", "1(a)"]);
}

/// Printing and exporting compute the paragraph numbers as F9 does, even with "Update fields
/// before printing" off, while the document keeps its stored results (measured in Word for
/// Windows with stale stored numbers).
#[test]
fn output_computes_paragraph_number_references() {
    let mut s = open(
        &[
            para(Some((4, 0)), &text("Synthetic one")),
            para(Some((4, 0)), &text("Synthetic two")),
            para(Some((4, 0)), &marked("_Three", 96, "Synthetic three")),
            para(Some((4, 1)), &text("Synthetic a")),
            para(Some((4, 1)), &marked("_SubB", 97, "Synthetic b")),
            para(
                None,
                &[
                    field(r"REF _Three \n \h", "9"),
                    field(r"REF _Three \w \h", "9"),
                    field(r"REF _SubB \r \h", "9"),
                    field(r"REF _SubB \p", "9"),
                    field(r"REF _SubB \r \p", "9"),
                    field(r"REF _Missing \n", "9"),
                    field(r"REF _SubB \w \d /", "KEEP DELIMITER"),
                ]
                .concat(),
            ),
        ]
        .concat(),
    );
    let error = wordcraft_engine::cmd::references::REFERENCE_NOT_FOUND;
    let out = wordcraft_engine::cmd::references::output_fields(&s.doc).expect("the stale numbers change");
    let results = |d: &wordcraft_doc::Document| -> Vec<String> {
        let last = d.para_paths(StoryRef::Body).pop().unwrap();
        d.para(StoryRef::Body, &last)
            .unwrap()
            .objects
            .iter()
            .filter_map(|o| match o {
                InlineObject::Field { result, .. } => Some(result.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(results(&out), ["3", "3", "(b)", "above", "(b) above", error, "KEEP DELIMITER"]);
    assert_eq!(results(&s.doc), ["9", "9", "9", "9", "9", "9", "KEEP DELIMITER"], "the document keeps them");
    // F9 gives the same numbers.
    s.run("references.updateFields", &json!({"all":true})).unwrap();
    assert_eq!(results(&s.doc), results(&out));
}
