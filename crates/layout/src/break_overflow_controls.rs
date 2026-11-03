use super::*;
use wordcraft_doc::props::{Borders, LineSpacing, ParaProps};
use wordcraft_doc::{Pos, Table, para_block};

const LINE_HEIGHT: f32 = 18.0;
const PAGE_SHORT: f32 = 300.0;
const PAGE_LONG: f32 = 420.0;
const MARGIN: f32 = 24.0;
const EPS: f32 = 0.01;

#[derive(Clone, Copy)]
enum Lead {
    Table,
    Paragraph,
}

fn paragraph(text: &str) -> Paragraph {
    let mut p = Paragraph::with_text(text, CharProps { font: Some("Arial".into()), size: Some(10.0), ..Default::default() });
    p.props = ParaProps {
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(LINE_HEIGHT)),
        widow_control: Some(false),
        ..Default::default()
    };
    p
}

#[derive(Clone, Copy)]
enum Paper {
    Landscape,
    Portrait,
}

fn section(paper: Paper) -> SectionProps {
    let landscape = matches!(paper, Paper::Landscape);
    SectionProps {
        page_w: if landscape { PAGE_LONG } else { PAGE_SHORT },
        page_h: if landscape { PAGE_SHORT } else { PAGE_LONG },
        landscape,
        margin_top: MARGIN,
        margin_bottom: MARGIN,
        margin_left: MARGIN,
        margin_right: MARGIN,
        ..Default::default()
    }
}

fn document(lead: Lead, height: f32, text: &str) -> Document {
    let first = match lead {
        Lead::Table => {
            let mut table = Table::new(1, 1, section(Paper::Landscape).text_width());
            table.props.style = None;
            table.props.cell_margins = Some([0.0; 4]);
            table.props.borders = Some(Borders::default());
            table.rows[0].props.height_rule = HeightRule::Exact;
            table.rows[0].props.height = Some(height);
            table.rows[0].cells[0].blocks = vec![para_block(paragraph("Anchor"))];
            Arc::new(Block::Table(table))
        }
        Lead::Paragraph => {
            let mut p = paragraph("Anchor");
            p.props.line_spacing = Some(LineSpacing::Exactly(height));
            para_block(p)
        }
    };
    let mut closing = paragraph("");
    closing.section = Some(Box::new(section(Paper::Landscape)));
    let mut intentional = paragraph("");
    intentional.section = Some(Box::new(section(Paper::Portrait)));
    let mut doc = Document::new();
    doc.last_section = section(Paper::Landscape);
    doc.body = vec![first, para_block(paragraph(text)), para_block(closing), para_block(intentional), para_block(paragraph("After"))];
    doc
}

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn witness(lead: Lead, text: &str) -> Document {
    // Leave room for the soft-break line, but not the later forced-break line.
    // Derive the threshold from actual line and lead frames, including table clearance.
    let probe_height = 2.0 * LINE_HEIGHT;
    let probe = lay(&document(lead, probe_height, text));
    let (y, para) = probe.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { path, y, para, .. } if path.0 == [1] => Some((*y, para)),
            _ => None,
        })
        .unwrap();
    let first = &para.lines[0];
    let second = &para.lines[1];
    assert_eq!(first.end, LineEnd::LineBreak);
    let first_bottom = first.height;
    let second_bottom = second.top + second.height - first.top;
    assert!(second_bottom > first_bottom + EPS);
    let remaining = (first_bottom + second_bottom) / 2.0;
    let gap = y - probe.pages[0].body.y - probe_height;
    let height = section(Paper::Landscape).text_height() - gap - remaining;
    assert!(height > probe_height);
    assert!(first_bottom < remaining && second_bottom > remaining);
    document(lead, height, text)
}

fn assert_transition(doc: &Document, laid: &DocLayout) {
    let landscape: Vec<_> = laid.pages.iter().map(|p| p.w > p.h).collect();
    assert_eq!(landscape, [true, false, true], "retain one intentional portrait page without an extra landscape blank");
    assert_eq!(laid.caret(&Pos::body(4, 0)).unwrap().page, 2);
    assert_eq!(laid.caret(&Pos::body(3, 0)).unwrap().page, 1);
    assert_eq!(laid.pages.iter().map(|p| p.number).collect::<Vec<_>>(), [1, 2, 3]);
    for text in ["Anchor", "After"] {
        let count = laid
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Lines { story, path, para, l0, l1, .. } => {
                    let p = doc.para(*story, path)?;
                    Some(para.lines[*l0..*l1].iter().filter(|l| p.text.get(l.start..l.stop).is_some_and(|t| t.contains(text))).count())
                }
                _ => None,
            })
            .sum::<usize>();
        assert_eq!(count, 1, "{text} must appear once");
    }
}

#[test]
fn table_soft_then_page_break() {
    let doc = witness(Lead::Table, "\n\u{000C}");
    let original = doc.clone();
    let laid = lay(&doc);
    assert_transition(&doc, &laid);
    assert_eq!(doc, original, "layout must not change the source");
}

#[test]
fn para_soft_then_page_break() {
    let doc = witness(Lead::Paragraph, "\n\u{000C}");
    assert_transition(&doc, &lay(&doc));
}

#[test]
fn visible_line_still_moves() {
    let doc = witness(Lead::Table, "\nVisible\u{000C}");
    let laid = lay(&doc);
    assert_eq!(laid.pages.len(), 4, "the later line has ink and needs its own landscape page");
    let caret = laid.caret(&Pos::body(1, 1)).unwrap();
    assert_eq!(caret.page, 1);
    assert!(caret.top >= laid.pages[1].body.y && caret.top + caret.height <= laid.pages[1].body.bottom());
    assert_eq!(laid.caret(&Pos::body(3, 0)).unwrap().page, 2);
}

#[test]
fn ordinary_overflow_still_moves() {
    let doc = witness(Lead::Table, "\nVisible");
    let laid = lay(&doc);
    assert_eq!(laid.pages.len(), 4);
    assert_eq!(laid.caret(&Pos::body(1, 1)).unwrap().page, 1);
    assert_eq!(laid.caret(&Pos::body(3, 0)).unwrap().page, 2);
}

#[test]
fn column_break_is_unchanged() {
    let doc = witness(Lead::Table, "\n\u{000E}");
    let laid = lay(&doc);
    // The single-column overflow and explicit column advance remain separate.
    assert_eq!(laid.pages.iter().map(|p| p.w > p.h).collect::<Vec<_>>(), [true, true, true, false, true]);
}

#[test]
fn note_line_is_not_pure_break() {
    use wordcraft_doc::para::NoteKind;
    let mut doc = witness(Lead::Table, "\n\u{000C}");
    let id = doc.add_part(wordcraft_doc::PartKind::Footnote, vec![para_block(paragraph("Footnote"))]);
    doc.insert_object(&Pos::body(1, 1), InlineObject::NoteRef { kind: NoteKind::Footnote, id, custom: String::new() }, &Default::default()).unwrap();
    let laid = lay(&doc);
    assert_eq!(laid.pages.len(), 4, "a note reference must not receive the ink-free exemption");
    assert_eq!(laid.caret(&Pos::body(1, 1)).unwrap().page, 1);
    let note_pages: Vec<_> = laid
        .pages
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.items.iter().any(|it| matches!(it, Placed::Lines { story: StoryRef::Part(part), .. } if *part == id)).then_some(i))
        .collect();
    assert_eq!(note_pages, [1]);
}

#[test]
fn first_pure_break_still_fits() {
    let doc = document(Lead::Table, section(Paper::Landscape).text_height(), "\u{000C}");
    assert_transition(&doc, &lay(&doc));
}

#[test]
fn intentional_breaks_survive() {
    let doc = witness(Lead::Table, "\n\u{000C}\u{000C}");
    let laid = lay(&doc);
    assert_eq!(laid.pages.iter().map(|p| p.w > p.h).collect::<Vec<_>>(), [true, true, false, true]);
    assert_eq!(laid.caret(&Pos::body(3, 0)).unwrap().page, 2);
    assert_eq!(laid.caret(&Pos::body(4, 0)).unwrap().page, 3);
}

#[test]
fn parity_and_restart_stay() {
    for start in [SectionStart::NextPage, SectionStart::OddPage, SectionStart::EvenPage] {
        let mut roomy = document(Lead::Table, LINE_HEIGHT * 2.0, "\n\u{000C}");
        let mut tight = witness(Lead::Table, "\n\u{000C}");
        for doc in [&mut roomy, &mut tight] {
            doc.last_section.start = start;
            doc.last_section.page_num_start = Some(7);
        }
        let roomy = lay(&roomy);
        let tight = lay(&tight);
        let summary = |l: &DocLayout| l.pages.iter().map(|p| (p.w, p.h, p.number, p.section)).collect::<Vec<_>>();
        assert_eq!(summary(&tight), summary(&roomy), "section start {start:?}");
        let last = tight.pages.last().unwrap();
        let number = if start == SectionStart::EvenPage { 8 } else { 7 };
        assert_eq!(last.number, number, "even-page starts advance an odd restart to the next even number");
        assert_eq!(tight.caret(&Pos::body(4, 0)).unwrap().page, tight.pages.len() - 1);
    }
}

#[test]
fn earlier_note_stays_complete() {
    use wordcraft_doc::para::NoteKind;

    #[derive(Debug)]
    enum NoteSource {
        SameParagraph,
        EarlierParagraph,
    }

    let mut failures = Vec::new();
    for source in [NoteSource::SameParagraph, NoteSource::EarlierParagraph] {
        let mut doc = Document::new();
        doc.last_section = section(Paper::Landscape);
        let note_height = 4.0 * LINE_HEIGHT;
        let filler_height = doc.last_section.text_height() - note_height - NOTE_SEP - 1.5 * LINE_HEIGHT;
        let mut filler = paragraph("Filler");
        filler.props.line_spacing = Some(LineSpacing::Exactly(filler_height));
        let id = doc.add_part(wordcraft_doc::PartKind::Footnote, vec![para_block(paragraph("One\nTwo\nThree\nFour"))]);
        let text = match source {
            NoteSource::SameParagraph => "Reference\n\u{000C}",
            NoteSource::EarlierParagraph => "Reference",
        };
        let mut reference = paragraph(text);
        reference
            .insert_object("Reference".len(), InlineObject::NoteRef { kind: NoteKind::Footnote, id, custom: String::new() }, &Default::default())
            .unwrap();
        doc.body = vec![para_block(filler), para_block(reference)];
        if matches!(source, NoteSource::EarlierParagraph) {
            doc.body.push(para_block(paragraph("\u{000C}")));
        }
        doc.body.push(para_block(paragraph("After")));
        let original = doc.clone();
        let laid = lay(&doc);
        assert_eq!(laid.caret(&Pos::body(1, 0)).unwrap().page, 0, "the reference and its whole note fit before the break");
        let note_pages: Vec<_> = laid
            .pages
            .iter()
            .enumerate()
            .filter_map(|(page, p)| {
                p.items.iter().any(|item| matches!(item, Placed::Lines { story: StoryRef::Part(part), .. } if *part == id)).then_some(page)
            })
            .collect();
        let note_lines: usize = laid
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                Placed::Lines { story: StoryRef::Part(part), l0, l1, .. } if *part == id => Some(l1 - l0),
                _ => None,
            })
            .sum();
        assert_eq!(note_lines, 4, "the whole note is retained");
        if note_pages != [0] {
            failures.push(format!("{source:?}: note on {note_pages:?} instead of [0]"));
        }
        assert_eq!(doc, original);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
