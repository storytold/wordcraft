//! Page fields read from RTF (#6222): a page-dependent field whose result is plain text in one
//! paragraph comes in as a field, so the layout works out its number; other fields stay their
//! result's text. Written by hand from the RTF specification's `\field`, `\fldinst`, `\fldrslt`,
//! `\fldlock` and `\chpgn`.

use super::*;
use wordcraft_doc::para::InlineObject;

/// The fields of each paragraph of a story, with the paragraph's text.
fn fields_of(blocks: &wordcraft_doc::Blocks) -> Vec<(String, Vec<(String, String, bool)>)> {
    blocks
        .iter()
        .filter_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None })
        .map(|p| {
            let fields = p
                .objects
                .iter()
                .filter_map(|o| match o {
                    InlineObject::Field { instr, result, locked, .. } => Some((instr.clone(), result.clone(), *locked)),
                    _ => None,
                })
                .collect();
            (p.text.clone(), fields)
        })
        .collect()
}

fn footer(d: &wordcraft_doc::Document) -> Vec<(String, Vec<(String, String, bool)>)> {
    let id = d.last_section.footers.default.expect("a footer");
    fields_of(&d.parts[&id].blocks)
}

#[test]
fn a_page_field_in_the_footer_is_a_field_not_its_stored_number() {
    // As Word writes it: the text before the field has the same formatting as its result.
    let rtf = br"{\rtf1\ansi\sectd
{\footerr \pard {\fs16 Pag. }{\field{\*\fldinst {\fs16  PAGE }}{\fldrslt {\fs16 2}}}\sectd \ltrsect{\par}}
\pard One\page Two\par}";
    let d = import(rtf).unwrap();
    let f = footer(&d);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].1, [("PAGE".to_string(), "2".to_string(), false)], "{f:?}");
    // "Pag. " stays text before the field: the result's run did not take it along.
    assert!(f[0].0.starts_with("Pag. "), "{f:?}");
    assert_eq!(f[0].0.matches("Pag. ").count(), 1, "{f:?}");
}

#[test]
fn page_fields_in_the_body_keep_their_code_result_and_lock() {
    let rtf = br#"{\rtf1\ansi
\pard Page {\field{\*\fldinst PAGE \\* roman}{\fldrslt ii}} of {\field\fldlock{\*\fldinst NUMPAGES}{\fldrslt 4}}\par
\pard {\field{\*\fldinst SECTIONPAGES}{\fldrslt }}x\par}"#;
    let d = import(rtf).unwrap();
    let f = fields_of(&d.body);
    assert_eq!(f[0].1, [("PAGE \\* roman".to_string(), "ii".to_string(), false), ("NUMPAGES".to_string(), "4".to_string(), true)], "{f:?}");
    assert!(f[0].0.starts_with("Page ") && f[0].0.contains(" of "), "{f:?}");
    // An empty result is still a field: the layout fills it in.
    assert_eq!(f[1].1, [("SECTIONPAGES".to_string(), String::new(), false)], "{f:?}");
}

#[test]
fn other_fields_stay_the_text_of_their_result() {
    // A field's result is drawn as one piece that does not wrap: a long merge field in a narrow
    // cell keeps its text, which wraps, as before.
    let rtf = br#"{\rtf1\ansi
\pard Before {\field{\*\fldinst DATE \\@ "dd/MM/yyyy"}{\fldrslt 11/11/2026}} after\par
\trowd\cellx600\pard\intbl {\field{\*\fldinst MERGEFIELD Approvato}{\fldrslt Presidente della Rete (Rossi Mario)}}\cell\row
\pard {\field{\*\fldinst AUTHOR}{\fldrslt }}x\par}"#;
    let d = import(rtf).unwrap();
    let f = fields_of(&d.body);
    assert_eq!((f[0].0.as_str(), f[0].1.len()), ("Before 11/11/2026 after", 0), "{f:?}");
    let cell = d.body.iter().find_map(|b| b.as_table()).and_then(|t| t.rows.first()).and_then(|r| r.cells.first()).unwrap();
    let cf = fields_of(&cell.blocks);
    assert_eq!((cf[0].0.as_str(), cf[0].1.len()), ("Presidente della Rete (Rossi Mario)", 0), "{cf:?}");
    assert!(f.last().is_some_and(|(t, fl)| t == "x" && fl.is_empty()), "{f:?}");
}

#[test]
fn fields_that_cannot_be_one_field_stay_text() {
    let rtf = br#"{\rtf1\ansi
\pard {\field{\*\fldinst{HYPERLINK "http://a.b"}}{\fldrslt{link}}}\par
\pard {\field{\*\fldinst TOC \\o "1-3"}{\fldrslt {Heading one\tab 1\par Heading two\tab 2}}}\par
\pard {\field{\*\fldinst PAGEREF a}{\fldrslt {\b 1}2}}\par
\pard {\field{\*\fldinst IF {\field{\*\fldinst PAGE}{\fldrslt 1}} = 1 "one" "many"}{\fldrslt one}}\par
\pard {\field{\*\fldinst PAGE}}\par
\pard {\field{\*\fldinst PAGE}{\fldrslt 1\par 2}}\par}"#;
    let d = import(rtf).unwrap();
    let f = fields_of(&d.body);
    let texts: Vec<&str> = f.iter().map(|(t, _)| t.as_str()).collect();
    // A link stays a link; a result over two paragraphs or in two formats stays its text.
    assert_eq!(&texts[..4], ["link", "Heading one\t1", "Heading two\t2", "12"], "{f:?}");
    // The PAGE inside IF's instruction is part of the code, not a field of the paragraph.
    assert_eq!(texts[4], "one", "{f:?}");
    // A field with no result group at all is left as it was (nothing).
    assert!(f[5].0.is_empty(), "{f:?}");
    assert_eq!(&texts[6..8], ["1", "2"], "{f:?}");
    assert!(f.iter().all(|(_, fl)| fl.is_empty()), "{f:?}");
}

#[test]
fn a_table_of_contents_keeps_its_links_and_its_page_numbers_become_fields() {
    // The shape Word writes: the TOC result spans paragraphs; each entry is a link around its
    // text and a PAGEREF to the heading's bookmark.
    let rtf = br#"{\rtf1\ansi
\pard {\field{\*\fldinst TOC \\o "1-3" \\h}{\fldrslt {\field{\*\fldinst HYPERLINK \\l "_Toc1"}{\fldrslt {One\tab {\field{\*\fldinst PAGEREF _Toc1 \\h}{\fldrslt 3}}}}}\par
{\field{\*\fldinst HYPERLINK \\l "_Toc2"}{\fldrslt {Two\tab {\field{\*\fldinst PAGEREF _Toc2 \\h}{\fldrslt 5}}}}}}}\par
\pard {\*\bkmkstart _Toc1}Heading{\*\bkmkend _Toc1}\par}"#;
    let d = import(rtf).unwrap();
    let f = fields_of(&d.body);
    assert_eq!(f[0].1, [("PAGEREF _Toc1 \\h".to_string(), "3".to_string(), false)], "{f:?}");
    assert_eq!(f[1].1, [("PAGEREF _Toc2 \\h".to_string(), "5".to_string(), false)], "{f:?}");
    assert!(f[0].0.starts_with("One\t") && f[1].0.starts_with("Two\t"), "{f:?}");
    // The entry text still links to its heading.
    let p = d.body[0].as_para().unwrap();
    assert!(p.props_at(0).link.is_some(), "{:?}", p.runs);
}

#[test]
fn hidden_styleref_tracked_and_object_results_stay_as_read() {
    let rtf = br#"{\rtf1\ansi{\*\revtbl{Unknown;}{Ann;}}
\pard a{\v {\field{\*\fldinst PAGE}{\fldrslt 2}}}b\par
\pard {\field{\*\fldinst STYLEREF "heading 1"}{\fldrslt Intro}}\par
\pard {\field{\*\fldinst PAGE}{\fldrslt {\revised\revauth1 2}}}\par
\pard {\field{\*\fldinst PAGE}{\fldrslt A{\*\bkmkstart m}B{\*\bkmkend m}}}\par}"#;
    let d = import(rtf).unwrap();
    let f = fields_of(&d.body);
    // Hidden: neither the result nor a field.
    assert_eq!((f[0].0.as_str(), f[0].1.len()), ("ab", 0), "{f:?}");
    // STYLEREF keeps Word's text.
    assert_eq!((f[1].0.as_str(), f[1].1.len()), ("Intro", 0), "{f:?}");
    // A tracked result and one holding a bookmark stay text.
    assert_eq!((f[2].0.as_str(), f[2].1.len()), ("2", 0), "{f:?}");
    // (The bookmark's start and end are object characters in the paragraph's text.)
    assert_eq!((f[3].0.replace('\u{FFFC}', ""), f[3].1.len()), ("AB".to_string(), 0), "{f:?}");
}

#[test]
fn the_page_number_symbol_is_a_page_field() {
    let d = import(br"{\rtf1\ansi{\footer \pard Page \chpgn\par}\pard Body\par}").unwrap();
    let f = footer(&d);
    assert_eq!(f[0].1, [("PAGE".to_string(), String::new(), false)], "{f:?}");
}

#[test]
fn truncated_and_hostile_fields_do_not_crash() {
    for rtf in [
        &br"{\rtf1{\field{\*\fldinst PAGE}{\fldrslt 2"[..],
        br"{\rtf1{\field{\*\fldinst PAGE}{\fldrslt 2}",
        br"{\rtf1{\field\field\field{\*\fldinst PAGE}}}}",
        br"{\rtf1{\field{\fldrslt x}{\*\fldinst PAGE}}\par}",
        br"{\rtf1 a{\field{\*\fldinst PAGE}{\fldrslt 1\par 2}}\par}",
    ] {
        let _ = import(rtf);
    }
    let mut deep = String::from(r"{\rtf1 ");
    for _ in 0..300 {
        deep.push_str(r"{\field{\*\fldinst PAGE}{\fldrslt ");
    }
    deep.push('x');
    let _ = import(deep.as_bytes());
}

#[test]
fn the_flow_reader_keeps_field_results_as_text() {
    // `parse` asks for no fields, as before: the result is the paragraph's text.
    let f = parse(br"{\rtf1\ansi\pard Pag. {\field{\*\fldinst PAGE}{\fldrslt 2}}\par}").unwrap();
    match &f.blocks[0] {
        FBlock::Para(p) => {
            assert!(p.inlines.iter().all(|i| matches!(i, Inline::Text(..))), "{:?}", p.inlines);
            assert_eq!(p.text(), "Pag. 2");
        }
        b => panic!("{b:?}"),
    }
}
