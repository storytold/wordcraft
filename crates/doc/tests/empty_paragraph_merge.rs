use std::sync::Arc;
use wordcraft_doc::props::{Align, CharProps, ParaProps};
use wordcraft_doc::{Block, Document, Paragraph, Path, Pos, StoryRef, Table, para_block};

/// Build an empty heading followed by explicitly formatted, mixed-run body text.
fn fixture(first: &str, style: Option<&str>) -> Document {
    let mut d = Document::from_text(&format!("{first}\nBody paragraph."));
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.props.style = Some("Heading1".into());
    p.props.outline_level = Some(0);
    p.mark = CharProps { size: Some(40.0), bold: Some(true), ..Default::default() };
    let p = d.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    p.props = ParaProps {
        style: style.map(str::to_string),
        align: Some(Align::Right),
        indent_left: Some(18.0),
        space_after: Some(7.0),
        keep_next: Some(false),
        ..Default::default()
    };
    p.mark = CharProps { size: Some(11.0), bold: Some(false), lang: Some("en-GB".into()), ..Default::default() };
    p.format(0, 4, &|c| c.italic = Some(true)).unwrap();
    d
}

/// Compare paragraph, mark and inline-run formatting independently of text.
fn assert_format(p: &Paragraph, expected: &Paragraph) {
    assert_eq!(p.props, expected.props, "the following paragraph keeps all its properties");
    assert_eq!(p.mark, expected.mark, "the empty heading's mark must not replace the following mark");
    assert_eq!(serde_json::to_value(&p.runs).unwrap(), serde_json::to_value(&expected.runs).unwrap());
}

/// Deleting an empty paragraph preserves the following style, including implicit Normal.
#[test]
fn deleting_an_empty_paragraph_keeps_the_following_format() {
    for first_style in ["Heading1", "Heading3", "Normal"] {
        for style in [None, Some("Normal"), Some("Heading2")] {
            let mut d = fixture("", style);
            d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.style = Some(first_style.into());
            let expected = d.para_at(&Pos::body(1, 0)).unwrap().clone();
            assert_eq!(d.delete_range(&Pos::body(0, 0), &Pos::body(1, 0)).unwrap(), Pos::body(0, 0));
            let p = d.para_at(&Pos::body(0, 0)).unwrap();
            assert_eq!(p.text, expected.text);
            assert_format(p, &expected);
        }
    }
}

/// A selection spanning several paragraphs keeps the surviving tail format in both directions.
#[test]
fn an_empty_start_preserves_the_last_paragraph_over_a_multi_paragraph_selection() {
    for backwards in [false, true] {
        let mut d = fixture("", None);
        d.body.insert(1, para_block(Paragraph::with_text("Removed middle paragraph", Default::default()).styled("Heading2")));
        let mut expected = d.para_at(&Pos::body(2, 0)).unwrap().clone();
        expected.delete(0, 5).unwrap();
        let (a, b) = (Pos::body(0, 0), Pos::body(2, 5));
        d.delete_range(if backwards { &b } else { &a }, if backwards { &a } else { &b }).unwrap();
        assert_eq!(d.body.len(), 1);
        let p = d.para_at(&Pos::body(0, 0)).unwrap();
        assert_eq!(p.text, "paragraph.");
        assert_format(p, &expected);
    }
}

/// A control whose closing marker survives must move across the join without replacing tail formatting.
#[test]
fn an_empty_start_keeps_a_control_from_a_deleted_middle_paragraph() {
    use wordcraft_doc::control::ContentControl;
    use wordcraft_doc::{InlineObject, Revision, RevisionKind};

    let mut d = fixture("", Some("Normal"));
    d.body.insert(1, para_block(Paragraph::with_text("Removed", Default::default())));
    d.body.push(para_block(Paragraph::with_text("After", Default::default())));
    d.revisions.push(Revision { kind: RevisionKind::Insert, ..Default::default() });
    d.revisions.push(Revision { kind: RevisionKind::Delete, ..Default::default() });
    let p = d.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    p.mark.ins = Some(0);
    p.mark.del = Some(1);
    let expected = p.clone();
    let marker = CharProps { bold: Some(true), ins: Some(0), ..Default::default() };
    d.insert_object(&Pos::body(1, 0), InlineObject::ControlStart { control: Box::new(ContentControl::default()) }, &marker).unwrap();
    d.insert_object(&Pos::body(3, 5), InlineObject::ControlEnd, &Default::default()).unwrap();
    assert_eq!(d.control_ranges(StoryRef::Body).len(), 1);

    d.delete_range(&Pos::body(0, 0), &Pos::body(2, 0)).unwrap();
    let p = d.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props, expected.props);
    assert_eq!(p.mark, expected.mark, "tail revision IDs and formatting survive the moved marker");
    assert_eq!(p.props_of_char(0), &marker);
    assert!(matches!(p.objects.as_slice(), [InlineObject::ControlStart { .. }]));
    assert_eq!(p.text, format!("\u{fffc}{}", expected.text));
    let ranges = d.control_ranges(StoryRef::Body);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start, Pos::body(0, 0));
    assert_eq!(ranges[0].end, Pos::body(1, 5));
    assert!(!d.has_unbalanced_controls());
}

/// The empty-paragraph rule applies in header stories and table-cell containers.
#[test]
fn empty_paragraph_merges_work_in_a_header_and_a_table_cell() {
    for cell in [false, true] {
        let mut d = fixture("", Some("Normal"));
        let expected = d.para_at(&Pos::body(1, 0)).unwrap().clone();
        let (story, a, b) = if cell {
            let mut table = Table::new(1, 1, 300.0);
            table.rows[0].cells[0].blocks = d.body.clone();
            d.body = vec![Arc::new(Block::Table(table)), para_block(Paragraph::new())];
            (StoryRef::Body, Path(vec![0, 0, 0, 0]), Path(vec![0, 0, 0, 1]))
        } else {
            let id = d.add_part(wordcraft_doc::PartKind::Header, d.body.clone());
            (StoryRef::Part(id), Path::top(0), Path::top(1))
        };
        d.delete_range(&Pos { story, path: a.clone(), off: 0 }, &Pos { story, path: b, off: 0 }).unwrap();
        assert_format(d.para(story, &a).unwrap(), &expected);
    }
}

/// Absorbing an empty section keeps the following format and inherited header.
#[test]
fn deleting_an_empty_section_paragraph_keeps_format_and_linked_headers() {
    let mut d = fixture("", None);
    let expected = d.para_at(&Pos::body(1, 0)).unwrap().clone();
    let header = d.add_part(wordcraft_doc::PartKind::Header, vec![para_block(Paragraph::with_text("Header", Default::default()))]);
    let mut section = d.last_section.clone();
    section.headers.default = Some(header);
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(section));
    d.last_section.margin_left = 96.0;
    d.delete_range(&Pos::body(0, 0), &Pos::body(1, 0)).unwrap();
    assert_format(d.para_at(&Pos::body(0, 0)).unwrap(), &expected);
    assert!(d.para_at(&Pos::body(0, 0)).unwrap().section.is_none());
    assert_eq!(d.last_section.headers.default, Some(header));
    assert_eq!(d.last_section.margin_left, 96.0);
}

/// Deleting actual text continues to use the first paragraph merge rule.
#[test]
fn joining_nonempty_paragraphs_keeps_the_existing_first_paragraph_rule() {
    for off in [0, 7] {
        let mut d = fixture("Heading", None);
        let expected = d.para_at(&Pos::body(0, 0)).unwrap().props.clone();
        d.delete_range(&Pos::body(0, off), &Pos::body(1, 0)).unwrap();
        assert_eq!(d.para_at(&Pos::body(0, 0)).unwrap().props, expected);
    }
}

/// Removing a soft line break must not change paragraph formatting.
#[test]
fn a_soft_break_is_not_an_empty_paragraph_merge() {
    let mut d = fixture("", None);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.insert_text(0, "\nHeading", &Default::default()).unwrap();
    let expected = p.props.clone();
    d.delete_range(&Pos::body(0, 0), &Pos::body(0, 1)).unwrap();
    assert_eq!(d.body.len(), 2);
    assert_eq!(d.para_at(&Pos::body(0, 0)).unwrap().props, expected);
}
