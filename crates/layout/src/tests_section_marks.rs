use super::headers::bookmark_pages;
use super::notes::v_span;
use super::*;
use wordcraft_doc::props::{LineSpacing, ParaProps};
use wordcraft_doc::section::{LineNumberRestart, LineNumbering, SectionProps};

const LINE_HEIGHT: f32 = 20.0;
const PAGE_WIDTH: f32 = 612.0;
const PAGE_HEIGHT: f32 = 792.0;
const FONT_NAME: &str = "Arial";
const FONT_SIZE: f32 = 10.0;
const COMPAT_MODE: u32 = 15;
const PAGE_MARGIN: f32 = 72.0;
const NUMBER_DISTANCE: f32 = 18.0;
const PAGE_NUMBERS: &[&[u32]] = &[&[1, 2, 3], &[1, 2], &[1, 2, 3], &[1], &[1, 2], &[1, 2, 3], &[1, 2, 3]];
const SECTION_NUMBERS: &[&[u32]] = &[&[1, 2, 3], &[1, 2], &[1, 2, 3], &[4], &[1, 2], &[1, 2, 3], &[4, 5, 6]];
const CONTINUOUS_NUMBERS: &[&[u32]] = &[&[1, 2, 3], &[4, 5], &[6, 7, 8], &[9], &[10, 11], &[12, 13, 14], &[15, 16, 17]];
const STRUCTURAL_ENDS: [usize; 4] = [3, 5, 9, 11];
const VISIBLE_PARAGRAPHS: [(usize, usize); 13] =
    [(0, 0), (1, 0), (4, 1), (5, 1), (6, 2), (7, 2), (9, 3), (10, 4), (11, 4), (12, 5), (13, 5), (16, 6), (17, 6)];

fn numbered_document(mode: LineNumberRestart) -> Document {
    let numbering = LineNumbering::default();
    assert_eq!(numbering.start, 1);
    assert_eq!(numbering.count_by, 1);
    let section = SectionProps {
        page_w: PAGE_WIDTH,
        page_h: PAGE_HEIGHT,
        margin_top: PAGE_MARGIN,
        margin_bottom: PAGE_MARGIN,
        margin_left: PAGE_MARGIN,
        margin_right: PAGE_MARGIN,
        header: 0.0,
        footer: 0.0,
        line_numbers: Some(LineNumbering { distance: NUMBER_DISTANCE, restart: mode, ..numbering }),
        ..Default::default()
    };
    let texts = [
        "Empty BEFORE 1",
        "Empty BEFORE 2",
        "\u{000C}",
        "",
        "Empty AFTER 1",
        "Empty AFTER 2",
        "Nonempty BEFORE 1",
        "Nonempty BEFORE 2",
        "\u{000C}",
        "NONEMPTY MARK",
        "Nonempty AFTER 1",
        "Nonempty AFTER 2",
        "Ordinary BEFORE 1",
        "Ordinary BEFORE 2",
        "\u{000C}",
        "",
        "Ordinary AFTER 1",
        "Ordinary AFTER 2",
    ];
    let mut doc = Document::new();
    doc.body.clear();
    doc.settings.compat_mode = COMPAT_MODE;
    let char_props = CharProps { font: Some(FONT_NAME.into()), size: Some(FONT_SIZE), ..Default::default() };
    doc.styles.default_chr = char_props.clone();
    doc.last_section = section.clone();
    for (index, text) in texts.into_iter().enumerate() {
        let mut p = Paragraph::with_text(text, char_props.clone());
        p.props = ParaProps {
            space_before: Some(0.0),
            space_after: Some(0.0),
            line_spacing: Some(LineSpacing::Exactly(LINE_HEIGHT)),
            widow_control: Some(false),
            snap_to_grid: Some(false),
            ..Default::default()
        };
        if STRUCTURAL_ENDS.contains(&index) {
            p.section = Some(Box::new(section.clone()));
        }
        doc.body.push(wordcraft_doc::para_block(p));
    }
    doc
}

fn bookmarked_document(mode: LineNumberRestart) -> Document {
    let mut doc = numbered_document(mode);
    for block in 0..doc.body.len() {
        let p = doc.para_mut(StoryRef::Body, &Path::top(block)).unwrap();
        let name = format!("p{block}");
        p.insert_object(0, InlineObject::BookmarkStart { name: name.clone() }, &Default::default()).unwrap();
        p.insert_object(p.len(), InlineObject::BookmarkEnd { name }, &Default::default()).unwrap();
    }
    doc
}

fn marker_paragraph(text: &str, objects: Vec<InlineObject>) -> Paragraph {
    let mut p = Paragraph::new();
    p.text = text.into();
    p.objects = objects;
    if !text.is_empty() {
        p.runs.push(wordcraft_doc::para::Run { len: text.len(), props: Default::default() });
    }
    p
}

// Read rendered digits through their actual faces, without inspecting the pagination counter.
fn shown_number(para: &para::ParaLayout) -> u32 {
    let text: String = para
        .clusters
        .iter()
        .map(|cluster| {
            assert_eq!(cluster.kind, para::ClKind::Text);
            let glyphs = &para.glyphs[cluster.g0 as usize..cluster.g1 as usize];
            assert_eq!(glyphs.len(), 1);
            assert_ne!(glyphs[0].gid, 0);
            let face = &para.styles[cluster.style as usize].face;
            ('0'..='9').find(|digit| wordcraft_fonts::first_glyph(face, &[*digit]) == glyphs[0].gid).unwrap()
        })
        .collect();
    text.parse().unwrap()
}

fn body_slot(laid: &DocLayout, block: usize) -> (usize, f32) {
    laid.pages
        .iter()
        .enumerate()
        .find_map(|(page, p)| {
            p.items.iter().find_map(|item| match item {
                Placed::Lines { story: StoryRef::Body, path, y, .. } if *path == Path::top(block) => Some((page, *y)),
                _ => None,
            })
        })
        .unwrap()
}

fn margin_numbers(laid: &DocLayout) -> Vec<Vec<u32>> {
    laid.pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Placed::Lines { story, para, .. } if *story == DECOR_STORY => Some(shown_number(para)),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn check_number_doc(doc: &Document, mode: LineNumberRestart, expected: &[&[u32]]) {
    let laid = layout(doc, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(laid.pages.len(), 7);
    let numbers = margin_numbers(&laid);
    let expected: Vec<Vec<u32>> = expected.iter().map(|page| page.to_vec()).collect();
    assert_eq!(numbers, expected, "restart={mode:?}");
    assert_eq!(numbers.iter().map(Vec::len).sum::<usize>(), 17);

    for (block, page) in VISIBLE_PARAGRAPHS {
        assert_eq!(body_slot(&laid, block).0, page, "restart={mode:?}, paragraph={block}");
    }
    let blank = body_slot(&laid, 15);
    let after = body_slot(&laid, 16);
    assert_eq!(blank.0, 6);
    assert_eq!(after.0, 6);
    assert!((after.1 - blank.1 - LINE_HEIGHT).abs() < 0.01);
}

#[test]
fn break_mark_counted_once() {
    let mut doc = numbered_document(LineNumberRestart::Continuous);
    doc.body = [0, 2, 4].map(|block| doc.body[block].clone()).to_vec();
    let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(laid.pages.len(), 2);
    assert_eq!(margin_numbers(&laid), vec![vec![1, 2], vec![3]]);
}

#[test]
fn page_mark_numbers() {
    check_number_doc(&numbered_document(LineNumberRestart::Page), LineNumberRestart::Page, PAGE_NUMBERS);
}

#[test]
fn section_mark_numbers() {
    check_number_doc(&numbered_document(LineNumberRestart::Section), LineNumberRestart::Section, SECTION_NUMBERS);
}

#[test]
fn continuous_mark_numbers() {
    check_number_doc(&numbered_document(LineNumberRestart::Continuous), LineNumberRestart::Continuous, CONTINUOUS_NUMBERS);
}

#[test]
fn bookmarked_page_numbers() {
    check_number_doc(&bookmarked_document(LineNumberRestart::Page), LineNumberRestart::Page, PAGE_NUMBERS);
}

#[test]
fn bookmarked_section_numbers() {
    check_number_doc(&bookmarked_document(LineNumberRestart::Section), LineNumberRestart::Section, SECTION_NUMBERS);
}

#[test]
fn bookmarked_continuous_numbers() {
    check_number_doc(&bookmarked_document(LineNumberRestart::Continuous), LineNumberRestart::Continuous, CONTINUOUS_NUMBERS);
}

#[test]
fn malformed_bookmarks_rejected() {
    assert!(only_bookmarks(&Paragraph::new()));
    let object = InlineObject::BookmarkStart { name: "marker".into() };
    let obj = wordcraft_doc::para::OBJ.to_string();
    let cases = [
        (obj.clone(), vec![]),
        (String::new(), vec![object.clone()]),
        (obj.clone(), vec![object.clone(), object.clone()]),
        (obj.repeat(2), vec![object.clone()]),
        ("x".into(), vec![object.clone()]),
        (format!("{obj}x"), vec![object.clone(), object]),
    ];
    for (text, objects) in cases {
        let p = marker_paragraph(&text, objects);
        assert!(!only_bookmarks(&p), "text={text:?}, objects={:?}", p.objects);
    }
}

#[test]
fn many_bookmarks_are_eligible() {
    const BOOKMARK_PAIRS: usize = 8192;
    const MARKER_COUNT: usize = 2 * BOOKMARK_PAIRS;
    let text = wordcraft_doc::para::OBJ.to_string().repeat(MARKER_COUNT);
    let objects = (0..BOOKMARK_PAIRS)
        .flat_map(|index| {
            let name = format!("marker{index}");
            [InlineObject::BookmarkStart { name: name.clone() }, InlineObject::BookmarkEnd { name }]
        })
        .collect();
    let mut p = marker_paragraph(&text, objects);
    assert_eq!(p.text.chars().count(), MARKER_COUNT);
    assert_eq!(p.objects.len(), MARKER_COUNT);
    assert!(only_bookmarks(&p));
    p.text.push(wordcraft_doc::para::OBJ);
    assert!(!only_bookmarks(&p));
}

#[test]
fn nonbookmark_targets_stay() {
    let cases = [
        vec![InlineObject::FieldStart { instr: "REF target".into(), locked: false, dirty: false }, InlineObject::FieldEnd],
        vec![InlineObject::ControlStart { control: Box::default() }, InlineObject::ControlEnd],
        vec![InlineObject::CommentStart { id: 1 }, InlineObject::CommentEnd { id: 1 }],
        vec![InlineObject::PermStart { id: 1, group: "everyone".into(), editor: String::new() }, InlineObject::PermEnd { id: 1 }],
        vec![InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() }],
        vec![InlineObject::Image {
            media: "original-layout-only-image".into(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        }],
    ];
    for mode in [LineNumberRestart::Page, LineNumberRestart::Section, LineNumberRestart::Continuous] {
        for objects in &cases {
            let mut doc = bookmarked_document(mode);
            let path = Path::top(3);
            let p = doc.para_mut(StoryRef::Body, &path).unwrap();
            for object in objects {
                p.insert_object(p.len(), object.clone(), &Default::default()).unwrap();
            }
            let original = p.objects.clone();
            let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
            assert_eq!(laid.pages.len(), 8, "restart={mode:?}, objects={objects:?}");
            assert_eq!(body_slot(&laid, 3).0, 1);
            assert_eq!(body_slot(&laid, 4).0, 2);
            assert_eq!(doc.para(StoryRef::Body, &path).unwrap().objects, original);
        }
    }
}

#[test]
fn relocated_bookmark_page_refs() {
    let mut doc = bookmarked_document(LineNumberRestart::Continuous);
    let mut footer = Paragraph::new();
    footer
        .insert_object(
            0,
            InlineObject::Field { instr: "PAGEREF p3".into(), result: "99".into(), locked: false, dirty: false, code: false, form: String::new() },
            &Default::default(),
        )
        .unwrap();
    let id = doc.add_part(wordcraft_doc::PartKind::Footer, vec![wordcraft_doc::para_block(footer)]);
    for block in STRUCTURAL_ENDS {
        doc.para_mut(StoryRef::Body, &Path::top(block)).unwrap().section.as_mut().unwrap().footers.default = Some(id);
    }
    doc.last_section.footers.default = Some(id);
    let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(laid.pages.len(), 7);
    let marks = bookmark_pages(&doc, &laid.pages, &doc.sections());
    let pages = [1, 1, 1, 1, 2, 2, 3, 3, 3, 4, 5, 5, 6, 6, 6, 7, 7, 7];
    for (block, page) in pages.into_iter().enumerate() {
        let p = doc.para(StoryRef::Body, &Path::top(block)).unwrap();
        assert_eq!(p.objects.len(), 2);
        assert!(matches!(p.objects[0], InlineObject::BookmarkStart { .. }));
        assert!(matches!(p.objects[1], InlineObject::BookmarkEnd { .. }));
        assert_eq!(marks.get(&format!("p{block}")).map(|(n, _)| *n), Some(page));
    }
    for page in &laid.pages {
        let shown: Vec<_> = page
            .footer
            .iter()
            .filter_map(|item| match item {
                Placed::Lines { para, .. } => Some(para.shown.iter().map(|shown| shown.1.as_str()).collect::<String>()),
                _ => None,
            })
            .collect();
        assert_eq!(shown, ["1"]);
    }
}

#[test]
fn mark_stays_above_footnote() {
    let mut doc = bookmarked_document(LineNumberRestart::Continuous);
    let note = doc.add_part(
        wordcraft_doc::PartKind::Footnote,
        vec![wordcraft_doc::para_block(Paragraph::with_text("Original footnote text.", Default::default()))],
    );
    let before = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    before
        .insert_object(
            before.len(),
            InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id: note, custom: String::new() },
            &Default::default(),
        )
        .unwrap();
    let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(laid.pages.len(), 7);
    let page = &laid.pages[0];
    let mark_path = Path::top(3);
    let body_bottom = page
        .items
        .iter()
        .filter(|item| match item {
            Placed::Lines { story: StoryRef::Body, path, .. } => *path != mark_path,
            Placed::Cell { story: StoryRef::Body, .. } => true,
            _ => false,
        })
        .map(|item| v_span(item).1)
        .fold(page.body.y, f32::max);
    let note_top = page
        .items
        .iter()
        .filter_map(|item| match item {
            Placed::Lines { story: StoryRef::Part(id), .. } if *id == note => Some(v_span(item).0),
            _ => None,
        })
        .reduce(f32::min)
        .unwrap();
    assert!(note_top > body_bottom + LINE_HEIGHT, "a real footnote must lie below the body");
    let caret = laid.caret(&wordcraft_doc::Pos::body(3, 0)).unwrap();
    assert_eq!(caret.page, 0);
    assert!(
        caret.top >= body_bottom - 0.01 && caret.top < body_bottom + LINE_HEIGHT,
        "caret {caret:?} must follow body bottom {body_bottom}, not footnote {note_top}"
    );
    assert!((body_slot(&laid, 3).1 - body_bottom).abs() < 0.01, "mark must start at body bottom {body_bottom}, not footnote {note_top}");
    assert!(caret.top + caret.height < note_top);
}

#[test]
fn mark_keeps_break_column() {
    for (columns, widths) in [(1, vec![]), (2, vec![]), (3, vec![]), (2, vec![(100.0, 36.0), (332.0, 0.0)])] {
        let mut doc = numbered_document(LineNumberRestart::Continuous);
        doc.body = [0, 2, 3, 4].map(|i| doc.body[i].clone()).to_vec();
        let section = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section.as_mut().unwrap();
        section.columns.count = columns;
        section.columns.widths = widths;
        let lead = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        lead.text = format!("{}Lead", "\u{000e}".repeat((columns - 1) as usize));
        lead.runs[0].len = lead.text.len();
        let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        let find = |block| {
            laid.pages[0]
                .items
                .iter()
                .find_map(|it| match it {
                    Placed::Lines { path, x, para, .. } if *path == Path::top(block) => Some((*x, para.lines[0].right)),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(laid.pages.len(), 2);
        let break_geometry = find(1);
        assert_eq!(find(2), break_geometry, "columns={columns}");
        let caret = laid.caret(&wordcraft_doc::Pos::body(2, 0)).unwrap();
        assert_eq!(caret.page, 0);
        assert_eq!(caret.x, break_geometry.0);
    }
}

#[test]
fn mark_keeps_break_column_bottom() {
    for widths in [vec![], vec![(100.0, 36.0), (332.0, 0.0)]] {
        let mut doc = numbered_document(LineNumberRestart::Continuous);
        doc.body = [0, 2, 3, 4].map(|i| doc.body[i].clone()).to_vec();
        let section = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section.as_mut().unwrap();
        section.columns.count = 2;
        section.columns.widths = widths;
        let lead = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        lead.text = "Tall\nTall\nTall\nTall\u{000e}Short".into();
        lead.runs[0].len = lead.text.len();
        let original = doc.clone();
        let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        assert_eq!(laid.pages.len(), 2);
        let page = &laid.pages[0];
        let find = |block| {
            page.items
                .iter()
                .find_map(|item| match item {
                    Placed::Lines { story: StoryRef::Body, path, x, y, .. } if *path == Path::top(block) => Some((*x, *y, v_span(item).1)),
                    _ => None,
                })
                .unwrap()
        };
        let break_geometry = find(1);
        let prior_bottom = page
            .items
            .iter()
            .filter_map(|item| match item {
                Placed::Lines { story: StoryRef::Body, x, .. } if *x < break_geometry.0 => Some(v_span(item).1),
                _ => None,
            })
            .reduce(f32::max)
            .unwrap();
        assert!(prior_bottom > break_geometry.2, "the prior column must be taller than the break column");
        let mark = find(2);
        assert_eq!(mark.0, break_geometry.0);
        assert_eq!(mark.1, break_geometry.2, "mark must follow its own break, not prior-column bottom {prior_bottom}");
        let caret = laid.caret(&wordcraft_doc::Pos::body(2, 0)).unwrap();
        assert_eq!(caret.page, 0);
        assert_eq!(caret.x, break_geometry.0);
        assert!(caret.top >= mark.1 && caret.top < mark.1 + LINE_HEIGHT);
        assert_eq!(doc, original);
    }
}

#[test]
fn mark_uses_indented_break_end() {
    for mirrored in [false, true] {
        let mut doc = numbered_document(LineNumberRestart::Continuous);
        doc.body = [0, 2, 3, 4].map(|i| doc.body[i].clone()).to_vec();
        doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section.as_mut().unwrap().columns.count = 2;
        let lead = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        lead.text = "\u{000e}Short".into();
        lead.runs[0].len = lead.text.len();
        let mut first_page = Paragraph::with_text("\u{000c}", doc.styles.default_chr.clone());
        first_page.props = doc.para(StoryRef::Body, &Path::top(1)).unwrap().props.clone();
        doc.body.insert(0, wordcraft_doc::para_block(first_page));
        for block in [1, 2] {
            let p = doc.para_mut(StoryRef::Body, &Path::top(block)).unwrap();
            p.props.indent_left = Some(if mirrored { 36.0 } else { -24.0 });
            p.props.indent_first = Some(-12.0);
            p.props.indent_right = Some(0.0);
            p.props.mirror_indents = Some(mirrored);
        }
        let original = doc.clone();
        let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        assert_eq!(laid.pages.len(), 3);
        let page = &laid.pages[1];
        let (x, bottom) = page
            .items
            .iter()
            .find_map(|item| match item {
                Placed::Lines { story: StoryRef::Body, path, x, .. } if *path == Path::top(2) => Some((*x, v_span(item).1)),
                _ => None,
            })
            .unwrap();
        let column_left = page.body.x + doc.sections()[0].1.column_boxes()[1].0;
        if mirrored {
            assert!(x < column_left, "mirrored indents move the stored origin outside its owning column");
        } else {
            assert_eq!(x, column_left, "ordinary negative indents leave the placement origin unchanged");
        }
        assert_eq!(body_slot(&laid, 3), (1, bottom), "mirrored={mirrored}");
        assert_eq!(laid.caret(&wordcraft_doc::Pos::body(3, 0)).unwrap().page, 1);
        assert_eq!(doc, original);
    }
}

#[test]
fn mark_follows_indented_table() {
    let mut doc = numbered_document(LineNumberRestart::Continuous);
    doc.body = [0, 2, 3, 4].map(|i| doc.body[i].clone()).to_vec();
    doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section.as_mut().unwrap().columns.count = 2;
    let lead = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    lead.text = "\u{000e}Short".into();
    lead.runs[0].len = lead.text.len();
    let mut cell = Paragraph::with_text("Cell", Default::default());
    cell.props = lead.props.clone();
    let mut table = wordcraft_doc::Table::new(1, 1, 100.0);
    table.props.indent = Some(-24.0);
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cell)];
    doc.body.insert(1, std::sync::Arc::new(wordcraft_doc::Block::Table(table)));
    let original = doc.clone();
    let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(laid.pages.len(), 2);
    let page = &laid.pages[0];
    let cell = page
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Cell { story: StoryRef::Body, table, rect, .. } if *table == Path::top(1) => Some(*rect),
            _ => None,
        })
        .unwrap();
    let (x, bottom) = page
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, path, x, .. } if *path == Path::top(2) => Some((*x, v_span(item).1)),
            _ => None,
        })
        .unwrap();
    assert!(cell.x < x, "the owning column's table starts outside its left edge");
    assert!(bottom >= cell.bottom());
    assert_eq!(body_slot(&laid, 3), (0, bottom));
    assert_eq!(doc, original);
}

#[test]
fn mark_follows_aligned_break() {
    use wordcraft_doc::section::PageVAlign;
    for align in [PageVAlign::Center, PageVAlign::Bottom, PageVAlign::Justify] {
        let mut doc = numbered_document(LineNumberRestart::Continuous);
        doc.body = [0, 2, 3, 4].map(|i| doc.body[i].clone()).to_vec();
        doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section.as_mut().unwrap().valign = align;
        let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        let bottom = laid.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                Placed::Lines { story: StoryRef::Body, path, .. } if *path == Path::top(1) => Some(v_span(item).1),
                _ => None,
            })
            .unwrap();
        assert_eq!(body_slot(&laid, 2), (0, bottom), "alignment={align:?}");
    }
}

#[test]
fn mark_keeps_prior_section_width() {
    let mut doc = numbered_document(LineNumberRestart::Continuous);
    doc.body = [0, 2, 3, 4].map(|i| doc.body[i].clone()).to_vec();
    let mut prior = doc.last_section.clone();
    prior.columns.count = 2;
    prior.columns.widths = vec![(100.0, 36.0), (332.0, 0.0)];
    doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section = Some(Box::new(prior));
    let mark = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    mark.section.as_mut().unwrap().start = SectionStart::Continuous;
    let lead = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    lead.text = "\u{000e}Lead".into();
    lead.runs[0].len = lead.text.len();
    let original = doc.clone();
    let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
    let find = |block| {
        laid.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                Placed::Lines { story: StoryRef::Body, path, x, para, .. } if *path == Path::top(block) => Some((*x, para.lines[0].right)),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(find(2), find(1), "a continuous section must not replace the break's column geometry");
    assert_eq!(doc, original);
}

#[test]
fn mark_stays_after_flow_table() {
    mark_after_table(None);
}

#[test]
fn mark_stays_after_float_table() {
    use wordcraft_doc::para::Anchor;
    mark_after_table(Some(wordcraft_doc::props::TableFloat {
        h_rel: Anchor::Page,
        v_rel: Anchor::Page,
        x: PAGE_MARGIN,
        y: PAGE_MARGIN,
        ..Default::default()
    }));
}

fn mark_after_table(float: Option<wordcraft_doc::props::TableFloat>) {
    let mut doc = numbered_document(LineNumberRestart::Continuous);
    doc.body = [0, 2, 3, 4].map(|i| doc.body[i].clone()).to_vec();
    let mut table = wordcraft_doc::Table::new(1, 1, 100.0);
    table.props.float = float;
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text("Intervening table", Default::default()))];
    doc.body.insert(2, std::sync::Arc::new(wordcraft_doc::Block::Table(table)));
    let original = doc.clone();
    let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
    let table_page = laid
        .pages
        .iter()
        .position(|page| page.items.iter().any(|item| matches!(item, Placed::Cell { story: StoryRef::Body, table, .. } if *table == Path::top(2))))
        .unwrap();
    assert_eq!(table_page, 1);
    assert_eq!(body_slot(&laid, 3).0, table_page, "the section mark must not cross a table after the break");
    assert_eq!(laid.caret(&wordcraft_doc::Pos::body(3, 0)).unwrap().page, table_page);
    assert_eq!(doc, original);
}
