use super::*;

#[test]
fn numeric_entity_rejects_sign() {
    assert_eq!(unescape("a&#+65;b"), "a&#+65;b");
    assert_eq!(unescape("a&#x+41;b"), "a&#x+41;b");
    assert_eq!(unescape("a&#65;&#x41;b"), "aAAb");
}

fn texts(f: &Flow) -> Vec<String> {
    f.blocks
        .iter()
        .map(|b| match b {
            FBlock::Para(p) => format!("{:?}{:?}:{}", p.kind, p.list.map(|l| (l.ordered, l.level)), p.text()),
            FBlock::Table(t) => format!("table{}x{}", t.rows.len(), t.cols()),
        })
        .collect()
}

fn table_style(html: &str) -> Option<String> {
    let doc = crate::model::to_doc(&parse(html));
    doc.body.iter().find_map(|b| match &**b {
        wordcraft_doc::Block::Table(t) => Some(t.props.style.clone().unwrap_or_default()),
        _ => None,
    })
}

#[test]
fn borderless_tables_get_no_grid_style() {
    assert_eq!(table_style("<table><tr><td>a<td>b</table>").as_deref(), Some(""));
    assert_eq!(table_style("<table style='border:none'><tr><td style='border:0'>a</table>").as_deref(), Some(""));
    assert_eq!(table_style("<table border=0><tr><td style='border: 0px none'>a</table>").as_deref(), Some(""));
    assert_eq!(table_style("<table border=1><tr><td>a</table>").as_deref(), Some("TableGrid"));
    assert_eq!(table_style("<table style='border:1px solid #000'><tr><td>a</table>").as_deref(), Some("TableGrid"));
    assert_eq!(table_style("<table><tr><td style='border:1px solid #999'>a</table>").as_deref(), Some("TableGrid"));
}

#[test]
fn borderless_tables_export_without_cell_borders() {
    // #438: a table without grid style and borders writes no cell borders and reopens borderless.
    for (src, grid) in [
        ("<table style='border:none'><tr><td>A<td>B</table>", false),
        ("<table><tr><td>A<td>B</table>", false),
        ("<table border=1><tr><td>A<td>B</table>", true),
    ] {
        let out = export(&crate::model::to_doc(&parse(src)));
        assert_eq!(out.contains("border:1px"), grid, "{src}: {out}");
        assert_eq!(table_style(&out).as_deref(), Some(if grid { "TableGrid" } else { "" }), "{src}");
        assert!(out.contains(">A</td>") && out.contains(">B</td>"), "{out}");
    }
}

#[test]
fn input_newlines_preserve_preformatted_lines() {
    for newline in ["\n", "\r\n", "\r"] {
        let f = parse(&format!("<pre>{newline}  Alpha{newline}  <b>Beta</b>{newline}{newline}  Gamma</pre>"));
        assert_eq!(texts(&f), ["CodeNone:  Alpha", "CodeNone:  Beta", "CodeNone:", "CodeNone:  Gamma"], "{newline:?}");
        let FBlock::Para(p) = &f.blocks[1] else { panic!() };
        assert!(matches!(p.inlines.last(), Some(Inline::Text(t, f)) if t == "Beta" && f.bold));
    }
}

#[test]
fn input_newlines_are_normalized_before_character_references() {
    let f = parse("<p style='white-space:pre'>A&#13;B\r\nC\rD</p><p>A\r\nB\rC</p>");
    assert_eq!(texts(&f), ["NormalNone:A\rB\nC\nD", "NormalNone:A B C"]);
    let f = parse("<p style='white-space:pre-wrap'>α\r\r\nβ&#13;γ</p><a href='https://example.test/a\r\nb\rc&#13;d'>link</a>");
    assert_eq!(texts(&f), ["NormalNone:α\n\nβ\rγ", "NormalNone:link"]);
    let FBlock::Para(p) = &f.blocks[1] else { panic!() };
    assert!(matches!(p.inlines.first(), Some(Inline::Text(_, f)) if f.link.as_deref() == Some("https://example.test/a\nb\nc\rd")));
}

#[test]
fn css_preformatted_paragraphs_keep_edge_spaces() {
    // #439: `white-space:pre` keeps leading and trailing spaces like `<pre>`; normal
    // paragraphs still collapse.
    let f = parse(
        "<p style=\"white-space:pre\">  Alpha  Beta  </p><p style=\"white-space:pre-wrap\">\n  Gamma  \n</p><pre>  Alpha  </pre><p>  Alpha  Beta  </p>",
    );
    assert_eq!(texts(&f), vec!["NormalNone:  Alpha  Beta  ", "NormalNone:  Gamma  ", "CodeNone:  Alpha  ", "NormalNone:Alpha Beta"]);
}

#[test]
fn tag_soup() {
    let f = parse(
        "<html><head><title>T &amp; U</title><style>p{}</style><script>if(a<b)x</script></head><body><h1>Head</h1><p>One <b>bold<p>Two <i>it</i>\n  text<ul><li>a<li>b<ol><li>c</ol></ul><table><tr><td>x<td colspan=2>y<tr><th>z</table><blockquote>q</blockquote><pre>l1\nl2</pre>",
    );
    assert_eq!(f.meta.title, "T & U");
    assert_eq!(
        texts(&f),
        vec![
            "Heading(1)None:Head",
            "NormalNone:One bold",
            "NormalNone:Two it text",
            "NormalSome((false, 0)):a",
            "NormalSome((false, 0)):b",
            "NormalSome((true, 1)):c",
            "table2x3",
            "QuoteNone:q",
            "CodeNone:l1",
            "CodeNone:l2",
        ]
    );
    let FBlock::Para(p) = &f.blocks[1] else { panic!() };
    assert!(matches!(&p.inlines[1], Inline::Text(t, f) if t == "bold" && f.bold));
}

#[test]
fn omitted_head_keeps_body_text() {
    let f = parse("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>T</title>\n<body><p>Hello world</p><p>Second</p></body></html>");
    assert_eq!(f.meta.title, "T");
    assert_eq!(texts(&f), vec!["NormalNone:Hello world", "NormalNone:Second"]);

    for body in [
        "<p>Hello world</p>",
        "<div>Hello world</div>",
        "<span>Hello world</span>",
        "<a href='https://example.test'>Hello world</a>",
        "<custom>Hello world</custom>",
        "<textarea>ignored</textarea>Hello world",
    ] {
        let f = parse(&format!(
            "<html><head><title>T &amp; U</title><meta name='author' content='Ann'><style>p{{}}</style><script>ignored</script>{body}",
        ));
        assert_eq!(f.meta.title, "T & U");
        assert_eq!(f.meta.author, "Ann");
        assert_eq!(texts(&f), vec!["NormalNone:Hello world"], "{body}");
    }
}

#[test]
fn styles_and_links() {
    let f = parse("<p style='text-align:center'><span style=\"color:#ff0000;font-weight:700\">red</span> <a href=\"http://x\">l</a><br>n</p>");
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    assert_eq!(p.align, Some(Align::Center));
    assert!(matches!(&p.inlines[0], Inline::Text(t, f) if t == "red" && f.bold && f.color == Some(wordcraft_doc::Rgb(255,0,0))));
    assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "l" && f.link.as_deref() == Some("http://x"))));
    assert!(p.text().ends_with("l\nn"));
}

/// The paragraphs of every cell of the first table, row by row.
fn cell_paras(f: &Flow) -> Vec<Vec<Vec<Para>>> {
    let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
    t.rows
        .iter()
        .map(|r| r.iter().map(|c| c.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.clone()) } else { None }).collect()).collect())
        .collect()
}

fn all_bold(p: &Para) -> bool {
    !p.inlines.is_empty() && p.inlines.iter().all(|i| matches!(i, Inline::Text(_, f) if f.bold))
}

fn any_bold(p: &Para) -> bool {
    p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) if f.bold))
}

#[test]
fn header_cells_are_bold_unless_their_css_says_otherwise() {
    // #1381: a <th> is bold, as in browsers and Word; `font-weight:normal` on it or inside it wins.
    let f = parse(
        "<table><thead><tr><th>Head</th><th style='font-weight:normal'>Plain</th><th><span style='font-weight:400'>Inner</span></th></tr></thead><tbody><tr><td>a<td>b<td>c</tbody></table>",
    );
    let rows = cell_paras(&f);
    assert!(all_bold(&rows[0][0][0]), "{:?}", rows[0][0]);
    assert!(!any_bold(&rows[0][1][0]), "{:?}", rows[0][1]);
    assert!(!any_bold(&rows[0][2][0]), "{:?}", rows[0][2]);
    assert!(!any_bold(&rows[1][0][0]), "td stays regular");
}

#[test]
fn cell_text_align_reaches_the_cell_paragraphs() {
    // #1381: `text-align` on a td/th, or inherited from its tr/thead/tbody/table, aligns the
    // cell's paragraphs; a paragraph's own text-align inside the cell wins.
    let f = parse(
        "<table style='text-align:center'><tbody style='text-align:justify'><tr style='text-align:right'>\
             <td style='text-align:right;'>r</td><td>inherited</td><td style='text-align:right'><p style='text-align:center'>own</p></td>\
             <td align=right>attr</td></tr><tr><td>body</td></tr></tbody><tr><td>table</td></tr></table>",
    );
    let rows = cell_paras(&f);
    assert_eq!(rows[0][0][0].align, Some(Align::Right), "td style");
    assert_eq!(rows[0][1][0].align, Some(Align::Right), "from tr");
    assert_eq!(rows[0][2][0].align, Some(Align::Center), "own paragraph wins");
    assert_eq!(rows[0][3][0].align, Some(Align::Right), "align attribute");
    assert_eq!(rows[1][0][0].align, Some(Align::Justify), "from tbody");
    assert_eq!(rows[2][0][0].align, Some(Align::Center), "from table");
    // `align` on a table places the table; it doesn't align the text in its cells.
    let f = parse("<table align=center><tr><td>x</td></tr></table>");
    assert_eq!(cell_paras(&f)[0][0][0].align, None);
}

#[test]
fn reporter_sample_header_bold_and_right_aligned_cells() {
    // #1381: the reporter's table.
    let f = parse(
        "<table border=\"1\" style=\"width:100%;border-collapse:collapse;\">\n<thead><tr style=\"background-color:#e8f0fa;\"><th>Indicatore</th><th>Valore atteso</th></tr></thead>\n<tbody>\n<tr><td>Audit</td><td style=\"text-align:right;\">4</td></tr>\n</tbody></table>",
    );
    let rows = cell_paras(&f);
    assert!(all_bold(&rows[0][0][0]) && all_bold(&rows[0][1][0]), "{:?}", rows[0]);
    assert_eq!(rows[1][1][0].align, Some(Align::Right));
    assert_eq!(rows[1][0][0].align, None);
}

#[test]
fn entities() {
    assert_eq!(unescape("a &lt;b&gt; &#65;&#x42; &bogus; &"), "a <b> AB &bogus; &");
    assert_eq!(unescape("&minus; &COPY; &EURO; &TRADE; &frac12; &frac34;"), "− © € ™ ½ ¾");
    assert_eq!(unescape("&theta; &lambda; &Delta; &nabla; &sum; &radic; &ne; &le; &ge; &hearts;"), "θ λ Δ ∇ ∑ √ ≠ ≤ ≥ ♥");
}

#[test]
fn wbr_word_break_opportunity() {
    let f = parse("<p>Super<wbr>cali<wbr>fragilistic</p>");
    let Some(FBlock::Para(p)) = f.blocks.first() else { panic!("expected para") };
    assert_eq!(p.text(), "Super\u{200B}cali\u{200B}fragilistic");
}

#[test]
fn page_break_after_and_before() {
    let f = parse("<p style='page-break-after: always'>First</p><p>Second</p>");
    let paras: Vec<_> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect();
    assert_eq!(paras.len(), 2);
    assert!(!paras[0].page_break);
    assert!(paras[1].page_break);

    let f2 = parse("<div style='break-after: page'><p>Part 1</p></div><p>Part 2</p>");
    let paras2: Vec<_> = f2.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect();
    assert_eq!(paras2.len(), 2);
    assert!(!paras2[0].page_break);
    assert!(paras2[1].page_break);
}

#[test]
fn font_size_relative_and_absolute() {
    let f = parse("<font size=\"+1\">larger</font><font size=\"-1\">smaller</font><font size=\"5\">huge</font>");
    let inls = match &f.blocks[0] {
        FBlock::Para(p) => &p.inlines,
        _ => panic!("expected para"),
    };
    let sizes: Vec<Option<f32>> = inls
        .iter()
        .map(|i| match i {
            Inline::Text(_, fmt) => fmt.size,
            _ => None,
        })
        .collect();
    assert_eq!(sizes, vec![Some(13.5), Some(10.0), Some(18.0)]);
}

#[test]
fn decode_with_charset_declarations() {
    let iso = b"<html><head><meta charset=\"iso-8859-1\"></head><body>\xE9l\xE8ve</body></html>";
    assert!(decode(iso).contains("élève"));

    let win = b"<html><head><meta charset='windows-1252'></head><body>\x80100</body></html>";
    assert!(decode(win).contains("€100"));

    let gbk = b"<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=gbk\"></head><body>\xC4\xE3\xBA\xC3</body></html>";
    assert!(decode(gbk).contains("你好"));

    let xml = b"<?xml version=\"1.0\" encoding=\"utf-8\"?><html><body>hello</body></html>";
    assert!(decode(xml).contains("hello"));
}

#[test]
fn raw_text_elements_skip_only_matching_end_tag() {
    let f = parse("<script>var a = \"</scripted>\"; var b = \"<b>malicious</b>\";</script><p>Hello</p>");
    assert_eq!(f.blocks.len(), 1);
    let p = match &f.blocks[0] {
        FBlock::Para(p) => p,
        b => panic!("expected para, got {b:?}"),
    };
    assert_eq!(p.text(), "Hello");

    let f = parse("<style>/* </styles> */ p { color: red; }</style><p>World</p>");
    assert_eq!(f.blocks.len(), 1);
    let p = match &f.blocks[0] {
        FBlock::Para(p) => p,
        b => panic!("expected para, got {b:?}"),
    };
    assert_eq!(p.text(), "World");
}

#[test]
fn leading_space_survives_round_trip() {
    let doc = Document::from_text(" lead");
    let html = export(&doc);
    assert!(!html.contains("<p> lead"), "{html}");
    let back = import(&html);
    assert_eq!(back.plain_text(Default::default()).replace('\u{a0}', " "), " lead");
}

/// A document that reads "I read " then a phonetic guide over `base`, then " daily.".
fn ruby_doc(base: &str, ruby: &str) -> Document {
    let props = wordcraft_doc::CharProps::default();
    let mut p = wordcraft_doc::Paragraph::with_text("I read ", props.clone());
    let n = p.len();
    p.insert_object(n, crate::model::ruby_object(base.into(), ruby.into(), 20.0), &props).unwrap();
    let n = p.len();
    p.insert_text(n, " daily.", &props).unwrap();
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(p)];
    d
}

/// The first flow ruby: its base text and reading.
fn ruby_of(f: &Flow) -> Option<(String, String)> {
    f.blocks.iter().find_map(|b| match b {
        FBlock::Para(p) => p.inlines.iter().find_map(|i| match i {
            Inline::Ruby { base, ruby, .. } => Some((base.clone(), ruby.clone())),
            _ => None,
        }),
        _ => None,
    })
}

/// The first document ruby: its base text and reading.
fn doc_ruby(d: &Document) -> Option<(String, String)> {
    d.body.iter().find_map(|b| match &**b {
        wordcraft_doc::Block::Para(p) => p.objects.iter().find_map(|o| match o {
            wordcraft_doc::InlineObject::Ruby { base, ruby, .. } => Some((base.clone(), ruby.clone())),
            _ => None,
        }),
        _ => None,
    })
}

/// The first document ruby: base size, ruby size, raise and alignment.
fn doc_ruby_attrs(d: &Document) -> Option<(f32, f32, f32, wordcraft_doc::para::RubyAlign)> {
    d.body.iter().find_map(|b| match &**b {
        wordcraft_doc::Block::Para(p) => p.objects.iter().find_map(|o| match o {
            wordcraft_doc::InlineObject::Ruby { base_size, size, raise, align, .. } => Some((*base_size, *size, *raise, *align)),
            _ => None,
        }),
        _ => None,
    })
}

#[test]
fn ruby_exports_as_ruby_markup() {
    // #1681: the base text with its reading in `<rt>`, not plain base text.
    let out = export(&ruby_doc("漢字", "かんじ"));
    assert!(out.contains("<p>I read <ruby>漢字<rt>かんじ</rt></ruby> daily.</p>"), "{out}");
    // The base and the reading are escaped like any other text.
    let out = export(&ruby_doc("A&B", "x<y"));
    assert!(out.contains("<ruby>A&amp;B<rt>x&lt;y</rt></ruby>"), "{out}");
}

#[test]
fn ruby_imports_with_its_surrounding_text() {
    let f = parse("<p>I read <ruby>漢字<rt>かんじ</rt></ruby> daily.</p>");
    assert_eq!(ruby_of(&f), Some(("漢字".into(), "かんじ".into())));
    assert_eq!(texts(&f), vec!["NormalNone:I read 漢字 daily."]);
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    assert!(matches!(&p.inlines[0], Inline::Text(t, _) if t == "I read "), "{:?}", p.inlines);
    assert!(matches!(p.inlines.last(), Some(Inline::Text(t, _)) if t == " daily."), "{:?}", p.inlines);
    // One document object, holding the base text and the reading.
    assert_eq!(doc_ruby(&crate::model::to_doc(&f)), Some(("漢字".into(), "かんじ".into())));
}

#[test]
fn ruby_rb_and_rp_fallbacks_import_as_one_guide() {
    // `<rb>` holds the base text and `<rp>` the fallback parentheses for browsers without
    // ruby: the parentheses are not text, and the base pieces join.
    let f = parse("<p><ruby><rb>漢</rb><rb>字</rb><rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby></p>");
    assert_eq!(ruby_of(&f), Some(("漢字".into(), "かんじ".into())));
    assert_eq!(texts(&f), vec!["NormalNone:漢字"]);
    assert_eq!(doc_ruby(&crate::model::to_doc(&f)), Some(("漢字".into(), "かんじ".into())));
}

#[test]
fn ruby_import_uses_the_phonetic_guide_defaults() {
    // HTML states no alignment, size or raise: the Phonetic Guide's own defaults over a
    // 20 pt run (half the base size, raised clear of the base text, centred)…
    let d = crate::html::import("<p style='font-size:20pt'><ruby>漢字<rt>かんじ</rt></ruby></p>");
    let Some((base_size, size, raise, align)) = doc_ruby_attrs(&d) else { panic!("no ruby") };
    assert_eq!((base_size, size, align.ooxml()), (20.0, 10.0, "center"));
    assert!((raise - 18.0).abs() < 0.001, "{raise}");
    // …and the guide's 10.5 pt fallback base size when the source states none.
    let d = crate::html::import("<p><ruby>漢字<rt>かんじ</rt></ruby></p>");
    let Some((base_size, size, raise, _)) = doc_ruby_attrs(&d) else { panic!("no ruby") };
    assert_eq!((base_size, size), (10.5, 5.0));
    assert!((raise - 9.4).abs() < 0.001, "{raise}");
}

#[test]
fn ruby_survives_an_html_round_trip() {
    let html = export(&ruby_doc("漢字", "かんじ"));
    assert_eq!(ruby_of(&parse(&html)), Some(("漢字".into(), "かんじ".into())));
    let back = crate::html::import(&html);
    assert_eq!(doc_ruby(&back), Some(("漢字".into(), "かんじ".into())));
    assert_eq!(back.plain_text(Default::default()), "I read 漢字 daily.");
}

#[test]
fn ruby_keeps_the_run_formatting() {
    let props = wordcraft_doc::CharProps { bold: Some(true), ..Default::default() };
    let mut p = wordcraft_doc::Paragraph::with_text("I read ", props.clone());
    let n = p.len();
    p.insert_object(n, crate::model::ruby_object("漢字".into(), "かんじ".into(), 20.0), &props).unwrap();
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(p)];
    assert!(export(&d).contains("<strong><ruby>漢字<rt>かんじ</rt></ruby></strong>"), "{}", export(&d));
    // Importing it back keeps the formatting for the base text and the reading.
    let f = parse("<p><strong><ruby>漢字<rt>かんじ</rt></ruby></strong></p>");
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    assert!(matches!(&p.inlines[0], Inline::Ruby { fmt, .. } if fmt.bold), "{:?}", p.inlines);
}

#[test]
fn malformed_ruby_keeps_its_text() {
    // No reading, an empty reading, a missing base, an unclosed element and a block boundary
    // closing it: none of them panics, and the text inside is kept.
    let cases: [(&str, &[&str]); 7] = [
        ("<p>a <ruby>漢字</ruby> b</p>", &["NormalNone:a 漢字 b"]),
        ("<p>a <ruby>漢字<rt></rt></ruby> b</p>", &["NormalNone:a 漢字 b"]),
        ("<p>a <ruby><rt>かんじ</rt></ruby> b</p>", &["NormalNone:a かんじ b"]),
        ("<p>a <ruby><rt></rt></ruby> b</p>", &["NormalNone:a b"]),
        ("<ruby>", &[]),
        ("<p>a <ruby>漢字<rt>かんじ</rt> b</p><p>next</p>", &["NormalNone:a 漢字 b", "NormalNone:next"]),
        ("<p>a <ruby>漢字<rt>かんじ</rt></p><p>next</p>", &["NormalNone:a 漢字", "NormalNone:next"]),
    ];
    for (src, want) in cases {
        assert_eq!(texts(&parse(src)), want, "{src}");
    }
    // Ruby in ruby: the inner element's text joins the outer guide's.
    assert_eq!(ruby_of(&parse("<p><ruby>漢字<ruby>字<rt>じ</rt></ruby><rt>かんじ</rt></ruby></p>")), Some(("漢字字".into(), "じかんじ".into())));
}

#[test]
fn small_big_and_css_font_sizes() {
    let f = parse(
        "<p><small>small</small> <big>big</big> <span style=\"font-size: xxx-large\">huge</span> <span style=\"font-size: smaller\">tiny</span></p>",
    );
    let FBlock::Para(p) = &f.blocks[0] else { panic!() };
    let sz = |i: usize| match &p.inlines[i] {
        Inline::Text(_, f) => f.size,
        _ => None,
    };
    assert_eq!(sz(0), Some(10.0));
    assert_eq!(sz(2), Some(14.0));
    assert_eq!(sz(4), Some(36.0));
    assert_eq!(sz(6), Some(10.0));
}

#[test]
fn css_important_casing_and_spacing() {
    let f = parse(
        "<p style='color: #ff0000 !IMPORTANT; font-size: 16pt ! important; font-weight: 700 !Important; text-align: center !IMPORTANT'>Styled</p>",
    );
    assert_eq!(f.blocks.len(), 1);
    let p = match &f.blocks[0] {
        FBlock::Para(p) => p,
        b => panic!("expected para, got {b:?}"),
    };
    assert_eq!(p.align, Some(Align::Center));
    match &p.inlines[0] {
        Inline::Text(_, fmt) => {
            assert_eq!(fmt.color, Some(wordcraft_doc::props::Rgb(0xFF, 0x00, 0x00)));
            assert_eq!(fmt.size, Some(16.0));
            assert!(fmt.bold);
        }
        i => panic!("expected text inline, got {i:?}"),
    }
}

#[test]
fn table_cell_vertical_alignment() {
    let f = parse("<table><tr><td valign=top>top</td><td valign=\"center\">mid</td><td style=\"vertical-align: bottom\">bot</td></tr></table>");
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.rows[0][0].valign, Some(VAlign::Top));
    assert_eq!(t.rows[0][1].valign, Some(VAlign::Center));
    assert_eq!(t.rows[0][2].valign, Some(VAlign::Bottom));
    let doc = crate::model::to_doc(&f);
    let wordcraft_doc::Block::Table(dt) = &*doc.body[0] else { panic!() };
    assert_eq!(dt.rows[0].cells[0].props.valign, Some(VAlign::Top));
    assert_eq!(dt.rows[0].cells[1].props.valign, Some(VAlign::Center));
    assert_eq!(dt.rows[0].cells[2].props.valign, Some(VAlign::Bottom));
    let html = export(&doc);
    assert!(html.contains("vertical-align:top"));
    assert!(html.contains("vertical-align:middle"));
    assert!(html.contains("vertical-align:bottom"));
}

#[test]
fn many_formatted_leading_breaks_trim_quickly() {
    // #3705: a paragraph opening with alternating bold and italic `<br>` was trimmed one
    // span at a time with `Vec::remove(0)`.
    let html = format!("<p>{}x</p>", "<b><br></b><i><br></i>".repeat(100_000));
    let t = std::time::Instant::now();
    let f = parse(&html);
    // About a second on a loaded machine; trimming one span at a time ran past 6 minutes.
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
    let texts: Vec<String> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.text()) } else { None }).collect();
    assert_eq!(texts, ["x"]);
}

#[test]
fn font_size_at_the_integer_limits_does_not_overflow() {
    // `<font size>` offsets were added to 3 unchecked, so the extreme values panicked in debug builds.
    for size in ["+2147483647", "--2147483648", "-2147483647", "+99999999999"] {
        let f = parse(&format!("<font size=\"{size}\">x</font>"));
        assert!(texts(&f).iter().any(|t| t.ends_with(":x")), "size={size}");
    }
    // The offsets saturate, then clamp to the 1..=7 scale: 7 is 36 pt, 1 is 7.5 pt.
    let size = |html: &str| match &parse(html).blocks[0] {
        FBlock::Para(p) => p.inlines.iter().find_map(|i| match i {
            Inline::Text(_, fmt) => fmt.size,
            _ => None,
        }),
        _ => None,
    };
    assert_eq!(size("<font size=\"+2147483647\">x</font>"), Some(36.0));
    assert_eq!(size("<font size=\"--2147483648\">x</font>"), Some(36.0));
    assert_eq!(size("<font size=\"-2147483647\">x</font>"), Some(7.5));
}

#[test]
fn empty_comments_do_not_swallow_the_rest() {
    for html in ["<p>a<!-->b</p><p>c</p>", "<p>a<!--->b</p><p>c</p>"] {
        let doc = crate::html::import(html);
        assert_eq!(doc.plain_text(Default::default()), "ab\nc", "{html}");
    }
}
