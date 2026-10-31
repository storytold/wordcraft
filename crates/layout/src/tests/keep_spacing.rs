use super::*;
use wordcraft_doc::props::{LineSpacing, NumRef};

/// Leave a measured page remainder before a kept heading and five short body lines.
fn heading_at_edge(room: f32) -> Document {
    let mut doc = pagination_fixture_document("SYNTHETIC FILLER");
    let body = doc.last_section.text_height();
    let plain = ParaProps {
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(14.0)),
        widow_control: Some(true),
        ..Default::default()
    };
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props = ParaProps { line_spacing: Some(LineSpacing::Exactly(body - room)), ..plain.clone() };
    let mut heading = wordcraft_doc::Paragraph::with_text("SYNTHETIC HEADING", Default::default());
    heading.props = ParaProps {
        keep_next: Some(true),
        keep_lines: Some(true),
        line_spacing: Some(LineSpacing::Exactly(16.0)),
        space_after: Some(6.0),
        ..plain.clone()
    };
    doc.body.push(wordcraft_doc::para_block(heading));
    let mut body = wordcraft_doc::Paragraph::with_text("Ba\nBb\nBc\nBd\nBe", Default::default());
    body.props = plain;
    doc.body.push(wordcraft_doc::para_block(body));
    doc
}

/// Return the pages of the heading and body lines while checking font and content retention.
fn pages(doc: &Document, body: usize) -> [usize; 4] {
    let layout = lay(doc);
    assert_pagination_fixture_font(&layout);
    let count: usize = layout
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, path, l0, l1, .. } if path.0 == [body as u32] => Some(l1 - l0),
            _ => None,
        })
        .sum();
    assert_eq!(count, 5, "all five body lines survive pagination");
    [Pos::body(1, 0), Pos::body(body, 0), Pos::body(body, 3), Pos::body(body, 12)].map(|pos| layout.caret(&pos).unwrap().page)
}

/// The office corpus heading has 6 pt after it: its 16 pt line and two 14 pt body lines
/// need 50 pt, not 44. With 48 pt left, Word carries the heading to the next page.
#[test]
fn kept_heading_counts_its_after_space_before_a_widow_controlled_paragraph() {
    assert_eq!(pages(&heading_at_edge(48.0), 2), [1, 1, 1, 1], "the heading follows the body instead of being stranded");
    assert_eq!(pages(&heading_at_edge(50.0), 2), [0, 0, 0, 1], "two body lines hold the heading when the complete gap fits");
}

/// Disabling either control preserves the independent placement allowed by that setting.
#[test]
fn kept_heading_spacing_respects_disabled_keep_and_widow_controls() {
    let mut doc = heading_at_edge(48.0);
    doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.keep_next = Some(false);
    assert_eq!(pages(&doc, 2), [0, 1, 1, 1], "without keep with next the heading stays");
    let mut doc = heading_at_edge(48.0);
    doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.widow_control = Some(false);
    assert_eq!(pages(&doc, 2), [0, 0, 1, 1], "without widow control one body line may remain below it");
}

/// Count a merged before/after gap once, while retaining the explicit additive setting.
#[test]
fn kept_heading_uses_the_collapsed_or_additive_gap_once() {
    let mut doc = heading_at_edge(54.0);
    doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.space_before = Some(8.0);
    assert_eq!(pages(&doc, 2), [0, 0, 0, 1], "the 6/8 pt sides collapse to 8 pt, so 52 pt fits");
    doc.settings.sum_paragraph_spacing = true;
    assert_eq!(pages(&doc, 2), [1, 1, 1, 1], "the additive 14 pt gap needs 58 pt and moves both paragraphs");
}

/// Each same-style paragraph suppresses only the spacing its contextual setting removes.
#[test]
fn kept_heading_uses_each_paragraphs_contextual_spacing() {
    for (heading_contextual, body_contextual, room) in [(true, false, 48.0), (false, true, 51.0), (true, true, 48.0)] {
        let mut doc = heading_at_edge(room);
        let heading = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
        heading.props.contextual_spacing = Some(heading_contextual);
        let body = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
        body.props.space_before = Some(8.0);
        body.props.contextual_spacing = Some(body_contextual);
        assert_eq!(pages(&doc, 2), [0, 0, 0, 1], "the actual contextual gap fits ({heading_contextual}, {body_contextual})");
    }
}

/// A list keeps its initial spacing, suppresses the intra-list gap and preserves numbering.
#[test]
fn kept_list_items_drop_automatic_spacing_without_losing_their_numbers() {
    // The first list item retains its own 14 pt before-space. Of the 62 pt remainder,
    // 48 pt is therefore available for the heading, the suppressed list gap and the body.
    let mut doc = heading_at_edge(62.0);
    let num = doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    for block in [1, 2] {
        let props = &mut doc.para_mut(StoryRef::Body, &Path::top(block)).unwrap().props;
        props.numbering = Some(NumRef { num, level: 0 });
        props.space_before_auto = Some(true);
        props.space_after_auto = Some(true);
    }
    assert_eq!(pages(&doc, 2), [0, 0, 0, 1], "automatic list spacing must not force an unnecessary page break");
    let layout = lay(&doc);
    for (block, expected) in [(1, "1."), (2, "2.")] {
        assert!(layout.pages.iter().flat_map(|p| &p.items).any(|item| {
            matches!(item, Placed::Lines { path, para, .. } if path.0 == [block] && para.label.as_ref().is_some_and(|label| label.text.trim() == expected))
        }), "lookahead must not consume or restart the list counter");
    }
}

/// A kept chain reserves both intervening gaps before deciding whether its last paragraph can start.
#[test]
fn kept_chain_counts_every_effective_gap_once() {
    let mut doc = heading_at_edge(68.0);
    let mut middle = wordcraft_doc::Paragraph::with_text("SYNTHETIC MIDDLE", Default::default());
    middle.props = ParaProps {
        keep_next: Some(true),
        line_spacing: Some(LineSpacing::Exactly(14.0)),
        space_before: Some(0.0),
        space_after: Some(6.0),
        ..Default::default()
    };
    doc.body.insert(2, wordcraft_doc::para_block(middle));
    assert_eq!(pages(&doc, 3), [1, 1, 1, 1], "16 + 6 + 14 + 6 + 28 pt does not fit in 68 pt");
    let layout = lay(&doc);
    assert_eq!(layout.caret(&Pos::body(2, 0)).unwrap().page, 1, "the middle of the chain moves with both ends");
}

/// A looked-ahead footnote requires a full multiple-spaced line, including in a kept chain.
#[test]
fn kept_lookahead_with_a_footnote_keeps_the_whole_line_height() {
    use wordcraft_doc::para::NoteKind;
    use wordcraft_doc::{Paragraph, PartKind, para_block};
    let check_font = |layout: &DocLayout| {
        let mut count = 0;
        for item in layout.pages.iter().flat_map(|page| &page.items) {
            if let Placed::Lines { para, .. } = item {
                for style in &para.styles {
                    count += 1;
                    assert_eq!(style.face.family, WRAP_TEST_FONT);
                    assert_eq!(style.face.style, "Regular");
                    assert!(style.face.data() == include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf").as_slice());
                    // Automatic note references are superscripted to 65% of the 12 pt body.
                    assert!((style.size - 12.0).abs() < 0.01 || (style.size - 7.8).abs() < 0.01);
                }
            }
        }
        assert!(count > 0);
    };
    for (kind, part_kind) in [(NoteKind::Footnote, PartKind::Footnote), (NoteKind::Endnote, PartKind::Endnote)] {
        for chained in [false, true] {
            let mut doc = heading_at_edge(120.0);
            let body = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
            body.props.line_spacing = Some(LineSpacing::Multiple(2.0));
            body.props.widow_control = Some(false);
            let mut note = Paragraph::with_text("SYNTHETIC NOTE", Default::default());
            note.props = ParaProps { line_spacing: Some(LineSpacing::Exactly(14.0)), space_after: Some(0.0), ..Default::default() };
            let id = doc.add_part(part_kind, vec![para_block(note)]);
            doc.insert_object(&Pos::body(2, 0), InlineObject::NoteRef { kind, id, custom: String::new() }, &Default::default()).unwrap();
            let body_block = if chained {
                let mut middle = Paragraph::with_text("SYNTHETIC MIDDLE", Default::default());
                middle.props = ParaProps {
                    keep_next: Some(true),
                    line_spacing: Some(LineSpacing::Exactly(14.0)),
                    space_before: Some(0.0),
                    space_after: Some(0.0),
                    ..Default::default()
                };
                doc.body.insert(2, para_block(middle));
                3
            } else {
                2
            };
            let probe = lay(&doc);
            check_font(&probe);
            let first_height = probe
                .pages
                .iter()
                .flat_map(|p| &p.items)
                .find_map(|item| match item {
                    Placed::Lines { story: StoryRef::Body, path, para, .. } if path.0 == [body_block as u32] => {
                        para.lines.first().map(|line| line.height)
                    }
                    _ => None,
                })
                .unwrap();
            // Just short of the complete heading + effective gap + next line. A leading discount
            // would leave the heading behind even though actual note placement needs the full line.
            let room = 16.0 + 6.0 + if chained { 14.0 } else { 0.0 } + first_height - 0.5;
            doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.line_spacing =
                Some(LineSpacing::Exactly(doc.last_section.text_height() - room));
            let layout = lay(&doc);
            check_font(&layout);
            let page = |block| layout.caret(&Pos::body(block, 0)).unwrap().page;
            let expected = if kind == NoteKind::Footnote { 1 } else { 0 };
            assert_eq!(page(1), expected, "{kind:?}, chained={chained}: heading follows the line's actual room requirement");
            assert_eq!(page(body_block), expected, "{kind:?}, chained={chained}: the first body line stays with the heading");
            if chained {
                assert_eq!(page(2), expected, "the middle paragraph travels with both ends");
            }
            if kind == NoteKind::Footnote {
                assert!(
                    layout.pages[1].items.iter().any(|item| matches!(item, Placed::Lines { story: StoryRef::Part(part), .. } if *part == id)),
                    "the note is placed on the reference page"
                );
            }
        }
    }
}
