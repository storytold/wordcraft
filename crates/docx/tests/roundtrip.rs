//! Round-trip tests: build documents programmatically → write → read → compare.

use std::sync::Arc;

use wordcraft_doc::numbering::ListKind;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CharProps, HeightRule, Highlight, LineSpacing, NumRef, ParaProps, Rgb, RowProps, TabAlign, TabLeader,
    TabStop, TableLook, TextColor, TextDirection, Underline, VAlign, VMerge, VertAlign,
};
use wordcraft_doc::section::{Columns, LineNumberRestart, LineNumbering, NumFormat, SectionProps, SectionStart};
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::table::{Cell, Table};
use wordcraft_doc::{Block, Blocks, Comment, Document, InlineObject, Paragraph, PartKind, Revision, RevisionKind, Watermark, para_block};

fn rt(doc: &Document) -> Document {
    let bytes = wordcraft_docx::write(doc).expect("write");
    wordcraft_docx::read(&bytes).expect("read")
}

fn paras(doc: &Document) -> Vec<&Paragraph> {
    doc.body.iter().filter_map(|b| b.as_para()).collect()
}

fn part_text(doc: &Document, id: Option<u32>) -> String {
    id.and_then(|i| doc.parts.get(&i))
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

fn doc_with(blocks: Vec<Paragraph>) -> Document {
    let mut d = Document::new();
    d.body = blocks.into_iter().map(para_block).collect();
    d
}

/// A paragraph built from (text, props) runs.
fn para_runs(runs: &[(&str, CharProps)]) -> Paragraph {
    let mut p = Paragraph::new();
    for (t, c) in runs {
        let off = p.len();
        p.insert_text(off, t, c).unwrap();
    }
    p
}

fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 80, y as u8 * 120, 200, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn every_char_prop_round_trips() {
    let all = CharProps {
        style: Some("Strong".into()),
        font: Some("Georgia".into()),
        size: Some(10.5),
        bold: Some(true),
        italic: Some(false),
        underline: Some(Underline::DotDash),
        underline_color: Some(Rgb(1, 2, 3)),
        strike: Some(true),
        double_strike: Some(false),
        color: Some(TextColor::Rgb(Rgb(0xAA, 0x10, 0x20))),
        highlight: Some(Highlight::Turquoise),
        shading: Some(Rgb(0xEE, 0xEE, 0x00)),
        border: Some(Border { style: BorderStyle::Double, width: 1.5, color: Some(Rgb(0xC0, 0, 0)), space: 2.0 }),
        vert_align: Some(VertAlign::Superscript),
        caps: Some(true),
        small_caps: Some(false),
        hidden: Some(true),
        spacing: Some(1.5),
        scale: Some(150.0),
        position: Some(3.0),
        kern: Some(8.0),
        outline: Some(true),
        shadow: Some(true),
        emboss: Some(false),
        engrave: Some(true),
        lang: Some("fr-FR".into()),
        no_proof: Some(true),
        rtl: Some(false),
        cs: Some(true),
        font_cs: Some("B Nazanin".into()),
        size_cs: Some(13.0),
        bold_cs: Some(false),
        italic_cs: Some(true),
        lang_bidi: Some("fa-IR".into()),
        link: None,
        ins: None,
        del: None,
        fmt_change: None,
    };
    let auto =
        CharProps { color: Some(TextColor::Auto), highlight: Some(Highlight::None), vert_align: Some(VertAlign::Subscript), ..Default::default() };
    let d = doc_with(vec![para_runs(&[("plain ", CharProps::default()), ("everything", all.clone()), (" auto", auto.clone())])]);
    let r = rt(&d);
    let p = paras(&r)[0];
    assert_eq!(p.text, "plain everything auto");
    assert_eq!(p.runs.len(), 3);
    assert_eq!(p.runs[1].props, all);
    assert_eq!(p.runs[2].props, auto);
    assert_eq!(p.runs[0].props, CharProps::default());
}

#[test]
fn every_para_prop_round_trips() {
    let b = Border { style: BorderStyle::Double, width: 0.75, color: Some(Rgb(9, 8, 7)), space: 4.0 };
    let pp = ParaProps {
        style: Some("Heading1".into()),
        align: Some(Align::Justify),
        indent_left: Some(36.0),
        indent_right: Some(18.0),
        indent_first: Some(-18.0),
        space_before: Some(6.0),
        space_after: Some(12.0),
        line_spacing: Some(LineSpacing::Multiple(1.5)),
        contextual_spacing: Some(true),
        keep_next: Some(true),
        keep_lines: Some(false),
        page_break_before: Some(true),
        widow_control: Some(false),
        outline_level: Some(2),
        numbering: Some(NumRef { num: 0, level: 0 }),
        tabs: Some(vec![
            TabStop { pos: 72.0, align: TabAlign::Center, leader: TabLeader::Dot },
            TabStop { pos: 144.0, align: TabAlign::Right, leader: TabLeader::None },
            TabStop { pos: 200.0, align: TabAlign::Decimal, leader: TabLeader::Underscore },
            TabStop { pos: 216.0, align: TabAlign::Clear, leader: TabLeader::None },
            TabStop { pos: 250.0, align: TabAlign::Bar, leader: TabLeader::MiddleDot },
        ]),
        shading: Some(Rgb(0xDD, 0xEE, 0xFF)),
        borders: Some(Borders { top: Some(b), left: Some(Border::single(0.5)), bottom: Some(b), right: None, between: Some(b), inside_v: None }),
        suppress_hyphens: Some(true),
        suppress_line_numbers: Some(true),
        bidi: Some(false),
        drop_cap: None,
        kinsoku: Some(false),
        word_wrap: Some(false),
        overflow_punct: Some(false),
        top_line_punct: Some(true),
        auto_space_de: Some(false),
        auto_space_dn: Some(true),
        num_change: None,
        fmt_change: None,
    };
    let variants = [
        ParaProps { line_spacing: Some(LineSpacing::AtLeast(14.0)), indent_first: Some(24.0), align: Some(Align::Center), ..Default::default() },
        ParaProps { line_spacing: Some(LineSpacing::Exactly(20.0)), align: Some(Align::Right), ..Default::default() },
        ParaProps { align: Some(Align::Distribute), ..Default::default() },
        ParaProps { kinsoku: Some(true), word_wrap: Some(true), auto_space_dn: Some(false), ..Default::default() },
        ParaProps::default(),
    ];
    let mut ps = vec![Paragraph::with_text("main", CharProps::default())];
    ps[0].props = pp.clone();
    ps[0].mark = CharProps { bold: Some(true), size: Some(14.0), ..Default::default() };
    for v in &variants {
        let mut p = Paragraph::with_text("v", CharProps::default());
        p.props = v.clone();
        ps.push(p);
    }
    let r = rt(&doc_with(ps));
    let got = paras(&r);
    assert_eq!(got[0].props, pp);
    assert_eq!(got[0].mark, CharProps { bold: Some(true), size: Some(14.0), ..Default::default() });
    for (i, v) in variants.iter().enumerate() {
        assert_eq!(&got[i + 1].props, v, "variant {i}");
    }
}

#[test]
fn styles_and_lists_round_trip() {
    let mut d = Document::new();
    d.styles.upsert(Style {
        id: "MyStyle".into(),
        name: "My Style".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        next: Some("Normal".into()),
        linked: Some("MyStyleChar".into()),
        para: ParaProps { space_after: Some(3.0), align: Some(Align::Center), ..Default::default() },
        chr: CharProps { italic: Some(true), color: Some(TextColor::Rgb(Rgb(1, 2, 3))), ..Default::default() },
        priority: Some(5),
        quick: true,
        hidden: false,
        builtin: false,
        table: None,
    });
    d.styles.upsert(Style {
        id: "MyStyleChar".into(),
        name: "My Style Char".into(),
        kind: StyleKind::Character,
        linked: Some("MyStyle".into()),
        chr: CharProps { italic: Some(true), ..Default::default() },
        hidden: true,
        ..Default::default()
    });
    let bullets = d.numbering.add_list(ListKind::Bullet);
    let nums = d.numbering.add_list(ListKind::Legal);
    let outline = d.numbering.add_list(ListKind::Outline);
    if let Some(n) = d.numbering.nums.iter_mut().find(|n| n.id == outline) {
        n.start_overrides.push((0, 4));
    }
    let mut ps = Vec::new();
    for (i, num) in [bullets, nums, nums, outline].iter().enumerate() {
        let mut p = Paragraph::with_text(&format!("item {i}"), CharProps::default());
        p.props.numbering = Some(NumRef { num: *num, level: (i % 2) as u8 });
        p.props.style = Some("MyStyle".into());
        ps.push(p);
    }
    d.body = ps.into_iter().map(para_block).collect();
    let r = rt(&d);
    assert_eq!(r.numbering, d.numbering);
    assert_eq!(r.styles.default_chr, d.styles.default_chr);
    assert_eq!(r.styles.default_para, d.styles.default_para);
    for s in &d.styles.styles {
        assert_eq!(r.styles.get(&s.id), Some(s), "style {}", s.id);
    }
    assert_eq!(r.styles.styles.len(), d.styles.styles.len());
    assert_eq!(paras(&r)[1].props.numbering, Some(NumRef { num: nums, level: 1 }));
}

#[test]
fn tables_with_merges_round_trip() {
    let mut t = Table::new(3, 3, 468.0);
    t.props.width = Some(468.0);
    t.props.align = Some(Align::Center);
    t.props.indent = Some(5.0);
    t.props.borders = Some(Borders::all(Border::single(0.5)));
    t.props.cell_margins = Some([1.0, 5.4, 1.0, 5.4]);
    t.props.fixed = true;
    t.props.look = TableLook { header_row: true, total_row: true, banded_rows: false, first_column: false, last_column: true, banded_columns: true };
    t.props.shading = Some(Rgb(1, 1, 1));
    t.props.caption = Some("Sales".into());
    t.rows[0].props = RowProps { height: Some(20.0), height_rule: HeightRule::Exact, header: true, cant_split: true, fmt_change: None };
    t.rows[1].props = RowProps { height: Some(15.0), height_rule: HeightRule::AtLeast, ..Default::default() };
    t.merge(0, 0, 0, 1); // horizontal
    t.merge(1, 2, 2, 2); // vertical
    {
        let c = &mut t.rows[1].cells[0];
        c.props.shading = Some(Rgb(0xFF, 0, 0));
        c.props.valign = VAlign::Bottom;
        c.props.text_direction = TextDirection::Up;
        c.props.no_wrap = true;
        c.props.margins = Some([2.0, 3.0, 4.0, 5.0]);
        c.props.width = None;
        c.props.width_pct = Some(40.0);
        c.props.borders = Some(Borders::box_(Border { style: BorderStyle::Dashed, width: 1.0, color: Some(Rgb(0, 0, 255)), space: 0.0 }));
        c.blocks = vec![para_block(Paragraph::with_text("red", CharProps::default()))];
    }
    // Nested table in a cell (followed by the required paragraph).
    let inner = Table::new(1, 2, 100.0);
    t.rows[2].cells[0].blocks = vec![Arc::new(Block::Table(inner)), para_block(Paragraph::new())];
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("before", CharProps::default())), Arc::new(Block::Table(t.clone())), para_block(Paragraph::new())];
    let r = rt(&d);
    let got = r.body[1].as_table().expect("table");
    assert_eq!(got.props, t.props);
    assert_eq!(got.grid, t.grid);
    assert_eq!(got.rows.len(), 3);
    for (gr, er) in got.rows.iter().zip(&t.rows) {
        assert_eq!(gr.props, er.props);
        assert_eq!(gr.cells.len(), er.cells.len());
        for (gc, ec) in gr.cells.iter().zip(&er.cells) {
            assert_eq!(gc.props, ec.props);
            assert_eq!(gc.blocks, ec.blocks);
        }
    }
    assert_eq!(got.rows[0].cells[0].span(), 2);
    assert_eq!(got.rows[2].cells[2].props.vmerge, VMerge::Continue);
}

#[test]
fn sections_headers_footers_round_trip() {
    let mut d = Document::new();
    let page = |instr: &str| {
        let mut p = Paragraph::with_text("Page ", CharProps::default());
        p.insert_object(5, InlineObject::Field { instr: instr.into(), result: "1".into(), locked: false }, &CharProps::default()).unwrap();
        p
    };
    let h1 = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Header one", CharProps::default()))]);
    let hf = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("First page header", CharProps::default()))]);
    let f1 = d.add_part(PartKind::Footer, vec![para_block(page("PAGE"))]);
    let f2 = d.add_part(PartKind::Footer, vec![para_block(page("NUMPAGES"))]);
    let mut s1 = SectionProps {
        title_page: true,
        page_num_start: Some(3),
        page_num_format: NumFormat::UpperRoman,
        valign: VAlign::Center,
        page_borders: Some(Borders::box_(Border::single(1.0))),
        line_numbers: Some(LineNumbering { count_by: 5, start: 1, distance: 18.0, restart: LineNumberRestart::Section }),
        gutter: 18.0,
        ..Default::default()
    };
    s1.headers.default = Some(h1);
    s1.headers.first = Some(hf);
    s1.footers.default = Some(f1);
    let mut s2 = SectionProps { start: SectionStart::OddPage, rtl: true, ..Default::default() };
    s2.set_landscape(true);
    s2.columns = Columns { count: 2, space: 24.0, separator: true, widths: Vec::new() };
    s2.footers.default = Some(f2);
    s2.headers.default = Some(h1);
    let mut p1 = Paragraph::with_text("Section one", CharProps::default());
    p1.section = Some(Box::new(s1.clone()));
    let mut s3 = SectionProps { start: SectionStart::Continuous, ..Default::default() };
    s3.columns = Columns { count: 2, space: 10.0, separator: false, widths: vec![(200.0, 20.0), (248.0, 0.0)] };
    let mut p2 = Paragraph::with_text("Section two", CharProps::default());
    p2.section = Some(Box::new(s2.clone()));
    d.body = vec![para_block(p1), para_block(p2), para_block(Paragraph::with_text("Last", CharProps::default()))];
    d.last_section = s3.clone();
    let r = rt(&d);
    let secs = r.sections();
    assert_eq!(secs.len(), 3);
    let strip = |s: &SectionProps| SectionProps { headers: Default::default(), footers: Default::default(), ..s.clone() };
    assert_eq!(strip(secs[0].1), strip(&s1));
    assert_eq!(strip(secs[1].1), strip(&s2));
    assert_eq!(strip(secs[2].1), strip(&s3));
    assert!(secs[1].1.landscape);
    assert_eq!(part_text(&r, secs[0].1.headers.default), "Header one");
    assert_eq!(part_text(&r, secs[0].1.headers.first), "First page header");
    assert_eq!(secs[0].1.headers.default, secs[1].1.headers.default, "shared header part stays shared");
    let fp = r.parts.get(&secs[0].1.footers.default.unwrap()).unwrap().blocks[0].as_para().unwrap();
    assert_eq!(fp.objects, vec![InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false }]);
    assert_eq!(part_text(&r, secs[1].1.footers.default), "Page 1");
    assert_eq!(r.parts.get(&secs[0].1.headers.default.unwrap()).unwrap().kind, PartKind::Header);
}

#[test]
fn images_round_trip() {
    let mut d = Document::new();
    let png = tiny_png();
    let key = d.add_media(png.clone(), "png");
    let inline = InlineObject::Image {
        media: key.clone(),
        w: 72.0,
        h: 48.0,
        alt: "A tiny picture".into(),
        // Room for a shadow: kept through save and load.
        float: Float { effect: [12.0, 12.0, 27.0, 27.0], ..Default::default() },
        crop: [0.1, 0.0, 0.25, 0.05],
        ole: None,
    };
    let floating = InlineObject::Image {
        media: key.clone(),
        w: 100.0,
        h: 50.0,
        alt: String::new(),
        float: Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Margin, x: 36.0, y: 12.5, dist: 9.0, ..Default::default() },
        crop: [0.0; 4],
        ole: None,
    };
    let aligned = InlineObject::Image {
        media: key.clone(),
        w: 20.0,
        h: 20.0,
        alt: String::new(),
        float: Float {
            wrap: Wrap::Square,
            h_rel: Anchor::RightMargin,
            h_align: Some(FloatAlign::Center),
            v_rel: Anchor::TopMargin,
            v_align: Some(FloatAlign::End),
            dist: 9.0,
            dist_top: 2.5,
            dist_bottom: 4.0,
            ..Default::default()
        },
        crop: [0.0; 4],
        ole: None,
    };
    let mut floats = vec![floating.clone(), aligned];
    for wrap in [Wrap::Tight, Wrap::Through, Wrap::TopAndBottom, Wrap::BehindText, Wrap::InFrontOfText] {
        floats.push(InlineObject::Image {
            media: key.clone(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 0.0, y: 0.0, dist: 0.0, ..Default::default() },
            crop: [0.0; 4],
            ole: None,
        });
    }
    let mut p = Paragraph::with_text("pic: ", CharProps::default());
    p.insert_object(5, inline.clone(), &CharProps::default()).unwrap();
    for f in &floats {
        let end = p.len();
        p.insert_object(end, f.clone(), &CharProps::default()).unwrap();
    }
    d.body = vec![para_block(p)];
    let r = rt(&d);
    let got = paras(&r)[0];
    let mut expect = vec![inline];
    expect.extend(floats);
    assert_eq!(got.objects, expect);
    assert_eq!(r.media.get(&key).map(|b| b.as_slice()), Some(png.as_slice()));
    assert_eq!(r.media.len(), 1);
}

#[test]
fn hyperlinks_and_bookmarks_round_trip() {
    let ext = CharProps { link: Some("https://example.com/a?b=1&c=2".into()), underline: Some(Underline::Single), ..Default::default() };
    let int = CharProps { link: Some("#target".into()), ..Default::default() };
    let mut p = para_runs(&[("see ", CharProps::default()), ("example", ext.clone()), (" and ", CharProps::default()), ("here", int.clone())]);
    p.insert_object(0, InlineObject::BookmarkStart { name: "target".into() }, &CharProps::default()).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::BookmarkEnd { name: "target".into() }, &CharProps::default()).unwrap();
    let d = doc_with(vec![p.clone()]);
    let r = rt(&d);
    let got = paras(&r)[0];
    assert_eq!(got.text, p.text);
    assert_eq!(got.objects, p.objects);
    assert_eq!(got.props_of_char(got.text.find("example").unwrap()), &ext);
    assert_eq!(got.props_of_char(got.text.find("here").unwrap()), &int);
    assert_eq!(r.bookmarks().len(), 1);
}

#[test]
fn comments_round_trip() {
    let mut d = Document::new();
    let c0 = d.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Please check this.", CharProps::default()))]);
    let c1 = d.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Done.", CharProps::default()))]);
    d.comments.insert(
        0,
        Comment { author: "Ann".into(), initials: "A".into(), date: "2026-01-02T03:04:05Z".into(), parent: None, resolved: true, part: c0 },
    );
    d.comments.insert(
        1,
        Comment { author: "Ben".into(), initials: "B".into(), date: "2026-01-03T00:00:00Z".into(), parent: Some(0), resolved: false, part: c1 },
    );
    let mut p = Paragraph::with_text("commented text", CharProps::default());
    p.insert_object(0, InlineObject::CommentStart { id: 0 }, &CharProps::default()).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::CommentEnd { id: 0 }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p.clone())];
    let r = rt(&d);
    assert_eq!(paras(&r)[0].objects, p.objects);
    assert_eq!(r.comments.len(), 2);
    for (id, c) in &d.comments {
        let g = r.comments.get(id).expect("comment");
        assert_eq!((&g.author, &g.initials, &g.date, g.parent, g.resolved), (&c.author, &c.initials, &c.date, c.parent, c.resolved));
        assert_eq!(part_text(&r, Some(g.part)), part_text(&d, Some(c.part)));
        assert_eq!(r.parts.get(&g.part).unwrap().kind, PartKind::Comment);
    }
}

/// A table style's cell text formatting, whole-table shading, cell margins and its header row and
/// row band conditional formats (#146) survive a save.
#[test]
fn table_style_formatting_round_trips() {
    use wordcraft_doc::styles::TableStyleParts;
    let mut d = doc_with(vec![Paragraph::with_text("x", CharProps::default())]);
    let parts = TableStyleParts {
        borders: Some(Borders { top: Some(Border::single(1.0)), ..Default::default() }),
        fill: Some(Rgb(0xDD, 0xEB, 0xF7)),
        cell_margins: Some([1.0, 14.4, 0.0, 14.4]),
        header_chr: CharProps { italic: Some(true), ..Default::default() },
        header_fill: Some(Rgb(0xFF, 0xFF, 0)),
        header_borders: Some(Borders::all(Border::single(1.5))),
        band_fill: Some(Rgb(0xF2, 0xF2, 0xF2)),
        band_chr: CharProps { bold: Some(true), color: Some(TextColor::Rgb(Rgb(0, 0x70, 0xC0))), ..Default::default() },
        band_borders: Some(Borders::box_(Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 })),
        band_size: Some(3),
        ..Default::default()
    };
    d.styles.upsert(Style {
        id: "Whole".into(),
        name: "Whole".into(),
        kind: StyleKind::Table,
        based_on: Some("TableGrid".into()),
        para: ParaProps { align: Some(Align::Center), space_after: Some(0.0), ..Default::default() },
        chr: CharProps { bold: Some(true), color: Some(TextColor::Rgb(Rgb(0xC0, 0, 0))), ..Default::default() },
        table: Some(parts.clone()),
        ..Default::default()
    });
    let r = rt(&d);
    let got = r.styles.get("Whole").unwrap();
    assert_eq!(got.based_on.as_deref(), Some("TableGrid"));
    assert_eq!(got.para.align, Some(Align::Center));
    assert_eq!(got.para.space_after, Some(0.0));
    assert_eq!((got.chr.bold, got.chr.color), (Some(true), Some(TextColor::Rgb(Rgb(0xC0, 0, 0)))));
    assert_eq!(got.table.as_ref(), Some(&parts));
}

#[test]
fn notes_round_trip() {
    let mut d = Document::new();
    let f = d.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("A footnote.", CharProps::default()))]);
    let f2 = d.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Custom.", CharProps::default()))]);
    let e = d.add_part(
        PartKind::Endnote,
        vec![
            para_block(Paragraph::with_text("An endnote.", CharProps::default())),
            para_block(Paragraph::with_text("Second para.", CharProps::default())),
        ],
    );
    let mut p = Paragraph::with_text("Text", CharProps::default());
    let sup = CharProps { vert_align: Some(VertAlign::Superscript), ..Default::default() };
    p.insert_object(4, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: String::new() }, &sup).unwrap();
    p.insert_object(1, InlineObject::NoteRef { kind: NoteKind::Endnote, id: e, custom: String::new() }, &sup).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f2, custom: "*".into() }, &sup).unwrap();
    d.settings.footnote_format = NumFormat::LowerLetter;
    d.settings.endnote_format = NumFormat::UpperRoman;
    d.body = vec![para_block(p.clone())];
    let r = rt(&d);
    let got = paras(&r)[0];
    assert_eq!(got.text, p.text);
    assert_eq!(got.runs, p.runs);
    let notes: Vec<(NoteKind, String, String)> = got
        .objects
        .iter()
        .map(|o| match o {
            InlineObject::NoteRef { kind, id, custom } => (*kind, part_text(&r, Some(*id)), custom.clone()),
            o => panic!("unexpected {o:?}"),
        })
        .collect();
    assert_eq!(
        notes,
        vec![
            (NoteKind::Endnote, "An endnote.\nSecond para.".into(), String::new()),
            (NoteKind::Footnote, "A footnote.".into(), String::new()),
            (NoteKind::Footnote, "Custom.".into(), "*".into()),
        ]
    );
    assert_eq!(r.settings.footnote_format, NumFormat::LowerLetter);
    assert_eq!(r.settings.endnote_format, NumFormat::UpperRoman);
}

/// #385: footnote and endnote options round-trip for the document (settings) and per section
/// (`w:sectPr`), in schema order: `w:footnotePr` and `w:endnotePr` before `w:type`.
#[test]
fn note_options_round_trip_per_section() {
    use wordcraft_doc::section::{NotePos, NoteProps, NoteRestart};
    let mut d = Document::new();
    d.settings.footnote_format = NumFormat::UpperLetter;
    d.settings.footnote_pr =
        NoteProps { pos: Some(NotePos::BeneathText), num_start: Some(4), num_restart: Some(NoteRestart::EachPage), ..Default::default() };
    d.settings.endnote_pr = NoteProps { pos: Some(NotePos::SectEnd), ..Default::default() };
    let mut first = Paragraph::with_text("Section one", CharProps::default());
    let own = NoteProps { num_fmt: Some(NumFormat::LowerRoman), num_start: Some(7), num_restart: Some(NoteRestart::EachSect), ..Default::default() };
    first.section = Some(Box::new(SectionProps {
        footnote_pr: own,
        endnote_pr: NoteProps { num_fmt: Some(NumFormat::Decimal), ..Default::default() },
        ..Default::default()
    }));
    d.body = vec![para_block(first), para_block(Paragraph::with_text("Section two", CharProps::default()))];
    let bytes = wordcraft_docx::write(&d).unwrap();
    let r = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(r.settings.footnote_format, NumFormat::UpperLetter);
    assert_eq!((r.settings.footnote_pr, r.settings.endnote_pr), (d.settings.footnote_pr, d.settings.endnote_pr));
    let sections = r.sections();
    assert_eq!(sections[0].1.footnote_pr, own);
    assert_eq!(sections[0].1.endnote_pr.num_fmt, Some(NumFormat::Decimal));
    assert!(sections[1].1.footnote_pr.is_empty() && sections[1].1.endnote_pr.is_empty());
    assert_eq!(r.note_options(sections[0].1, false).num_start, 7);
    assert_eq!(r.note_options(sections[1].1, false).num_start, 4);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    let at = |s: &str| xml.find(s).unwrap_or_else(|| panic!("{s} missing"));
    assert!(
        at("<w:footnotePr><w:numFmt w:val=\"lowerRoman\"/><w:numStart w:val=\"7\"/><w:numRestart w:val=\"eachSect\"/></w:footnotePr>")
            < at("<w:type")
    );
}

/// `references.footnote` starts the note's own text with a reference to the note, so the editor shows
/// its number. In the file that is the `w:footnoteRef` mark; a `w:footnoteReference` there would make
/// the note cite itself, which LibreOffice refuses to open.
#[test]
fn note_never_references_itself() {
    let mut d = Document::new();
    let f = d.add_part(PartKind::Footnote, Vec::new());
    let mut note = Paragraph::with_text(" The note.", CharProps::default());
    note.insert_object(0, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: String::new() }, &CharProps::default()).unwrap();
    d.parts.get_mut(&f).unwrap().blocks = vec![para_block(note)];
    let mut p = Paragraph::with_text("Text", CharProps::default());
    p.insert_object(4, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: String::new() }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];

    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/footnotes.xml").unwrap(), &mut xml).unwrap();
    assert!(!xml.contains("<w:footnoteReference"), "a footnote cites itself: {xml}");
    assert_eq!(xml.matches("<w:footnoteRef/>").count(), 1, "{xml}");

    let r = wordcraft_docx::read(&bytes).expect("read");
    let got = paras(&r)[0];
    let Some(InlineObject::NoteRef { id, .. }) = got.objects.first() else { panic!("{:?}", got.objects) };
    assert_eq!(part_text(&r, Some(*id)).trim(), "The note.");
}

/// A note read from a Word file starts with its own number (a reference to itself, from the
/// `w:footnoteRef` mark). Saving writes that mark back exactly once, with its character style, and
/// never as a `w:footnoteReference`; reading the saved file gives the same note again.
#[test]
fn word_note_mark_round_trips_once() {
    let mut d = Document::new();
    let f = d.add_part(PartKind::Footnote, Vec::new());
    let e = d.add_part(PartKind::Endnote, Vec::new());
    for (id, kind, style) in [(f, NoteKind::Footnote, "FootnoteReference"), (e, NoteKind::Endnote, "EndnoteReference")] {
        let mut note =
            Paragraph::with_text(" The note.", CharProps::default()).styled(if kind == NoteKind::Footnote { "FootnoteText" } else { "EndnoteText" });
        note.insert_object(
            0,
            InlineObject::NoteRef { kind, id, custom: String::new() },
            &CharProps { style: Some(style.into()), ..Default::default() },
        )
        .unwrap();
        // A second paragraph: the mark is not repeated there.
        d.parts.get_mut(&id).unwrap().blocks = vec![para_block(note), para_block(Paragraph::with_text("More.", CharProps::default()))];
    }
    let mut p = Paragraph::with_text("Text", CharProps::default());
    p.insert_object(4, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: String::new() }, &CharProps::default()).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::NoteRef { kind: NoteKind::Endnote, id: e, custom: String::new() }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];

    let mut doc = d;
    for pass in 0..2 {
        let bytes = wordcraft_docx::write(&doc).expect("write");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        for (file, mark, reference, style) in [
            ("word/footnotes.xml", "<w:footnoteRef/>", "<w:footnoteReference", "FootnoteReference"),
            ("word/endnotes.xml", "<w:endnoteRef/>", "<w:endnoteReference", "EndnoteReference"),
        ] {
            let mut xml = String::new();
            std::io::Read::read_to_string(&mut zip.by_name(file).unwrap(), &mut xml).unwrap();
            assert_eq!(xml.matches(mark).count(), 1, "pass {pass}: {xml}");
            assert!(!xml.contains(reference), "pass {pass}: a note cites itself: {xml}");
            assert!(xml.contains(&format!(r#"<w:rStyle w:val="{style}"/></w:rPr>{mark}"#)), "pass {pass}: {xml}");
        }
        let r = wordcraft_docx::read(&bytes).expect("read");
        let body = paras(&r)[0];
        assert_eq!(body.objects.len(), 2, "pass {pass}");
        for o in &body.objects {
            let InlineObject::NoteRef { kind, id, .. } = o else { panic!("{o:?}") };
            let blocks = &r.parts.get(id).unwrap().blocks;
            assert_eq!(blocks.len(), 2, "pass {pass}");
            let first = blocks[0].as_para().unwrap();
            assert_eq!(first.objects, vec![InlineObject::NoteRef { kind: *kind, id: *id, custom: String::new() }], "pass {pass}");
            assert_eq!(first.plain_text(), " The note.");
            assert!(blocks[1].as_para().unwrap().objects.is_empty(), "pass {pass}");
        }
        doc = r;
    }
}

/// The TOC is a `Contents` paragraph ending in an empty `TOC` field, followed by TOC-styled entries.
/// In the file the field must contain the entries, inside Word's Table of Contents content control,
/// or Word shows them as plain text with no Update Table.
#[test]
fn toc_field_wraps_its_entries() {
    let mut head = Paragraph::with_text("Contents", CharProps::default()).styled("TOCHeading");
    let end = head.len();
    head.insert_object(
        end,
        InlineObject::Field { instr: "TOC \\o \"1-2\" \\h \\z \\u".into(), result: String::new(), locked: false },
        &CharProps::default(),
    )
    .unwrap();
    let entries = [("Intro\t1", "TOC1"), ("Detail\t2", "TOC2")];
    let mut blocks = vec![head];
    blocks.extend(entries.iter().map(|(t, st)| Paragraph::with_text(t, CharProps::default()).styled(st)));
    blocks.push(Paragraph::with_text("Body", CharProps::default()));
    let d = doc_with(blocks);

    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    let sdt = xml.find("<w:sdt>").expect("TOC content control");
    assert!(xml.contains(r#"<w:docPartGallery w:val="Table of Contents"/>"#), "{xml}");
    let (begin, fend) = (xml.find(r#"fldCharType="begin""#).unwrap(), xml.find(r#"fldCharType="end""#).unwrap());
    let last_entry = xml.find("Detail").unwrap();
    assert!(sdt < xml.find("Contents").unwrap() && xml.find("</w:sdt>").unwrap() < xml.find(">Body<").unwrap(), "{xml}");
    assert!(begin < xml.find("Intro").unwrap() && fend > last_entry && fend < xml.find("</w:sdt>").unwrap(), "field must span the entries: {xml}");
    // Word starts the field in the first entry; started in the heading, LibreOffice splits the heading.
    let heading_end = xml[..xml.find("Intro").unwrap()].rfind("</w:p>").unwrap();
    assert!(begin > heading_end, "field must start in the first entry, not the heading: {xml}");

    // WordCraft reads its own TOC back unchanged.
    let r = wordcraft_docx::read(&bytes).expect("read");
    let ps = paras(&r);
    assert_eq!(ps.iter().map(|p| p.plain_text()).collect::<Vec<_>>(), ["Contents", "Intro\t1", "Detail\t2", "Body"]);
    assert!(matches!(ps[0].objects.first(), Some(InlineObject::Field { instr, .. }) if instr.starts_with("TOC")), "{:?}", ps[0].objects);
    assert!(ps[1].objects.is_empty(), "the field goes back on the heading: {:?}", ps[1].objects);
    assert_eq!(ps[1].props.style.as_deref(), Some("TOC1"));
}

/// Table Layout › Text Direction (#226): each direction is written as Word's `w:textDirection`
/// value and read back.
#[test]
fn cell_text_direction_round_trips() {
    let mut t = Table::new(1, 3, 300.0);
    let dirs = [TextDirection::Horizontal, TextDirection::Down, TextDirection::Up];
    for (c, dir) in t.rows[0].cells.iter_mut().zip(dirs) {
        c.props.text_direction = dir;
    }
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(t)), para_block(Paragraph::new())];
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"<w:textDirection w:val="tbRl"/>"#), "{xml}");
    assert!(xml.contains(r#"<w:textDirection w:val="btLr"/>"#), "{xml}");
    assert_eq!(xml.matches("w:textDirection").count(), 2, "horizontal cells write nothing: {xml}");
    let r = wordcraft_docx::read(&bytes).expect("read");
    let got: Vec<TextDirection> = r.body[0].as_table().expect("table").rows[0].cells.iter().map(|c| c.props.text_direction).collect();
    assert_eq!(got, dirs);
}

/// The TOC field's begin and end belong to the entries' own paragraphs, even when the heading or
/// an entry holds a text box: the box's paragraphs are written inside them and must not take either.
#[test]
fn toc_field_skips_nested_stories() {
    let mut d = Document::new();
    let text_box = |d: &mut Document, text: &str| {
        let story = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(text, CharProps::default()))]);
        InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w: 72.0,
            h: 36.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            float: Float::default(),
            story: Some(story),
            freeform: None,
            effects: Default::default(),
        }
    };
    let mut head = Paragraph::with_text("Contents", CharProps::default()).styled("TOCHeading");
    let end = head.len();
    head.insert_object(
        end,
        InlineObject::Field { instr: "TOC \\o \"1-1\" \\h \\z \\u".into(), result: String::new(), locked: false },
        &CharProps::default(),
    )
    .unwrap();
    let end = head.len();
    head.insert_object(end, text_box(&mut d, "Heading box"), &CharProps::default()).unwrap();
    let mut last = Paragraph::with_text("Detail\t2", CharProps::default()).styled("TOC1");
    let end = last.len();
    last.insert_object(end, text_box(&mut d, "Boxed"), &CharProps::default()).unwrap();
    d.body = vec![para_block(head), para_block(last), para_block(Paragraph::with_text("Body", CharProps::default()))];

    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    let boxes: Vec<(usize, usize)> = xml.match_indices("<w:txbxContent>").map(|(a, _)| (a, a + xml[a..].find("</w:txbxContent>").unwrap())).collect();
    assert_eq!(boxes.len(), 2, "{xml}");
    let in_box = |i: usize| boxes.iter().any(|(a, b)| (*a..*b).contains(&i));
    for kind in ["begin", "separate", "end"] {
        let at: Vec<usize> = xml.match_indices(&format!(r#"fldCharType="{kind}""#)).map(|(i, _)| i).collect();
        assert_eq!(at.len(), 1, "{kind}: {xml}");
        assert!(!in_box(at[0]), "field {kind} written inside a text box: {xml}");
    }
    let (begin, end) = (xml.find(r#"fldCharType="begin""#).unwrap(), xml.find(r#"fldCharType="end""#).unwrap());
    assert!(begin > boxes[0].1 && end > boxes[1].1 && end < xml.find("</w:sdt>").unwrap(), "{xml}");
}

#[test]
fn tracked_changes_round_trip() {
    let mut d = Document::new();
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "Alice".into(), date: "2026-05-01T10:00:00Z".into() });
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "Bob".into(), date: "2026-05-02T10:00:00Z".into() });
    d.settings.track_changes = true;
    let ins = CharProps { ins: Some(0), ..Default::default() };
    let del = CharProps { del: Some(1), bold: Some(true), ..Default::default() };
    let both = CharProps { ins: Some(0), del: Some(1), ..Default::default() };
    let p = para_runs(&[("kept ", CharProps::default()), ("added\t", ins.clone()), ("removed", del.clone()), ("gone", both.clone())]);
    let mut p = p;
    let end = p.len();
    p.insert_object(end, InlineObject::Field { instr: "DATE".into(), result: "today".into(), locked: false }, &del).unwrap();
    d.body = vec![para_block(p.clone())];
    let r = rt(&d);
    assert_eq!(r.revisions, d.revisions);
    let got = paras(&r)[0];
    assert_eq!(got.text, p.text);
    assert_eq!(got.runs, p.runs);
    assert_eq!(got.objects, p.objects);
    assert!(r.settings.track_changes);
}

#[test]
fn tracked_paragraph_marks_round_trip() {
    // Issue #229: an inserted / deleted paragraph mark is `w:ins` / `w:del` in the mark's
    // `w:rPr` (ECMA-376 §17.13.5.16, §17.13.5.15), alone or with mark formatting.
    let mut d = Document::new();
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "Alice".into(), date: "2026-05-01T10:00:00Z".into() });
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "Bob".into(), date: "2026-05-02T10:00:00Z".into() });
    let mut split = Paragraph::with_text("Owned ALPHA ", CharProps::default());
    split.mark.ins = Some(0);
    let mut joined = Paragraph::with_text("Owned BETA", CharProps::default());
    joined.mark = CharProps { bold: Some(true), del: Some(1), ..Default::default() };
    let last = Paragraph::with_text("plain", CharProps::default());
    d.body = vec![para_block(split), para_block(joined), para_block(last)];
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"<w:rPr><w:ins w:id="#), "{xml}");
    assert!(xml.contains(r#"<w:rPr><w:del w:id="#), "{xml}");
    let r = wordcraft_docx::read(&bytes).expect("read");
    let got = paras(&r);
    assert_eq!(got.len(), 3);
    let rev = |i: Option<u32>| i.and_then(|i| r.revisions.get(i as usize)).map(|v| (v.kind, v.author.as_str(), v.date.as_str()));
    assert_eq!(rev(got[0].mark.ins), Some((RevisionKind::Insert, "Alice", "2026-05-01T10:00:00Z")));
    assert_eq!(got[0].mark.del, None);
    assert_eq!(rev(got[1].mark.del), Some((RevisionKind::Delete, "Bob", "2026-05-02T10:00:00Z")));
    assert_eq!(got[1].mark.bold, Some(true));
    assert_eq!(got[1].mark.ins, None);
    assert_eq!((got[2].mark.ins, got[2].mark.del), (None, None));
    assert_eq!(got.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Owned ALPHA ", "Owned BETA", "plain"]);
}

#[test]
fn fields_and_special_chars_round_trip() {
    let mut p = Paragraph::with_text("a\tb\nc\u{000C}d\u{000E}e\u{2011}f\u{00AD}g  spaced  ", CharProps::default());
    let fields = [
        InlineObject::Field { instr: "PAGE".into(), result: "4".into(), locked: false },
        InlineObject::Field { instr: "DATE \\@ \"M/d/yyyy\"".into(), result: "1/2/2026".into(), locked: true },
        InlineObject::Field { instr: "NUMPAGES".into(), result: String::new(), locked: false },
    ];
    for f in &fields {
        let end = p.len();
        p.insert_object(end, f.clone(), &CharProps { bold: Some(true), ..Default::default() }).unwrap();
    }
    let r = rt(&doc_with(vec![p.clone()]));
    let got = paras(&r)[0];
    assert_eq!(got.text, p.text);
    assert_eq!(got.objects, p.objects);
    assert_eq!(got.runs, p.runs);
}

#[test]
fn shapes_textboxes_equations_dropcaps_round_trip() {
    let mut d = Document::new();
    let story =
        d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Inside the box", CharProps { bold: Some(true), ..Default::default() }))]);
    let tb = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 144.0,
        h: 72.0,
        fill: Some(Rgb(255, 255, 200)),
        stroke: Some(Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: Float { wrap: Wrap::Square, h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, x: 10.0, y: 20.0, dist: 0.0, ..Default::default() },
        story: Some(story),
        freeform: None,
        effects: Default::default(),
    };
    let star = InlineObject::Shape {
        kind: ShapeKind::Star,
        w: 20.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
    };
    let eq = InlineObject::Equation { linear: "x=(-b±√(b^2-4ac))/2a".into(), display: false, math: Default::default() };
    let mut p = Paragraph::with_text("shapes ", CharProps::default());
    for o in [tb, star.clone(), eq.clone()] {
        let end = p.len();
        p.insert_object(end, o, &CharProps::default()).unwrap();
    }
    let mut dc = Paragraph::with_text("Once upon a time", CharProps::default());
    dc.props.drop_cap = Some(3);
    dc.props.space_after = Some(4.0);
    d.body = vec![para_block(p.clone()), para_block(dc.clone())];
    let r = rt(&d);
    let got = paras(&r);
    assert_eq!(got[0].objects.len(), 3);
    match &got[0].objects[0] {
        InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story, .. } => {
            assert_eq!(
                (*kind, *w, *h, *fill, *stroke, *stroke_width),
                (ShapeKind::TextBox, 144.0, 72.0, Some(Rgb(255, 255, 200)), Some(Rgb(0, 0, 0)), 1.0)
            );
            assert_eq!(float.wrap, Wrap::Square);
            assert_eq!(part_text(&r, *story), "Inside the box");
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(got[0].objects[1], star);
    match &got[0].objects[2] {
        InlineObject::Equation { linear, display, math } => {
            assert_eq!(linear, "x=(-b±√(b^2-4ac))/2a");
            assert!(!display);
            assert_eq!(math.nodes, wordcraft_doc::math::parse_linear(linear), "structure is written as OMML and read back");
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(got.len(), 2, "drop cap paragraph merges back");
    assert_eq!(got[1].text, dc.text);
    assert_eq!(got[1].props, dc.props);
}

/// A group of a picture, a shape and a text box (`wpg:wgp`) comes back as it was saved: its
/// size, placement and members' space, and each member's offset, size and content.
#[test]
fn groups_round_trip() {
    use wordcraft_doc::para::GroupChild;
    let mut d = Document::new();
    let key = d.add_media(tiny_png(), "png");
    let story = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("In the group", CharProps::default()))]);
    let pic = InlineObject::Image { media: key, w: 60.0, h: 40.0, alt: "A picture".into(), float: Float::default(), crop: [0.0; 4], ole: None };
    let oval = InlineObject::Shape {
        kind: ShapeKind::Ellipse,
        w: 50.0,
        h: 30.0,
        fill: Some(Rgb(200, 0, 0)),
        stroke: None,
        stroke_width: 0.0,
        float: Float::default(),
        story: None,
        freeform: None,
        // A member's shape effects (#275) come back too.
        effects: wordcraft_doc::effects::ShapeEffects {
            shadow: Some(wordcraft_doc::effects::Shadow {
                color: Rgb(0x20, 0x30, 0x40),
                transparency: 60.0,
                blur: 4.0,
                distance: 3.0,
                angle: 135.0,
                rot_with_shape: false,
            }),
            glow: None,
            soft_edge: Some(2.5),
        },
    };
    let tb = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 100.0,
        h: 40.0,
        fill: Some(Rgb(255, 255, 255)),
        stroke: Some(Rgb(0, 0, 0)),
        stroke_width: 0.75,
        float: Float::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
    };
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 20.0, y: 10.0, dist: 9.0, ..Default::default() };
    let group = InlineObject::Group {
        w: 240.0,
        h: 120.0,
        float,
        ch_w: 200.0,
        ch_h: 100.0,
        children: vec![
            GroupChild { x: 0.0, y: 0.0, obj: pic },
            GroupChild { x: 150.0, y: 10.0, obj: oval.clone() },
            GroupChild { x: 50.0, y: 60.0, obj: tb },
        ],
    };
    let mut p = Paragraph::with_text("grouped ", CharProps::default());
    let end = p.len();
    p.insert_object(end, group, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let r = rt(&d);
    let got = paras(&r);
    let InlineObject::Group { w, h, float: f, ch_w, ch_h, children } = &got[0].objects[0] else { panic!("{:?}", got[0].objects) };
    assert_eq!((*w, *h, *ch_w, *ch_h), (240.0, 120.0, 200.0, 100.0));
    assert_eq!(*f, float);
    assert_eq!(children.iter().map(|c| (c.x, c.y)).collect::<Vec<_>>(), [(0.0, 0.0), (150.0, 10.0), (50.0, 60.0)]);
    match &children[0].obj {
        InlineObject::Image { media, w, h, alt, .. } => {
            assert_eq!((*w, *h, alt.as_str()), (60.0, 40.0, "A picture"));
            assert_eq!(r.media.get(media).map(|m| m.as_slice()), Some(tiny_png().as_slice()));
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(children[1].obj, oval);
    match &children[2].obj {
        InlineObject::Shape { kind, w, h, story, .. } => {
            assert_eq!((*kind, *w, *h), (ShapeKind::TextBox, 100.0, 40.0));
            assert_eq!(part_text(&r, *story), "In the group");
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn compatibility_mode_round_trips() {
    // New documents are Word 2013+ documents; an older file keeps its mode, so saving it doesn't
    // change how Word lays it out.
    assert_eq!(rt(&Document::new()).settings.compat_mode, wordcraft_doc::COMPAT_MODE_CURRENT);
    for mode in [11, 12, 14, 15] {
        let mut d = Document::new();
        d.settings.compat_mode = mode;
        assert_eq!(rt(&d).settings.compat_mode, mode);
    }
}

#[test]
fn settings_core_theme_round_trip() {
    let mut d = Document::new();
    d.settings.track_changes = true;
    d.settings.default_tab = 18.0;
    d.settings.even_odd_headers = true;
    d.settings.mirror_margins = true;
    d.settings.auto_hyphenation = true;
    d.settings.page_color = Some(Rgb(250, 240, 230));
    d.settings.major_font = "Cambria".into();
    d.settings.minor_font = "Calibri".into();
    d.settings.theme_colors[4] = Rgb(1, 2, 3);
    d.settings.theme_name = "Mine".into();
    d.settings.protection = Some("readOnly".into());
    d.settings.grid_h = 5.5;
    d.settings.grid_v = 18.0;
    d.settings.math = Some(wordcraft_doc::math::MathProps {
        brk_bin: wordcraft_doc::math::BrkBin::Repeat,
        brk_bin_sub: wordcraft_doc::math::BrkBinSub::PlusMinus,
        wrap_indent: 36.0,
        wrap_right: false,
    });
    d.core.title = "Title & <stuff>".into();
    d.core.subject = "Subj".into();
    d.core.creator = "Me".into();
    d.core.keywords = "a, b".into();
    d.core.description = "Desc".into();
    d.core.category = "Cat".into();
    d.core.last_modified_by = "You".into();
    d.core.created = "2026-01-01T00:00:00Z".into();
    d.core.modified = "2026-02-01T00:00:00Z".into();
    d.core.revision = 7;
    let r = rt(&d);
    assert_eq!(r.settings, d.settings);
    assert_eq!(r.core, d.core);
    // Watermarks aren't stored in DOCX by this crate (yet).
    let mut w = d.clone();
    w.settings.watermark = Some(Watermark::default());
    assert_eq!(rt(&w).settings.watermark, None);
}

/// A document that uses everything, for second-generation stability checks.
fn kitchen_sink() -> Document {
    let mut d = Document::new();
    let png = tiny_png();
    let key = d.add_media(png, "png");
    let num = d.numbering.add_list(ListKind::Numbered);
    let h = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Head", CharProps::default()))]);
    let f = d.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Note", CharProps::default()))]);
    let c = d.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Comment", CharProps::default()))]);
    d.comments.insert(3, Comment { author: "X".into(), part: c, ..Default::default() });
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "A".into(), date: String::new() });
    d.last_section.headers.default = Some(h);
    let mut blocks: Blocks = Vec::new();
    let mut p = para_runs(&[
        ("Hello ", CharProps::default()),
        ("bold", CharProps { bold: Some(true), ..Default::default() }),
        (" link", CharProps { link: Some("https://x.org".into()), ..Default::default() }),
    ]);
    p.insert_object(0, InlineObject::CommentStart { id: 3 }, &CharProps::default()).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::CommentEnd { id: 3 }, &CharProps::default()).unwrap();
    let at = p.text.find("bold").unwrap();
    p.insert_object(at, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: String::new() }, &CharProps::default()).unwrap();
    blocks.push(para_block(p));
    let mut li = Paragraph::with_text("listed", CharProps { ins: Some(0), ..Default::default() });
    li.props.numbering = Some(NumRef { num, level: 0 });
    blocks.push(para_block(li));
    let mut img = Paragraph::new();
    img.insert_object(
        0,
        InlineObject::Image { media: key, w: 30.0, h: 20.0, alt: "pic".into(), float: Float::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    blocks.push(para_block(img));
    let mut t = Table::new(2, 2, 300.0);
    t.merge(0, 1, 1, 1);
    t.rows[0].cells[0] = Cell::with_text("cell");
    blocks.push(Arc::new(Block::Table(t)));
    blocks.push(para_block(Paragraph::new()));
    d.body = blocks;
    d
}

#[test]
fn second_generation_is_stable() {
    let d1 = rt(&kitchen_sink());
    let d2 = rt(&d1);
    assert_eq!(d2.body, d1.body);
    assert_eq!(d2.parts, d1.parts);
    assert_eq!(d2.styles, d1.styles);
    assert_eq!(d2.numbering, d1.numbering);
    assert_eq!(d2.comments, d1.comments);
    assert_eq!(d2.revisions, d1.revisions);
    assert_eq!(d2.settings, d1.settings);
    assert_eq!(d2.media, d1.media);
    assert_eq!(d2, d1);
}

#[test]
fn builtin_stylesheet_survives() {
    let d = Document::new();
    let r = rt(&d);
    assert_eq!(r.styles, d.styles);
    assert_eq!(r.body, d.body);
    assert_eq!(r.last_section, d.last_section);
}

#[test]
fn hostile_model_values_still_write() {
    let mut d = Document::new();
    let mut p = Paragraph::with_text(
        "x\u{1}\u{FFFE}y<&>\"'",
        CharProps { size: Some(f32::NAN), spacing: Some(f32::INFINITY), font: Some("A\"B<".into()), ..Default::default() },
    );
    p.props.indent_left = Some(f32::NEG_INFINITY);
    p.props.line_spacing = Some(LineSpacing::Multiple(f32::NAN));
    p.insert_object(
        0,
        InlineObject::Opaque { format: "docx".into(), xml: "<w:r><w:t>raw</w:t></w:r>".into(), text: "raw".into() },
        &CharProps::default(),
    )
    .unwrap();
    p.insert_object(0, InlineObject::Opaque { format: "docx".into(), xml: "<broken".into(), text: "fallback".into() }, &CharProps::default())
        .unwrap();
    p.insert_object(
        0,
        InlineObject::Image {
            media: "missing.png".into(),
            w: -5.0,
            h: f32::NAN,
            alt: String::new(),
            float: Float::default(),
            crop: [f32::NAN; 4],
            ole: None,
        },
        &CharProps::default(),
    )
    .unwrap();
    p.insert_object(0, InlineObject::CommentEnd { id: 99 }, &CharProps::default()).unwrap();
    p.insert_object(0, InlineObject::NoteRef { kind: NoteKind::Endnote, id: 12345, custom: String::new() }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    d.media.insert("weird name/../x".into(), Arc::new(b"GIF89a....".to_vec()));
    d.media.insert("".into(), Arc::new(vec![0, 1, 2]));
    let bytes = wordcraft_docx::write(&d).unwrap();
    let r = wordcraft_docx::read(&bytes).unwrap();
    let t = paras(&r)[0].plain_text();
    assert!(t.contains("y<&>\"'"), "{t}");
    assert!(t.contains("raw") && t.contains("fallback"), "{t}");
    // Media nothing references aren't written.
    assert!(r.media.is_empty());
}

#[test]
fn ensure_empty_and_table_end_document_is_valid() {
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(Table::new(1, 1, 100.0)))];
    let r = rt(&d);
    assert!(matches!(*r.body[0], Block::Table(_)));
    assert!(matches!(**r.body.last().unwrap(), Block::Para(_)));
    let mut e = Document::new();
    e.body.clear();
    let r = rt(&e);
    assert_eq!(r.body.len(), 1);
}

#[test]
fn self_showing_text_box_saves_bounded() {
    // Crafted: a box whose 30 shapes all show that same box. Unbounded, saving would nest it.
    let mut d = Document::from_text("Body");
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Box", CharProps::default()))]);
    let shape = || InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 40.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float::default(),
        story: Some(id),
        freeform: None,
        effects: Default::default(),
    };
    for _ in 0..30 {
        d.insert_object(
            &wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Part(id), path: wordcraft_doc::Path::top(0), off: 0 },
            shape(),
            &CharProps::default(),
        )
        .unwrap();
    }
    d.insert_object(&wordcraft_doc::Pos::body(0, 0), shape(), &CharProps::default()).unwrap();
    let bytes = wordcraft_docx::write(&d).unwrap();
    assert!(bytes.len() < 200_000, "{} bytes", bytes.len());
    // It still opens, with the box's text.
    let back = wordcraft_docx::read(&bytes).unwrap();
    assert!(back.parts.values().any(|p| p.kind == PartKind::TextBox));
}

#[test]
fn char_border_written_between_u_and_shd() {
    let c =
        CharProps { border: Some(Border::single(0.5)), underline: Some(Underline::Single), shading: Some(Rgb(0xFF, 0xFF, 0)), ..Default::default() };
    let bytes = wordcraft_docx::write(&doc_with(vec![para_runs(&[("x", c)])])).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    let bdr = r#"<w:bdr w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#;
    assert_eq!(xml.matches(bdr).count(), 1, "{xml}");
    let at = xml.find(bdr).unwrap();
    assert!(xml.find("<w:u ").unwrap() < at && at < xml.find("<w:shd ").unwrap(), "{xml}");
}

#[test]
fn floating_tables_round_trip() {
    use wordcraft_doc::para::{Anchor, FloatAlign};
    use wordcraft_doc::props::TableFloat;
    let mut d = Document::new();
    let mut t = Table::new(1, 2, 200.0);
    let f = TableFloat {
        h_rel: Anchor::Margin,
        v_rel: Anchor::Paragraph,
        x: 0.0,
        y: -1.65,
        h_align: None,
        v_align: None,
        dist: [9.0, 0.0, 12.0, 3.0],
        overlap: false,
    };
    t.props.float = Some(f);
    let mut page = Table::new(1, 1, 100.0);
    let centred =
        TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Margin, h_align: Some(FloatAlign::Center), y: 36.0, overlap: true, ..Default::default() };
    page.props.float = Some(centred);
    d.body = vec![Arc::new(Block::Table(t)), Arc::new(Block::Table(page)), para_block(Paragraph::with_text("after", CharProps::default()))];
    let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let floats: Vec<_> = back.body.iter().filter_map(|b| if let Block::Table(t) = &**b { t.props.float } else { None }).collect();
    assert_eq!(floats, [f, centred]);
}

#[test]
fn list_level_overrides_round_trip() {
    use wordcraft_doc::numbering::{Counters, Level};
    use wordcraft_doc::section::NumFormat;
    let mut d = Document::new();
    let num = d.numbering.add_list(ListKind::Numbered);
    let restart = d.numbering.restart(num).unwrap();
    let own = Level { format: NumFormat::DecimalZero, text: "%1.%2".into(), indent: 26.5, hanging: 26.5, ..Level::default() };
    if let Some(n) = d.numbering.nums.iter_mut().find(|n| n.id == restart) {
        n.level_overrides = vec![(1, own.clone())];
    }
    let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let n = back.numbering.num(restart).unwrap();
    assert_eq!(n.level_overrides.len(), 1);
    let (lvl, got) = &n.level_overrides[0];
    assert_eq!((*lvl, got.format, got.text.as_str(), got.indent, got.hanging), (1, NumFormat::DecimalZero, "%1.%2", 26.5, 26.5));
    // The start overrides that `restart` wrote survive beside it.
    assert_eq!(n.start_overrides, d.numbering.num(restart).unwrap().start_overrides);
    let mut c = Counters::default();
    assert_eq!(c.next_label(&back.numbering, restart, 0).unwrap().0, "1.");
    assert_eq!(c.next_label(&back.numbering, restart, 1).unwrap().0, "1.01");
}

/// Ink strokes and freeform shapes are written as DrawingML custom geometry (`a:custGeom`,
/// `a:moveTo`/`a:lnTo`) in floating drawings and read back with their points, pen and opacity.
#[test]
fn ink_and_freeforms_round_trip_as_custom_geometry() {
    use wordcraft_doc::freeform::{FreePath, Freeform, InkTool};
    let shape = |fill, stroke_width, wrap, f: Freeform| InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: f.w,
        h: f.h,
        fill,
        stroke: Some(Rgb(0xC0, 0, 0)),
        stroke_width,
        float: Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 30.0, y: 12.0, ..Default::default() },
        story: None,
        freeform: Some(Arc::new(f)),
        effects: Default::default(),
    };
    let pen = shape(None, 2.0, Wrap::InFrontOfText, Freeform::ink(InkTool::Pen, 60.0, 20.0, vec![[1.0, 1.0], [30.0, 19.0], [59.0, 4.0]]));
    let marker = shape(None, 12.0, Wrap::BehindText, Freeform::ink(InkTool::Highlighter, 80.0, 12.0, vec![[6.0, 6.0], [74.0, 6.0]]));
    let triangle = Freeform {
        w: 40.0,
        h: 40.0,
        paths: vec![FreePath { pts: vec![[0.0, 40.0], [20.0, 0.0], [40.0, 40.0]], closed: true }],
        ..Default::default()
    };
    let tri = shape(Some(Rgb(0, 0x80, 0)), 1.0, Wrap::Square, triangle);
    let mut p = Paragraph::with_text("Inked", CharProps::default());
    for o in [pen.clone(), marker, tri.clone()] {
        p.insert_object(0, o, &CharProps::default()).unwrap();
    }
    let d = doc_with(vec![p]);
    let bytes = wordcraft_docx::write(&d).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert_eq!(xml.matches("<a:custGeom>").count(), 3, "{xml}");
    assert!(xml.contains(r#"<a:path w="762000" h="254000" fill="none"><a:moveTo><a:pt x="12700" y="12700"/></a:moveTo><a:lnTo>"#), "{xml}");
    assert!(xml.contains(r#"<a:alpha val="50000"/>"#) && xml.contains(r#"cap="rnd""#), "{xml}");
    let r = rt(&d);
    let objs = &paras(&r)[0].objects;
    assert_eq!(objs.len(), 3);
    // Written in reverse (each inserted at the start): the triangle, the highlighter, the pen.
    assert_eq!(objs[0], tri, "a filled closed freeform comes back as it was");
    let InlineObject::Shape { freeform: Some(m), float, stroke_width, .. } = &objs[1] else { panic!("{:?}", objs[1]) };
    assert_eq!((m.ink, m.alpha, float.wrap, *stroke_width), (Some(InkTool::Highlighter), 0.5, Wrap::BehindText, 12.0));
    assert_eq!(objs[2], pen, "the pen stroke comes back point for point");
}

/// Shape effects (#275): `a:effectLst` with an outer shadow, a glow and soft edges round-trips,
/// and the effect extent leaves room for them.
#[test]
fn shape_effects_round_trip() {
    use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
    let effects = ShapeEffects {
        shadow: Some(Shadow { color: Rgb(0x20, 0x30, 0x40), transparency: 60.0, blur: 4.0, distance: 3.0, angle: 135.0, rot_with_shape: false }),
        glow: Some(Glow { color: Rgb(0xC0, 0x50, 0x10), size: 8.0, transparency: 40.0 }),
        soft_edge: Some(2.5),
    };
    let shape = InlineObject::Shape {
        kind: ShapeKind::RoundedRectangle,
        w: 100.0,
        h: 50.0,
        fill: Some(Rgb(0x15, 0x60, 0x82)),
        stroke: None,
        stroke_width: 0.0,
        float: Float::default(),
        story: None,
        freeform: None,
        effects,
    };
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(1, shape, &CharProps::default()).unwrap();
    let bytes = wordcraft_docx::write(&doc_with(vec![p])).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(
        xml.contains(
            r#"<a:outerShdw blurRad="50800" dist="38100" dir="8100000" algn="tr" rotWithShape="0"><a:srgbClr val="203040"><a:alpha val="40000"/>"#
        ),
        "{xml}"
    );
    assert!(xml.contains(r#"<a:glow rad="101600"><a:srgbClr val="C05010"><a:alpha val="60000"/>"#), "{xml}");
    assert!(xml.contains(r#"<a:softEdge rad="31750"/>"#), "{xml}");
    let r = wordcraft_docx::read(&bytes).expect("read");
    match paras(&r)[0].objects.first() {
        Some(InlineObject::Shape { effects: got, float, .. }) => {
            assert_eq!(*got, effects);
            assert_eq!(float.effect_extent(), effects.extent());
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn equation_manual_breaks_round_trip() {
    use wordcraft_doc::math::{MNode, MRun, Math};
    let nodes = vec![
        MNode::Run(MRun::new("a=b")),
        MNode::Run(MRun { brk: Some(1), ..MRun::new("+c") }),
        MNode::Run(MRun { brk: Some(0), ..MRun::new("+d") }),
    ];
    let mut d = Document::new();
    let math = Math { nodes: nodes.clone(), ..Default::default() };
    let mut p = Paragraph::with_text("", CharProps::default());
    p.insert_object(0, InlineObject::Equation { linear: "a=b+c+d".into(), display: true, math }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let r = rt(&d);
    let got = paras(&r).iter().flat_map(|p| p.objects.iter()).find_map(|o| match o {
        InlineObject::Equation { math, .. } => Some(math.nodes.clone()),
        _ => None,
    });
    assert_eq!(got, Some(nodes));
    // Word's own settings round-trip; a document without them gets none.
    assert_eq!(r.settings.math, None);
}

/// #332: rotation (`a:xfrm/@rot`, 60000ths of a degree) and flips of pictures, shapes, text boxes
/// and groups survive save and load; the effect extent written covers the rotated bounds, and
/// reading takes that overhang back out of the effects room.
#[test]
fn rotation_and_flips_round_trip() {
    let mut d = Document::new();
    let key = d.add_media(tiny_png(), "png");
    let pic = InlineObject::Image {
        media: key,
        w: 100.0,
        h: 50.0,
        alt: String::new(),
        float: Float { rot: 30.0, effect: [6.0; 4], ..Default::default() },
        crop: [0.0; 4],
        ole: None,
    };
    let shape = |kind, float| InlineObject::Shape {
        kind,
        w: 80.0,
        h: 40.0,
        fill: Some(Rgb(1, 2, 3)),
        stroke: None,
        stroke_width: 0.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
    };
    let square = Float { wrap: Wrap::Square, ..Default::default() };
    let tri = shape(ShapeKind::Triangle, Float { rot: 90.0, flip_h: true, ..square });
    let flipped = shape(ShapeKind::Rectangle, Float { rot: 315.5, flip_v: true, ..square });
    let group = InlineObject::Group {
        w: 120.0,
        h: 60.0,
        float: Float { rot: 45.0, ..square },
        ch_w: 120.0,
        ch_h: 60.0,
        children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::Ellipse, Float { rot: 10.0, ..Default::default() }) }],
    };
    let mut p = Paragraph::with_text("turned ", CharProps::default());
    for o in [pic, tri, flipped, group] {
        let end = p.len();
        p.insert_object(end, o, &CharProps::default()).unwrap();
    }
    d.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"<a:xfrm rot="1800000">"#), "{xml}");
    assert!(xml.contains(r#"<a:xfrm rot="5400000" flipH="1">"#), "{xml}");
    assert!(xml.contains(r#"<a:xfrm rot="18930000" flipV="1">"#), "{xml}");
    // The 100 × 50 picture turned 30° covers 111.6 × 93.3: 5.8 and 21.7 pt more on each side,
    // plus its 6 pt of effects room.
    assert!(xml.contains(r#"<wp:effectExtent l="149876" t="351163" r="149876" b="351163"/>"#), "{xml}");
    let r = wordcraft_docx::read(&bytes).expect("read");
    let got = &paras(&r)[0].objects;
    let spins: Vec<(f32, bool, bool)> = got.iter().map(|o| o.frame().unwrap().2).map(|f| (f.rot, f.flip_h, f.flip_v)).collect();
    assert_eq!(spins, [(30.0, false, false), (90.0, true, false), (315.5, false, true), (45.0, false, false)]);
    let effect = got[0].frame().unwrap().2.effect;
    assert!(effect.iter().all(|e| (e - 6.0).abs() < 0.01), "{effect:?}");
    assert!(got[1].frame().unwrap().2.effect.iter().all(|e| e.abs() < 0.01), "no effects room appears");
    let InlineObject::Group { children, .. } = &got[3] else { panic!("{got:?}") };
    assert_eq!(children[0].obj.frame().unwrap().2.rot, 10.0);
}

/// #332: a group's own turn and flips are on its `wpg:grpSpPr/a:xfrm` and its members keep
/// theirs inside it; whether a member's shadow turns with it (`rotWithShape`) survives too.
#[test]
fn rotated_group_and_shadow_rotation_round_trip() {
    use wordcraft_doc::effects::{Shadow, ShapeEffects};
    let member = |rot_with_shape: bool| InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 60.0,
        h: 40.0,
        fill: Some(Rgb(1, 2, 3)),
        stroke: None,
        stroke_width: 0.0,
        float: Float { rot: 20.0, ..Default::default() },
        story: None,
        freeform: None,
        effects: ShapeEffects { shadow: Some(Shadow { rot_with_shape, ..Default::default() }), ..Default::default() },
    };
    let group = InlineObject::Group {
        w: 120.0,
        h: 40.0,
        float: Float { wrap: Wrap::Square, rot: 135.0, flip_h: true, ..Default::default() },
        ch_w: 120.0,
        ch_h: 40.0,
        children: vec![
            wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: member(true) },
            wordcraft_doc::para::GroupChild { x: 60.0, y: 0.0, obj: member(false) },
        ],
    };
    let mut p = Paragraph::with_text("g", CharProps::default());
    p.insert_object(1, group, &CharProps::default()).unwrap();
    let bytes = wordcraft_docx::write(&doc_with(vec![p])).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"<wpg:grpSpPr><a:xfrm rot="8100000" flipH="1">"#), "{xml}");
    assert!(xml.contains(r#"rotWithShape="1""#) && xml.contains(r#"rotWithShape="0""#), "{xml}");
    let r = wordcraft_docx::read(&bytes).expect("read");
    let Some(InlineObject::Group { float, children, .. }) = paras(&r)[0].objects.first() else { panic!("{:?}", paras(&r)[0].objects) };
    assert_eq!((float.rot, float.flip_h, float.flip_v), (135.0, true, false));
    let got: Vec<(f32, Option<bool>)> = children
        .iter()
        .map(|c| match &c.obj {
            InlineObject::Shape { float, effects, .. } => (float.rot, effects.shadow.map(|s| s.rot_with_shape)),
            o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(got, [(20.0, Some(true)), (20.0, Some(false))]);
}
