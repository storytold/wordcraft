//! Round-trip tests: build documents programmatically → write → read → compare.

use std::sync::Arc;
use wordcraft_doc::fill::Fill;

use wordcraft_doc::numbering::ListKind;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CharProps, HeightRule, Highlight, LineSpacing, NumRef, ParaProps, Rgb, RowProps, TabAlign, TabLeader,
    TabStop, TableLook, TextColor, TextDirection, Underline, VAlign, VMerge, VertAlign,
};
use wordcraft_doc::section::{Columns, DocGrid, DocGridType, LineNumberRestart, LineNumbering, NumFormat, PageVAlign, SectionProps, SectionStart};
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::table::{Cell, Table};
use wordcraft_doc::{Block, Blocks, Comment, Document, InlineObject, Paragraph, PartKind, Revision, RevisionKind, Source, Watermark, para_block};

fn rt(doc: &Document) -> Document {
    let bytes = wordcraft_docx::write(doc).expect("write");
    wordcraft_docx::read(&bytes).expect("read")
}

fn paras(doc: &Document) -> Vec<&Paragraph> {
    doc.body.iter().filter_map(|b| b.as_para()).collect()
}

/// `doc` with the stacking order of its body's floating objects cleared: saving gives objects
/// without one an order (`relativeHeight`), so tests about other properties compare without it.
fn unstacked(mut doc: Document) -> Document {
    for b in &mut doc.body {
        if let Block::Para(p) = std::sync::Arc::make_mut(b) {
            for o in &mut p.objects {
                if let Some(f) = o.float_mut() {
                    f.z = 0;
                }
            }
        }
    }
    doc
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
        // The symbol's own font: the run is written as `w:sym` characters (#2036).
        sym_font: Some("Wingdings".into()),
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
        web_hidden: Some(true),
        snap_to_grid: Some(false),
        spacing: Some(1.5),
        scale: Some(150.0),
        position: Some(3.0),
        kern: Some(8.0),
        ligatures: Some(wordcraft_doc::props::Ligatures { standard: true, historical: true, ..Default::default() }),
        outline: Some(true),
        shadow: Some(true),
        emboss: Some(false),
        engrave: Some(true),
        // WordArt effects ([MS-DOCX] w14 run properties).
        text_effects: Some(Box::new(wordcraft_doc::wordart::TextEffects {
            fill: Some(wordcraft_doc::wordart::TextFill::Gradient {
                stops: vec![
                    wordcraft_doc::wordart::GradStop { pos: 0.0, color: Rgb(0x15, 0x60, 0x82), transparency: 0.0 },
                    wordcraft_doc::wordart::GradStop { pos: 100.0, color: Rgb(0xE9, 0x71, 0x32), transparency: 25.0 },
                ],
                angle: 90.0,
            }),
            outline: Some(wordcraft_doc::wordart::TextOutline { color: Some(Rgb(0x0E, 0x28, 0x41)), width: 1.5, transparency: 0.0 }),
            shadow: Some(wordcraft_doc::effects::Shadow {
                color: Rgb::BLACK,
                transparency: 60.0,
                blur: 4.0,
                distance: 3.0,
                angle: 45.0,
                rot_with_shape: false,
            }),
            glow: Some(wordcraft_doc::effects::Glow { color: Rgb(0x0F, 0x9E, 0xD5), size: 5.0, transparency: 40.0 }),
            reflection: Some(wordcraft_doc::wordart::Reflection::default()),
        })),
        lang: Some("fr-FR".into()),
        no_proof: Some(true),
        rtl: Some(false),
        cs: Some(true),
        font_cs: Some("B Nazanin".into()),
        font_ea: Some("宋体".into()),
        size_cs: Some(13.0),
        bold_cs: Some(false),
        italic_cs: Some(true),
        lang_bidi: Some("fa-IR".into()),
        lang_ea: Some("ja-JP".into()),
        east_asian_layout: Some(wordcraft_doc::props::EastAsianLayout {
            id: Some(3),
            combine: Some(true),
            combine_brackets: Some(wordcraft_doc::props::CombineBrackets::Square),
            vert: Some(false),
            vert_compress: Some(true),
        }),
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
        indent_left_chars: Some(1.5),
        indent_right_chars: Some(0.0),
        indent_first_chars: Some(-1.0),
        space_before: Some(6.0),
        space_after: Some(12.0),
        space_before_auto: Some(true),
        space_after_auto: Some(false),
        line_spacing: Some(LineSpacing::Multiple(1.5)),
        contextual_spacing: Some(true),
        mirror_indents: Some(true),
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
        suppress_overlap: Some(false),
        bidi: Some(false),
        drop_cap: None,
        drop_cap_margin: None,
        drop_cap_distance: None,
        drop_cap_font: None,
        frame: Some(wordcraft_doc::props::TextFrame {
            w: Some(144.0),
            h: Some(36.0),
            h_exact: true,
            x: 0.0,
            y: 12.0,
            x_align: Some("right".into()),
            y_align: None,
            h_anchor: "margin".into(),
            v_anchor: "text".into(),
            wrap: "around".into(),
            h_space: 9.0,
            v_space: 0.0,
        }),
        kinsoku: Some(false),
        word_wrap: Some(false),
        overflow_punct: Some(false),
        top_line_punct: Some(true),
        auto_space_de: Some(false),
        auto_space_dn: Some(true),
        num_change: None,
        fmt_change: None,
        snap_to_grid: Some(false),
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
        locked: true,
        default: false,
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
    t.rows[0].props = RowProps { height: Some(20.0), height_rule: HeightRule::Exact, header: true, cant_split: true, ..Default::default() };
    t.rows[1].props = RowProps { height: Some(15.0), height_rule: HeightRule::AtLeast, ..Default::default() };
    t.merge(0, 0, 0, 1); // horizontal
    t.merge(1, 2, 2, 2); // vertical
    {
        let c = &mut t.rows[1].cells[0];
        c.props.shading = Some(Rgb(0xFF, 0, 0));
        c.props.valign = Some(VAlign::Bottom);
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
        p.insert_object(
            5,
            InlineObject::Field { instr: instr.into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
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
        valign: PageVAlign::Center,
        page_borders: Some(Borders::box_(Border::single(1.0))),
        line_numbers: Some(LineNumbering { count_by: 5, start: 1, distance: 18.0, restart: LineNumberRestart::Section }),
        gutter: 18.0,
        // #391: a line and character grid (Word writes `w:docGrid` even without a grid).
        doc_grid: Some(DocGrid { kind: DocGridType::LinesAndChars, line_pitch: 17.5, char_space: -1843 }),
        ..Default::default()
    };
    s1.headers.default = Some(h1);
    s1.headers.first = Some(hf);
    s1.footers.default = Some(f1);
    let mut s2 = SectionProps { start: SectionStart::OddPage, rtl: true, doc_grid: Some(DocGrid::default()), ..Default::default() };
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
    assert_eq!(
        fp.objects,
        vec![InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() }]
    );
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
    let r = unstacked(rt(&d));
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
        Comment {
            author: "Ann".into(),
            initials: "A".into(),
            date: "2026-01-02T03:04:05Z".into(),
            parent: None,
            resolved: true,
            part: c0,
            ..Default::default()
        },
    );
    d.comments.insert(
        1,
        Comment {
            author: "Ben".into(),
            initials: "B".into(),
            date: "2026-01-03T00:00:00Z".into(),
            parent: Some(0),
            resolved: false,
            part: c1,
            ..Default::default()
        },
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

/// #714: every table style part (ECMA-376 §17.7.6 `w:tblStylePr` types) keeps its run
/// formatting, cell shading, cell borders and horizontal and vertical alignment through a save.
#[test]
fn every_table_style_part_round_trips() {
    use wordcraft_doc::styles::{PartFormat, TablePart};
    let mut d = doc_with(vec![Paragraph::with_text("x", CharProps::default())]);
    let mut st = Style { id: "Parts".into(), name: "Parts".into(), kind: StyleKind::Table, based_on: Some("TableGrid".into()), ..Default::default() };
    let aligns = [Align::Left, Align::Center, Align::Right];
    let valigns = [VAlign::Top, VAlign::Center, VAlign::Bottom];
    for (i, part) in TablePart::ALL.into_iter().enumerate() {
        let n = i as u8;
        let f = PartFormat {
            chr: CharProps {
                bold: Some(i % 2 == 0),
                italic: Some(i % 3 == 0),
                underline: (i % 4 == 0).then_some(Underline::Single),
                size: Some(8.0 + i as f32),
                font: Some(format!("Face {i}")),
                color: Some(TextColor::Rgb(Rgb(n, 0x20, 0x40))),
                ..Default::default()
            },
            fill: Some(Rgb(0x10 + n, 0x80, 0xF0 - n)),
            borders: Some(Borders {
                top: Some(Border { style: BorderStyle::Double, width: 1.5, color: Some(Rgb(n, 0, 0)), space: 0.0 }),
                bottom: Some(Border::single(0.5 + i as f32 / 4.0)),
                left: Some(Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 }),
                inside_v: (i % 2 == 1).then(|| Border::single(0.25)),
                ..Default::default()
            }),
            align: aligns.get(i % 3).copied(),
            v_align: valigns.get(i % 3).copied(),
            fit_text: (i % 2 == 0).then_some(i % 3 != 0),
        };
        st.set_table_part(part, f);
    }
    d.styles.upsert(st.clone());
    let r = rt(&d);
    let got = r.styles.get("Parts").unwrap();
    for part in TablePart::ALL {
        assert_eq!(got.table_part(part), st.table_part(part), "{}", part.ooxml());
    }
    assert_eq!(got.table, st.table);
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

/// A note with a custom mark keeps it in the note's own text: a plain run with the mark, not the
/// `w:footnoteRef` / `w:endnoteRef` placeholder that Word replaces by the automatic number.
#[test]
fn custom_note_mark_stays_in_the_note() {
    let mut d = Document::new();
    let f = d.add_part(PartKind::Footnote, Vec::new());
    let e = d.add_part(PartKind::Endnote, Vec::new());
    for (id, kind) in [(f, NoteKind::Footnote), (e, NoteKind::Endnote)] {
        let mut note = Paragraph::with_text(" The note.", CharProps::default());
        note.insert_object(0, InlineObject::NoteRef { kind, id, custom: "*".into() }, &CharProps::default()).unwrap();
        d.parts.get_mut(&id).unwrap().blocks = vec![para_block(note)];
    }
    let mut p = Paragraph::with_text("Text", CharProps::default());
    p.insert_object(4, InlineObject::NoteRef { kind: NoteKind::Footnote, id: f, custom: "*".into() }, &CharProps::default()).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::NoteRef { kind: NoteKind::Endnote, id: e, custom: "*".into() }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];

    let bytes = wordcraft_docx::write(&d).expect("write");
    for (file, mark) in [("word/footnotes.xml", "<w:footnoteRef/>"), ("word/endnotes.xml", "<w:endnoteRef/>")] {
        let xml = part_xml(&bytes, file);
        assert!(!xml.contains(mark), "{file}: the custom mark became an automatic number: {xml}");
        assert!(xml.contains(r#"<w:t xml:space="preserve">*</w:t>"#), "{file}: {xml}");
    }
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
    for entries in [&[("Intro\t1", "TOC1")][..], &[("Intro\t1", "TOC1"), ("Detail\t2", "TOC2")][..]] {
        let mut head = Paragraph::with_text("Contents", CharProps::default()).styled("TOCHeading");
        let end = head.len();
        head.insert_object(
            end,
            InlineObject::Field {
                instr: "TOC \\o \"1-2\" \\h \\z \\u".into(),
                result: String::new(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
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
        let last_entry = xml.find(entries.last().unwrap().0.split('\t').next().unwrap()).unwrap();
        assert!(sdt < xml.find("Contents").unwrap() && xml.find("</w:sdt>").unwrap() < xml.find(">Body<").unwrap(), "{xml}");
        assert!(
            begin < xml.find("Intro").unwrap() && fend > last_entry && fend < xml.find("</w:sdt>").unwrap(),
            "field must span the entries: {xml}"
        );
        // Word starts the field in the first entry; started in the heading, LibreOffice splits the heading.
        let heading_end = xml[..xml.find("Intro").unwrap()].rfind("</w:p>").unwrap();
        assert!(begin > heading_end, "field must start in the first entry, not the heading: {xml}");

        // WordCraft reads its own TOC back unchanged.
        let r = wordcraft_docx::read(&bytes).expect("read");
        let ps = paras(&r);
        let expected: Vec<_> = std::iter::once("Contents").chain(entries.iter().map(|(t, _)| *t)).chain(std::iter::once("Body")).collect();
        assert_eq!(ps.iter().map(|p| p.plain_text()).collect::<Vec<_>>(), expected);
        assert!(matches!(ps[0].objects.first(), Some(InlineObject::Field { instr, .. }) if instr.starts_with("TOC")), "{:?}", ps[0].objects);
        assert!(ps[1].objects.is_empty(), "the field goes back on the heading: {:?}", ps[1].objects);
        assert_eq!(ps[1].props.style.as_deref(), Some("TOC1"));
    }
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
            extra: Default::default(),
        }
    };
    let mut head = Paragraph::with_text("Contents", CharProps::default()).styled("TOCHeading");
    let end = head.len();
    head.insert_object(
        end,
        InlineObject::Field {
            instr: "TOC \\o \"1-1\" \\h \\z \\u".into(),
            result: String::new(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
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
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "Alice".into(), date: "2026-05-01T10:00:00Z".into(), move_name: None });
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "Bob".into(), date: "2026-05-02T10:00:00Z".into(), move_name: None });
    d.settings.track_changes = true;
    let ins = CharProps { ins: Some(0), ..Default::default() };
    let del = CharProps { del: Some(1), bold: Some(true), ..Default::default() };
    let both = CharProps { ins: Some(0), del: Some(1), ..Default::default() };
    let p = para_runs(&[("kept ", CharProps::default()), ("added\t", ins.clone()), ("removed", del.clone()), ("gone", both.clone())]);
    let mut p = p;
    let end = p.len();
    p.insert_object(
        end,
        InlineObject::Field { instr: "DATE".into(), result: "today".into(), locked: false, dirty: false, code: false, form: String::new() },
        &del,
    )
    .unwrap();
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
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "Alice".into(), date: "2026-05-01T10:00:00Z".into(), move_name: None });
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "Bob".into(), date: "2026-05-02T10:00:00Z".into(), move_name: None });
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
        InlineObject::Field { instr: "PAGE".into(), result: "4".into(), locked: false, dirty: false, code: false, form: String::new() },
        InlineObject::Field {
            instr: "DATE \\@ \"M/d/yyyy\"".into(),
            result: "1/2/2026".into(),
            locked: true,
            dirty: false,
            code: false,
            form: String::new(),
        },
        InlineObject::Field { instr: "NUMPAGES".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
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

/// The `document.xml` of a written package.
fn document_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    xml
}

/// #877: a field begin marker's `w:dirty` (the cached result is stale; Word refreshes it on
/// open) survives a round trip, and a field without the flag doesn't gain one. Generators that
/// can't paginate (pandoc-style pipelines, python-docx templates) set the flag instead of a result.
#[test]
fn field_dirty_flag_round_trips() {
    let mut p = Paragraph::with_text("", CharProps::default());
    for instr in ["PAGE", "DATE"] {
        let end = p.len();
        p.insert_object(
            end,
            InlineObject::Field { instr: instr.into(), result: "7".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
    }
    let bytes = wordcraft_docx::write(&doc_with(vec![p])).expect("write");
    // Another producer marks the first field's begin as dirty.
    let xml = document_xml(&bytes);
    let marked = xml.replacen(r#"<w:fldChar w:fldCharType="begin"/>"#, r#"<w:fldChar w:fldCharType="begin" w:dirty="true"/>"#, 1);
    assert_ne!(&marked, &xml, "the written begin marker must be bare: {xml}");
    let r = wordcraft_docx::read(&replace_part(&bytes, "word/document.xml", marked.as_bytes())).expect("read");
    let out = document_xml(&wordcraft_docx::write(&r).expect("write"));
    assert_eq!(out.matches("w:dirty").count(), 1, "the dirty field keeps its flag, the other gains none: {out}");
    assert!(out.contains(r#"<w:fldChar w:fldCharType="begin" w:dirty="true"/>"#), "{out}");
}

/// #877: a TOC imported as a range field keeps `w:dirty` through a round trip — the flag sits on
/// the begin marker Word rebuilds the field from (#878's fixture shape: a placeholder result).
#[test]
fn range_toc_dirty_flag_round_trips() {
    let mut toc = Paragraph::new();
    toc.insert_object(0, InlineObject::FieldStart { instr: r#"TOC \o "1-3" \h \z \u"#.into(), locked: false, dirty: false }, &CharProps::default())
        .unwrap();
    let end = toc.len();
    toc.insert_text(end, "Placeholder: update fields (F9).", &CharProps::default()).unwrap();
    let end = toc.len();
    toc.insert_object(end, InlineObject::FieldEnd, &CharProps::default()).unwrap();
    let mut blocks = vec![toc];
    for (t, style) in [("Alpha", Some("Heading1")), ("Body text.", None), ("Beta", Some("Heading2")), ("Gamma", Some("Heading1"))] {
        let mut h = Paragraph::with_text(t, CharProps::default());
        h.props.style = style.map(str::to_string);
        blocks.push(h);
    }
    let bytes = wordcraft_docx::write(&doc_with(blocks)).expect("write");
    let xml = document_xml(&bytes);
    let marked = xml.replacen(r#"<w:fldChar w:fldCharType="begin"/>"#, r#"<w:fldChar w:fldCharType="begin" w:dirty="true"/>"#, 1);
    assert_ne!(&marked, &xml, "the written begin marker must be bare: {xml}");
    let marked = replace_part(&bytes, "word/document.xml", marked.as_bytes());
    let r = wordcraft_docx::read(&marked).expect("read");
    // Still the imported shape: a range field holding the placeholder as ordinary content.
    assert!(matches!(paras(&r)[0].objects.first(), Some(InlineObject::FieldStart { .. })), "{:?}", paras(&r)[0].objects);
    assert!(paras(&r)[0].objects.last().is_some_and(|o| matches!(o, InlineObject::FieldEnd)));
    assert_eq!(paras(&r)[0].plain_text(), "Placeholder: update fields (F9).");
    let out = document_xml(&wordcraft_docx::write(&r).expect("write"));
    assert_eq!(out.matches("w:dirty").count(), 1, "the TOC keeps its flag: {out}");
    let (begin, placeholder) = (out.find("fldCharType=\"begin\"").unwrap(), out.find("Placeholder").unwrap());
    let (separate, end) = (out.find("fldCharType=\"separate\"").unwrap(), out.find("fldCharType=\"end\"").unwrap());
    assert!(begin < separate && separate < placeholder && placeholder < end, "the placeholder stays the field's result: {out}");
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
        fill: Some(Fill::solid(Rgb(255, 255, 200))),
        stroke: Some(Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: Float { wrap: Wrap::Square, h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, x: 10.0, y: 20.0, dist: 0.0, ..Default::default() },
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
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
        extra: Default::default(),
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
                (*kind, *w, *h, fill.clone(), *stroke, *stroke_width),
                (ShapeKind::TextBox, 144.0, 72.0, Some(Fill::solid(Rgb(255, 255, 200))), Some(Rgb(0, 0, 0)), 1.0)
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
        fill: Some(Fill::solid(Rgb(200, 0, 0))),
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
        extra: Default::default(),
    };
    let tb = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 100.0,
        h: 40.0,
        fill: Some(Fill::solid(Rgb(255, 255, 255))),
        stroke: Some(Rgb(0, 0, 0)),
        stroke_width: 0.75,
        float: Float::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 20.0, y: 10.0, dist: 9.0, ..Default::default() };
    let group = InlineObject::Group {
        w: 240.0,
        h: 120.0,
        float: float.clone(),
        ch_w: 200.0,
        ch_h: 100.0,
        children: vec![
            GroupChild { x: 0.0, y: 0.0, obj: pic },
            GroupChild { x: 150.0, y: 10.0, obj: oval.clone() },
            GroupChild { x: 50.0, y: 60.0, obj: tb },
        ],
        canvas: None,
    };
    let mut p = Paragraph::with_text("grouped ", CharProps::default());
    let end = p.len();
    p.insert_object(end, group, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let r = unstacked(rt(&d));
    let got = paras(&r);
    let InlineObject::Group { w, h, float: f, ch_w, ch_h, children, .. } = &got[0].objects[0] else { panic!("{:?}", got[0].objects) };
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

/// A Drawing Canvas (`wpc:wpc`, #344) with a picture and a shape comes back as a canvas: its
/// frame, background and outline, and each member at its offset in the canvas, unscaled.
#[test]
fn drawing_canvas_round_trips() {
    use wordcraft_doc::para::{CanvasStyle, GroupChild};
    let mut d = Document::new();
    let key = d.add_media(tiny_png(), "png");
    let pic = InlineObject::Image { media: key, w: 60.0, h: 40.0, alt: "In the canvas".into(), float: Float::default(), crop: [0.0; 4], ole: None };
    let star = InlineObject::Shape {
        kind: ShapeKind::Star,
        w: 72.0,
        h: 72.0,
        fill: Some(Fill::solid(Rgb(0xE0, 0xA0, 0x20))),
        stroke: Some(Rgb(0x40, 0x40, 0x40)),
        stroke_width: 1.0,
        float: Float::default(),
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let style = CanvasStyle {
        fill: Some(Fill::Solid { color: Rgb(0xF0, 0xF4, 0xFF), transparency: 25.0 }),
        stroke: Some(Rgb(0x20, 0x40, 0x80)),
        stroke_width: 1.5,
    };
    let canvas = InlineObject::Group {
        w: 432.0,
        h: 216.0,
        float: Float::default(),
        ch_w: 432.0,
        ch_h: 216.0,
        children: vec![GroupChild { x: 12.0, y: 18.0, obj: pic }, GroupChild { x: 300.0, y: 100.0, obj: star.clone() }],
        canvas: Some(style.clone()),
    };
    let mut p = Paragraph::with_text("drawn ", CharProps::default());
    let end = p.len();
    p.insert_object(end, canvas, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&d).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains("wordprocessingCanvas") && xml.contains("<wpc:wpc>") && xml.contains("<wpc:bg>") && xml.contains("<wpc:whole>"), "{xml}");
    let r = wordcraft_docx::read(&bytes).unwrap();
    let got = paras(&r);
    let InlineObject::Group { w, h, ch_w, ch_h, children, canvas, .. } = &got[0].objects[0] else { panic!("{:?}", got[0].objects) };
    assert_eq!((*w, *h, *ch_w, *ch_h), (432.0, 216.0, 432.0, 216.0));
    assert_eq!(*canvas, Some(style));
    assert_eq!(children.iter().map(|c| (c.x, c.y)).collect::<Vec<_>>(), [(12.0, 18.0), (300.0, 100.0)]);
    assert!(matches!(&children[0].obj, InlineObject::Image { w, h, alt, .. } if (*w, *h, alt.as_str()) == (60.0, 40.0, "In the canvas")));
    assert_eq!(children[1].obj, star);
}

/// A turned and flipped Drawing Canvas (#344 with #332): the canvas schema has no transform, so
/// its turn rides in our `wpc:extLst` extension, after its members; members keep their own turn.
/// An unturned canvas writes no extension.
#[test]
fn rotated_drawing_canvas_round_trips() {
    use wordcraft_doc::para::{CanvasStyle, GroupChild};
    let member = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 60.0,
        h: 40.0,
        fill: Some(Fill::solid(Rgb(1, 2, 3))),
        stroke: None,
        stroke_width: 0.0,
        float: Float { rot: 15.0, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let canvas = |float: Float| InlineObject::Group {
        w: 200.0,
        h: 100.0,
        float,
        ch_w: 200.0,
        ch_h: 100.0,
        children: vec![GroupChild { x: 10.0, y: 20.0, obj: member.clone() }],
        canvas: Some(CanvasStyle { fill: Some(Fill::solid(Rgb(0xF0, 0xF4, 0xFF))), stroke: None, stroke_width: 0.0 }),
    };
    let square = Float { wrap: Wrap::Square, ..Default::default() };
    let mut p = Paragraph::with_text("c", CharProps::default());
    p.insert_object(1, canvas(Float { rot: 30.0, flip_h: true, ..square.clone() }), &CharProps::default()).unwrap();
    let end = p.len();
    p.insert_object(end, canvas(square), &CharProps::default()).unwrap();
    let bytes = wordcraft_docx::write(&doc_with(vec![p])).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(
        xml.contains(r#"<wpc:extLst><a:ext uri="urn:wordcraft:canvas-xfrm"><a:xfrm rot="1800000" flipH="1"/></a:ext></wpc:extLst></wpc:wpc>"#),
        "{xml}"
    );
    assert_eq!(xml.matches("<wpc:extLst>").count(), 1, "{xml}");
    let r = wordcraft_docx::read(&bytes).expect("read");
    let got = &paras(&r)[0].objects;
    let spins: Vec<(f32, bool, bool)> = got.iter().map(|o| o.frame().unwrap().2).map(|f| (f.rot, f.flip_h, f.flip_v)).collect();
    assert_eq!(spins, [(30.0, true, false), (0.0, false, false)]);
    let InlineObject::Group { children, canvas, .. } = &got[0] else { panic!("{got:?}") };
    assert!(canvas.is_some());
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].obj.frame().unwrap().2.rot, 15.0);
}

#[test]
fn table_contextual_spacing_compatibility_flag_round_trips() {
    for mode in [11, 12, 14, 15] {
        for allow in [false, true] {
            let mut d = Document::new();
            d.settings.compat_mode = mode;
            d.settings.allow_space_of_same_style_in_table = allow;
            let r = rt(&d);
            assert_eq!(r.settings.allow_space_of_same_style_in_table, allow);
            assert_eq!(r.settings.compat_mode, mode);
            if allow {
                let bytes = wordcraft_docx::write(&d).unwrap();
                let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
                let mut xml = String::new();
                std::io::Read::read_to_string(&mut zip.by_name("word/settings.xml").unwrap(), &mut xml).unwrap();
                // CT_Compat requires the boolean option before compatSetting.
                assert!(xml.find("<w:allowSpaceOfSameStyleInTable").unwrap() < xml.find("<w:compatSetting").unwrap());
            }
        }
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

/// Tools › Templates and Add-ins: the attached template (an external relationship of the settings
/// part) and "Automatically update document styles" (`w:linkStyles`) round-trip, written in the
/// schema's order (after `w:mirrorMargins`, before `w:trackRevisions`).
#[test]
fn attached_template_and_link_styles_round_trip() {
    let mut d = Document::new();
    d.settings.attached_template = Some("file:///C:/Templates/Report & Co.dotx".into());
    d.settings.link_styles = true;
    d.settings.mirror_margins = true;
    d.settings.track_changes = true;
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/settings.xml").unwrap(), &mut xml).unwrap();
    let at = |tag: &str| xml.find(tag).unwrap_or_else(|| panic!("{tag} missing: {xml}"));
    assert!(at("<w:mirrorMargins") < at("<w:attachedTemplate"));
    assert!(at("<w:attachedTemplate") < at("<w:linkStyles"));
    assert!(at("<w:linkStyles") < at("<w:trackRevisions"));
    let mut rels = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/_rels/settings.xml.rels").unwrap(), &mut rels).unwrap();
    assert!(rels.contains("relationships/attachedTemplate") && rels.contains("TargetMode=\"External\""), "{rels}");

    let r = wordcraft_docx::read(&bytes).expect("read");
    assert_eq!(r.settings.attached_template.as_deref(), Some("file:///C:/Templates/Report & Co.dotx"));
    assert!(r.settings.link_styles);

    // Detached: neither element nor relationship is written.
    let plain = rt(&Document::new());
    assert_eq!(plain.settings.attached_template, None);
    assert!(!plain.settings.link_styles);
}

/// #995: the zoom a document was saved at (`w:zoom`), One Page (`fullPage`) included, is read
/// and written back.
#[test]
fn zoom_round_trips() {
    let settings = |bytes: &[u8]| {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/settings.xml").unwrap(), &mut xml).unwrap();
        xml
    };
    let plain = wordcraft_docx::write(&Document::new()).unwrap();
    let xml = settings(&plain);
    assert!(xml.contains(r#"<w:zoom w:percent="100"/>"#), "{xml}");
    for (zoom, written) in [
        (r#"<w:zoom w:val="fullPage" w:percent="62"/>"#, r#"<w:zoom w:val="fullPage" w:percent="62"/>"#),
        (r#"<w:zoom w:percent="75%"/>"#, r#"<w:zoom w:percent="75"/>"#),
        (r#"<w:zoom w:val="bestFit" w:percent="99999"/>"#, r#"<w:zoom w:val="bestFit" w:percent="500"/>"#),
        (r#"<w:zoom w:val="sideways" w:percent="80"/>"#, r#"<w:zoom w:percent="80"/>"#),
    ] {
        let file = replace_part(&plain, "word/settings.xml", xml.replace(r#"<w:zoom w:percent="100"/>"#, zoom).as_bytes());
        let back = settings(&wordcraft_docx::write(&wordcraft_docx::read(&file).unwrap()).unwrap());
        assert!(back.contains(written), "{zoom}: {back}");
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
    // Hyphenation Options (#407): zone, words in capitals, consecutive hyphens.
    d.settings.hyphenation_zone = 27.0;
    d.settings.hyphenate_caps = false;
    d.settings.consecutive_hyphen_limit = 2;
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
    // #1290: the watermark is written as a VML text shape in the headers and read back.
    let mut w = d.clone();
    w.settings.watermark = Some(Watermark::default());
    assert_eq!(rt(&w).settings.watermark, w.settings.watermark);
}

fn part_xml(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name(name).unwrap(), &mut xml).unwrap();
    xml
}

/// Header parts in the document, by id.
fn header_ids(doc: &Document) -> Vec<u32> {
    doc.parts.iter().filter(|(_, p)| p.kind == PartKind::Header).map(|(id, _)| *id).collect()
}

/// #1290: a watermark round-trips through Word's VML text shape: text, font, colour, diagonal and
/// semitransparent, defaults or not.
#[test]
fn watermark_roundtrips() {
    for wm in [
        Watermark::default(),
        Watermark {
            text: "Riservato & <interno>".into(),
            font: "Georgia".into(),
            color: Rgb(0x80, 0x10, 0x20),
            diagonal: false,
            semitransparent: false,
            size: None,
        },
        // A box of its own (not Word's default for the page) is kept.
        Watermark { text: "COPY".into(), size: Some([300.0, 80.0]), ..Default::default() },
    ] {
        let mut d = doc_with(vec![Paragraph::with_text("Body", CharProps::default())]);
        d.settings.watermark = Some(wm.clone());
        let r = rt(&d);
        assert_eq!(r.settings.watermark, Some(wm));
        assert_eq!(paras(&r).len(), 1, "the body gains nothing");
    }
}

/// #3994: a watermark font name round-trips through the textpath's CSS-like style whatever it
/// carries — apostrophes, semicolons, colons, quotes, backslashes. Before, those characters
/// were deleted (the font was saved under a different name) and a trailing backslash escaped
/// the style's closing quote.
#[test]
fn watermark_font_roundtrips_through_the_textpath_style() {
    for font in ["Georgia", "Bob's Serif", "A;Named:Font", "Quote\"Inside", "Ends with \\", "All \"of\" ;it: 'here\\", "'Serif'", "Trailing'"] {
        let mut d = doc_with(vec![Paragraph::with_text("Body", CharProps::default())]);
        d.settings.watermark = Some(Watermark { font: font.into(), ..Default::default() });
        assert_eq!(rt(&d).settings.watermark.map(|w| w.font), Some(font.into()), "{font:?}");
    }
}

/// #3994: the written textpath style stays well-formed CSS whatever the font name: control
/// characters are dropped (no font name carries one) and a backslash or quote is a CSS escape
/// (`\5c `, `\22 `) instead of escaping the string's closing quote.
#[test]
fn watermark_font_name_writes_wellformed_css() {
    let mut d = doc_with(vec![Paragraph::with_text("Body", CharProps::default())]);
    d.settings.watermark = Some(Watermark { font: "Ends with \\ and \n\"quoted\"".into(), ..Default::default() });
    let bytes = wordcraft_docx::write(&d).unwrap();
    let hdr = part_xml(&bytes, "word/header1.xml");
    let style = hdr.split("v:textpath style=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or_default();
    assert!(!style.contains('\n'), "control characters are dropped: {style}");
    assert!(style.contains(r#"font-family:&quot;Ends with \5c  and \22 quoted\22 &quot;;"#), "CSS escapes: {style}");
}

/// #1290: a document without headers gains one default header holding only the watermark (an
/// empty paragraph once read back); saving again neither adds another nor duplicates the shape.
#[test]
fn watermark_without_header_gains_one() {
    let mut d = doc_with(vec![Paragraph::with_text("Body", CharProps::default())]);
    d.settings.watermark = Some(Watermark::default());
    let bytes = wordcraft_docx::write(&d).unwrap();
    let hdr = part_xml(&bytes, "word/header1.xml");
    assert_eq!(hdr.matches("<v:shape ").count(), 1);
    assert!(hdr.contains(r#"id="PowerPlusWaterMarkObject1""#) && hdr.contains("rotation:315") && hdr.contains(r#"<v:fill opacity=".5"/>"#));
    // The text sits between the top and bottom edges: on one line Word gives it no height.
    assert!(hdr.contains(r#"path="m0,0l21600,0m0,21600l21600,21600e""#));
    assert!(part_xml(&bytes, "word/document.xml").contains(r#"<w:headerReference w:type="default""#));
    let r = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(r.settings.watermark, d.settings.watermark);
    let ids = header_ids(&r);
    assert_eq!(ids.len(), 1);
    assert_eq!(r.last_section.headers.default, ids.first().copied());
    let blocks = &r.parts[&ids[0]].blocks;
    assert_eq!(blocks.len(), 1);
    let p = blocks[0].as_para().unwrap();
    assert!(p.text.is_empty() && p.objects.is_empty(), "the shape leaves nothing behind: {p:?}");
    // Second generation: stable.
    let r2 = rt(&r);
    assert_eq!(r2.settings.watermark, d.settings.watermark);
    assert_eq!(header_ids(&r2).len(), 1);
    assert_eq!(r2.parts[&header_ids(&r2)[0]].blocks.len(), 1);
}

/// #1290: a header keeps its own text, and the watermark goes into its first paragraph (no extra
/// line). Each header part carries the shape; footers don't.
#[test]
fn watermark_joins_existing_headers() {
    let mut d = doc_with(vec![Paragraph::with_text("Body", CharProps::default())]);
    let h = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Head", CharProps::default()))]);
    let hf = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("First", CharProps::default()))]);
    let f = d.add_part(PartKind::Footer, vec![para_block(Paragraph::with_text("Foot", CharProps::default()))]);
    d.last_section.headers.default = Some(h);
    d.last_section.headers.first = Some(hf);
    d.last_section.title_page = true;
    d.last_section.footers.default = Some(f);
    d.settings.watermark = Some(Watermark::default());
    let bytes = wordcraft_docx::write(&d).unwrap();
    for name in ["word/header1.xml", "word/header2.xml"] {
        assert_eq!(part_xml(&bytes, name).matches("<v:shape ").count(), 1, "{name}");
    }
    assert!(!part_xml(&bytes, "word/footer1.xml").contains("v:shape"));
    let r = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(r.settings.watermark, d.settings.watermark);
    assert_eq!(header_ids(&r).len(), 2, "no header was added");
    assert_eq!(part_text(&r, r.last_section.headers.default), "Head");
    assert_eq!(part_text(&r, r.last_section.headers.first), "First");
    for id in header_ids(&r) {
        assert_eq!(r.parts[&id].blocks.len(), 1);
    }
}

/// #1290: only the slots the pages show and nothing fills get the watermark's own header: a
/// later section still links to the earlier one's header, and a title page with no first-page
/// header gets one.
#[test]
fn watermark_header_respects_linking() {
    let mut d = Document::new();
    let h = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Head", CharProps::default()))]);
    let mut s1 = SectionProps::default();
    s1.headers.default = Some(h);
    s1.title_page = true;
    let mut p1 = Paragraph::with_text("One", CharProps::default());
    p1.section = Some(Box::new(s1));
    d.body = vec![para_block(p1), para_block(Paragraph::with_text("Two", CharProps::default()))];
    d.last_section.title_page = true;
    d.settings.watermark = Some(Watermark::default());
    let r = rt(&d);
    assert_eq!(r.settings.watermark, d.settings.watermark);
    let secs = r.sections();
    assert_eq!(part_text(&r, secs[0].1.headers.default), "Head");
    let first = secs[0].1.headers.first.expect("a first-page header for the watermark");
    assert_eq!(part_text(&r, Some(first)), "");
    assert_eq!(secs[1].1.headers, Default::default(), "the second section still links to the first");
    assert_eq!(header_ids(&r).len(), 2);
}

/// #1290: a header holding only a table takes the watermark in the paragraph after the table
/// (one every story ends with), so it gains no line.
#[test]
fn watermark_in_table_only_header() {
    let mut d = doc_with(vec![Paragraph::with_text("Body", CharProps::default())]);
    let t = Table::new(1, 2, 400.0);
    let h = d.add_part(PartKind::Header, vec![Arc::new(Block::Table(t))]);
    d.last_section.headers.default = Some(h);
    let blocks = |d: &Document| d.parts[&d.last_section.headers.default.unwrap()].blocks.len();
    let plain = blocks(&rt(&d));
    d.settings.watermark = Some(Watermark::default());
    let r = rt(&d);
    assert_eq!(r.settings.watermark, d.settings.watermark);
    assert_eq!(blocks(&r), plain);
}

/// #1290: with different odd and even pages, a document with only a default header gains an
/// even-page header for the watermark; a later section stays linked to both.
#[test]
fn watermark_even_pages_header() {
    let mut d = Document::new();
    let h = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Head", CharProps::default()))]);
    let mut s1 = SectionProps::default();
    s1.headers.default = Some(h);
    let mut p1 = Paragraph::with_text("One", CharProps::default());
    p1.section = Some(Box::new(s1));
    d.body = vec![para_block(p1), para_block(Paragraph::with_text("Two", CharProps::default()))];
    d.settings.even_odd_headers = true;
    d.settings.watermark = Some(Watermark::default());
    let r = rt(&d);
    assert_eq!(r.settings.watermark, d.settings.watermark);
    let secs = r.sections();
    assert_eq!(part_text(&r, secs[0].1.headers.default), "Head");
    assert!(secs[0].1.headers.even.is_some() && secs[0].1.headers.first.is_none());
    assert_eq!(secs[1].1.headers, Default::default());
}

/// #1290: a blank watermark isn't written (it wouldn't read back), and adds no header.
#[test]
fn blank_watermark_writes_nothing() {
    let mut d = doc_with(vec![Paragraph::with_text("Body", CharProps::default())]);
    d.settings.watermark = Some(Watermark { text: "  ".into(), ..Watermark::default() });
    let r = rt(&d);
    assert_eq!(r.settings.watermark, None);
    assert!(header_ids(&r).is_empty());
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
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "A".into(), date: String::new(), move_name: None });
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
    d.sources.push(Source {
        tag: "Sink2026".into(),
        kind: "book".into(),
        author: "Author, First".into(),
        title: "Title".into(),
        year: "2026".into(),
        editor: "Editor, Chief".into(),
        edition: "3rd".into(),
        issue: "2".into(),
        month: "08".into(),
        day: "24".into(),
        accessed: "2026-08-25".into(),
        doi: "10.1234/sink.2026".into(),
        ..Default::default()
    });
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
    assert_eq!(d2.sources, d1.sources);
    assert_eq!(d2, d1);
}

/// A Word XML Document (Flat OPC) carries the same package as a .docx: reading it back gives
/// what the .docx round trip gives, pictures included (#548).
#[test]
fn flat_opc_round_trip_equals_docx() {
    let d = kitchen_sink();
    let xml = wordcraft_docx::write_flat(&d).expect("write flat");
    let text = std::str::from_utf8(&xml).expect("utf-8");
    assert!(text.starts_with("<?xml") && text.contains("<?mso-application progid=\"Word.Document\"?>"));
    assert!(text.contains("pkg:name=\"/word/document.xml\"") && text.contains("<pkg:binaryData>"));
    assert_eq!(wordcraft_docx::sniff_xml(&xml), wordcraft_docx::XmlKind::FlatOpc);
    let back = wordcraft_docx::read_xml(&xml).expect("read flat");
    assert_eq!(back, rt(&d));
    assert_eq!(back.media.len(), 1);
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
        extra: Default::default(),
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
/// `a:moveTo`/`a:lnTo`) in floating drawings and read back with their points, pen, opacity and
/// drawing order.
#[test]
fn ink_and_freeforms_round_trip_as_custom_geometry() {
    use wordcraft_doc::freeform::{FreePath, Freeform, InkTool};
    let shape = |fill: Option<Rgb>, stroke_width, wrap, f: Freeform| InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: f.w,
        h: f.h,
        fill: fill.map(Fill::solid),
        stroke: Some(Rgb(0xC0, 0, 0)),
        stroke_width,
        float: Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 30.0, y: 12.0, ..Default::default() },
        story: None,
        freeform: Some(Arc::new(f)),
        effects: Default::default(),
        extra: Default::default(),
    };
    // The pen stroke was the seventh drawn: Ink Replay's order comes back too.
    let pen = shape(
        None,
        2.0,
        Wrap::InFrontOfText,
        Freeform { order: Some(7), ..Freeform::ink(InkTool::Pen, 60.0, 20.0, vec![[1.0, 1.0], [30.0, 19.0], [59.0, 4.0]]) },
    );
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
    let r = unstacked(rt(&d));
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
        fill: Some(Fill::solid(Rgb(0x15, 0x60, 0x82))),
        stroke: None,
        stroke_width: 0.0,
        float: Float::default(),
        story: None,
        freeform: None,
        effects,
        extra: Default::default(),
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
        fill: Some(Fill::solid(Rgb(1, 2, 3))),
        stroke: None,
        stroke_width: 0.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let square = Float { wrap: Wrap::Square, ..Default::default() };
    let tri = shape(ShapeKind::Triangle, Float { rot: 90.0, flip_h: true, ..square.clone() });
    let flipped = shape(ShapeKind::Rectangle, Float { rot: 315.5, flip_v: true, ..square.clone() });
    let group = InlineObject::Group {
        w: 120.0,
        h: 60.0,
        float: Float { rot: 45.0, ..square },
        ch_w: 120.0,
        ch_h: 60.0,
        children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::Ellipse, Float { rot: 10.0, ..Default::default() }) }],
        canvas: None,
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
        fill: Some(Fill::solid(Rgb(1, 2, 3))),
        stroke: None,
        stroke_width: 0.0,
        float: Float { rot: 20.0, ..Default::default() },
        story: None,
        freeform: None,
        effects: ShapeEffects { shadow: Some(Shadow { rot_with_shape, ..Default::default() }), ..Default::default() },
        extra: Default::default(),
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
        canvas: None,
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

#[test]
fn bibliography_sources_round_trip() {
    let mut d = Document::new();
    d.sources.push(Source {
        tag: "Rivera2021".into(),
        kind: "book".into(),
        author: "Rivera, Alex; Chen, Mei".into(),
        title: "Shared Spaces".into(),
        year: "2021".into(),
        publisher: "Harbor Books".into(),
        city: "Portland".into(),
        editor: "Johnson, Robert".into(),
        edition: "2nd".into(),
        month: "10".into(),
        day: "15".into(),
        ..Default::default()
    });
    d.sources.push(Source {
        tag: "Smith2023".into(),
        kind: "article".into(),
        author: "Smith, John David".into(),
        title: "Modern Typography".into(),
        year: "2023".into(),
        journal: "Design Quarterly".into(),
        volume: "12".into(),
        pages: "45-60".into(),
        url: "https://example.com/typo".into(),
        issue: "4".into(),
        doi: "10.1000/182".into(),
        ..Default::default()
    });
    d.sources.push(Source {
        tag: "WebSite2024".into(),
        kind: "website".into(),
        author: "Solo".into(),
        title: "Open Fonts".into(),
        year: "2024".into(),
        url: "https://example.org".into(),
        accessed: "2024-05-15".into(),
        ..Default::default()
    });
    d.sources.push(Source {
        tag: "Chapter2025".into(),
        kind: "bookSection".into(),
        author: "Lee, Harper".into(),
        title: "Chapter One".into(),
        year: "2025".into(),
        journal: "Anthology of Prose".into(),
        pages: "1-20".into(),
        ..Default::default()
    });
    d.sources.push(Source {
        tag: "Misc2026".into(),
        kind: "other".into(),
        title: "Archive Manuscript".into(),
        year: "2026".into(),
        city: "Oxford".into(),
        ..Default::default()
    });

    let bytes = wordcraft_docx::write(&d).expect("write");

    assert_bibliography_parts(&bytes);
    assert_bibliography_sources_round_trip(&bytes);
}

/// #4848: a corporate author saves as Word's `b:Corporate`, in place of a name list (a role holds
/// one or the other), and reads back whole.
#[test]
fn a_corporate_author_round_trips() {
    let mut d = Document::new();
    d.sources.push(Source {
        tag: "Wor20".into(),
        kind: "report".into(),
        corporate: "World Health Organization".into(),
        title: "World report on hearing".into(),
        year: "2020".into(),
        editor: "Chadha, Shelly".into(),
        ..Default::default()
    });
    d.sources.push(Source {
        tag: "Both".into(),
        author: "Rivera, Alex".into(),
        corporate: "Harbor Guild, Inc.".into(),
        title: "Shared Spaces".into(),
        ..Default::default()
    });
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    let mut item_xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("customXml/item1.xml").unwrap(), &mut item_xml).unwrap();
    assert!(item_xml.contains("<b:Author><b:Author><b:Corporate>World Health Organization</b:Corporate></b:Author><b:Editor>"), "{item_xml}");
    assert!(item_xml.contains("<b:Corporate>Harbor Guild, Inc.</b:Corporate>"), "{item_xml}");
    assert!(!item_xml.contains("<b:Last>Rivera</b:Last>"), "the corporate author takes the person authors' place: {item_xml}");
    let back = wordcraft_docx::read(&bytes).expect("read");
    let got: Vec<(&str, &str, &str)> = back.sources.iter().map(|s| (s.corporate.as_str(), s.author.as_str(), s.editor.as_str())).collect();
    assert_eq!(got, [("World Health Organization", "", "Chadha, Shelly"), ("Harbor Guild, Inc.", "", "")]);
}

fn assert_bibliography_parts(bytes: &[u8]) {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert!(zip.by_name("customXml/item1.xml").is_ok());
    assert!(zip.by_name("customXml/itemProps1.xml").is_ok());
    assert!(zip.by_name("customXml/_rels/item1.xml.rels").is_ok());

    let mut rels_xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("_rels/.rels").unwrap(), &mut rels_xml).unwrap();
    assert!(rels_xml.contains("relationships/customXml"), "{rels_xml}");
    assert!(rels_xml.contains("Target=\"customXml/item1.xml\""), "{rels_xml}");

    let mut ct_xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("[Content_Types].xml").unwrap(), &mut ct_xml).unwrap();
    assert!(ct_xml.contains("PartName=\"/customXml/item1.xml\""), "{ct_xml}");
    assert!(ct_xml.contains("PartName=\"/customXml/itemProps1.xml\""), "{ct_xml}");

    let mut item_xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("customXml/item1.xml").unwrap(), &mut item_xml).unwrap();
    assert_bibliography_types(&item_xml);
    assert_bibliography_people(&item_xml);
    assert_bibliography_details(&item_xml);
}

fn assert_bibliography_types(item_xml: &str) {
    assert!(item_xml.contains("SelectedStyle=\"\\APA.XSL\""), "{item_xml}");
    assert!(item_xml.contains("StyleName=\"APA\""), "{item_xml}");
    assert!(item_xml.contains("<b:SourceType>Book</b:SourceType>"), "{item_xml}");
    assert!(item_xml.contains("<b:SourceType>JournalArticle</b:SourceType>"), "{item_xml}");
    assert!(item_xml.contains("<b:SourceType>InternetSite</b:SourceType>"), "{item_xml}");
    assert!(item_xml.contains("<b:SourceType>BookSection</b:SourceType>"), "{item_xml}");
    assert!(item_xml.contains("<b:SourceType>Misc</b:SourceType>"), "{item_xml}");
}

fn assert_bibliography_people(item_xml: &str) {
    assert!(item_xml.contains("<b:Author><b:Author><b:NameList><b:Person>"), "{item_xml}");
    assert!(item_xml.contains("<b:Last>Rivera</b:Last><b:First>Alex</b:First>"), "{item_xml}");
    assert!(item_xml.contains("<b:Last>Smith</b:Last><b:First>John</b:First><b:Middle>David</b:Middle>"), "{item_xml}");
    assert!(item_xml.contains("<b:Last>Solo</b:Last>"), "{item_xml}");
    assert!(item_xml.contains("<b:Editor>"), "{item_xml}");
    assert!(item_xml.contains("<b:Last>Johnson</b:Last><b:First>Robert</b:First>"), "{item_xml}");
}

fn assert_bibliography_details(item_xml: &str) {
    assert!(item_xml.contains("<b:Edition>2nd</b:Edition>"), "{item_xml}");
    assert!(item_xml.contains("<b:Month>10</b:Month>"), "{item_xml}");
    assert!(item_xml.contains("<b:Day>15</b:Day>"), "{item_xml}");
    assert!(item_xml.contains("<b:Issue>4</b:Issue>"), "{item_xml}");
    assert!(item_xml.contains("<b:DOI>10.1000/182</b:DOI>"), "{item_xml}");
    assert!(item_xml.contains("<b:YearAccessed>2024</b:YearAccessed>"), "{item_xml}");
    assert!(item_xml.contains("<b:MonthAccessed>05</b:MonthAccessed>"), "{item_xml}");
    assert!(item_xml.contains("<b:DayAccessed>15</b:DayAccessed>"), "{item_xml}");
    assert!(item_xml.contains("<b:Accessed>2024-05-15</b:Accessed>"), "{item_xml}");
}

fn assert_bibliography_sources_round_trip(bytes: &[u8]) {
    let back = wordcraft_docx::read(bytes).expect("read");
    assert_eq!(back.sources.len(), 5);
    assert_bibliography_source_0(&back.sources[0]);
    assert_bibliography_source_1(&back.sources[1]);
    assert_bibliography_source_2(&back.sources[2]);
    assert_bibliography_source_3(&back.sources[3]);
    assert_bibliography_source_4(&back.sources[4]);
}

fn assert_bibliography_source_0(source: &Source) {
    assert_eq!(source.tag, "Rivera2021");
    assert_eq!(source.kind, "book");
    assert_eq!(source.author, "Rivera, Alex; Chen, Mei");
    assert_eq!(source.title, "Shared Spaces");
    assert_eq!(source.year, "2021");
    assert_eq!(source.publisher, "Harbor Books");
    assert_eq!(source.city, "Portland");
    assert_eq!(source.editor, "Johnson, Robert");
    assert_eq!(source.edition, "2nd");
    assert_eq!(source.month, "10");
    assert_eq!(source.day, "15");
}

fn assert_bibliography_source_1(source: &Source) {
    assert_eq!(source.tag, "Smith2023");
    assert_eq!(source.kind, "article");
    assert_eq!(source.author, "Smith, John David");
    assert_eq!(source.title, "Modern Typography");
    assert_eq!(source.journal, "Design Quarterly");
    assert_eq!(source.volume, "12");
    assert_eq!(source.pages, "45-60");
    assert_eq!(source.url, "https://example.com/typo");
    assert_eq!(source.issue, "4");
    assert_eq!(source.doi, "10.1000/182");
}

fn assert_bibliography_source_2(source: &Source) {
    assert_eq!(source.tag, "WebSite2024");
    assert_eq!(source.kind, "website");
    assert_eq!(source.author, "Solo");
    assert_eq!(source.title, "Open Fonts");
    assert_eq!(source.url, "https://example.org");
    assert_eq!(source.accessed, "2024-05-15");
}

fn assert_bibliography_source_3(source: &Source) {
    assert_eq!(source.tag, "Chapter2025");
    assert_eq!(source.kind, "bookSection");
    assert_eq!(source.author, "Lee, Harper");
    assert_eq!(source.title, "Chapter One");
    assert_eq!(source.journal, "Anthology of Prose");
    assert_eq!(source.pages, "1-20");
}

fn assert_bibliography_source_4(source: &Source) {
    assert_eq!(source.tag, "Misc2026");
    assert_eq!(source.kind, "other");
    assert_eq!(source.title, "Archive Manuscript");
    assert_eq!(source.city, "Oxford");
}

#[test]
fn wrap_polygons_round_trip() {
    use wordcraft_doc::wrap::{WrapPolygon, shape_outline};
    let mut d = Document::new();
    let shape = |kind, float| InlineObject::Shape {
        kind,
        w: 80.0,
        h: 40.0,
        fill: Some(Fill::solid(Rgb(1, 2, 3))),
        stroke: None,
        stroke_width: 0.0,
        float,
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    // A hand-edited polygon, including points outside the frame.
    let poly = WrapPolygon::new(true, [(-300, 0), (21_600, 1200), (18_000, 21_900), (0, 21_600)]).map(Arc::new);
    let edited = Float { wrap: Wrap::Tight, v_rel: Anchor::Paragraph, wrap_polygon: poly.clone(), ..Default::default() };
    // None: Through wrapping needs one, so the ellipse's own outline is written.
    let derived = Float { wrap: Wrap::Through, ..Default::default() };
    let mut p = Paragraph::with_text("wrapped ", CharProps::default());
    for o in [shape(ShapeKind::Rectangle, edited.clone()), shape(ShapeKind::Ellipse, derived)] {
        let end = p.len();
        p.insert_object(end, o, &CharProps::default()).unwrap();
    }
    d.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"<wp:wrapPolygon edited="1"><wp:start x="-300" y="0"/><wp:lineTo x="21600" y="1200"/>"#), "{xml}");
    assert!(xml.contains(r#"<wp:lineTo x="-300" y="0"/></wp:wrapPolygon>"#), "closed: {xml}");
    let r = unstacked(wordcraft_docx::read(&bytes).expect("read"));
    let got = paras(&r)[0];
    let floats: Vec<Float> = got.objects.iter().filter_map(|o| o.frame().map(|(_, _, f)| f.clone())).collect();
    assert_eq!(floats[0], edited);
    let ellipse = WrapPolygon::from_unit(false, &shape_outline(ShapeKind::Ellipse, 80.0, 40.0, None).unwrap());
    assert_eq!(floats[1].wrap, Wrap::Through);
    assert_eq!(floats[1].wrap_polygon.as_deref(), ellipse.as_ref());
}

/// Enclosed characters (#297) are written as Word stores them — an `eq \o\ac(○,字)` field with no
/// result, the symbol and the text in runs of their own sizes — and read back with their style,
/// whether the field's size is set on the run or comes from the styles.
#[test]
fn enclosed_characters_round_trip_in_words_structure() {
    use wordcraft_doc::eq::{EncloseShape, EncloseStyle, Enclosure};
    let mut p = Paragraph::new();
    let cases = [
        (EncloseShape::Circle, EncloseStyle::Shrink, Some(11.0)),
        (EncloseShape::Triangle, EncloseStyle::Enlarge, Some(11.0)),
        (EncloseShape::Square, EncloseStyle::Shrink, None),
        (EncloseShape::Diamond, EncloseStyle::Enlarge, None),
    ];
    for (shape, style, size) in cases {
        let e = Enclosure { shape, style, text: "字".into() };
        let off = p.len();
        p.insert_object(
            off,
            InlineObject::Field { instr: e.instr(), result: "字".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps { size, ..Default::default() },
        )
        .unwrap();
    }
    let doc = doc_with(vec![p]);
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"<w:instrText xml:space="preserve"> eq \o\ac(○</w:instrText>"#), "{xml}");
    assert!(!xml.contains("separate") && !xml.contains("enlarge"), "{xml}");
    // Shrink text at 11 pt: the character at 7.5 pt (half-points 15), raised.
    assert!(xml.contains(r#"<w:sz w:val="15"/>"#), "{xml}");
    let back = wordcraft_docx::read(&bytes).unwrap();
    let got: Vec<Enclosure> = paras(&back)[0]
        .objects
        .iter()
        .filter_map(|o| if let InlineObject::Field { instr, result, .. } = o { Enclosure::parse(instr).filter(|_| result == "字") } else { None })
        .collect();
    let want: Vec<Enclosure> = cases.iter().map(|(shape, style, _)| Enclosure { shape: *shape, style: *style, text: "字".into() }).collect();
    assert_eq!(got, want);
}

#[test]
fn ruby_phonetic_guide_round_trips_as_w_ruby() {
    use wordcraft_doc::para::RubyAlign;
    let mut p = Paragraph::with_text("Read  daily.", CharProps::default());
    let bold = CharProps { bold: Some(true), ..Default::default() };
    let ruby = InlineObject::Ruby {
        base: "漢字".into(),
        ruby: "かんじ".into(),
        align: RubyAlign::DistributeSpace,
        size: 5.5,
        raise: 10.0,
        base_size: 11.0,
        lang: "ja-JP".into(),
        props: Box::new(CharProps { font: Some("Noto Sans CJK JP".into()), ..Default::default() }),
    };
    p.insert_object(5, ruby.clone(), &bold).unwrap();
    let d = doc_with(vec![p]);
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    for part in [
        r#"<w:rubyAlign w:val="distributeSpace"/><w:hps w:val="11"/><w:hpsRaise w:val="20"/><w:hpsBaseText w:val="22"/><w:lid w:val="ja-JP"/>"#,
        "<w:rt><w:r>",
        ">かんじ</w:t>",
        "<w:rubyBase><w:r><w:rPr><w:b/>",
        ">漢字</w:t>",
    ] {
        assert!(xml.contains(part), "missing {part}: {xml}");
    }
    let r = wordcraft_docx::read(&bytes).expect("read");
    let q = paras(&r)[0];
    assert_eq!(q.plain_text(), "Read 漢字 daily.");
    assert_eq!(q.objects, vec![ruby]);
    assert_eq!(q.props_of_char(5).bold, Some(true));
}

/// #354: the sides text wraps on (`@wrapText`) are read and written for Square, Tight and
/// Through; both sides is the default.
#[test]
fn wrap_text_sides_round_trip() {
    use wordcraft_doc::para::WrapText;
    let mut d = Document::new();
    let shape = |float| InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 80.0,
        h: 40.0,
        fill: Some(Fill::solid(Rgb(1, 2, 3))),
        stroke: None,
        stroke_width: 0.0,
        float,
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let sides =
        [(Wrap::Square, WrapText::Left), (Wrap::Tight, WrapText::Right), (Wrap::Through, WrapText::Largest), (Wrap::Square, WrapText::BothSides)];
    let mut p = Paragraph::with_text("wrapped ", CharProps::default());
    for (wrap, side) in sides {
        let end = p.len();
        p.insert_object(end, shape(Float { wrap, wrap_text: side, v_rel: Anchor::Paragraph, ..Default::default() }), &CharProps::default()).unwrap();
    }
    d.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    for tag in [
        r#"<wp:wrapSquare wrapText="left"/>"#,
        r#"<wp:wrapTight wrapText="right">"#,
        r#"<wp:wrapThrough wrapText="largest">"#,
        r#"<wp:wrapSquare wrapText="bothSides"/>"#,
    ] {
        assert!(xml.contains(tag), "{tag}: {xml}");
    }
    let r = wordcraft_docx::read(&bytes).expect("read");
    let got: Vec<_> = paras(&r)[0].objects.iter().filter_map(|o| o.frame().map(|(_, _, f)| (f.wrap, f.wrap_text))).collect();
    assert_eq!(got, sides);
    // A value outside the schema is no side (read as both sides).
    assert_eq!(WrapText::from_ooxml("middle"), None);
}

#[test]
fn alt_text_title_and_decorative_round_trip_for_every_object() {
    use wordcraft_doc::para::AltText;
    let mut d = Document::new();
    let key = d.add_media(tiny_png(), "png");
    let pic = InlineObject::Image {
        media: key,
        w: 100.0,
        h: 50.0,
        alt: "A kiln at dusk".into(),
        float: Float { alt: AltText { title: "Kiln".into(), ..Default::default() }, ..Default::default() },
        crop: [0.0; 4],
        ole: None,
    };
    let shape = |alt: AltText| InlineObject::Shape {
        kind: ShapeKind::Ellipse,
        w: 80.0,
        h: 40.0,
        fill: Some(Fill::solid(Rgb(1, 2, 3))),
        stroke: None,
        stroke_width: 0.0,
        float: Float { alt, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let described = shape(AltText { text: "Process flow & \"steps\"".into(), ..Default::default() });
    let decorative = shape(AltText { decorative: true, ..Default::default() });
    let group = InlineObject::Group {
        w: 120.0,
        h: 60.0,
        float: Float { alt: AltText { text: "Two circles".into(), ..Default::default() }, ..Default::default() },
        ch_w: 120.0,
        ch_h: 60.0,
        children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: shape(AltText { text: "Left circle".into(), ..Default::default() }) }],
        canvas: None,
    };
    let mut p = Paragraph::with_text("objects ", CharProps::default());
    for o in [pic, described, decorative, group] {
        let end = p.len();
        p.insert_object(end, o, &CharProps::default()).unwrap();
    }
    let mut t = Table::new(2, 2, 300.0);
    t.props.caption = Some("Prices".into());
    t.props.description = Some("Prices by year".into());
    d.body = vec![para_block(p), Arc::new(Block::Table(t))];
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"{C183D7F6-B498-43B3-948B-1728B52AA6E4}"><adec:decorative xmlns:adec="http://schemas.microsoft.com/office/drawing/2017/decorative" val="1"/>"#), "{xml}");
    assert!(
        xml.contains(r#"<w:tblLook "#) && xml.contains(r#"<w:tblCaption w:val="Prices"/><w:tblDescription w:val="Prices by year"/></w:tblPr>"#),
        "{xml}"
    );
    let r = wordcraft_docx::read(&bytes).expect("read");
    let objs = &paras(&r)[0].objects;
    let alts: Vec<_> =
        objs.iter().map(|o| (o.alt_text().to_string(), o.frame().map(|f| f.2.alt.title.clone()).unwrap_or_default(), o.is_decorative())).collect();
    assert_eq!(
        alts,
        [
            ("A kiln at dusk".into(), "Kiln".into(), false),
            ("Process flow & \"steps\"".into(), String::new(), false),
            (String::new(), String::new(), true),
            ("Two circles".into(), String::new(), false),
        ]
    );
    let InlineObject::Group { children, .. } = &objs[3] else { panic!("{objs:?}") };
    assert_eq!(children[0].obj.alt_text(), "Left circle");
    let Some(Block::Table(t)) = r.body.iter().map(|b| &**b).find(|b| matches!(b, Block::Table(_))) else { panic!() };
    assert_eq!((t.props.caption.as_deref(), t.props.description.as_deref()), (Some("Prices"), Some("Prices by year")));
}

/// WordArt's warp (`a:prstTxWarp` with its adjust value) and connectors glued to shapes
/// (`a:stCxn`/`a:endCxn`, with arrowheads) come back as saved; a shape no connector is glued to
/// loses its drawing id, and hostile values are reduced.
#[test]
fn warps_and_glued_connectors_round_trip() {
    use wordcraft_doc::connector::{ConnEnd, ShapeExtra};
    use wordcraft_doc::wordart::TextWarp;
    let mut d = Document::new();
    let story = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Bent", CharProps::default()))]);
    let float = |x: f32| Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x, y: 10.0, ..Default::default() };
    let shape = |kind, story, extra| InlineObject::Shape {
        kind,
        w: 72.0,
        h: 36.0,
        fill: None,
        stroke: Some(Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: float(0.0),
        story,
        freeform: None,
        effects: Default::default(),
        extra,
    };
    let mut warp = TextWarp::new("textArchUp").unwrap();
    warp.set_adj("adj", 9_000_000);
    let art = shape(ShapeKind::TextBox, Some(story), ShapeExtra { id: 5, warp: Some(warp), ..Default::default() });
    let target = shape(ShapeKind::Ellipse, None, ShapeExtra { id: 7, ..Default::default() });
    let lonely = shape(ShapeKind::Rectangle, None, ShapeExtra { id: 9, ..Default::default() });
    let link = ShapeExtra { start: Some(ConnEnd { id: 5, site: 3 }), end: Some(ConnEnd { id: 7, site: 2 }), arrow_end: true, ..Default::default() };
    let mut conn = shape(ShapeKind::ElbowConnector, None, link);
    if let InlineObject::Shape { float: f, .. } = &mut conn {
        *f = Float { flip_v: true, ..float(80.0) };
    }
    let mut p = Paragraph::with_text("", CharProps::default());
    for o in [art.clone(), conn.clone(), target.clone(), lonely] {
        let end = p.len();
        p.insert_object(end, o, &CharProps::default()).unwrap();
    }
    d.body = vec![para_block(p)];
    let r = rt(&d);
    let got = &paras(&r)[0].objects;
    // The glued-to shapes keep their ids, the connector its ends, arrowhead and flip.
    let extra = |o: &InlineObject| match o {
        InlineObject::Shape { extra, .. } => extra.clone(),
        _ => panic!("{o:?}"),
    };
    assert_eq!(extra(&got[0]), extra(&art));
    assert_eq!(extra(&got[1]), extra(&conn));
    assert!(matches!(&got[1], InlineObject::Shape { kind: ShapeKind::ElbowConnector, float, .. } if float.flip_v));
    assert_eq!(extra(&got[2]), extra(&target));
    assert_eq!(extra(&got[3]).id, 0, "nothing is glued to it");

    // Hostile: an unknown warp, a huge adjust value, a glue to a missing shape.
    let xml = wordcraft_docx::write(&r).unwrap();
    let mut pkg = zip::ZipArchive::new(std::io::Cursor::new(xml)).unwrap();
    let mut doc_xml = String::new();
    std::io::Read::read_to_string(&mut pkg.by_name("word/document.xml").unwrap(), &mut doc_xml).unwrap();
    let doc_xml = doc_xml
        .replace("prst=\"textArchUp\"", "prst=\"textNotAWarp\"")
        .replace("a:stCxn id=\"5\" idx=\"3\"", "a:stCxn id=\"404\" idx=\"99999999999\"");
    let bytes = replace_part(&wordcraft_docx::write(&r).unwrap(), "word/document.xml", doc_xml.as_bytes());
    let h = wordcraft_docx::read(&bytes).unwrap();
    let got = &paras(&h)[0].objects;
    assert_eq!(extra(&got[0]).warp, None);
    assert_eq!(extra(&got[1]).start, Some(ConnEnd { id: 404, site: wordcraft_doc::connector::MAX_SITE }));
}

/// `zip` with part `name` replaced by `data`.
fn replace_part(zip_bytes: &[u8], name: &str, data: &[u8]) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let n = f.name().to_string();
        out.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        if n == name {
            std::io::Write::write_all(&mut out, data).unwrap();
        } else {
            std::io::copy(&mut f, &mut out).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

/// A texture or picture page background (`v:background/v:fill`) survives a save (#999): the image
/// part, its relationship and the fill attributes.
#[test]
fn page_background_picture_round_trips() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 150, 100, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut d = Document::new();
    d.settings.page_color = Some(Rgb(0xFF, 0xFF, 0xCC));
    d.media.insert("paper.png".into(), Arc::new(png.clone()));
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "paper.png".into(), fill_type: "tile".into(), title: "Paper".into() });
    let r = rt(&d);
    let pic = r.settings.page_picture.as_ref().expect("picture kept");
    assert_eq!((pic.fill_type.as_str(), pic.title.as_str()), ("tile", "Paper"));
    assert_eq!(r.media.get(&pic.media).map(|b| b.as_slice()), Some(png.as_slice()));
    assert_eq!(r.settings.page_color, d.settings.page_color);
    // Saved again, it is still there.
    assert!(rt(&r).settings.page_picture.is_some());
}

/// #2601: Drop Cap Options — a dropped cap with its lines, distance from the text and font, and
/// one in the margin, survive a save: `w:framePr w:dropCap="drop|margin" w:lines w:hSpace` on the
/// letter's own paragraph, the font on the letter's run (ECMA-376 §17.3.1.11). The text after the
/// letter keeps its own font.
#[test]
fn drop_cap_options_round_trip() {
    let body = CharProps { font: Some("Liberation Serif".into()), ..Default::default() };
    let mut dropped = Paragraph::with_text("Once upon a time", body.clone());
    dropped.props.drop_cap = Some(2);
    dropped.props.drop_cap_distance = Some(6.0);
    dropped.props.drop_cap_font = Some("Liberation Mono".into());
    let mut margin = Paragraph::with_text("In the margin", body.clone());
    margin.props.drop_cap = Some(4);
    margin.props.drop_cap_margin = Some(true);
    let d = doc_with(vec![dropped, margin]);
    let bytes = wordcraft_docx::write(&d).expect("write");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains(r#"w:dropCap="drop" w:lines="2" w:hSpace="120""#), "{xml}");
    assert!(xml.contains(r#"w:dropCap="margin" w:lines="4""#), "{xml}");
    assert!(xml.contains(r#"w:ascii="Liberation Mono""#), "the letter's font on its run: {xml}");
    let r = wordcraft_docx::read(&bytes).expect("read");
    let got = paras(&r);
    assert_eq!(got.len(), 2, "each letter merges back into its paragraph");
    let (a, b) = (&got[0], &got[1]);
    assert_eq!(a.plain_text(), "Once upon a time");
    assert_eq!(
        (a.props.drop_cap, a.props.drop_cap_margin, a.props.drop_cap_distance, a.props.drop_cap_font.as_deref()),
        (Some(2), None, Some(6.0), Some("Liberation Mono"))
    );
    assert!(a.runs.iter().all(|r| r.props.font.as_deref() == Some("Liberation Serif")), "the letter keeps the text's font: {:?}", a.runs);
    assert_eq!(b.plain_text(), "In the margin");
    assert_eq!(
        (b.props.drop_cap, b.props.drop_cap_margin, b.props.drop_cap_distance, b.props.drop_cap_font.as_deref()),
        (Some(4), Some(true), None, None)
    );
}
