//! Round trips: documents written as `.doc` and read back with the reader, which follows the
//! same specification independently of the writer.

use std::sync::Arc;

use wordcraft_doc::para::{InlineObject, NoteKind, Paragraph};
use wordcraft_doc::props::*;
use wordcraft_doc::section::{Columns, SectionProps, SectionStart};
use wordcraft_doc::table::{Cell, Row, Table};
use wordcraft_doc::{Block, Document, PartKind, para_block};

use crate::{read, write};

fn doc_of(paras: Vec<Paragraph>) -> Document {
    let mut d = Document::new();
    d.body = paras.into_iter().map(para_block).collect();
    d
}

fn round(d: &Document) -> Document {
    let bytes = write(d).expect("write");
    read(&bytes).expect("read back")
}

fn texts(d: &Document) -> Vec<String> {
    d.body.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect()
}

fn para(text: &str) -> Paragraph {
    Paragraph::with_text(text, CharProps::default())
}

#[test]
fn plain_text_round_trips() {
    let d = doc_of(vec![
        para("Hello, world."),
        para(""),
        para("Ünïcødé — “quotes” 😀 𝄞 中文"),
        para("tab\there, line\nbreak, nb\u{2011}hyphen, soft\u{AD}hyphen"),
    ]);
    let r = round(&d);
    assert_eq!(texts(&r), texts(&d));
}

#[test]
fn an_empty_document_is_one_paragraph() {
    let r = round(&Document::new());
    assert_eq!(texts(&r), vec![String::new()]);
}

#[test]
fn character_formatting_round_trips() {
    let mut p = para("bold italic under red big font high sup");
    let fmt = |p: &mut Paragraph, a: usize, b: usize, f: &dyn Fn(&mut CharProps)| p.format(a, b, f).unwrap();
    fmt(&mut p, 0, 4, &|c| c.bold = Some(true));
    fmt(&mut p, 5, 11, &|c| c.italic = Some(true));
    fmt(&mut p, 12, 17, &|c| c.underline = Some(Underline::Double));
    fmt(&mut p, 18, 21, &|c| c.color = Some(TextColor::Rgb(wordcraft_doc::Rgb(0xC0, 0x10, 0x20))));
    fmt(&mut p, 22, 25, &|c| c.size = Some(18.5));
    fmt(&mut p, 26, 30, &|c| c.font = Some("Georgia".into()));
    fmt(&mut p, 31, 35, &|c| c.highlight = Some(Highlight::Yellow));
    fmt(&mut p, 36, 39, &|c| {
        c.strike = Some(true);
        c.small_caps = Some(true);
    });
    let r = round(&doc_of(vec![p.clone()]));
    let q = r.body[0].as_para().unwrap();
    assert_eq!(q.plain_text(), p.plain_text());
    let at = |i: usize| q.props_at(i + 1).clone();
    assert_eq!(at(0).bold, Some(true));
    assert_eq!(at(5).italic, Some(true));
    assert_eq!(at(12).underline, Some(Underline::Double));
    assert_eq!(at(18).color, Some(TextColor::Rgb(wordcraft_doc::Rgb(0xC0, 0x10, 0x20))));
    assert_eq!(at(22).size, Some(18.5));
    assert_eq!(at(26).font.as_deref(), Some("Georgia"));
    assert_eq!(at(31).highlight, Some(Highlight::Yellow));
    assert_eq!(at(36).strike, Some(true));
    assert_eq!(at(36).small_caps, Some(true));
    assert_ne!(at(5).bold, Some(true));
}

#[test]
fn paragraph_formatting_round_trips() {
    let mut p = para("Centred and indented");
    p.props.align = Some(Align::Center);
    p.props.indent_left = Some(36.0);
    p.props.indent_right = Some(18.0);
    p.props.indent_first = Some(-9.0);
    p.props.space_before = Some(12.0);
    p.props.space_after = Some(6.0);
    p.props.line_spacing = Some(LineSpacing::Multiple(1.5));
    p.props.keep_next = Some(true);
    p.props.page_break_before = Some(true);
    p.props.tabs = Some(vec![TabStop { pos: 72.0, align: TabAlign::Right, leader: TabLeader::Dot }]);
    p.props.borders = Some(Borders {
        bottom: Some(Border { style: BorderStyle::Double, width: 1.5, color: Some(wordcraft_doc::Rgb(0, 0, 255)), space: 1.0 }),
        ..Default::default()
    });
    let mut exact = para("Exactly 14 pt");
    exact.props.line_spacing = Some(LineSpacing::Exactly(14.0));
    let r = round(&doc_of(vec![p.clone(), exact]));
    let q = &r.body[0].as_para().unwrap().props;
    assert_eq!(q.align, Some(Align::Center));
    assert_eq!(q.indent_left, Some(36.0));
    assert_eq!(q.indent_right, Some(18.0));
    assert_eq!(q.indent_first, Some(-9.0));
    assert_eq!(q.space_before, Some(12.0));
    assert_eq!(q.space_after, Some(6.0));
    assert_eq!(q.line_spacing, Some(LineSpacing::Multiple(1.5)));
    assert_eq!(q.keep_next, Some(true));
    assert_eq!(q.page_break_before, Some(true));
    assert_eq!(q.tabs, p.props.tabs);
    assert_eq!(q.borders.and_then(|b| b.bottom), p.props.borders.and_then(|b| b.bottom));
    assert_eq!(r.body[1].as_para().unwrap().props.line_spacing, Some(LineSpacing::Exactly(14.0)));
}

#[test]
fn styles_round_trip() {
    let mut d = Document::new();
    let h1 = d.styles.styles.iter().find(|s| s.name == "Heading 1").map(|s| s.id.clone()).expect("a Heading 1 style");
    d.styles.styles.push(wordcraft_doc::styles::Style {
        id: "MyQuote".into(),
        name: "My Quote".into(),
        kind: wordcraft_doc::styles::StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        chr: CharProps { italic: Some(true), ..Default::default() },
        para: ParaProps { indent_left: Some(24.0), ..Default::default() },
        ..Default::default()
    });
    d.body = vec![para_block(para("Title text").styled(&h1)), para_block(para("Quoted").styled("MyQuote")), para_block(para("Body"))];
    let r = round(&d);
    assert_eq!(texts(&r), vec!["Title text", "Quoted", "Body"]);
    let style_of = |i: usize| r.body[i].as_para().unwrap().props.style.clone().and_then(|id| r.styles.get(&id).map(|s| s.name.clone()));
    assert_eq!(style_of(0).as_deref(), Some("Heading 1"));
    assert_eq!(style_of(1).as_deref(), Some("My Quote"));
    let q = r.styles.styles.iter().find(|s| s.name == "My Quote").unwrap();
    assert_eq!(q.chr.italic, Some(true));
    assert_eq!(q.para.indent_left, Some(24.0));
    // The heading keeps its outline level, the stylesheet its default font.
    let h = r.styles.styles.iter().find(|s| s.name == "Heading 1").unwrap();
    assert_eq!(h.para.outline_level, Some(0));
}

#[test]
fn many_paragraphs_page_their_formatting() {
    let paras: Vec<Paragraph> = (0..2000)
        .map(|i| {
            let mut p = para(&format!("Paragraph {i} with some text"));
            if i % 3 == 0 {
                p.format(0, 9, &|c| c.bold = Some(true)).unwrap();
            }
            if i % 5 == 0 {
                p.props.indent_left = Some(i as f32 % 100.0);
            }
            p
        })
        .collect();
    let d = doc_of(paras);
    let r = round(&d);
    assert_eq!(texts(&r), texts(&d));
    for i in [0usize, 3, 999, 1998] {
        let q = r.body[i].as_para().unwrap();
        assert_eq!(q.props_at(1).bold, if i % 3 == 0 { Some(true) } else { q.props_at(1).bold.filter(|_| false) }, "paragraph {i}");
        if i % 5 == 0 {
            assert_eq!(q.props.indent_left, Some(i as f32 % 100.0));
        }
    }
}

fn cell(text: &str) -> Cell {
    Cell { props: CellProps { span: 1, ..Default::default() }, blocks: vec![para_block(para(text))], controls: Default::default() }
}

#[test]
fn tables_round_trip() {
    let mut a = cell("A1");
    a.props.shading = Some(wordcraft_doc::Rgb(0xEE, 0xDD, 0x00));
    a.props.vmerge = VMerge::Restart;
    let mut b = cell("");
    b.props.vmerge = VMerge::Continue;
    let mut multi = cell("B1 first");
    multi.blocks.push(para_block(para("B1 second")));
    let t = Table {
        props: TableProps {
            borders: Some(Borders { top: Some(Border::single(0.5)), bottom: Some(Border::single(0.5)), ..Default::default() }),
            ..Default::default()
        },
        grid: vec![100.0, 150.0, 80.0],
        rows: vec![
            Row { props: RowProps { header: true, ..Default::default() }, cells: vec![a, multi, cell("C1")], controls: Default::default() },
            Row {
                props: RowProps { height: Some(30.0), height_rule: HeightRule::Exact, ..Default::default() },
                cells: vec![b, cell("B2"), cell("C2")],
                controls: Default::default(),
            },
        ],
        controls: Default::default(),
    };
    let mut d = Document::new();
    d.body = vec![para_block(para("Before")), Arc::new(Block::Table(t)), para_block(para("After"))];
    let r = round(&d);
    assert_eq!(r.body.len(), 3, "{:?}", r.body.iter().map(|b| b.as_para().map(|p| p.plain_text())).collect::<Vec<_>>());
    let t = r.body[1].as_table().expect("a table");
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.grid, vec![100.0, 150.0, 80.0]);
    let cell_text = |r: usize, c: usize| t.rows[r].cells[c].blocks.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect::<Vec<_>>();
    assert_eq!(cell_text(0, 0), vec!["A1"]);
    assert_eq!(cell_text(0, 1), vec!["B1 first", "B1 second"]);
    assert_eq!(cell_text(1, 2), vec!["C2"]);
    assert_eq!(t.rows[0].cells[0].props.shading, Some(wordcraft_doc::Rgb(0xEE, 0xDD, 0x00)));
    assert_eq!(t.rows[0].cells[0].props.vmerge, VMerge::Restart);
    assert_eq!(t.rows[1].cells[0].props.vmerge, VMerge::Continue);
    assert!(t.rows[0].props.header);
    assert_eq!(t.rows[1].props.height, Some(30.0));
    assert_eq!(t.rows[1].props.height_rule, HeightRule::Exact);
    assert_eq!(texts(&r), vec!["Before", "After"]);
}

#[test]
fn sections_headers_and_footers_round_trip() {
    let mut d = Document::new();
    let h1 = d.add_part(PartKind::Header, vec![para_block(para("First header"))]);
    let f1 = d.add_part(PartKind::Footer, vec![para_block(para("First footer"))]);
    let h2 = d.add_part(PartKind::Header, vec![para_block(para("Landscape header"))]);
    let mut first = para("Portrait section");
    let mut s1 = SectionProps::default();
    s1.headers.default = Some(h1);
    s1.footers.default = Some(f1);
    s1.margin_left = 54.0;
    first.section = Some(Box::new(s1));
    let mut last =
        SectionProps { landscape: true, start: SectionStart::NextPage, columns: Columns { count: 2, ..Default::default() }, ..Default::default() };
    (last.page_w, last.page_h) = (792.0, 612.0);
    last.headers.default = Some(h2);
    d.body = vec![para_block(first), para_block(para("Landscape section"))];
    d.last_section = last;
    let r = round(&d);
    assert_eq!(texts(&r), vec!["Portrait section", "Landscape section"]);
    let s = r.body[0].as_para().unwrap().section.as_ref().expect("the first section ends here");
    assert_eq!(s.margin_left, 54.0);
    assert!(!s.landscape);
    let part_text =
        |id: Option<u32>| id.and_then(|i| r.parts.get(&i)).and_then(|p| p.blocks.first()).and_then(|b| b.as_para()).map(|p| p.plain_text());
    assert_eq!(part_text(s.headers.default).as_deref(), Some("First header"));
    assert_eq!(part_text(s.footers.default).as_deref(), Some("First footer"));
    assert!(r.last_section.landscape);
    assert_eq!((r.last_section.page_w, r.last_section.page_h), (792.0, 612.0));
    assert_eq!(r.last_section.columns.count, 2);
    assert_eq!(part_text(r.last_section.headers.default).as_deref(), Some("Landscape header"));
}

#[test]
fn notes_round_trip() {
    let mut d = Document::new();
    let f = d.add_part(PartKind::Footnote, vec![para_block(para("The footnote."))]);
    let e = d.add_part(PartKind::Endnote, vec![para_block(para("The endnote."))]);
    let mut p = para("Text.");
    p.insert_object(5, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: String::new() }, &CharProps::default()).unwrap();
    let end = p.text.len();
    p.insert_object(end, InlineObject::NoteRef { kind: NoteKind::Endnote, id: e, custom: String::new() }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p), para_block(para("After."))];
    let r = round(&d);
    assert_eq!(texts(&r)[1], "After.");
    let q = r.body[0].as_para().unwrap();
    let notes: Vec<(NoteKind, String)> = q
        .objects
        .iter()
        .filter_map(|o| match o {
            InlineObject::NoteRef { kind, id, .. } => {
                Some((*kind, r.parts.get(id).and_then(|p| p.blocks.first()).and_then(|b| b.as_para()).map(|p| p.plain_text()).unwrap_or_default()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(notes, vec![(NoteKind::Footnote, "The footnote.".to_string()), (NoteKind::Endnote, "The endnote.".to_string())]);
}

#[test]
fn fields_links_and_bookmarks_round_trip() {
    let mut p = para("Page , see Example and here.");
    p.insert_object(
        5,
        InlineObject::Field { instr: "PAGE".into(), result: "7".into(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    )
    .unwrap();
    let at = p.text.find("Example").unwrap();
    p.format(at, at + 7, &|c| c.link = Some("https://example.org/a?b=1".into())).unwrap();
    let here = p.text.find("here").unwrap();
    p.insert_object(here + 4, InlineObject::BookmarkEnd { name: "Spot".into() }, &CharProps::default()).unwrap();
    p.insert_object(here, InlineObject::BookmarkStart { name: "Spot".into() }, &CharProps::default()).unwrap();
    let r = round(&doc_of(vec![p]));
    let q = r.body[0].as_para().unwrap();
    assert!(q.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, result, .. } if instr == "PAGE" && result == "7")), "{:?}", q.objects);
    let link_at = q.text.find("Example").unwrap();
    assert_eq!(q.props_at(link_at + 1).link.as_deref(), Some("https://example.org/a?b=1"));
    assert!(q.objects.iter().any(|o| matches!(o, InlineObject::BookmarkStart { name } if name == "Spot")));
    assert!(q.objects.iter().any(|o| matches!(o, InlineObject::BookmarkEnd { name } if name == "Spot")));
}

/// A 2×1 PNG.
fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 1, image::Rgba([10, 200, 30, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

#[test]
fn inline_pictures_round_trip() {
    let mut d = Document::new();
    let bytes = png();
    d.media.insert("pic.png".into(), Arc::new(bytes.clone()));
    let mut p = para("Picture: .");
    p.insert_object(
        9,
        InlineObject::Image {
            media: "pic.png".into(),
            w: 144.0,
            h: 72.0,
            alt: "A green bar".into(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let r = round(&d);
    let q = r.body[0].as_para().unwrap();
    let Some(InlineObject::Image { media, w, h, alt, .. }) = q.objects.first() else { panic!("no picture: {:?}", q.objects) };
    assert!((w - 144.0).abs() < 0.1 && (h - 72.0).abs() < 0.1, "{w}×{h}");
    assert_eq!(alt, "A green bar");
    assert_eq!(r.media.get(media).map(|m| m.as_slice()), Some(bytes.as_slice()));
}

#[test]
fn document_properties_round_trip() {
    let mut d = Document::new();
    d.core.title = "Quarterly".into();
    d.core.creator = "A. Writer".into();
    let r = round(&d);
    assert_eq!(r.core.title, "Quarterly");
    assert_eq!(r.core.creator, "A. Writer");
}

#[test]
fn lists_round_trip() {
    use wordcraft_doc::numbering::{AbstractNum, Level, Num};
    use wordcraft_doc::section::NumFormat;
    let mut d = Document::new();
    let levels = vec![
        Level { start: 1, format: NumFormat::Decimal, text: "%1.".into(), indent: 36.0, hanging: 18.0, ..Level::default() },
        Level { start: 1, format: NumFormat::LowerLetter, text: "%1.%2)".into(), indent: 72.0, hanging: 18.0, ..Level::default() },
        Level { start: 1, format: NumFormat::Bullet, text: "\u{2022}".into(), indent: 108.0, hanging: 18.0, ..Level::default() },
    ];
    d.numbering.abstracts = vec![AbstractNum { id: 7, levels, ..Default::default() }];
    d.numbering.nums = vec![Num { id: 3, abstract_id: 7, start_overrides: vec![(0, 4)], ..Default::default() }];
    let item = |text: &str, level: u8| {
        let mut p = para(text);
        p.props.numbering = Some(NumRef { num: 3, level });
        para_block(p)
    };
    d.body = vec![item("One", 0), item("One a", 1), item("Dot", 2)];
    let r = round(&d);
    let num = |i: usize| r.body[i].as_para().unwrap().props.numbering.expect("numbered");
    assert_eq!((num(0).level, num(1).level, num(2).level), (0, 1, 2));
    let n = r.numbering.nums.iter().find(|n| n.id == num(0).num).expect("the list");
    assert_eq!(n.start_overrides, vec![(0, 4)]);
    let a = r.numbering.abstracts.iter().find(|a| a.id == n.abstract_id).expect("its levels");
    assert_eq!(a.levels[0].text, "%1.");
    assert_eq!(a.levels[1].text, "%1.%2)");
    assert_eq!(a.levels[1].format, NumFormat::LowerLetter);
    assert_eq!(a.levels[2].text, "\u{2022}");
    assert_eq!(a.levels[2].format, NumFormat::Bullet);
    assert_eq!((a.levels[1].indent, a.levels[1].hanging), (72.0, 18.0));
}

#[test]
fn tracked_changes_round_trip() {
    use wordcraft_doc::{Revision, RevisionKind};
    let mut d = Document::new();
    d.revisions = vec![
        Revision::new(RevisionKind::Insert, "Ada", "2025-03-04T05:06:00Z"),
        Revision::new(RevisionKind::Delete, "Grace", "2025-03-05T10:20:00Z"),
    ];
    let mut p = para("kept added removed");
    p.format(5, 10, &|c| c.ins = Some(0)).unwrap();
    p.format(11, 18, &|c| c.del = Some(1)).unwrap();
    d.body = vec![para_block(p)];
    let r = round(&d);
    let q = r.body[0].as_para().unwrap();
    assert_eq!(q.plain_text(), "kept added removed");
    let ins = q.props_at(6).ins.and_then(|i| r.revisions.get(i as usize)).expect("an insertion");
    assert_eq!((ins.kind, ins.author.as_str(), ins.date.as_str()), (RevisionKind::Insert, "Ada", "2025-03-04T05:06:00Z"));
    let del = q.props_at(12).del.and_then(|i| r.revisions.get(i as usize)).expect("a deletion");
    assert_eq!((del.kind, del.author.as_str(), del.date.as_str()), (RevisionKind::Delete, "Grace", "2025-03-05T10:20:00Z"));
    assert_eq!(q.props_at(1).ins, None);
    assert_eq!(q.mark.ins, None);
}

/// An inserted or deleted paragraph break (the paragraph mark's revision) round-trips too.
#[test]
fn tracked_paragraph_marks_round_trip() {
    use wordcraft_doc::{Revision, RevisionKind};
    let mut d = Document::new();
    d.revisions = vec![
        Revision::new(RevisionKind::Insert, "Ada", "2025-03-04T05:06:00Z"),
        Revision::new(RevisionKind::Delete, "Grace", "2025-03-05T10:20:00Z"),
    ];
    let (mut inserted, mut deleted) = (para("inserted break"), para("deleted break"));
    inserted.mark.ins = Some(0);
    deleted.mark.del = Some(1);
    d.body = vec![para_block(inserted), para_block(deleted), para_block(para("last"))];
    let r = round(&d);
    let mark = |i: usize| r.body[i].as_para().unwrap().mark.clone();
    let ins = mark(0).ins.and_then(|i| r.revisions.get(i as usize)).expect("an inserted mark");
    assert_eq!((ins.kind, ins.author.as_str()), (RevisionKind::Insert, "Ada"));
    let del = mark(1).del.and_then(|i| r.revisions.get(i as usize)).expect("a deleted mark");
    assert_eq!((del.kind, del.author.as_str()), (RevisionKind::Delete, "Grace"));
    assert_eq!((mark(2).ins, mark(2).del), (None, None));
    assert_eq!(r.body[0].as_para().unwrap().props_at(1).ins, None, "the text itself isn't marked");
}

/// A tracked insertion or deletion of a picture, a note reference or a field round-trips: the
/// reader takes each anchor's revision marks from its CHPX.
#[test]
fn tracked_object_anchors_round_trip() {
    use wordcraft_doc::{Revision, RevisionKind};
    let mut d = Document::new();
    d.revisions = vec![
        Revision::new(RevisionKind::Insert, "Ada", "2025-03-04T05:06:00Z"),
        Revision::new(RevisionKind::Delete, "Grace", "2025-03-05T10:20:00Z"),
    ];
    d.media.insert("pic.png".into(), Arc::new(png()));
    let f = d.add_part(PartKind::Footnote, vec![para_block(para("The footnote."))]);
    let inserted = CharProps { ins: Some(0), ..Default::default() };
    let deleted = CharProps { del: Some(1), ..Default::default() };
    let mut p = para("a b c d");
    let picture =
        InlineObject::Image { media: "pic.png".into(), w: 20.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
    let page = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() };
    let note = InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: String::new() };
    // Inserted back to front so the offsets stay put: picture after "a", note after "b",
    // field after "c" and an untracked field after "d".
    p.insert_object(7, page.clone(), &CharProps::default()).unwrap();
    p.insert_object(5, page, &inserted).unwrap();
    p.insert_object(3, note, &deleted).unwrap();
    p.insert_object(1, picture, &inserted).unwrap();
    d.body = vec![para_block(p)];
    let r = round(&d);
    let q = r.body[0].as_para().unwrap();
    let anchors: Vec<(&str, Option<RevisionKind>, Option<RevisionKind>)> = q
        .text
        .match_indices('\u{FFFC}')
        .zip(&q.objects)
        .map(|((at, _), o)| {
            let props = q.props_of_char(at);
            let kind = |rev: Option<u32>| rev.and_then(|i| r.revisions.get(i as usize)).map(|rev| rev.kind);
            let name = match o {
                InlineObject::Image { .. } => "picture",
                InlineObject::NoteRef { .. } => "note",
                InlineObject::Field { .. } => "field",
                _ => "other",
            };
            (name, kind(props.ins), kind(props.del))
        })
        .collect();
    assert_eq!(
        anchors,
        vec![
            ("picture", Some(RevisionKind::Insert), None),
            ("note", None, Some(RevisionKind::Delete)),
            ("field", Some(RevisionKind::Insert), None),
            ("field", None, None),
        ]
    );
}

#[test]
fn overlapping_bookmarks_keep_their_ends() {
    // Outer starts first and ends last; inner ends before it: the ends sort apart from the starts.
    let mut p = para("abcdef");
    p.insert_object(6, InlineObject::BookmarkEnd { name: "Outer".into() }, &CharProps::default()).unwrap();
    p.insert_object(4, InlineObject::BookmarkEnd { name: "Inner".into() }, &CharProps::default()).unwrap();
    p.insert_object(2, InlineObject::BookmarkStart { name: "Inner".into() }, &CharProps::default()).unwrap();
    p.insert_object(0, InlineObject::BookmarkStart { name: "Outer".into() }, &CharProps::default()).unwrap();
    let r = round(&doc_of(vec![p.clone()]));
    let q = r.body[0].as_para().unwrap();
    let marks = |p: &Paragraph| -> Vec<String> {
        p.objects
            .iter()
            .filter_map(|o| match o {
                InlineObject::BookmarkStart { name } => Some(format!("+{name}")),
                InlineObject::BookmarkEnd { name } => Some(format!("-{name}")),
                _ => None,
            })
            .collect()
    };
    assert_eq!(marks(q), marks(&p));
    assert_eq!(q.text, p.text);
}

#[test]
fn highlight_position_and_effects_round_trip() {
    let mut p = para("hl sup sub up dbl emb");
    p.format(0, 2, &|c| c.highlight = Some(Highlight::BrightGreen)).unwrap();
    p.format(3, 6, &|c| c.vert_align = Some(VertAlign::Superscript)).unwrap();
    p.format(7, 10, &|c| c.vert_align = Some(VertAlign::Subscript)).unwrap();
    p.format(11, 13, &|c| c.position = Some(3.0)).unwrap();
    p.format(14, 17, &|c| c.double_strike = Some(true)).unwrap();
    p.format(18, 21, &|c| {
        c.emboss = Some(true);
        c.spacing = Some(1.5);
        c.underline = Some(Underline::DoubleWave);
    })
    .unwrap();
    let r = round(&doc_of(vec![p]));
    let q = r.body[0].as_para().unwrap();
    assert_eq!(q.props_at(1).highlight, Some(Highlight::BrightGreen));
    assert_eq!(q.props_at(4).vert_align, Some(VertAlign::Superscript));
    assert_eq!(q.props_at(8).vert_align, Some(VertAlign::Subscript));
    assert_eq!(q.props_at(12).position, Some(3.0));
    assert_ne!(q.props_at(12).vert_align, Some(VertAlign::Superscript));
    assert_eq!(q.props_at(15).double_strike, Some(true));
    assert_eq!(q.props_at(19).emboss, Some(true));
    assert_eq!(q.props_at(19).spacing, Some(1.5));
    assert_eq!(q.props_at(19).underline, Some(Underline::DoubleWave));
}

#[test]
fn every_highlight_colour_round_trips() {
    for h in Highlight::ALL {
        let mut p = para("x");
        p.format(0, 1, &|c| c.highlight = Some(h)).unwrap();
        let r = round(&doc_of(vec![p]));
        assert_eq!(r.body[0].as_para().unwrap().props_at(0).highlight, Some(h), "{h:?}");
    }
}

#[test]
fn page_numbering_and_paragraph_shading_round_trip() {
    let mut d = Document::new();
    d.last_section.page_num_start = Some(5);
    d.last_section.page_num_format = wordcraft_doc::section::NumFormat::LowerRoman;
    d.last_section.columns = Columns { count: 3, space: 18.0, ..Default::default() };
    let mut p = para("Shaded");
    p.props.shading = Some(wordcraft_doc::Rgb(0xDD, 0xEE, 0xFF));
    p.props.widow_control = Some(false);
    p.props.contextual_spacing = Some(true);
    d.body = vec![para_block(p)];
    let r = round(&d);
    assert_eq!(r.last_section.page_num_start, Some(5));
    assert_eq!(r.last_section.page_num_format, wordcraft_doc::section::NumFormat::LowerRoman);
    assert_eq!((r.last_section.columns.count, r.last_section.columns.space), (3, 18.0));
    let q = &r.body[0].as_para().unwrap().props;
    assert_eq!(q.shading, Some(wordcraft_doc::Rgb(0xDD, 0xEE, 0xFF)));
    assert_eq!(q.widow_control, Some(false));
    assert_eq!(q.contextual_spacing, Some(true));
}

fn table_of(grid: Vec<f32>, rows: Vec<Vec<Cell>>) -> Table {
    Table {
        props: TableProps::default(),
        grid,
        rows: rows.into_iter().map(|cells| Row { props: RowProps::default(), cells, controls: Default::default() }).collect(),
        controls: Default::default(),
    }
}

#[test]
fn nested_tables_become_paragraphs_of_their_cell() {
    let inner = table_of(vec![50.0, 50.0], vec![vec![cell("i1"), cell("i2")]]);
    let mut outer_cell = cell("before");
    outer_cell.blocks.push(Arc::new(Block::Table(inner)));
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(table_of(vec![200.0], vec![vec![outer_cell]]))), para_block(para("end"))];
    let r = round(&d);
    let t = r.body[0].as_table().expect("the outer table");
    let texts: Vec<String> = t.rows[0].cells[0].blocks.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect();
    assert_eq!(texts, vec!["before", "i1", "i2"]);
}

#[test]
fn a_table_ending_the_document_gets_a_paragraph_after_it() {
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(table_of(vec![100.0], vec![vec![cell("only")]])))];
    let r = round(&d);
    assert!(r.body[0].as_table().is_some());
    assert_eq!(texts(&r), vec![String::new()]);
}

#[test]
fn rows_wider_than_the_format_keep_their_first_cells() {
    let cells: Vec<Cell> = (0..80).map(|i| cell(&format!("c{i}"))).collect();
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(table_of(vec![10.0; 80], vec![cells]))), para_block(para("after"))];
    // Never a panic; the first 63 cells (the format's most) keep their text.
    let r = round(&d);
    let all: Vec<String> = r
        .body
        .iter()
        .flat_map(|b| match &**b {
            Block::Para(p) => vec![p.plain_text()],
            Block::Table(t) => {
                t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).filter_map(|b| b.as_para().map(|p| p.plain_text())).collect()
            }
        })
        .collect();
    for i in [0, 30, 62, 79] {
        assert!(all.iter().any(|t| t == &format!("c{i}")), "c{i} missing: {all:?}");
    }
    assert_eq!(texts(&r).last().map(String::as_str), Some("after"));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
    /// Any text, with any run boundaries and formatting, writes without panicking and reads back
    /// as the same text.
    #[test]
    fn any_text_round_trips(lines in proptest::collection::vec("\\PC{0,40}", 1..8), bold in proptest::collection::vec(proptest::prelude::any::<bool>(), 8)) {
        let paras: Vec<Paragraph> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let clean: String = l.chars().filter(|c| !c.is_control() && *c != '\u{FFFC}').collect();
                let mut p = para(&clean);
                if bold.get(i).copied().unwrap_or(false) && !clean.is_empty() {
                    let end = clean.char_indices().nth(clean.chars().count() / 2).map_or(clean.len(), |(i, _)| i);
                    p.format(0, end, &|c| c.bold = Some(true)).unwrap();
                }
                p
            })
            .collect();
        let d = doc_of(paras);
        let r = round(&d);
        proptest::prop_assert_eq!(texts(&r), texts(&d));
    }
}
