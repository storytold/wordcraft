//! Footnotes and endnotes (ODF 1.3 §6.3 `text:note`): read into note stories, written back.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::para::{InlineObject, NoteKind, OBJ};
use wordcraft_doc::{Document, PartKind};

fn body(xml: &str) -> Vec<u8> {
    package(
        &format!(r#"<office:document-content {NS_T}><office:body><office:text>{xml}</office:text></office:body></office:document-content>"#),
        None,
    )
}

/// Every note referenced from the body, in order: (kind, custom mark, the note's text).
fn notes(d: &Document) -> Vec<(NoteKind, String, String)> {
    let mut out = Vec::new();
    for b in d.body.iter() {
        let Some(p) = b.as_para() else { continue };
        for o in &p.objects {
            if let InlineObject::NoteRef { kind, id, custom } = o {
                let part = d.parts.get(id).expect("the note's story");
                assert_eq!(part.kind, if *kind == NoteKind::Footnote { PartKind::Footnote } else { PartKind::Endnote });
                let text = part.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(OBJ, "")).collect::<Vec<_>>().join("|");
                out.push((*kind, custom.clone(), text.trim().to_string()));
            }
        }
    }
    out
}

const NOTES: &str = concat!(
    r#"<text:p>Text<text:note text:id="ftn1" text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p text:style-name="Footnote">The footnote body.</text:p></text:note-body></text:note>"#,
    r#" and<text:note text:id="ftn2" text:note-class="endnote"><text:note-citation>i</text:note-citation><text:note-body><text:p>End one.</text:p><text:p>End two.</text:p></text:note-body></text:note>"#,
    r#" a mark<text:note text:id="ftn3" text:note-class="footnote"><text:note-citation text:label="*">*</text:note-citation><text:note-body><text:p>Starred.</text:p></text:note-body></text:note>.</text:p>"#,
    "<text:p>After.</text:p>"
);

#[test]
fn notes_open_as_note_stories() {
    let d = import(&body(NOTES)).unwrap();
    assert_eq!(
        notes(&d),
        [
            (NoteKind::Footnote, String::new(), "The footnote body.".to_string()),
            (NoteKind::Endnote, String::new(), "End one.|End two.".to_string()),
            (NoteKind::Footnote, "*".to_string(), "Starred.".to_string()),
        ]
    );
    // The citation numbers are not text; the paragraph goes on after each note.
    let p = d.body[0].as_para().unwrap();
    assert_eq!(p.text.replace(OBJ, "#"), "Text# and# a mark#.");
    assert_eq!(d.body[1].as_para().unwrap().text, "After.");
    // As Word's: the reference is in the reference style, the note's text in the note style and
    // starts with its own mark.
    let first = d.parts.values().find(|p| p.kind == PartKind::Footnote).unwrap();
    let np = first.blocks[0].as_para().unwrap();
    assert_eq!(np.props.style.as_deref(), Some("FootnoteText"));
    assert!(matches!(np.objects.first(), Some(InlineObject::NoteRef { .. })));
    assert_eq!(p.runs.iter().filter(|r| r.props.style.as_deref() == Some("FootnoteReference")).count(), 2);
}

#[test]
fn notes_round_trip() {
    let d = import(&body(NOTES)).unwrap();
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(notes(&back), notes(&d));
    assert_eq!(back.body[0].as_para().unwrap().text, d.body[0].as_para().unwrap().text);
    // Saving again changes nothing more.
    assert_eq!(notes(&import(&export(&back).unwrap()).unwrap()), notes(&d));
}

#[test]
fn a_note_starting_with_a_tab_keeps_it() {
    // ONLYOFFICE's notes start with a tab after the number: written back with it, and only the
    // space WordCraft puts after the mark is left out.
    let xml = r#"<text:p>A<text:note text:id="n1" text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p><text:span/><text:tab/>Lorem</text:p></text:note-body></text:note> and<text:note text:id="n2" text:note-class="footnote"><text:note-citation>2</text:note-citation><text:note-body><text:p>Plain</text:p></text:note-body></text:note>.</text:p>"#;
    let d = import(&body(xml)).unwrap();
    let c = crate::odt::tests::content_of(&export(&d).unwrap());
    assert!(c.contains("<text:tab/>Lorem"), "{c}");
    assert!(c.contains(">Plain<") && !c.contains("> Plain"), "{c}");
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(notes(&back), notes(&d));
}

#[test]
fn a_note_keeps_formatting_and_the_text_around_it() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style></office:automatic-styles><office:body><office:text><text:list><text:list-item><text:p><text:span text:style-name="B">bold<text:note text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>in <text:span text:style-name="B">note</text:span></text:p></text:note-body></text:note> still bold</text:span></text:p></text:list-item></text:list></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    let p = d.body[0].as_para().unwrap();
    assert!(p.props.numbering.is_some(), "the list survives the note");
    let tail = p.runs.last().unwrap();
    assert_eq!(tail.props.bold, Some(true));
    assert_eq!(p.text.replace(OBJ, "#"), "bold# still bold");
    let note = d.parts.values().find(|p| p.kind == PartKind::Footnote).unwrap().blocks[0].as_para().unwrap().clone();
    assert!(note.runs.iter().any(|r| r.props.bold == Some(true)));
    assert!(note.props.numbering.is_none(), "the body's list doesn't reach into the note");
}

#[test]
fn hostile_notes_do_not_panic() {
    let deep = "<text:note><text:note-body><text:p>".repeat(200);
    for xml in [
        "<text:p>a<text:note>".to_string(),
        "<text:p>a<text:note><text:note-body><text:p>b".to_string(),
        "<text:p><text:note><text:note-body><text:p>x<text:note><text:note-body><text:p>y</text:p></text:note-body></text:note></text:p></text:note-body></text:note></text:p>".to_string(),
        format!("<text:p>{deep}</text:p>"),
        "<text:note><text:note-body><text:p>outside a paragraph</text:p></text:note-body></text:note>".to_string(),
        "<text:p><text:note><text:note-citation text:label=\"a very long label that goes on and on\">x</text:note-citation></text:note></text:p>".to_string(),
    ] {
        let d = import(&body(&xml)).unwrap();
        let _ = export(&d).unwrap();
    }
}

#[test]
fn a_note_inside_a_table_cell_is_kept() {
    let xml = r#"<table:table><table:table-column/><table:table-row><table:table-cell><text:p>cell<text:note text:note-class="endnote"><text:note-citation>i</text:note-citation><text:note-body><text:p>cell note</text:p></text:note-body></text:note></text:p></table:table-cell></table:table-row></table:table>"#;
    let d = import(&body(xml)).unwrap();
    let back = import(&export(&d).unwrap()).unwrap();
    let ends: Vec<_> = back.parts.values().filter(|p| p.kind == PartKind::Endnote).collect();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].blocks[0].as_para().unwrap().text.replace(OBJ, "").trim(), "cell note");
}

#[test]
fn citations_number_footnotes_and_endnotes_apart() {
    let d = import(&body(NOTES)).unwrap();
    let xml = crate::odt::tests::content_of(&export(&d).unwrap());
    let citations: Vec<&str> =
        xml.split("<text:note-citation").skip(1).filter_map(|c| c.split_once('>')).map(|(_, rest)| rest.split('<').next().unwrap_or("")).collect();
    assert_eq!(citations, ["1", "i", "*"]);
}

#[test]
fn a_note_cut_off_by_the_end_of_the_file_keeps_its_paragraph() {
    // Truncated XML: no end for the note, its paragraph or the body.
    let content = format!(r#"<office:document-content {NS_T}><office:body><office:text><text:p>before<text:note><text:note-body><text:p>in note"#);
    let d = import(&package(&content, None)).unwrap();
    assert_eq!(d.body[0].as_para().unwrap().text.replace(OBJ, "#"), "before#");
    assert_eq!(notes(&d), [(NoteKind::Footnote, String::new(), "in note".to_string())]);
}

#[test]
fn a_footnote_keeps_its_text_beside_a_footer() {
    // The footer and the note are both stories of their own: neither takes the other's place.
    let styles = format!(
        r#"<office:document-styles {}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:footer><text:p>Footer text</text:p></style:footer></style:master-page></office:master-styles></office:document-styles>"#,
        crate::odt::tests::NS_T
    );
    let content = format!(
        r#"<office:document-content {}><office:body><office:text><text:p>Body<text:note text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>The note.</text:p></text:note-body></text:note> after.</text:p></office:text></office:body></office:document-content>"#,
        crate::odt::tests::NS_T
    );
    let d = import(&crate::odt::tests::package(&content, Some(&styles))).unwrap();
    // A story's text, without the note's own mark.
    let text = |id: u32| {
        d.parts
            .get(&id)
            .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let note = d.body[0].as_para().unwrap().objects.iter().find_map(|o| match o {
        wordcraft_doc::para::InlineObject::NoteRef { id, .. } => Some(*id),
        _ => None,
    });
    assert_eq!(note.map(text).as_deref(), Some("The note."));
    assert_eq!(d.last_section.footers.default.map(text).as_deref(), Some("Footer text"));
}

#[test]
fn citations_are_numbered_as_the_document_numbers_its_notes() {
    // §6.3.3: the citation is the text shown. Footnotes in A, B, C from the third (ODF counts the
    // first number from 0), endnotes as configured; without a configuration 1, 2, 3 and i, ii.
    let mut d = import(&body(r#"<text:p>One<text:note text:note-class="footnote"><text:note-citation>x</text:note-citation><text:note-body><text:p>a</text:p></text:note-body></text:note> two<text:note text:note-class="footnote"><text:note-citation>x</text:note-citation><text:note-body><text:p>b</text:p></text:note-body></text:note> three<text:note text:note-class="endnote"><text:note-citation>x</text:note-citation><text:note-body><text:p>c</text:p></text:note-body></text:note></text:p>"#)).unwrap();
    let citations = |d: &Document, model: bool| {
        let saved = if model { crate::odt::tests::export_model(d) } else { export(d) };
        let c = crate::odt::tests::entry(&saved.unwrap(), "content.xml");
        c.split("<text:note-citation>").skip(1).filter_map(|s| s.split('<').next()).map(str::to_string).collect::<Vec<_>>()
    };
    assert_eq!(citations(&d, true), ["1", "2", "i"]);
    // Unedited, the file's own (as read).
    assert_eq!(citations(&d, false), ["x", "x", "x"]);
    d.settings.footnote_format = wordcraft_doc::section::NumFormat::UpperLetter;
    d.settings.footnote_pr.num_start = Some(3);
    d.settings.endnote_format = wordcraft_doc::section::NumFormat::Decimal;
    // The numbering changed: its notes' paragraph written from the model, numbered anew.
    assert_eq!(citations(&d, false), ["C", "D", "1"]);
}
