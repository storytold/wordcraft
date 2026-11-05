use super::*;

#[test]
fn numeric_entity_rejects_sign() {
    assert_eq!(unescape("a&#+65;b"), "a&#+65;b");
    assert_eq!(unescape("a&#x+41;b"), "a&#x+41;b");
    assert_eq!(unescape("a&#65;&#x41;b"), "aAAb");
}

#[test]
fn important_declarations_win_the_cascade() {
    let color = |html: &str| fmt_of(paras(&parse(html))[0]).color;
    let red = Some(wordcraft_doc::Rgb(255, 0, 0));
    let blue = Some(wordcraft_doc::Rgb(0, 0, 255));
    assert_eq!(color("<style>p { color: red }</style><p style='color: blue'>x</p>"), blue);
    assert_eq!(color("<style>p { color: red !important }</style><p style='color: blue'>x</p>"), red);
    assert_eq!(color("<style>p { color: red !important }</style><p style='color: blue !important'>x</p>"), blue);
    assert_eq!(color("<p style='color: red !important; color: blue'>x</p>"), red);
}

#[test]
fn a_declared_charset_decodes_the_page() {
    // ISO 8859-2 (Central European): 0xA9 Š, 0xB9 š; named by the caller or the page.
    assert_eq!(decode_as(&[0xA9, b' ', 0xB9], Some("iso-8859-2")), "Š š");
    assert!(decode_as(b"<meta charset=iso-8859-2><p>\xA9</p>", None).contains("<p>Š</p>"));
    assert_eq!(decode_as(&[0x8A], Some("windows-1252")), "Š");
    // A page that says UTF-16 without a BOM is read as UTF-8, as browsers do.
    assert_eq!(decode_as("é".as_bytes(), Some("utf-16")), "é");
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
    // The last declaration decides: a `style` attribute's over the style sheet's.
    assert_eq!(table_style("<style>table { border: 1px solid }</style><table><tr><td>a</table>").as_deref(), Some("TableGrid"));
    assert_eq!(table_style("<style>table { border: 1px solid }</style><table style='border:none'><tr><td>a</table>").as_deref(), Some(""));
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

#[test]
fn css_escapes_read_every_kind_of_newline() {
    // A backslash before CR LF, CR, LF or form feed continues the line; after hex digits, one
    // of them (CR LF as one) ends the escape.
    assert_eq!(css_unescape("A\\\r\nB\\\rC\\\nD\\\u{C}E"), "ABCDE");
    assert_eq!(css_unescape("\\41\r\nB \\42\u{C}C \\43 D"), "AB BC CD");
    assert_eq!(css_unescape("x\\'y\\"), "x'y");
}

#[test]
fn font_names_are_written_as_css_strings() {
    // A font name could end the CSS string or the declaration and add CSS of its own (#3739).
    let fonts = ["Arial\n;position:fixed", "Back\\slash", "O'Brien \"Sans\"", "A,B", "Times New Roman", "M+ 1p"];
    let mut p = Para::default();
    for (i, font) in fonts.iter().enumerate() {
        p.push_text(&format!("t{i}"), &Fmt { font: Some((*font).into()), ..Default::default() });
    }
    let html = export(&model::to_doc(&Flow { blocks: vec![FBlock::Para(p)], meta: Meta::default() }));
    assert!(html.contains("font-family:'Times New Roman'") && html.contains("font-family:'M+ 1p'"), "{html}");
    let styles: Vec<&str> = html.split(" style=\"").skip(1).filter_map(|s| s.split('"').next()).filter(|s| s.contains("font-family")).collect();
    assert_eq!(styles.len(), fonts.len(), "{html}");
    for s in styles {
        assert!(!s.contains([';', '\n']) && s.matches('\'').count() == 2, "{s}");
    }
    // Read back, each run has its font as it was.
    let back = parse(&html);
    let runs: Vec<(String, Option<String>)> = back
        .blocks
        .iter()
        .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
        .flat_map(|p| p.inlines.iter())
        .filter_map(|i| if let Inline::Text(t, f) = i { Some((t.clone(), f.font.as_deref().map(String::from))) } else { None })
        .collect();
    for (i, font) in fonts.iter().enumerate() {
        assert!(runs.contains(&(format!("t{i}"), Some(font.to_string()))), "{font:?} in {runs:?}");
    }
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

fn paras(f: &Flow) -> Vec<&Para> {
    f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
}
fn fmt_of(p: &Para) -> Fmt {
    p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap_or_default()
}

#[test]
fn style_sheet_rules_apply_in_cascade_order() {
    let f = parse(
        "<style><!-- p { color: #00ff00; font-size: 10pt } .big { font-size: 20pt } p.big { font-weight: bold } \
         /* a comment { } */ @media print { p { color: #ff0000 } } div p, li:hover { font-style: italic } .000003 { text-align: center } --></style>\
         <p>plain</p><p class='big x'>big</p><p class=big style='font-size: 8pt'>inline wins</p><p class=000003>digits</p>",
    );
    let ps = paras(&f);
    let (plain, big, inline, digits) = (fmt_of(ps[0]), fmt_of(ps[1]), fmt_of(ps[2]), fmt_of(ps[3]));
    assert_eq!((plain.color, plain.size, plain.bold, plain.italic), (Some(wordcraft_doc::Rgb(0, 255, 0)), Some(10.0), false, false));
    assert_eq!((big.size, big.bold), (Some(20.0), true));
    assert_eq!(inline.size, Some(8.0));
    assert_eq!(ps[3].align, Some(Align::Center));
    assert_eq!(digits.size, Some(10.0));
}

#[test]
fn paragraph_css_states_spacing_indents_and_line_height() {
    let f = parse(
        "<style>p.a { margin: 6pt 1in 3pt; text-indent: -18pt; line-height: 150% }</style>\
         <p class=a>a</p><p style='margin-top: 14,4px; margin-bottom: auto; line-height: 20pt'>b</p><p>c</p>",
    );
    let ps = paras(&f);
    assert_eq!(
        ps[0].spacing,
        Spacing {
            before: Some(6.0),
            after: Some(3.0),
            left: Some(72.0),
            right: Some(72.0),
            first: Some(-18.0),
            line: Some(LineSpacing::Multiple(1.5)),
            ..Spacing::default()
        }
    );
    // Values that don't parse (a comma decimal, `auto`) state nothing.
    assert_eq!(ps[1].spacing, Spacing { line: Some(LineSpacing::Exactly(20.0)), ..Spacing::default() });
    assert_eq!(ps[2].spacing, Spacing::default());
}

#[test]
fn a_page_opens_as_word_opens_one() {
    let html = "<style>p.k { margin-bottom: 13px } span.z { margin: 0in }</style>\
                <p>first</p><h1>Head</h1><p class=k><span class=z>kept</span></p><p class=k>own</p><div>bare</div><p>&nbsp;</p>";
    let f = parse_in(html, &|_| None, true);
    let ps = paras(&f);
    // Times New Roman 12 pt on single lines; headings bold and black at a browser's sizes.
    let first = fmt_of(ps[0]);
    assert_eq!((first.font.as_deref(), first.size), (Some(PAGE_FONT), Some(12.0)));
    assert_eq!(ps[0].spacing.line, Some(LineSpacing::Multiple(1.0)));
    let cell = parse_in("<table><tr><td>cell</td></tr></table>", &|_| None, true);
    let FBlock::Table(t) = &cell.blocks[0] else { panic!() };
    let FBlock::Para(cp) = &t.rows[0][0].blocks[0] else { panic!() };
    assert_eq!(cp.spacing.line, Some(LineSpacing::Multiple(1.0)));
    let head = fmt_of(ps[1]);
    assert_eq!((head.bold, head.size, head.color), (true, Some(24.0), Some(wordcraft_doc::Rgb(0, 0, 0))));
    // A colour the heading inherits stays.
    // A generic family names a face on a page; pasted text keeps the document's font.
    let generic = parse_in("<p style='font-family: sans-serif'>a</p><p style='font-family: serif'>b</p>", &|_| None, true);
    let fonts: Vec<Option<String>> = paras(&generic).iter().map(|p| fmt_of(p).font.map(|f| f.to_string())).collect();
    assert_eq!(fonts, [Some("Arial".to_string()), Some(PAGE_FONT.to_string())]);
    assert_eq!(fmt_of(paras(&parse("<p style='font-family: sans-serif'>a</p>"))[0]).font, None);
    let inherited = parse_in("<body style='color: #000080'><h1>Head</h1></body>", &|_| None, true);
    assert_eq!(fmt_of(paras(&inherited)[0]).color, Some(wordcraft_doc::Rgb(0, 0, 0x80)));
    // Automatic spacing where a p or heading leaves its margins out (layout drops it at the
    // top of the page and of a cell).
    let auto = |p: &Para| (p.spacing.before_auto, p.spacing.after_auto, p.spacing.before, p.spacing.after);
    assert_eq!(auto(ps[0]), (true, true, None, None));
    assert_eq!(auto(ps[1]), (true, true, None, None));
    // A span's margin replaces the one its paragraph states; the one it leaves out stays automatic.
    assert_eq!(auto(ps[2]), (true, false, None, Some(0.0)));
    assert_eq!(auto(ps[3]), (true, false, None, Some(9.75)));
    // Other blocks get no automatic spacing.
    assert_eq!(auto(ps[4]), (false, false, None, None));
    // The document says so, and the empty paragraph's mark keeps its size.
    let d = model::to_doc(&f);
    let props = |i: usize| match &*d.body[i] {
        wordcraft_doc::Block::Para(p) => p.clone(),
        b => panic!("{b:?}"),
    };
    assert_eq!((props(0).props.space_before_auto, props(0).props.space_after_auto), (Some(true), Some(true)));
    assert_eq!((props(2).props.space_after_auto, props(2).props.space_after), (None, Some(0.0)));
    assert_eq!(props(5).mark.size, Some(12.0));
    // `<p>&nbsp;</p>` keeps the size of its space for its line.
    assert!(ps[5].inlines.is_empty());
    assert_eq!(ps[5].mark.as_ref().and_then(|m| m.size), Some(12.0));
    // A fragment (a paste) keeps the document's own formatting.
    let pasted = parse(html);
    assert_eq!(fmt_of(paras(&pasted)[0]).font, None);
    assert_eq!(paras(&pasted)[0].spacing, Spacing::default());
}

#[test]
fn cells_give_their_columns_widths() {
    let f = parse(
        "<style>td.n { width: 42px }</style><table border=1><tr><td colspan=3>t</td></tr><tr><td class=n>1</td><td width=200>a</td><td style='width: 2in'>b</td></tr></table>",
    );
    let FBlock::Table(t) = &f.blocks[0] else { panic!("{:?}", f.blocks) };
    assert_eq!(t.widths, [31.5, 150.0, 144.0]);
    // A column without a width leaves them all to the layout.
    let f = parse("<table><tr><td width=100>a</td><td>b</td></tr></table>");
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert!(t.widths.is_empty());
    // A cell beside a cell spanning rows from above gives its own column its width.
    let f =
        parse("<table><tr><td rowspan=2>a</td><td width=200>b</td></tr><tr><td width=300>c</td></tr><tr><td width=100>d</td><td>e</td></tr></table>");
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.widths, [75.0, 150.0]);
}

#[test]
fn cells_take_their_css_borders() {
    use wordcraft_doc::props::{Border, BorderStyle};
    let f = parse(
        "<style>td.b { border-top: solid windowtext 1px; border-bottom: 2pt double #ff0000; border-left: none; border-right: 0px solid; vertical-align: bottom }</style><table><tr><td class=b>a</td><td>b</td></tr></table>",
    );
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    // The style sheet's other cell properties apply too.
    assert_eq!(t.rows[0][0].valign, Some(wordcraft_doc::props::VAlign::Bottom));
    let b = t.rows[0][0].borders.expect("borders");
    assert_eq!(b.top, Some(Border::single(0.75)));
    assert_eq!(b.bottom, Some(Border { style: BorderStyle::Double, width: 2.0, color: Some(wordcraft_doc::Rgb(255, 0, 0)), space: 0.0 }));
    assert_eq!(b.left.map(|l| l.style), Some(BorderStyle::None));
    // A border of no width is none (OOXML has no zero-width line).
    assert_eq!(b.right.map(|r| r.style), Some(BorderStyle::None));
    assert_eq!(t.rows[0][1].borders, None);
}

#[test]
fn style_sheet_edge_cases() {
    let f = parse_in(
        "<style>@charset \"utf-8\"; @import url(a.css); p.a { color: #0000ff } body { margin: 20px } p.b { margin: 10 5% medium }</style>\
         <body><p class=a>a</p>bare<p class=b>b</p></body>",
        &|_| None,
        true,
    );
    let ps = paras(&f);
    // A rule right after statement at-rules still applies.
    assert_eq!(fmt_of(ps[0]).color, Some(wordcraft_doc::Rgb(0, 0, 255)));
    // The body's margins aren't the paragraphs' indents.
    assert_eq!((ps[1].spacing.left, ps[1].spacing.before), (None, None));
    // Bare numbers, percentages and keywords state no margin: automatic spacing stays.
    assert_eq!((ps[2].spacing.before_auto, ps[2].spacing.left), (true, None));
    // The `style` attribute wins even over a sheet with more declarations than are read.
    let many = (0..600).map(|i| format!("x-{i}: 1")).collect::<Vec<_>>().join(";");
    let f = parse(&format!("<style>p {{ {many}; color: #ff0000 }}</style><p style='color: #00ff00'>c</p>"));
    assert_eq!(fmt_of(paras(&f)[0]).color, Some(wordcraft_doc::Rgb(0, 255, 0)));
}

#[test]
fn hostile_style_sheets_never_panic() {
    // One declaration block shared by many selectors isn't copied for each.
    let selectors = vec!["p"; 20_000].join(",");
    let huge = format!("<style>{selectors} {{ color: red; {} }}</style><p>x</p>", "x: y;".repeat(200_000));
    let _ = parse_in(&huge, &|_| None, true);
    let many = ".a { color: red } ".repeat(20_000);
    for css in ["{", "}", "p {", "@media { p { x } ", "p { color: red", "/* open", ".{}", "p..a{}", "\u{0}.é{x:y}", many.as_str()] {
        let _ = parse_in(&format!("<style>{css}</style><p class='a é'>x</p><td style='width: 1e40px'>y</td>"), &|_| None, true);
    }
}

/// Each paragraph's inlines as text, equations as `[linear]` (`[[linear]]` for display ones),
/// pictures as `<img>`.
fn inline_marks(f: &Flow) -> Vec<String> {
    f.blocks
        .iter()
        .filter_map(|b| match b {
            FBlock::Para(p) => Some(p),
            FBlock::Table(_) => None,
        })
        .map(|p| {
            p.inlines
                .iter()
                .map(|i| match i {
                    Inline::Text(t, _) => t.clone(),
                    Inline::Equation { linear, display: false } => format!("[{linear}]"),
                    Inline::Equation { linear, display: true } => format!("[[{linear}]]"),
                    Inline::Image(_) => "<img>".into(),
                    Inline::FieldStart(code) => format!("{{{code}|"),
                    Inline::FieldEnd => "}".into(),
                    _ => String::new(),
                })
                .collect::<String>()
        })
        .collect()
}

#[test]
fn ms_equation_comments_import_their_office_math() {
    // #5582's reproduction: the conditional comment's equation stands where the comment does,
    // and MathML beside it still imports.
    let html = r#"<!DOCTYPE html><html xmlns:m="http://schemas.microsoft.com/office/2004/12/omml"><body>
<p>Before <!--[if gte msEquation 12]><m:oMath><m:r><m:t>x=1</m:t></m:r></m:oMath><![endif]--> after</p>
<p>Math <math xmlns="http://www.w3.org/1998/Math/MathML"><mi>y</mi><mo>=</mo><mn>2</mn></math> end</p>
</body></html>"#;
    assert_eq!(inline_marks(&parse(html)), ["Before [x=1] after", "Math [y=2] end"]);
}

#[test]
fn ms_equation_fallback_picture_is_not_imported_too() {
    // Word writes a picture for readers without Office Math right after the comment.
    let html = r#"<p class=MsoNormal><!--[if gte msEquation 12]><m:oMathPara><m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara><![endif]--><![if !msEquation]><img width=20 height=30 src="data:image/png;base64,iVBORw0KGgo="><![endif]></p><p>next</p>"#;
    assert_eq!(inline_marks(&parse(html)), ["[[a/b]]", "next"]);
}

#[test]
fn ms_equation_para_reads_as_one_display_equation_per_o_math() {
    // As the DOCX reader: `a=bc=d` is no equation of the file's, and the fallback still goes.
    let html = r#"<p><!--[if gte msEquation 12]><m:oMathPara><m:oMathParaPr><m:jc m:val="center"/></m:oMathParaPr><m:oMath><m:r><m:t>a=b</m:t></m:r></m:oMath><m:oMath><m:r><m:t>c=d</m:t></m:r></m:oMath></m:oMathPara><![endif]--><![if !msEquation]>gone<![endif]></p>"#;
    assert_eq!(inline_marks(&parse(html)), ["[[a=b]][[c=d]]"]);
}

#[test]
fn ms_equation_fallback_stays_when_the_equations_are_not_all_read() {
    const FALLBACK: &str = "<![if !msEquation]>kept<![endif]>";
    let q = "<m:oMath><m:r><m:t>q</m:t></m:r></m:oMath>";
    let page = |math: &str| format!("<p><!--[if gte msEquation 12]>{math}<![endif]-->{FALLBACK}</p>");
    // One of several reads as nothing.
    assert_eq!(inline_marks(&parse(&page(&format!("{q}<m:oMath></m:oMath>")))), ["[q]kept"]);
    let para = format!("<m:oMathPara>{q}<m:oMath></m:oMath></m:oMathPara>");
    assert_eq!(inline_marks(&parse(&page(&para))), ["[[q]]kept"]);
    // More equations than one comment brings in.
    let many = q.repeat(crate::omml::MAX_EQUATIONS + 1);
    let marks = inline_marks(&parse(&page(&many)));
    assert_eq!(marks, [format!("{}kept", "[q]".repeat(crate::omml::MAX_EQUATIONS))]);
    // As many as it can, in one `m:oMathPara`.
    let many = format!("<m:oMathPara>{}</m:oMathPara>", q.repeat(crate::omml::MAX_EQUATIONS + 1));
    let marks = inline_marks(&parse(&page(&many)));
    assert_eq!(marks, [format!("{}kept", "[[q]]".repeat(crate::omml::MAX_EQUATIONS))]);
    // Exactly the cap is all of them.
    let all = inline_marks(&parse(&page(&q.repeat(crate::omml::MAX_EQUATIONS))));
    assert_eq!(all, ["[q]".repeat(crate::omml::MAX_EQUATIONS)]);
    // An equation too long to read whole.
    let long = format!("<m:oMath><m:r><m:t>q</m:t></m:r>{}</m:oMath>", "<m:r></m:r>".repeat(crate::mathml::MAX_NODES));
    let marks = inline_marks(&parse(&page(&long)));
    assert!(marks.first().is_some_and(|m| m.ends_with("]kept")), "{marks:?}");
}

#[test]
fn other_comments_and_hostile_ms_equation_comments_stay_harmless() {
    assert_eq!(inline_marks(&parse("<p>a<!-- [if gte msEquation 12] not math -->b</p>")), ["ab"]);
    assert_eq!(inline_marks(&parse("<p>a<!--[if gte mso 9]><xml><w:WordDocument/></xml><![endif]-->b</p>")), ["ab"]);
    // Unclosed comment, unclosed section, empty equation, junk: nothing panics, no stray text.
    for html in [
        "<p>a<!--[if gte msEquation 12]><m:oMath><m:r><m:t>q",
        "<p>a<!--[if gte msEquation 12]><m:oMath><m:r><m:t>q</m:t></m:r></m:oMath>-->b</p>",
        "<p>a<!--[if gte msEquation 12]><![endif]-->b</p>",
        "<p>a<!--[if gte msEquation-->b</p>",
        "<p>a<!--[if gte msEquation 12]><m:oMath></m:oMath><![endif]-->b</p>",
        "<p>a<!--[if gte msEquation 12]>]]]><<<>>>&amp;<![endif]-->b</p>",
        "<p>a<!--[if gte msEquation 12]><m:oMath><m:r><m:t>é</m:t></m:r></m:oMath><![endif]--><![if !msEquation]>",
    ] {
        let marks = inline_marks(&parse(html));
        assert!(marks.iter().all(|m| !m.contains("m:") && !m.contains("endif")), "{html}: {marks:?}");
    }
    assert_eq!(inline_marks(&parse("<p>a<!--[if gte msEquation 12]><m:oMath><m:r><m:t>q</m:t></m:r></m:oMath>-->b</p>")), ["a[q]b"]);
    // In the head, it is not body content.
    let head = "<html><head><!--[if gte msEquation 12]><m:oMath><m:r><m:t>h</m:t></m:r></m:oMath><![endif]--></head><body><p>x</p></body></html>";
    assert_eq!(inline_marks(&parse(head)), ["x"]);
}

#[test]
fn ms_equation_fallback_stays_when_no_equation_is_read_or_it_is_unclosed() {
    // An Office Math comment that reads as nothing leaves the fallback picture in its place.
    let empty = r#"<p>a<!--[if gte msEquation 12]><m:oMath></m:oMath><![endif]--><![if !msEquation]><img width=20 height=30 src="data:image/png;base64,iVBORw0KGgo="><![endif]>b</p>"#;
    assert_eq!(inline_marks(&parse(empty)), ["a<img>b"]);
    // A fallback opener without its end doesn't swallow the rest of the page.
    let unclosed = "<p>a<!--[if gte msEquation 12]><m:oMath><m:r><m:t>q</m:t></m:r></m:oMath><![endif]--><![if !msEquation]>b</p><p>c</p>";
    assert_eq!(inline_marks(&parse(unclosed)), ["a[q]b", "c"]);
}

#[test]
fn support_fields_comments_import_as_fields_around_their_result() {
    // #5581's reproduction (Better Notes for Zotero): the whole field in one comment.
    let html = r#"<!DOCTYPE html><html><body>
<p>Before <!--[if supportFields]>
<span style='mso-element:field-begin'></span>
ADDIN ZOTERO_ITEM CSL_CITATION {&quot;citationID&quot;:&quot;a&quot;,&quot;citationItems&quot;:[]}
<span style='mso-element:field-separator'></span>
<span>(Cached, 2021)</span>
<span style='mso-element:field-end'></span>
<![endif]--> after</p>
</body></html>"#;
    let marks = inline_marks(&parse(html));
    assert_eq!(marks.len(), 1, "{marks:?}");
    let m = &marks[0];
    assert!(m.starts_with("Before {ADDIN ZOTERO_ITEM CSL_CITATION {\"citationID\":\"a\",\"citationItems\":[]}|"), "{m}");
    assert!(m.contains("(Cached, 2021)") && m.ends_with("} after"), "{m}");
}

#[test]
fn word_pages_keep_a_fields_result_between_two_comments() {
    // Word's own web pages: the result is ordinary HTML between the comments, with its formatting.
    let html = "<p>Page <!--[if supportFields]><span style='mso-element:field-begin'></span><span style='mso-spacerun:yes'> </span>PAGE <span style='mso-element:field-separator'></span><![endif]--><b>3</b><!--[if supportFields]><span style='mso-element:field-end'></span><![endif]--> of</p>";
    let f = parse(html);
    assert_eq!(inline_marks(&f), ["Page {PAGE|3} of"]);
    let Some(FBlock::Para(p)) = f.blocks.first() else { panic!("no paragraph") };
    assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, fmt) if t == "3" && fmt.bold)), "{:?}", p.inlines);
}

#[test]
fn support_fields_edge_cases_stay_balanced() {
    // A field without a separator has an empty result; one nested in another's code is part of
    // that code; a field the page leaves open ends with it; a mark's empty span doesn't close an
    // enclosing span.
    let no_sep =
        "<p>a<!--[if supportFields]><span style='mso-element:field-begin'></span>AUTHOR<span style='mso-element:field-end'></span><![endif]-->b</p>";
    assert_eq!(inline_marks(&parse(no_sep)), ["a{AUTHOR|}b"]);
    let nested = "<p><!--[if supportFields]><span style='mso-element:field-begin'></span>IF <span style='mso-element:field-begin'></span>PAGE<span style='mso-element:field-separator'></span>2<span style='mso-element:field-end'></span> = 2 yes<span style='mso-element:field-separator'></span><![endif]-->yes<!--[if supportFields]><span style='mso-element:field-end'></span><![endif]--></p>";
    assert_eq!(inline_marks(&parse(nested)), ["{IF PAGE2 = 2 yes|yes}"]);
    let open = "<p>a<!--[if supportFields]><span style='mso-element:field-begin'></span>DATE<span style='mso-element:field-separator'></span><![endif]-->today</p>";
    assert_eq!(inline_marks(&parse(open)), ["a{DATE|today}"]);
    let wrapped = "<p><i>x<!--[if supportFields]><span style='mso-element:field-begin'></span>PAGE<span style='mso-element:field-separator'></span><![endif]-->1<!--[if supportFields]><span style='mso-element:field-end'></span><![endif]-->y</i></p>";
    let f = parse(wrapped);
    assert_eq!(inline_marks(&f), ["x{PAGE|1}y"]);
    let Some(FBlock::Para(p)) = f.blocks.first() else { panic!("no paragraph") };
    assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, fmt) if t == "y" && fmt.italic)), "{:?}", p.inlines);
    // Junk: stray ends and separators, no code, unclosed comments, many begins.
    for html in [
        "<p>a<!--[if supportFields]><span style='mso-element:field-end'></span><span style='mso-element:field-separator'></span><![endif]-->b</p>",
        "<p>a<!--[if supportFields]><span style='mso-element:field-begin'></span><span style='mso-element:field-separator'></span>r<span style='mso-element:field-end'></span><![endif]-->b</p>",
        "<p>a<!--[if supportFields]><span style='mso-element:field-begin'></span>PAGE",
        "<p>a<!--[if supportFields]>",
    ] {
        let marks = inline_marks(&parse(html));
        let opens = marks.iter().map(|m| m.matches('{').count()).sum::<usize>();
        let closes = marks.iter().map(|m| m.matches('}').count()).sum::<usize>();
        assert_eq!(opens, closes, "{html}: {marks:?}");
    }
    let many = format!(
        "<p><!--[if supportFields]>{}<![endif]-->z</p>",
        "<span style='mso-element:field-begin'></span>F<span style='mso-element:field-separator'></span>".repeat(200)
    );
    let marks = inline_marks(&parse(&many));
    assert_eq!(marks.iter().map(|m| m.matches('{').count()).sum::<usize>(), 64);
    assert_eq!(marks.iter().map(|m| m.matches('}').count()).sum::<usize>(), 64);
}

#[test]
fn field_marks_never_take_page_text() {
    // A field that never gets past its begin mark leaves the page as it was: only a comment
    // holds a code's text.
    let lone = "<!--[if supportFields]><span style='mso-element:field-begin'></span><![endif]--><p>Para 1</p><p>Para 2</p>";
    assert_eq!(inline_marks(&parse(lone)), ["Para 1", "Para 2"]);
    let coded = "<p><!--[if supportFields]><span style='mso-element:field-begin'></span>PAGE<![endif]-->text</p><p>more</p>";
    assert_eq!(inline_marks(&parse(coded)), ["{PAGE|}text", "more"]);
    // A code can span comments (Word puts other comments between them).
    let spanned = r"<p><!--[if supportFields]><span style='mso-element:field-begin'></span>PAGE <![endif]--> <!--[if gte mso 9]><xml></xml><![endif]--><!--[if supportFields]>\* ARABIC<span style='mso-element:field-separator'></span><![endif]-->1<!--[if supportFields]><span style='mso-element:field-end'></span><![endif]--></p>";
    assert_eq!(inline_marks(&parse(spanned)), [r"{PAGE \* ARABIC|1}"]);
    // A mark on an element with no end tag, or with content, doesn't skip the page after it.
    let void = r#"<p>a<br style="mso-element:field-end">b</p><p>c</p>"#;
    let marks = inline_marks(&parse(void));
    assert_eq!(marks.last().map(String::as_str), Some("c"));
    assert!(marks.concat().contains('b'), "{marks:?}");
    let content = "<p><span style='mso-element:field-separator'>Smith 2020</span> after</p>";
    assert_eq!(inline_marks(&parse(content)), ["Smith 2020 after"]);
    // The mark's own end tag is skipped through white space, and closes nothing else.
    let spaced = "<p><b>x<span style='mso-element:field-end'> </span >y</b></p>";
    let f = parse(spaced);
    assert_eq!(inline_marks(&f), ["xy"]);
}

#[test]
fn support_fields_comments_read_raw_text_as_the_page_does() {
    let f = parse("<p>a<!--[if supportFields]><style>p{color:#ff0000}</style><script>hidden()</script><![endif]-->b</p>");
    assert_eq!(inline_marks(&f), ["ab"]);
}

#[test]
fn support_fields_result_keeps_its_own_spaces() {
    let mark = |m: &str| format!("<span style='mso-element:field-{m}'></span>");
    let one = |r: &str| format!("<p>a<!--[if supportFields]>{}F{}{r}{}<![endif]-->b</p>", mark("begin"), mark("separator"), mark("end"));
    assert_eq!(inline_marks(&parse(&one(" "))), ["a{F| }b"]);
    assert_eq!(inline_marks(&parse(&one("<b>x</b> <b>y</b>"))), ["a{F|x y}b"]);
    // Line breaks around a mark are the source's layout.
    assert_eq!(inline_marks(&parse(&one("\n<b>x</b>\n"))), ["a{F|x}b"]);
}

#[test]
fn nested_support_fields_openers_dont_exhaust_the_stack() {
    // Comments don't nest: each opener inside runs to the end, and is read on in a loop.
    let deep = format!("<p>a<!--{}x-->b</p>", "[if supportFields]><span style='mso-element:field-begin'></span>F<!--".repeat(200_000));
    let marks = inline_marks(&parse(&deep));
    let opens = marks.iter().map(|m| m.matches('{').count()).sum::<usize>();
    assert_eq!(opens, marks.iter().map(|m| m.matches('}').count()).sum::<usize>());
    // A field never separated keeps the code its comment holds, and
    // only the first openers are read.
    assert!(marks.concat().matches('F').count() <= 17, "{marks:?}");
    // One nested opener: what follows it is still read.
    let one = "<p>a<!--[if supportFields]><span style='mso-element:field-begin'></span>PAGE<span style='mso-element:field-separator'></span><!--[if supportFields]>1<span style='mso-element:field-end'></span><![endif]-->b</p>";
    assert_eq!(inline_marks(&parse(one)), ["a{PAGE|1}b"]);
}

#[test]
fn ms_equation_fallback_ends_at_its_own_endif() {
    // Word's fallback holds a VML section of its own: the fallback ends at the endif matching it.
    let vml = r#"<p>a<!--[if gte msEquation 12]><m:oMath><m:r><m:t>q</m:t></m:r></m:oMath><![endif]--><![if !msEquation]><span><![if !vml]><img width=20 height=30 src="data:image/png;base64,iVBORw0KGgo="><![endif]>v</span><![endif]>b</p>"#;
    assert_eq!(inline_marks(&parse(vml)), ["a[q]b"]);
    // An unclosed fallback doesn't take a later, unrelated section's endif as its own.
    let other = r#"<p>a<!--[if gte msEquation 12]><m:oMath><m:r><m:t>q</m:t></m:r></m:oMath><![endif]--><![if !msEquation]>b</p><p>c<![if !vml]>d<![endif]>e</p>"#;
    assert_eq!(inline_marks(&parse(other)), ["a[q]b", "cde"]);
}

#[test]
fn many_unclosed_ms_equation_fallbacks_import_quickly() {
    // Each unclosed fallback opener searched the rest of the page for its end, so a page of
    // equations each followed by one took quadratic time: about a minute here, now well under a
    // second.
    let one = "<!--[if gte msEquation 12]><m:oMath><m:r><m:t>q</m:t></m:r></m:oMath><![endif]--><![if !msEquation]>";
    let html = format!("<p>a{}b</p>", one.repeat(40_000));
    let t = std::time::Instant::now();
    let f = parse(&html);
    assert!(t.elapsed() < std::time::Duration::from_secs(10), "{:?}", t.elapsed());
    let marks = inline_marks(&f);
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].matches("[q]").count(), 40_000);
    assert!(marks[0].starts_with("a[q]") && marks[0].ends_with("[q]b"), "{}", marks[0].get(..40).unwrap_or(""));
}
