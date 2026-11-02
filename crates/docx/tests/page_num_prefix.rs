//! A section's page-number prefix (`page_num_prefix`) rides in Word's document variables
//! (`w:settings/w:docVars`), because `w:sectPr` has no extension slot: emitting `w:extLst` or a
//! `w:pgNumPrefix` element there is not valid OOXML. The value still round-trips.

use std::io::Read;

use quick_xml::Reader;
use quick_xml::events::Event;
use wordcraft_doc::section::{PAGE_NUM_PREFIX_DOCVAR, SectionProps};
use wordcraft_doc::{Document, InlineObject, Paragraph, PartKind, para_block};
use wordcraft_docx::{read, write};

/// One part of a written package as text.
fn part(bytes: Vec<u8>, name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut xml = String::new();
    zip.by_name(name).expect("the part").read_to_string(&mut xml).expect("read");
    xml
}

fn sect(doc: &Document) -> SectionProps {
    doc.sections().into_iter().next().map(|(_, s)| s.clone()).expect("a section")
}

fn doc_with_prefix(prefix: &str) -> Document {
    let mut d = Document::new();
    let field = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() };
    let mut fp = Paragraph::new();
    fp.insert_text(0, prefix, &Default::default()).unwrap();
    fp.insert_object(prefix.len(), field, &Default::default()).unwrap();
    let id = d.add_part(PartKind::Footer, vec![para_block(fp)]);
    d.last_section.footers.default = Some(id);
    d.last_section.page_num_start = Some(1);
    d.last_section.page_num_prefix = Some(prefix.to_string());
    d
}

/// The qualified names of `parent`'s direct child elements in `xml`.
fn children_of(xml: &str, parent: &str) -> Vec<String> {
    let mut r = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut inside = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if inside {
                    if depth == 0 {
                        out.push(name);
                    }
                    depth += 1;
                } else if name == parent {
                    inside = true;
                    depth = 0;
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if inside && depth == 0 {
                    out.push(name);
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if inside {
                    if depth == 0 && name == parent {
                        inside = false;
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

#[test]
fn page_num_prefix_is_a_document_variable_not_an_invalid_section_child() {
    let bytes = write(&doc_with_prefix("A")).expect("write");
    let doc_xml = part(bytes.clone(), "word/document.xml");
    assert!(!doc_xml.contains("w:pgNumPrefix"), "must not emit the invalid section element: {doc_xml}");
    assert!(!doc_xml.contains("w:extLst"), "must not add an extension to w:sectPr: {doc_xml}");
    let settings = part(bytes, "word/settings.xml");
    assert!(settings.contains("w:docVars") && settings.contains("w:docVar"), "{settings}");
    assert!(settings.contains(&format!("{PAGE_NUM_PREFIX_DOCVAR}0")) && settings.contains("w:val=\"A\""), "{settings}");
}

#[test]
fn every_sectpr_child_is_valid_and_docvars_holds_only_docvar() {
    let bytes = write(&doc_with_prefix("A")).expect("write");
    let doc_xml = part(bytes.clone(), "word/document.xml");
    // ECMA-376 (WordprocessingML) CT_SectPr: the only permitted children of w:sectPr.
    let allowed = [
        "w:headerReference",
        "w:footerReference",
        "w:footnotePr",
        "w:endnotePr",
        "w:type",
        "w:pgSz",
        "w:pgMar",
        "w:paperSrc",
        "w:pgBorders",
        "w:lnNumType",
        "w:pgNumType",
        "w:cols",
        "w:formProt",
        "w:vAlign",
        "w:noEndnote",
        "w:titlePg",
        "w:textDirection",
        "w:bidi",
        "w:rtlGutter",
        "w:docGrid",
        "w:printerSettings",
        "w:sectPrChange",
    ];
    for name in children_of(&doc_xml, "w:sectPr") {
        assert!(allowed.contains(&name.as_str()), "`{name}` is not a valid w:sectPr child: {doc_xml}");
    }
    let settings = part(bytes, "word/settings.xml");
    let vars = children_of(&settings, "w:docVars");
    assert!(!vars.is_empty(), "the prefix is stored as a document variable: {settings}");
    for name in vars {
        assert_eq!(name, "w:docVar", "w:docVars may only hold w:docVar children: {settings}");
    }
}

#[test]
fn page_num_prefix_round_trips_through_document_variables() {
    let doc = doc_with_prefix("Add. ");
    let back = read(&write(&doc).expect("write")).expect("read");
    assert_eq!(sect(&back).page_num_prefix.as_deref(), Some("Add. "));
    // The consumed variable does not linger on the re-read document.
    assert!(back.settings.doc_vars.is_empty(), "{:?}", back.settings.doc_vars);
}

#[test]
fn other_document_variables_are_preserved_but_reserved_ones_are_consumed() {
    let mut doc = doc_with_prefix("A");
    doc.settings.doc_vars.push(("MyVar".into(), "kept".into()));
    let back = read(&write(&doc).expect("write")).expect("read");
    assert_eq!(back.settings.doc_vars, vec![("MyVar".to_string(), "kept".to_string())]);
    assert_eq!(sect(&back).page_num_prefix.as_deref(), Some("A"));
}

#[test]
fn section_prefix_survives_a_full_document_variable_list() {
    // The reader caps the total number of document variables. A full ordinary list must not crowd
    // the prefix out, or a document's page numbering is lost the next time it is opened.
    let mut doc = doc_with_prefix("A");
    doc.settings.doc_vars = (0..wordcraft_docx::MAX_DOC_VARS).map(|i| (format!("Var{i}"), "x".to_string())).collect();
    let back = read(&write(&doc).expect("write")).expect("read");
    assert_eq!(sect(&back).page_num_prefix.as_deref(), Some("A"), "the section prefix must survive a full variable list");
    assert_eq!(back.settings.doc_vars.len(), wordcraft_docx::MAX_DOC_VARS - 1, "the ordinary variables fill only what the section prefix leaves");
}
