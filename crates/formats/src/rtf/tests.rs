use super::*;

fn texts(f: &Flow) -> Vec<String> {
    f.blocks
        .iter()
        .map(|b| match b {
            FBlock::Para(p) => format!("{:?}:{}", p.kind, p.text()),
            FBlock::Table(t) => {
                format!("table{}x{}:{}", t.rows.len(), t.cols(), t.rows.iter().flatten().map(|c| c.text()).collect::<Vec<_>>().join("|"))
            }
        })
        .collect()
}

#[test]
fn basic_reader() {
    let rtf = br#"{\rtf1\ansi{\fonttbl{\f0 Arial;}{\f1 Courier New;}}{\colortbl;\red255\green0\blue0;}{\stylesheet{\s0 Normal;}{\s1 heading 1;}}
{\info{\title My Doc}{\author Ann}}
\pard\s1 Head\par
\pard Plain {\b bold} \i it\i0  {\cf1 red} caf\'e9 \u8364? and\line next\tab x\par
{\*\unknown skip me}{\field{\*\fldinst{HYPERLINK "http://a.b"}}{\fldrslt{link}}}\par
\trowd\cellx100\cellx200 \pard\intbl a\cell b\cell\row
\pard after\par}"#;
    let f = parse(rtf).unwrap();
    assert_eq!(f.meta.title, "My Doc");
    assert_eq!(f.meta.author, "Ann");
    let t = texts(&f);
    assert_eq!(t[0], "Heading(1):Head");
    assert_eq!(t[1], "Normal:Plain bold it red café € and\nnext\tx");
    assert_eq!(t[2], "Normal:link");
    assert_eq!(t[3], "table1x2:a|b");
    assert_eq!(t[4], "Normal:after");
    let FBlock::Para(p) = &f.blocks[1] else { panic!() };
    assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "bold" && f.bold)));
    assert!(p.inlines.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "red" && f.color == Some(Rgb(255, 0, 0)))));
    let FBlock::Para(l) = &f.blocks[2] else { panic!() };
    assert!(matches!(&l.inlines[0], Inline::Text(_, f) if f.link.as_deref() == Some("http://a.b")));
}

#[test]
fn paragraph_borders() {
    // A heading with a grey rule under it, as hh.ru résumés write their section titles.
    let rtf = br"{\rtf1\ansi{\colortbl;\red0\green0\blue0;\red216\green216\blue216;}
\pard\brdrb\brdrs\brdrw10\brdrcf2 Section\par
\pard Body\par
\pard\brdrt\brdrdb\brdrl\brdrs\brdrr\brdrs Boxed\par
\pard\brdrb\brdrs\par}";
    let f = parse(rtf).unwrap();
    let paras: Vec<&Para> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect();
    let b = paras[0].borders.unwrap();
    let bottom = b.bottom.unwrap();
    assert_eq!((bottom.width, bottom.color), (0.5, Some(Rgb(216, 216, 216))));
    assert!(b.top.is_none());
    assert_eq!(paras[0].kind, Kind::Normal);
    // \pard resets them.
    assert!(paras[1].borders.is_none());
    let b = paras[2].borders.unwrap();
    assert_eq!(b.top.map(|x| x.style), Some(wordcraft_doc::props::BorderStyle::Double));
    assert!(b.left.is_some() && b.right.is_some() && b.bottom.is_none());
    // An empty paragraph with only a bottom border is still a rule.
    assert_eq!(paras[3].kind, Kind::Rule);
    // The document's paragraph carries them.
    let d = import(rtf).unwrap();
    let first = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None }).unwrap();
    assert!(first.props.borders.and_then(|b| b.bottom).is_some());
    // Paragraph borders round-trip through export and import.
    let exported = export(&d);
    assert!(exported.contains("\\brdrb\\brdrs\\brdrw10\\brdrcf1"), "{exported}");
    let back = import(exported.as_bytes()).unwrap();
    let first = back.body.iter().find_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None }).unwrap();
    let b = first.props.borders.unwrap().bottom.unwrap();
    assert_eq!((b.style, b.width, b.color), (wordcraft_doc::props::BorderStyle::Single, 0.5, Some(Rgb(216, 216, 216))));
}

#[test]
fn headers_and_footers() {
    let rtf = br"{\rtf1\ansi\titlepg
{\header \pard\qr Running head\par}
{\headerf \pard First head {\b bold}\par}
{\footer \pard Page \chpgn\par}
\pard Body text\par
\trowd\cellx2000\pard\intbl cell\cell\row
\pard after\par}";
    let d = import(rtf).unwrap();
    let s = &d.last_section;
    assert!(s.title_page);
    let text = |id: Option<u32>| {
        let p = id.and_then(|id| d.parts.get(&id)).unwrap();
        p.blocks
            .iter()
            .filter_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p.text.clone()) } else { None })
            .collect::<Vec<_>>()
            .join("|")
    };
    assert_eq!(text(s.headers.default), "Running head");
    assert_eq!(text(s.headers.first), "First head bold");
    assert!(text(s.footers.default).starts_with("Page"));
    assert!(s.footers.first.is_none() && s.headers.even.is_none());
    // The body keeps only its own text, and its table survives the header groups.
    let f = parse(rtf).unwrap();
    let t = texts(&f);
    assert_eq!(t, ["Normal:Body text", "table1x1:cell", "Normal:after"]);
    // Without \titlepg there is no separate first page.
    let d = import(br"{\rtf1\ansi{\headerf x\par} y\par}").unwrap();
    assert!(!d.last_section.title_page && d.last_section.headers.first.is_some());
    // An unterminated header still ends, and the body text after it isn't lost.
    assert!(import(br"{\rtf1\ansi{\header x").is_ok());
}

#[test]
fn paragraph_spacing_and_indents() {
    use wordcraft_doc::props::LineSpacing;
    let rtf = br"{\rtf1\ansi
\pard\sb120\sa150\sl360\slmult1\li720\ri360\fi-360 a\par
\pard\sl-280 b\par
\pard\sl300\slmult0 c\par
\pard d\par}";
    let f = parse(rtf).unwrap();
    let sp: Vec<model::Spacing> = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.spacing) } else { None }).collect();
    assert_eq!(sp[0].before, Some(6.0));
    assert_eq!(sp[0].after, Some(7.5));
    assert_eq!(sp[0].line, Some(LineSpacing::Multiple(1.5)));
    assert_eq!((sp[0].left, sp[0].right, sp[0].first), (Some(36.0), Some(18.0), Some(-18.0)));
    assert_eq!(sp[1].line, Some(LineSpacing::Exactly(14.0)));
    assert_eq!(sp[2].line, Some(LineSpacing::AtLeast(15.0)));
    // \pard resets them.
    assert_eq!(sp[3], model::Spacing::default());
    // The document's defaults are RTF's: single, no space after; the paragraph's own win.
    let d = import(rtf).unwrap();
    assert_eq!(d.styles.default_para.space_after, Some(0.0));
    assert_eq!(d.styles.default_para.line_spacing, Some(LineSpacing::Multiple(1.0)));
    let first = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None }).unwrap();
    assert_eq!(first.props.space_after, Some(7.5));
    assert_eq!(first.props.indent_first, Some(-18.0));
}

#[test]
fn default_font_is_deff_or_font_zero() {
    // Text that never selects a font is in the default font: \deff, else font 0.
    let d = import(br"{\rtf1\ansi{\fonttbl{\f0\fnil Times New Roman Cyr;}{\f1 Arial;}}\plain x\par}").unwrap();
    assert_eq!(d.styles.default_chr.font.as_deref(), Some("Times New Roman Cyr"));
    let d = import(br"{\rtf1\ansi\deff1{\fonttbl{\f0 Times New Roman;}{\f1 Arial;}} x\par}").unwrap();
    assert_eq!(d.styles.default_chr.font.as_deref(), Some("Arial"));
    // No font table: our default stays.
    let d = import(br"{\rtf1\ansi x\par}").unwrap();
    assert_eq!(d.styles.default_chr.font, wordcraft_doc::StyleSheet::builtin().default_chr.font);
}

#[test]
fn unicode_escapes_in_font_names() {
    // Ordinary fallback, hex fallback and \uc0 all give "Café Sans"; the fallback is not kept.
    for name in [r"Caf\u233? Sans", r"Caf\u233\'e9 Sans", r"\uc0 Caf\u233  Sans", r"\uc2 Caf\u233\'e9\'e9 Sans"] {
        let rtf = format!(r"{{\rtf1\ansi\deff0{{\fonttbl{{\f0 {name};}}{{\f1 \u923?ambda;}}}}\plain x {{\f1 y}}\par}}");
        let d = import(rtf.as_bytes()).unwrap();
        assert_eq!(d.styles.default_chr.font.as_deref(), Some("Café Sans"), "{name}");
        // A body run in a Unicode-named font keeps that name.
        let f = parse(rtf.as_bytes()).unwrap();
        let y_font = f.blocks.iter().find_map(|b| match b {
            FBlock::Para(p) => p.inlines.iter().find_map(|i| match i {
                Inline::Text(t, fmt) if t.trim() == "y" => fmt.font.clone(),
                _ => None,
            }),
            FBlock::Table(_) => None,
        });
        assert_eq!(y_font.as_deref(), Some("Λambda"), "{name}");
        // Our export writes font names as `\uN?`; reading it back gives the same name.
        let back = import(export(&d).as_bytes()).unwrap();
        assert_eq!(back.styles.default_chr.font.as_deref(), Some("Café Sans"), "{name}");
    }
}

#[test]
fn page_size_and_margins() {
    let d = import(br"{\rtf1\ansi\paperw11906\paperh16838\margl1134\margr850\margt1134\margb1134 x\par}").unwrap();
    let s = &d.last_section;
    assert!((s.page_w - 595.3).abs() < 0.1 && (s.page_h - 841.9).abs() < 0.1, "{} x {}", s.page_w, s.page_h);
    assert!((s.margin_left - 56.7).abs() < 0.1 && (s.margin_right - 42.5).abs() < 0.1);
    assert!(!s.landscape);
    let d = import(br"{\rtf1\ansi\paperw11906\paperh16838\landscape x\par}").unwrap();
    assert!(d.last_section.landscape && d.last_section.page_w > d.last_section.page_h);
    let exported = export(&d);
    assert!(exported.contains("\\landscape"), "exported RTF must include \\landscape: {exported}");
    // Nonsense keeps Word's defaults.
    let d = import(br"{\rtf1\ansi\paperw0\margl99999 x\par}").unwrap();
    assert_eq!(d.last_section, wordcraft_doc::SectionProps::default());
}

#[test]
fn table_grid_from_cell_edges_and_borders() {
    // Rows whose cells don't line up share one grid: edges 2000, 3000, 9000.
    let rtf = br"{\rtf1\ansi
\trowd\cellx2000\cellx9000 \pard\intbl a\cell b\cell\row
\trowd\cellx3000\cellx9000 \pard\intbl c\cell d\cell\row
\pard after\par}";
    let f = parse(rtf).unwrap();
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert_eq!(t.widths, [100.0, 50.0, 300.0]);
    assert_eq!(t.rows[0].iter().map(|c| c.colspan).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(t.rows[1].iter().map(|c| c.colspan).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(t.cols(), 3);
    // RTF tables have no grid style: each cell draws the borders it names, nothing else.
    assert!(t.borderless);
    assert!(t.rows.iter().flatten().all(|c| c.borders.is_none()));
    // A rule under a cell (a letterhead's line): the bottom side only, its width and colour.
    let f = parse(br"{\rtf1\ansi{\colortbl;\red174\green174\blue174;}\trowd\clbrdrb\brdrs\brdrw15\brdrcf1\cellx2000\pard\intbl a\cell\row}").unwrap();
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    let b = t.rows[0][0].borders.unwrap();
    assert!(b.top.is_none() && b.left.is_none() && b.right.is_none());
    let bottom = b.bottom.unwrap();
    assert_eq!((bottom.style, bottom.width, bottom.color), (wordcraft_doc::props::BorderStyle::Single, 0.75, Some(Rgb(174, 174, 174))));
    // Every side, in different styles, per cell.
    let f = parse(
        br"{\rtf1\ansi\trowd\clbrdrt\brdrdb\clbrdrl\brdrs\brdrw10\clbrdrb\brdrdot\clbrdrr\brdrth\cellx2000\cellx4000\pard\intbl a\cell b\cell\row}",
    )
    .unwrap();
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    let b = t.rows[0][0].borders.unwrap();
    use wordcraft_doc::props::BorderStyle as S;
    assert_eq!(
        [b.top.map(|x| x.style), b.left.map(|x| x.style), b.bottom.map(|x| x.style), b.right.map(|x| x.style)],
        [Some(S::Double), Some(S::Single), Some(S::Dotted), Some(S::Thick)]
    );
    assert_eq!(b.left.map(|x| x.width), Some(0.5));
    assert!(t.rows[0][1].borders.is_none(), "the second cell names no borders");
    // A border side set to none isn't a border.
    let f = parse(br"{\rtf1\ansi\trowd\clbrdrl\brdrnone\cellx2000\pard\intbl a\cell\row}").unwrap();
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    assert!(t.rows[0][0].borders.is_none());
    // The document's cells carry them.
    let d = import(br"{\rtf1\ansi\trowd\clbrdrb\brdrs\cellx2000\pard\intbl a\cell\row}").unwrap();
    let t = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
    assert!(t.props.style.is_none());
    assert!(t.rows[0].cells[0].props.borders.and_then(|b| b.bottom).is_some());
    // Borderless tables export without cell borders and round-trip borderless.
    let d = import(br"{\rtf1\ansi\trowd\cellx2000\pard\intbl a\cell\row}").unwrap();
    let exported = export(&d);
    assert!(!exported.contains("\\clbrdr"), "{exported}");
    let back = import(exported.as_bytes()).unwrap();
    let tb = back.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
    assert!(tb.props.style.is_none());
    assert!(tb.rows[0].cells[0].props.borders.is_none());

    // Cell borders round-trip through export and import.
    let d =
        import(br"{\rtf1\ansi{\colortbl;\red174\green174\blue174;}\trowd\clbrdrb\brdrdb\brdrw15\brdrcf1\cellx2000\pard\intbl a\cell\row}").unwrap();
    let exported = export(&d);
    assert!(exported.contains("\\clbrdrb\\brdrdb\\brdrw15\\brdrcf1"), "{exported}");
    let back = import(exported.as_bytes()).unwrap();
    let tb = back.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
    let b = tb.rows[0].cells[0].props.borders.unwrap().bottom.unwrap();
    assert_eq!((b.style, b.width, b.color), (wordcraft_doc::props::BorderStyle::Double, 0.75, Some(Rgb(174, 174, 174))));

    // The table sizes to the document's page, not Letter.
    let d = import(br"{\rtf1\ansi\paperw11906\margl1134\margr1134\trowd\cellx4818\cellx9638\pard\intbl a\cell b\cell\row}").unwrap();
    let t = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).unwrap();
    assert_eq!(t.grid, [240.9, 241.0]);
}

#[test]
fn code_pages_from_ansicpg_and_font_charsets() {
    // The document's code page: Russian RTF as accounting systems write it.
    let f =
        parse(br"{\rtf1\ansi\ansicpg1251\deff0{\fonttbl{\f0 Times New Roman;}}\f0 \'cf\'f0\'e8\'e2\'e5\'f2, \'b8\'eb\'ea\'e0 \'b9 5\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:Привет, ёлка № 5"]);
    // Font charsets switch inside a Windows-1252 document: Western, Cyrillic, Central
    // European, then back to the first font.
    let f = parse(
        br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset204 Arial CYR;}{\f2\fcharset238 Arial CE;}}\f0 caf\'e9 {\f1 \'c4\'e0} \f2 \'a3\'f3d\'9f \f0 na\'efve\par}",
    )
    .unwrap();
    assert_eq!(texts(&f), ["Normal:café Да Łódź naïve"]);
    // The default font's charset applies before any \f; an explicit \cpg wins over \fcharset.
    let f = parse(br"{\rtf1\ansi\deff1{\fonttbl{\f1\fcharset204 Times;}{\f2\fcharset0\cpg1253 Greek;}}\'cc\'e8\'f0 \f2 \'c1\'e2\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:Мир Αβ"]);
    // Raw 8-bit text (not escaped) follows the same code page.
    let f = parse(b"{\\rtf1\\ansi\\ansicpg1251 \xc4\xee\xec\\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:Дом"]);
    // #1653: a Macintosh document (LibreOffice's fdo85889-mac.rtf) is Mac OS Roman, and so is
    // a font with the Mac charset in a Windows document.
    let f = parse(br"{\rtf1\mac \deff0{\fonttbl{\f0\fmodern Helvetica;}}\pard\f0\fs20 \'f1\'f2\'f3\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:ÒÚÛ"]);
    let f = parse(br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset77 Geneva;}}caf\'e9 {\f1 caf\'8e \'d2ok\'d3}\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:café café “ok”"]);
    // \fcharset255 is the standard OEM character set (CP437).
    let f = parse(br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset255 Terminal;}}caf\'e9 {\f1 caf\'82}\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:café café"]);
    // DOS documents: `\pc` is code page 437, `\pca` 850; a font can name one with `\cpg`.
    let f = parse(br"{\rtf1\pc\deff0{\fonttbl{\f0 Courier;}}caf\'82 \'a0 la carte\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:café á la carte"]);
    let f = parse(br"{\rtf1\pca\deff0{\fonttbl{\f0 Courier;}{\f1\cpg437 Courier;}}\'9d {\f1 \'9d}\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:Ø ¥"]);
    // A code page number out of range is ignored, not truncated to another one.
    let f = parse(br"{\rtf1\ansi\ansicpg65973 caf\'e9\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:café"]);
    // The OEM charset (255) is DOS code page 437, like 254.
    let f = parse(br"{\rtf1\ansi\deff0{\fonttbl{\f0\fcharset255 Courier New;}}\f0 caf\'82\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:café"]);
}

/// #1653: the East Asian code pages (932 Shift-JIS, 936 GBK, 949 UHC, 950 Big5) decode their
/// bytes as a run, so a lead byte and its trail byte decode together, escaped or raw.
#[test]
fn east_asian_multibyte_code_pages() {
    // Japanese `\ansicpg932` (CP932: 日 = 93 fa, 本 = 96 7b, 語 = 8c ea).
    let f = parse(br"{\rtf1\ansi\ansicpg932 \'93\'fa\'96\'7b\'8c\'ea\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:日本語"]);
    // Raw 8-bit text (not `\'hh`), and raw bytes mixed with one escaped byte: RTF can't carry a
    // raw `{`, `}` or `\`, so 本's trail byte 7b is written `\'7b` here.
    let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x8a\xbf\x8e\x9a\\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:漢字"]);
    let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\xfa\x96\\'7b\x8c\xea\\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:日本語"]);
    // A font names the code page with `\fcharset128` or with `\cpg932`, and `\f0` selects it.
    let f = parse(br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fcharset128 MS Mincho;}}\f0 \'93\'fa\'96\'7b\'8c\'ea\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:日本語"]);
    let f = parse(br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\cpg932 MS Mincho;}}\f0 \'93\'fa\'96\'7b\'8c\'ea\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:日本語"]);
    // Raw 8-bit bytes in the font table (the font's name) decode in the document's code page.
    let d = import(b"{\\rtf1\\ansi\\ansicpg932\\deff0{\\fonttbl{\\f0\\fcharset128 \x96\xbe\x92\xa9;}}\\f0 x\\par}").unwrap();
    assert_eq!(d.styles.default_chr.font.as_deref(), Some("明朝"));
    // Korean `\ansicpg949` (UHC: 한국어 = c7 d1 b1 b9 be ee).
    let f = parse(br"{\rtf1\ansi\ansicpg949 \'c7\'d1\'b1\'b9\'be\'ee\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:한국어"]);
    // Simplified Chinese `\ansicpg936` (GBK: 中文 = d6 d0 ce c4).
    let f = parse(br"{\rtf1\ansi\ansicpg936 \'d6\'d0\'ce\'c4\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:中文"]);
    // Traditional Chinese `\ansicpg950` (Big5: 中文 = a4 a4 a4 e5).
    let f = parse(br"{\rtf1\ansi\ansicpg950 \'a4\'a4\'a4\'e5\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:中文"]);
    // `\uN?` escapes still decode alongside multibyte text (`\u26412?` is 本, its `?` fallback
    // skipped by `\uc1`).
    let f = parse(br"{\rtf1\ansi\ansicpg932 \'93\'fa\u26412?\'8c\'ea x\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:日本語 x"]);
    // An invalid sequence decodes as U+FFFD, as Word writes it, and doesn't lose the rest.
    let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\\par after\\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:\u{FFFD}", "Normal:after"]);
    let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\xfa\x93 x\x96\x8c\\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:日\u{FFFD} x膜"]);
}

/// The decoder never sniffs a BOM: the code page is named by `\ansicpg`, `\cpg` or `\fcharset`,
/// so a run that starts `EF BB BF` is a character pair and a lead byte, not a UTF-8 BOM (which
/// would switch the rest of the document to UTF-8 and garble it).
#[test]
fn a_bom_like_byte_triple_is_text_in_a_multibyte_code_page() {
    // GBK EF BB = 锘, then BF B5 = 康; UHC EF BB = 癤, then BF C0 = 오; Big5 EF BB = 嚜, then
    // BF 40 = 濃.
    let f = parse(br"{\rtf1\ansi\ansicpg936 \'ef\'bb\'bf\'b5\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:锘康"]);
    let f = parse(br"{\rtf1\ansi\ansicpg949 \'ef\'bb\'bf\'c0\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:癤오"]);
    let f = parse(br"{\rtf1\ansi\ansicpg950 \'ef\'bb\'bf\'40\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:嚜濃"]);
}

/// A multibyte run longer than any fixed buffer keeps every character: the streaming decoder
/// holds a lead byte and its trail byte together across decode steps instead of cutting the
/// character at a buffer size (the review's finding on the byte-capped flush).
#[test]
fn a_long_multibyte_run_never_splits_a_character() {
    let mut bytes = b"{\\rtf1\\ansi\\ansicpg932 ".to_vec();
    bytes.push(b'A');
    // 2048 × あ (CP932 82 a0) is 4096 bytes after the one ASCII byte: a fixed 4096-byte buffer
    // would flush with the last あ's lead byte in one step and its trail byte in the next.
    for _ in 0..2048 {
        bytes.extend_from_slice(&[0x82, 0xA0]);
    }
    bytes.extend_from_slice(b"\\par}");
    let f = parse(&bytes).unwrap();
    let expected: String = std::iter::once('A').chain((0..2048).map(|_| 'あ')).collect();
    assert_eq!(texts(&f), [format!("Normal:{expected}")]);
}

/// A lead byte with no trail byte at the end of the document becomes U+FFFD, as Word writes a
/// malformed sequence — the streaming decoder is finalised, not left holding it.
#[test]
fn an_incomplete_lead_byte_at_the_end_is_replacement() {
    let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93}").unwrap();
    assert_eq!(texts(&f), ["Normal:\u{FFFD}"]);
}

/// #1653: a code page switch (`\f`, `\fcharset`, `\cpg`, `\ansicpg`), a group end and the end
/// of the document decode the multibyte bytes read so far, before the new code page applies.
#[test]
fn multibyte_code_page_switches_flush_pending_bytes() {
    // Japanese (932) → ASCII → Cyrillic (`\fcharset204` with `\cpg1251`): each run decodes in
    // its own code page, the last bytes before the switch included.
    let f = parse(
        br"{\rtf1\ansi\ansicpg932\deff0{\fonttbl{\f0 MS Mincho;}{\f1\fcharset204\cpg1251 Arial CYR;}}\f0 \'93\'fa\'96\'7b\'8c\'ea mixed \f1 \'cf\'f0\'e8\'e2\'e5\'f2\par}",
    )
    .unwrap();
    assert_eq!(texts(&f), ["Normal:日本語 mixed Привет"]);
    // A group end flushes: the group's 932 bytes decode before the outer 1252 text.
    let f = parse(br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Arial;}{\f1\fcharset128 MS Mincho;}}\f0 {\f1 \'93\'fa\'96\'7b\'8c\'ea}caf\'e9\par}")
        .unwrap();
    assert_eq!(texts(&f), ["Normal:日本語café"]);
    // An `\ansicpg` switch mid-document flushes too.
    let f = parse(br"{\rtf1\ansi\ansicpg932 \'93\'fa\ansicpg1251 \'cf\'f0\'e8\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:日При"]);
    // A run left open at the end of the document isn't lost.
    let f = parse(b"{\\rtf1\\ansi\\ansicpg932 \x93\xfa").unwrap();
    assert_eq!(texts(&f), ["Normal:日"]);
}

/// A Windows-1252 document (the default) reads exactly as before: high bytes one at a time.
#[test]
fn windows_1252_text_is_unchanged() {
    let f = parse(b"{\\rtf1\\ansi\\ansicpg1252 caf\xe9 na\xefve \x93quoted\x94 \\'80 5\\'b0\\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:café naïve “quoted” € 5°"]);
    // No `\ansicpg`: the default is 1252; the byte values of an East Asian code page keep
    // their 1252 meaning.
    let f = parse(b"{\\rtf1\\ansi \xe9\\'92\\'93 \\'fa\\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:é’“ ú"]);
}

#[test]
fn shppict_imports_one_image_and_skips_legacy_duplicate() -> Result<(), String> {
    let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([40, 80, 160, 255]));
    let mut png = Vec::new();
    pixels.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let hex: String = png.iter().map(|b| format!("{b:02x}")).collect();
    let pict = format!("{{\\pict\\pngblip\\picwgoal1440\\pichgoal720 {hex}}}");
    // Also use a supported PNG as the legacy duplicate: it must still be skipped.
    for legacy in [format!("{{\\pict\\wmetafile8 {hex}}}"), pict.clone()] {
        for picture in [format!("{{\\*\\shppict{pict}}}"), pict.clone()] {
            let f = parse(format!("{{\\rtf1\\ansi Before{picture}{{\\nonshppict{legacy}}}After\\par}}").as_bytes())?;
            let images: Vec<_> = f
                .blocks
                .iter()
                .filter_map(|b| match b {
                    FBlock::Para(p) => Some(&p.inlines),
                    FBlock::Table(_) => None,
                })
                .flatten()
                .filter_map(|i| match i {
                    Inline::Image(img) => Some(img),
                    _ => None,
                })
                .collect();
            assert_eq!(images.len(), 1, "{picture}");
            let img = images.first().ok_or("missing picture")?;
            assert_eq!(img.data.as_ref(), &png);
            assert_eq!(img.ext, "png");
            assert_eq!((img.w, img.h), (72.0, 36.0));
            assert_eq!(texts(&f), vec!["Normal:BeforeAfter"]);
        }
    }
    // Recognizing shppict must not resurrect pictures inside skipped destinations.
    for dest in ["nonshppict", "unknown"] {
        let f = parse(format!("{{\\rtf1{{\\*\\{dest}{{\\*\\shppict{pict}}}}}Before\\par}}").as_bytes())?;
        assert_eq!(texts(&f), vec!["Normal:Before"]);
        assert!(f.blocks.iter().all(|b| match b {
            FBlock::Para(p) => p.inlines.iter().all(|i| !matches!(i, Inline::Image(_))),
            FBlock::Table(_) => false,
        }));
    }
    Ok(())
}

#[test]
fn unicode_in_info_bookmarks_and_field_instructions() {
    // #435: `\uN` (with its `\ucN` fallback skipped) decodes in every text destination.
    let rtf = br#"{\rtf1\ansi\uc1{\info{\title Caf\u233?}{\author Caf\u233\'3f}}{\*\bkmkstart caf\u233?}{\*\bkmkend caf\u233?}{\field{\*\fldinst{HYPERLINK "https://example.test/caf\u233?"}}{\fldrslt{link}}}\par Caf\u233?\par}"#;
    let f = parse(rtf).unwrap();
    assert_eq!(f.meta.title, "Café");
    assert_eq!(f.meta.author, "Café");
    let inl: Vec<&Inline> = f.blocks.iter().flat_map(|b| if let FBlock::Para(p) = b { p.inlines.iter().collect() } else { vec![] }).collect();
    assert!(inl.iter().any(|i| matches!(i, Inline::Anchor(n) if n == "café")), "{inl:?}");
    assert!(inl.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "link" && f.link.as_deref() == Some("https://example.test/café"))), "{inl:?}");
    assert_eq!(texts(&f).last().map(String::as_str), Some("Normal:Café"));
}

/// A hand-written RTF with tracked changes: Alice inserts "added ", Bob deletes "gone", and
/// "both" is inserted by Alice then deleted by Bob.
pub(crate) const TRACKED: &[u8] = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}{\*\revtbl {Unknown;}{Alice;}{Bob;}}
\pard Keep {\revised\revauth1\revdttm-1480361314 added }text{\deleted\revauthdel2\revdttmdel-2016341499  gone} end{\revised\revauth1\revdttm-1480361314\deleted\revauthdel2\revdttmdel-2016341499  both}.\par}";

/// (text, insertion (author, date), deletion (author, date)) of each run of the first paragraph.
#[allow(clippy::type_complexity)]
fn tracked_runs(doc: &Document) -> Vec<(String, Option<(String, String)>, Option<(String, String)>)> {
    let rev = |i: Option<u32>, kind: wordcraft_doc::RevisionKind| {
        i.map(|i| {
            let r = doc.revisions.get(i as usize).unwrap();
            assert_eq!(r.kind, kind);
            (r.author.clone(), r.date.clone())
        })
    };
    let Some(wordcraft_doc::Block::Para(p)) = doc.body.first().map(|b| &**b) else { panic!("no paragraph") };
    p.run_ranges()
        .map(|(r, cp)| (p.text[r].to_string(), rev(cp.ins, wordcraft_doc::RevisionKind::Insert), rev(cp.del, wordcraft_doc::RevisionKind::Delete)))
        .collect()
}

fn expected_tracked() -> Vec<(String, Option<(String, String)>, Option<(String, String)>)> {
    let a = Some(("Alice".to_string(), "2024-03-15T10:30:00Z".to_string()));
    let b = Some(("Bob".to_string(), "2025-01-02T08:05:00Z".to_string()));
    vec![
        ("Keep ".into(), None, None),
        ("added ".into(), a.clone(), None),
        ("text".into(), None, None),
        (" gone".into(), None, b.clone()),
        (" end".into(), None, None),
        (" both".into(), a, b),
        (".".into(), None, None),
    ]
}

#[test]
fn tracked_changes_import_with_authors_and_dates() {
    let doc = import(TRACKED).unwrap();
    assert_eq!(tracked_runs(&doc), expected_tracked());
    // One revision per (kind, author, date).
    assert_eq!(doc.revisions.len(), 2);
}

#[test]
fn tracked_changes_survive_export_and_import() {
    let doc = import(TRACKED).unwrap();
    let rtf = export(&doc);
    assert!(rtf.contains("{\\*\\revtbl {Unknown;}{Alice;}{Bob;}}"), "{rtf}");
    assert!(rtf.contains("\\deleted\\revauthdel2\\revdttmdel-2016341499"), "{rtf}");
    let back = import(rtf.as_bytes()).unwrap();
    assert_eq!(tracked_runs(&back), expected_tracked());
}

#[test]
fn plain_text_export_still_drops_deleted_text() {
    // Only RTF keeps the deletions: the shared flow for other formats leaves them out.
    let doc = import(TRACKED).unwrap();
    let flow = model::from_doc(&doc);
    assert_eq!(texts(&flow), vec!["Normal:Keep added text end."]);
}

#[test]
fn dttm_converts_to_and_from_iso() {
    assert_eq!(dttm_to_iso(-1480361314i32 as u32).as_deref(), Some("2024-03-15T10:30:00Z"));
    assert_eq!(dttm_to_iso(-2016341499i32 as u32).as_deref(), Some("2025-01-02T08:05:00Z"));
    assert_eq!(iso_to_dttm("2024-03-15T10:30:00Z"), Some(-1480361314i32 as u32));
    assert_eq!(iso_to_dttm("2025-01-02T08:05:59.123+02:00"), Some(-2016341499i32 as u32));
    assert_eq!(iso_to_dttm("2000-02-29T00:00:00Z").and_then(dttm_to_iso).as_deref(), Some("2000-02-29T00:00:00Z"));
    // Zero, out-of-range fields and impossible days are no date.
    assert_eq!(dttm_to_iso(0), None);
    assert_eq!(dttm_to_iso(13 << 16 | 1 << 11 | 100 << 20), None);
    assert_eq!(dttm_to_iso(2 << 16 | 30 << 11 | 101 << 20), None);
    assert_eq!(dttm_to_iso(1 << 16 | 1 << 11 | 100 << 20 | 60), None);
    assert_eq!(iso_to_dttm(""), None);
    assert_eq!(iso_to_dttm("1899-12-31T00:00:00Z"), None);
    assert_eq!(iso_to_dttm("2024-02-30T00:00:00Z"), None);
    assert_eq!(iso_to_dttm("2024-1é-01T00:00:00Z"), None);
    assert_eq!(iso_to_dttm("2024-03-15"), iso_to_dttm("2024-03-15T00:00:00Z"));
}

#[test]
fn hostile_revision_tables_and_indices() {
    // An author index past the table, a negative one, and a malformed table.
    let rtf = br"{\rtf1{\*\revtbl {Unknown;}{Ann}{;}{\b;}}{\revised\revauth99999999 a}{\deleted\revauthdel-5\revdttmdel4294967295 b}{\revised\revauth1 c}\par}";
    let doc = import(rtf).unwrap();
    let runs = tracked_runs(&doc);
    assert_eq!(runs.first().and_then(|r| r.1.clone()), Some(("Unknown".to_string(), String::new())));
    assert_eq!(runs.get(1).and_then(|r| r.2.clone()), Some(("Unknown".to_string(), String::new())));
    assert_eq!(runs.get(2).and_then(|r| r.1.clone()), Some(("Ann".to_string(), String::new())));
    // A huge table is capped, and indices past the cap are "Unknown".
    let mut rtf = String::from("{\\rtf1{\\*\\revtbl ");
    for i in 0..20_000 {
        rtf.push_str(&format!("{{A{i};}}"));
    }
    rtf.push_str("}{\\revised\\revauth15000 x}{\\revised\\revauth9999 y}\\par}");
    let doc = import(rtf.as_bytes()).unwrap();
    let runs = tracked_runs(&doc);
    assert_eq!(runs.first().and_then(|r| r.1.clone()).map(|a| a.0).as_deref(), Some("Unknown"));
    assert_eq!(runs.get(1).and_then(|r| r.1.clone()).map(|a| a.0).as_deref(), Some("A9999"));
    // Unterminated table and revision words outside the body never panic.
    for bad in [&br"{\rtf1{\*\revtbl {Al"[..], br"{\rtf1{\*\revtbl \revised\deleted{x}}y\par}", br"{\rtf1\revauth\revdttm\deleted"] {
        let _ = import(bad);
    }
}

#[test]
fn unicode_author_names_survive_import_and_round_trip() {
    // `\uN` in the author table: "José" and "Дмитрий" (not "Jos?" / "???????").
    let rtf = br"{\rtf1\uc1{\*\revtbl {Unknown;}{Jos\u233?;}{\u1044?\u1084?\u1080?\u1090?\u1088?\u1080?\u1081?;}}{\revised\revauth1 a}{\deleted\revauthdel2 b}\par}";
    let authors = |d: &Document| tracked_runs(d).into_iter().map(|r| (r.1.map(|a| a.0), r.2.map(|a| a.0))).collect::<Vec<_>>();
    let want = vec![(Some("José".to_string()), None), (None, Some("Дмитрий".to_string()))];
    let doc = import(rtf).unwrap();
    assert_eq!(authors(&doc), want);
    let back = import(export(&doc).as_bytes()).unwrap();
    assert_eq!(authors(&back), want);
}

#[test]
fn pard_keeps_revision_marks() {
    // Revision marks are character formatting: `\pard` resets paragraph formatting only.
    let rtf = br"{\rtf1{\*\revtbl {Unknown;}{Bob;}}\deleted\revauthdel1 gone\par\pard more\par}";
    let doc = import(rtf).unwrap();
    let Some(wordcraft_doc::Block::Para(p)) = doc.body.get(1).map(|b| &**b) else { panic!("no second paragraph") };
    assert_eq!(p.text, "more");
    let del = p.props_of_char(0).del.and_then(|i| doc.revisions.get(i as usize));
    assert_eq!(del.map(|r| (r.kind, r.author.as_str())), Some((wordcraft_doc::RevisionKind::Delete, "Bob")));
}

/// The body's paragraphs.
fn doc_paras(doc: &Document) -> Vec<&wordcraft_doc::Paragraph> {
    doc.body
        .iter()
        .filter_map(|b| match &**b {
            wordcraft_doc::Block::Para(p) => Some(p),
            _ => None,
        })
        .collect()
}

/// A document of `paras` with `revisions`.
fn doc_with(paras: Vec<wordcraft_doc::Paragraph>, revisions: Vec<wordcraft_doc::Revision>) -> Document {
    let mut doc = Document::new();
    doc.body = paras.into_iter().map(wordcraft_doc::para_block).collect();
    doc.revisions = revisions;
    doc
}

/// Kind, author and date of revision `i`.
fn rev_of(doc: &Document, i: Option<u32>) -> Option<(wordcraft_doc::RevisionKind, String, String)> {
    i.and_then(|i| doc.revisions.get(i as usize)).map(|r| (r.kind, r.author.clone(), r.date.clone()))
}

#[test]
fn character_formatting_change_round_trips() {
    use wordcraft_doc::props::{CharProps, PropChange};
    use wordcraft_doc::{Revision, RevisionKind};
    // "Bold" was made bold by Alice; before, it was plain italic.
    let old = CharProps { italic: Some(true), ..Default::default() };
    let cp = CharProps { bold: Some(true), fmt_change: PropChange::boxed(0, old), ..Default::default() };
    let doc =
        doc_with(vec![wordcraft_doc::Paragraph::with_text("Bold", cp)], vec![Revision::new(RevisionKind::Format, "Alice", "2024-03-15T10:30:00Z")]);
    let rtf = export(&doc);
    assert!(rtf.contains("\\crauth1\\crdate-1480361314"), "{rtf}");
    assert!(rtf.contains("{\\*\\oldcprops"), "{rtf}");
    let back = import(rtf.as_bytes()).unwrap();
    let paras = doc_paras(&back);
    let p = paras.first().unwrap();
    assert_eq!(p.text, "Bold");
    let cp = p.props_of_char(0);
    assert_eq!(cp.bold, Some(true));
    let change = cp.fmt_change.as_deref().expect("formatting change read back");
    assert_eq!(rev_of(&back, Some(change.rev)), Some((RevisionKind::Format, "Alice".into(), "2024-03-15T10:30:00Z".into())));
    assert_eq!(change.old.italic, Some(true));
    assert_ne!(change.old.bold, Some(true));
}

#[test]
fn paragraph_formatting_change_round_trips() {
    use wordcraft_doc::props::{CharProps, ParaProps, PropChange};
    use wordcraft_doc::{Align, Revision, RevisionKind};
    // Bob centred a right-aligned paragraph.
    let mut p = wordcraft_doc::Paragraph::with_text("Centred", CharProps::default());
    p.props.align = Some(Align::Center);
    p.props.fmt_change = PropChange::boxed(0, ParaProps { align: Some(Align::Right), ..Default::default() });
    let doc = doc_with(vec![p], vec![Revision::new(RevisionKind::Format, "Bob", "2025-01-02T08:05:00Z")]);
    let rtf = export(&doc);
    assert!(rtf.contains("{\\*\\oldpprops"), "{rtf}");
    assert!(rtf.contains("\\prauth1\\prdate-2016341499"), "{rtf}");
    let back = import(rtf.as_bytes()).unwrap();
    let paras = doc_paras(&back);
    let p = paras.first().unwrap();
    assert_eq!(p.text, "Centred");
    assert_eq!(p.props.align, Some(Align::Center));
    let change = p.props.fmt_change.as_deref().expect("paragraph formatting change read back");
    assert_eq!(rev_of(&back, Some(change.rev)), Some((RevisionKind::Format, "Bob".into(), "2025-01-02T08:05:00Z".into())));
    assert_eq!(change.old.align, Some(Align::Right));
}

#[test]
fn paragraph_mark_revisions_round_trip() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Revision, RevisionKind};
    // Bob deleted the mark after "One" (joining it to "Two"); Alice inserted the one after "Two".
    let mut one = wordcraft_doc::Paragraph::with_text("One", CharProps::default());
    one.mark.del = Some(0);
    let mut two = wordcraft_doc::Paragraph::with_text("Two", CharProps::default());
    two.mark.ins = Some(1);
    let three = wordcraft_doc::Paragraph::with_text("Three", CharProps::default());
    let doc = doc_with(
        vec![one, two, three],
        vec![
            Revision::new(RevisionKind::Delete, "Bob", "2025-01-02T08:05:00Z"),
            Revision::new(RevisionKind::Insert, "Alice", "2024-03-15T10:30:00Z"),
        ],
    );
    let rtf = export(&doc);
    let back = import(rtf.as_bytes()).unwrap();
    let paras = doc_paras(&back);
    let texts: Vec<&str> = paras.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(texts.get(..3), Some(&["One", "Two", "Three"][..]), "{rtf}");
    let marks: Vec<_> = paras.iter().take(3).map(|p| (rev_of(&back, p.mark.ins), rev_of(&back, p.mark.del))).collect();
    assert_eq!(
        marks,
        vec![
            (None, Some((RevisionKind::Delete, "Bob".into(), "2025-01-02T08:05:00Z".into()))),
            (Some((RevisionKind::Insert, "Alice".into(), "2024-03-15T10:30:00Z".into())), None),
            (None, None),
        ],
        "{rtf}"
    );
}

#[test]
fn formatting_revisions_read_as_the_spec_writes_them() {
    use wordcraft_doc::{Align, RevisionKind};
    // RTF 1.9.1 "Revision Marks": `\crauth`/`\crdate` with `{\*\oldcprops …}` on a run,
    // `\prauth`/`\prdate` with `{\*\oldpprops …}` on a paragraph, and a `\par` inside a
    // `\revised` group.
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\revtbl {Unknown;}{Alice;}{Bob;}}
\pard\qc\prauth2\prdate-2016341499{\*\oldpprops \pard\ql}Plain {\b\crauth1\crdate-1480361314{\*\oldcprops \plain\i}bold}\par
\pard Next{\revised\revauth1\revdttm-1480361314 \par}After\par}";
    let doc = import(rtf).unwrap();
    let paras = doc_paras(&doc);
    let texts: Vec<&str> = paras.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(texts.get(..3), Some(&["Plain bold", "Next", "After"][..]));
    let p = paras.first().unwrap();
    assert_eq!(p.props.align, Some(Align::Center));
    let pc = p.props.fmt_change.as_deref().expect("paragraph change");
    assert_eq!(rev_of(&doc, Some(pc.rev)), Some((RevisionKind::Format, "Bob".into(), "2025-01-02T08:05:00Z".into())));
    assert_eq!(pc.old.align, Some(Align::Left));
    assert!(p.props_of_char(0).fmt_change.is_none());
    let cp = p.props_of_char(6);
    assert_eq!(cp.bold, Some(true));
    let cc = cp.fmt_change.as_deref().expect("character change");
    assert_eq!(rev_of(&doc, Some(cc.rev)), Some((RevisionKind::Format, "Alice".into(), "2024-03-15T10:30:00Z".into())));
    assert_eq!(cc.old.italic, Some(true));
    assert_ne!(cc.old.bold, Some(true));
    let next = paras.get(1).unwrap();
    assert_eq!(rev_of(&doc, next.mark.ins), Some((RevisionKind::Insert, "Alice".into(), "2024-03-15T10:30:00Z".into())));
    assert!(paras.first().unwrap().mark.ins.is_none());
    assert!(paras.get(2).unwrap().mark.ins.is_none());
}

#[test]
fn list_item_with_a_formatting_change_keeps_its_list_on_reject() {
    use model::{FmtChange, ListInfo};
    // A numbered item Alice centred (it was left-aligned), and a second plain item.
    let li = Some(ListInfo { ordered: true, level: 0, start: 1 });
    let mark = RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
    let old = Para { list: li, align: Some(Align::Left), ..Default::default() };
    let mut item = Para { list: li, align: Some(Align::Center), fmt_change: Some(Box::new(FmtChange { mark, old })), ..Default::default() };
    item.push_text("Item", &Fmt::default());
    let mut next = Para { list: li, ..Default::default() };
    next.push_text("Next", &Fmt::default());
    let flow = Flow { blocks: vec![FBlock::Para(item), FBlock::Para(next)], meta: Meta::default() };
    let check = |doc: &Document, what: &str| {
        let paras = doc_paras(doc);
        let p = paras.first().unwrap();
        assert!(p.props.numbering.is_some(), "{what}: the item is in a list");
        let change = p.props.fmt_change.as_deref().unwrap_or_else(|| panic!("{what}: change kept"));
        assert_eq!(change.old.align, Some(Align::Left), "{what}");
        // Reject puts the old properties back: the item stays in its list, with its style.
        let rejected = change.old.formatting();
        assert_eq!(rejected.numbering, p.props.numbering, "{what}");
        assert_eq!(rejected.style, p.props.style, "{what}");
    };
    let doc = model::to_doc(&flow);
    check(&doc, "flow to document");
    let rtf = export(&doc);
    check(&import(rtf.as_bytes()).unwrap(), &format!("RTF round trip\n{rtf}"));
}

#[test]
fn paragraph_that_was_a_list_item_gets_its_old_list_back_on_reject() {
    use model::{FmtChange, ListInfo};
    // Alice took the paragraph out of a numbered list: it is no list item now, but was one.
    let mark = RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
    let old = Para { list: Some(ListInfo { ordered: true, level: 0, start: 1 }), ..Default::default() };
    let mut item = Para { fmt_change: Some(Box::new(FmtChange { mark, old })), ..Default::default() };
    item.push_text("Item", &Fmt::default());
    let flow = Flow { blocks: vec![FBlock::Para(item)], meta: Meta::default() };
    let doc = model::to_doc(&flow);
    let paras = doc_paras(&doc);
    let p = paras.first().unwrap();
    assert!(p.props.numbering.is_none());
    let change = p.props.fmt_change.as_deref().expect("change kept");
    assert!(change.old.formatting().numbering.is_some(), "Reject restores the list");
}

#[test]
fn numbered_item_made_bulleted_gets_its_numbered_list_back_on_reject() {
    use model::{FmtChange, ListInfo};
    let mark = RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
    let old = Para { list: Some(ListInfo { ordered: true, level: 0, start: 1 }), ..Default::default() };
    let mut item = Para {
        list: Some(ListInfo { ordered: false, level: 0, start: 1 }),
        fmt_change: Some(Box::new(FmtChange { mark, old })),
        ..Default::default()
    };
    item.push_text("Item", &Fmt::default());
    let flow = Flow { blocks: vec![FBlock::Para(item)], meta: Meta::default() };
    let doc = model::to_doc(&flow);
    let paras = doc_paras(&doc);
    let p = paras.first().unwrap();
    let change = p.props.fmt_change.as_deref().expect("change kept");
    let (now, was) = (p.props.numbering.unwrap(), change.old.formatting().numbering.unwrap());
    assert_ne!(now.num, was.num, "the old list is a list of its own");
}

#[test]
fn heading_formatting_change_has_no_style_formatting_in_the_old_formatting() {
    // A heading's runs repeat its style's bold and size; the old formatting does too.
    let rtf = br"{\rtf1{\stylesheet{\s0 Normal;}{\s1 heading 1;}}{\*\revtbl {Unknown;}{Alice;}}
\pard\s1\b\fs32 {\i\crauth1{\*\oldcprops \plain\b\fs32}Head}\par}";
    let doc = import(rtf).unwrap();
    let paras = doc_paras(&doc);
    let cp = paras.first().unwrap().props_of_char(0);
    let old = cp.fmt_change.as_deref().map(|c| c.old.clone()).expect("change kept");
    assert_eq!((old.bold, old.size), (cp.bold, cp.size));
}

#[test]
fn word_oldcprops_hold_only_the_changed_properties() {
    use wordcraft_doc::RevisionKind;
    // Word for Mac 16.113 saving tracked formatting changes as RTF: "beta" made bold,
    // "gamma" (bold) grown from 12 to 16 pt. `\oldcprops` holds only what changed, with
    // `\crauth`/`\crdate` inside the group; the rest of the old formatting is the run's.
    let rtf = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}{\f1 Arial;}}{\*\revtbl {Unknown;}{Alice;}}
\pard\plain \f1\fs24 alpha {\f1\fs24\b\insrsid12079894\charrsid12079894 {\*\oldcprops \b0\crauth1\crdate-2014675576\insrsid12079894\charrsid12079894}beta} {\b\f1\fs32 {\*\oldcprops \fs24\crauth1\crdate-2014675576\insrsid12079894\charrsid12079894}gamma}\par}";
    let doc = import(rtf).unwrap();
    let paras = doc_paras(&doc);
    let p = paras.first().unwrap();
    assert_eq!(p.text, "alpha beta gamma");
    let word_rev = Some((RevisionKind::Format, "Alice".to_string(), "2026-10-15T22:08:00Z".to_string()));
    // beta: Reject takes the bold off; font and size stay.
    let beta = p.props_of_char(6);
    assert_eq!(beta.bold, Some(true));
    let old = beta.fmt_change.as_deref().expect("beta's change");
    assert_eq!(rev_of(&doc, Some(old.rev)), word_rev);
    assert_ne!(old.old.bold, Some(true), "{:?}", old.old);
    assert_eq!((old.old.font.as_deref(), old.old.size), (beta.font.as_deref(), Some(12.0)), "{:?}", old.old);
    // gamma: Reject puts 12 pt back; bold and font stay.
    let gamma = p.props_of_char(11);
    assert_eq!(gamma.size, Some(16.0));
    let old = gamma.fmt_change.as_deref().expect("gamma's change");
    assert_eq!(rev_of(&doc, Some(old.rev)), word_rev);
    assert_eq!((old.old.bold, old.old.font.as_deref(), old.old.size), (Some(true), gamma.font.as_deref(), Some(12.0)), "{:?}", old.old);
    assert_eq!(gamma.font.as_deref(), Some("Arial"));
}

#[test]
fn word_oldpprops_hold_the_full_old_paragraph_formatting() {
    use wordcraft_doc::{Align, RevisionKind};
    // Word for Mac 16.113: a paragraph centred with tracking on. `\oldpprops` is the full
    // old set from `\pard`; `\prauth`/`\prdate` are on the paragraph.
    let rtf = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}}{\*\revtbl {Unknown;}{Alice;}}
\pard\plain \ltrpar\qc \li0\ri0\sa160\sl278\slmult1\widctlpar\wrapdefault\aspalpha\aspnum\faauto\adjustright\rin0\lin0\itap0\prauth1\prdate-2014675576{\*\oldpprops \pard \ltrpar\ql \li0\ri0\sa160\sl278\slmult1\widctlpar\wrapdefault\aspalpha\aspnum\faauto\adjustright\rin0\lin0\itap0\pararsid12079894}\fs24 Centred\par}";
    let doc = import(rtf).unwrap();
    let paras = doc_paras(&doc);
    let p = paras.first().unwrap();
    assert_eq!(p.text, "Centred");
    assert_eq!(p.props.align, Some(Align::Center));
    let c = p.props.fmt_change.as_deref().expect("paragraph change");
    assert_eq!(rev_of(&doc, Some(c.rev)), Some((RevisionKind::Format, "Alice".into(), "2026-10-15T22:08:00Z".into())));
    assert_eq!(c.old.align, Some(Align::Left));
    assert_eq!(c.old.space_after, p.props.space_after);
    assert_eq!(c.old.line_spacing, p.props.line_spacing);
    assert!(p.props.space_after.is_some());
}

#[test]
fn oldcprops_are_written_as_the_differences() {
    use wordcraft_doc::props::{CharProps, PropChange};
    use wordcraft_doc::{Revision, RevisionKind};
    let rev = || vec![Revision::new(RevisionKind::Format, "Alice", "2024-03-15T10:30:00Z")];
    // Made bold: the group holds `\b0` and who changed it, nothing else.
    let old = CharProps { font: Some("Arial".into()), size: Some(14.0), ..Default::default() };
    let cp =
        CharProps { bold: Some(true), font: Some("Arial".into()), size: Some(14.0), fmt_change: PropChange::boxed(0, old), ..Default::default() };
    let rtf = export(&doc_with(vec![wordcraft_doc::Paragraph::with_text("Bold", cp)], rev()));
    assert!(rtf.contains("{\\*\\oldcprops \\b0\\crauth1\\crdate-1480361314}"), "{rtf}");
    // Several changes, on and off, round-trip.
    let red = wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(255, 0, 0));
    let old = CharProps {
        italic: Some(true),
        underline: Some(wordcraft_doc::props::Underline::Single),
        size: Some(14.0),
        color: Some(red),
        font: Some("Arial".into()),
        ..Default::default()
    };
    let cp = CharProps {
        bold: Some(true),
        size: Some(16.0),
        font: Some("Courier New".into()),
        fmt_change: PropChange::boxed(0, old.clone()),
        ..Default::default()
    };
    let rtf = export(&doc_with(vec![wordcraft_doc::Paragraph::with_text("Mixed", cp)], rev()));
    let back = import(rtf.as_bytes()).unwrap();
    let paras = doc_paras(&back);
    let cp = paras.first().unwrap().props_of_char(0);
    assert_eq!((cp.bold, cp.size), (Some(true), Some(16.0)), "{rtf}");
    let got = &cp.fmt_change.as_deref().expect("change read back").old;
    assert_ne!(got.bold, Some(true), "{rtf}");
    assert_eq!(
        (got.italic, got.underline, got.size, got.color, got.font.as_deref()),
        (old.italic, old.underline, old.size, old.color, old.font.as_deref()),
        "{rtf}"
    );
}

#[test]
fn old_formatting_in_styled_paragraphs_is_the_styles() {
    use model::{FmtChange, Kind};
    let mark = || RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None };
    // A heading 2 run grown to 20 pt and a code run grown to 14 pt, both from no direct
    // formatting: Reject gives back the style's size, not RTF's default 12 pt. (A code run's
    // own font doesn't reach the flow from a document, so the code font is checked below.)
    let mut head = Para { kind: Kind::Heading(2), ..Default::default() };
    head.push_text(
        "Head",
        &Fmt { size: Some(20.0), fmt_change: Some(Box::new(FmtChange { mark: mark(), old: Fmt::default() })), ..Default::default() },
    );
    let mut code = Para { kind: Kind::Code, ..Default::default() };
    code.push_text(
        "let x",
        &Fmt { size: Some(14.0), fmt_change: Some(Box::new(FmtChange { mark: mark(), old: Fmt::default() })), ..Default::default() },
    );
    let flow = Flow { blocks: vec![FBlock::Para(head), FBlock::Para(code)], meta: Meta::default() };
    let rtf = export(&model::to_doc(&flow));
    let back = parse(rtf.as_bytes()).unwrap();
    let runs: Vec<&Fmt> = back
        .blocks
        .iter()
        .filter_map(|b| match b {
            FBlock::Para(p) => p.inlines.first().and_then(|i| if let Inline::Text(_, f) = i { Some(f) } else { None }),
            _ => None,
        })
        .collect();
    let head = runs.first().expect("heading run");
    // The import takes a heading's direct size as the style's (as on main); the old size
    // written is the style's, so Reject gives it whichever way the import reads it.
    assert!(rtf.contains("{\\*\\oldcprops \\fs32\\crauth1"), "{rtf}");
    let old = &head.fmt_change.as_deref().expect("heading change").old;
    assert!(old.size.is_none_or(|s| s == 16.0), "{old:?}\n{rtf}");
    let code = runs.get(1).expect("code run");
    assert!(rtf.contains("{\\*\\oldcprops \\fs20\\crauth1"), "{rtf}");
    let old = &code.fmt_change.as_deref().expect("code change").old;
    assert!(old.size.is_none_or(|s| s == 10.0), "{old:?}\n{rtf}");
    assert!(old.font.is_none() || old.code, "{old:?}\n{rtf}");
}

#[test]
fn old_formatting_in_a_code_paragraph_has_the_code_font() {
    use model::{FmtChange, Kind};
    // A code run set in Arial from no direct font: the old font written is the code font.
    let mark = RevMark { author: "Alice".into(), date: String::new(), id: None };
    let mut p = Para { kind: Kind::Code, ..Default::default() };
    p.push_text("x", &Fmt { font: Some("Arial".into()), fmt_change: Some(Box::new(FmtChange { mark, old: Fmt::default() })), ..Default::default() });
    let mut w = Writer {
        t: Tables { fonts: vec!["Aptos".into()], colors: Vec::new() },
        lists: Vec::new(),
        authors: None,
        base: Fmt::default(),
        sections: Vec::new(),
        sect_i: 0,
    };
    let mut out = String::new();
    w.para(&p, false, None, "\\par", &mut out);
    let mono = w.t.font(model::MONO_FONT);
    assert!(out.contains(&format!("{{\\*\\oldcprops \\f{mono}\\crauth1}}")), "{out}");
}

/// The text runs of `rtf`'s paragraphs, in order, with their formatting.
fn text_runs(rtf: &[u8]) -> Vec<(String, Fmt)> {
    let f = parse(rtf).unwrap();
    f.blocks
        .iter()
        .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
        .flat_map(|p| p.inlines.iter())
        .filter_map(|i| if let Inline::Text(t, f) = i { Some((t.clone(), f.clone())) } else { None })
        .collect()
}

/// The formatting of the run whose text is `t`.
fn run_fmt<'a>(runs: &'a [(String, Fmt)], t: &str) -> &'a Fmt {
    runs.iter().find(|r| r.0 == t).map(|r| &r.1).unwrap_or_else(|| panic!("no run {t:?} in {runs:?}"))
}

#[test]
fn heading_and_code_runs_take_their_style_formatting_on_import() {
    // A stylesheet entry without formatting words (WordCraft's older files, minimal
    // stylesheets): WordCraft's own heading and code formatting stand in for the style's.
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}{\f1 Arial;}{\f2 Courier New;}}{\stylesheet{\s0 Normal;}{\s2 heading 2;}{\s9 Code;}}
\pard\s2\b\fs32 Head\par
\pard\s2\b\fs26 Small\par
\pard\s9\f2\b\fs21 code\par
\pard\s9\f1\fs21 arial\par}";
    let runs = text_runs(rtf);
    let head = run_fmt(&runs, "Head");
    assert_eq!((head.size, head.bold), (None, false));
    // 13 pt isn't heading 2's 16 pt: a size set by hand.
    let small = run_fmt(&runs, "Small");
    assert_eq!((small.size, small.bold), (Some(13.0), false));
    let code = run_fmt(&runs, "code");
    assert_eq!((code.size, code.bold, code.font.as_deref(), code.code), (None, false, None, false));
    // Arial isn't the code font: set by hand.
    let arial = run_fmt(&runs, "arial");
    assert_eq!((arial.size, arial.font.as_deref(), arial.code), (None, Some("Arial"), false));
}

#[test]
fn heading_runs_are_compared_with_the_files_heading_style() {
    // Word writes resolved values: a 13 pt bold Heading 2 has `\b\fs26` in its stylesheet
    // entry and on its runs. Only what differs from the entry is direct formatting.
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}}{\stylesheet{\ql \f0\fs24 Normal;}
{\s2\ql \keepn\outlinelevel1 \b\fs26 \sbasedon0 \snext0 heading 2;}{\s3\ql \keepn\b0\fs24 \sbasedon0 \snext0 heading 3;}}
\pard\s2\b\fs26 Head\par
\pard\s2\b\fs40 Big\par
\pard\s3\b\fs24 Bolded\par}";
    let runs = text_runs(rtf);
    let head = run_fmt(&runs, "Head");
    assert_eq!((head.size, head.bold), (None, false));
    let big = run_fmt(&runs, "Big");
    assert_eq!((big.size, big.bold), (Some(20.0), false));
    // Heading 3 isn't bold in this file: the bold is the user's.
    let bolded = run_fmt(&runs, "Bolded");
    assert_eq!((bolded.size, bolded.bold), (None, true));
}

#[test]
fn style_formatting_is_resolved_through_sbasedon() {
    // LibreOffice's stylesheet: Heading 2 is based on Heading, which sets the boldness and
    // font; Code is based on a style that sets the font.
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Liberation Serif;}{\f1 Liberation Sans;}{\f2 Iosevka;}{\f3 Courier New;}}
{\stylesheet{\s0\f0\fs24 Normal;}{\s1\sbasedon0\snext2\f1\b0\fs28 Heading;}{\s2\sbasedon1\snext3\ql\fs32 Heading 2;}
{\s10\sbasedon0\f2\fs20 Source Base;}{\s9\sbasedon10 Code;}}
\pard\s2\f1\fs32 Plain\par
\pard\s2\f1\b\fs32 Bolded\par
\pard\s9\f2\fs20 base\par
\pard\s9\f3\fs20 courier\par}";
    let runs = text_runs(rtf);
    let plain = run_fmt(&runs, "Plain");
    assert_eq!((plain.size, plain.bold), (None, false));
    // Heading isn't bold, so neither is Heading 2.
    let bolded = run_fmt(&runs, "Bolded");
    assert_eq!((bolded.size, bolded.bold), (None, true));
    let base = run_fmt(&runs, "base");
    assert_eq!((base.size, base.font.as_deref(), base.code), (None, None, false));
    // Code's font is Iosevka here, so Courier New was set by hand.
    let courier = run_fmt(&runs, "courier");
    assert_eq!((courier.size, courier.font.as_deref(), courier.code), (None, None, true));
}

#[test]
fn code_runs_in_the_files_code_font_have_no_direct_font() {
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}{\f1 Courier New;}{\f2 Arial;}}{\stylesheet{\s0 Normal;}{\s9\sbasedon0\sa0\f1\fs21 Code;}}
\pard\s9\f1\fs21 mono\par
\pard\s9\f2\fs21 arial\par}";
    let runs = text_runs(rtf);
    let mono = run_fmt(&runs, "mono");
    assert_eq!((mono.size, mono.font.as_deref(), mono.code), (None, None, false));
    let arial = run_fmt(&runs, "arial");
    assert_eq!((arial.size, arial.font.as_deref(), arial.code), (None, Some("Arial"), false));
}

#[test]
fn hostile_sbasedon_chains_are_harmless() {
    let mut chain = String::new();
    // s100 (heading 5) based on s101 … s139, the last one not bold: past the bound, so
    // heading 5 takes WordCraft's bold.
    for i in 100..140 {
        let name = if i == 100 { "heading 5".to_string() } else { format!("x{i}") };
        let b0 = if i == 139 { "\\b0" } else { "" };
        chain.push_str(&format!("{{\\s{i}\\sbasedon{}{b0} {name};}}", i + 1));
    }
    let rtf = format!(
        r"{{\rtf1{{\stylesheet{{\s0 Normal;}}{{\s1\sbasedon1 heading 1;}}{{\s3\sbasedon4\b0 heading 3;}}{{\s4\sbasedon3 heading 4;}}
{{\s6\sbasedon-5 heading 6;}}{{\s2\sbasedon2147483647\fs-40 heading 2;}}{{\s-2147483648\sbasedon-2147483648 Title;}}{chain}}}
\pard\s1\b\fs40 one\par
\pard\s3\b\fs24 three\par
\pard\s4\b\fs24 four\par
\pard\s6\b\fs22 six\par
\pard\s2\b\fs32 two\par
\pard\s100\b\fs24 five\par
\pard\s-2147483648\fs56 title\par}}"
    );
    let runs = text_runs(rtf.as_bytes());
    // Self-based heading 1: WordCraft's values.
    assert_eq!((run_fmt(&runs, "one").size, run_fmt(&runs, "one").bold), (None, false));
    // A loop between heading 3 and 4: both take heading 3's `\b0`.
    assert!(run_fmt(&runs, "three").bold);
    assert!(run_fmt(&runs, "four").bold);
    assert_eq!((run_fmt(&runs, "six").size, run_fmt(&runs, "six").bold), (None, false));
    assert_eq!((run_fmt(&runs, "two").size, run_fmt(&runs, "two").bold), (None, false));
    assert!(!run_fmt(&runs, "five").bold);
    let _ = run_fmt(&runs, "title");
}

#[test]
fn a_word_365_heading_is_not_bold_so_a_bold_run_there_is_the_users() {
    // Word 365's Heading 1 isn't bold: its entry sets the font and size but no `\b`, so a
    // bold run, or a tracked change to or from bold, is the user's.
    let rtf = br"{\rtf1\ansi{\fonttbl{\f1 Aptos;}{\f2 Aptos Display;}}{\colortbl;\red15\green71\blue97;}
{\stylesheet{\ql \f1\fs24 Normal;}{\s1\ql \keepn\outlinelevel0 \f2\fs40\cf1 \sbasedon0 \snext0 heading 1;}}
{\*\revtbl {Unknown;}{Alice;}}
\pard\s1\f2\fs40 Plain\par
\pard\s1\f2\b\fs40 Bold\par
\pard\s1\f2\fs40 {\b{\*\oldcprops \b0\crauth1}Bolded}{\b0{\*\oldcprops \b\crauth1}Unbolded}\par}";
    let runs = text_runs(rtf);
    let plain = run_fmt(&runs, "Plain");
    assert_eq!((plain.size, plain.bold), (None, false));
    let bold = run_fmt(&runs, "Bold");
    assert_eq!((bold.size, bold.bold), (None, true));
    let bolded = run_fmt(&runs, "Bolded");
    let old = &bolded.fmt_change.as_deref().expect("change kept").old;
    assert_eq!((bolded.bold, old.bold, old.size), (true, false, None));
    let unbolded = run_fmt(&runs, "Unbolded");
    let old = &unbolded.fmt_change.as_deref().expect("change kept").old;
    assert_eq!((unbolded.bold, old.bold), (false, true));
}

#[test]
fn a_style_chain_without_a_size_has_rtfs_default_size() {
    // The entries carry formatting words but no `\fs`: the style's size is RTF's 12 pt.
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}}{\stylesheet{\s0\f0 Normal;}{\s2\sbasedon0\b heading 2;}}
\pard\s2\b\fs24 Twelve\par
\pard\s2\b\fs40 Twenty\par}";
    let runs = text_runs(rtf);
    let twelve = run_fmt(&runs, "Twelve");
    assert_eq!((twelve.size, twelve.bold), (None, false));
    let twenty = run_fmt(&runs, "Twenty");
    assert_eq!((twenty.size, twenty.bold), (Some(20.0), false));
}

#[test]
fn a_code_paragraph_round_trips_without_a_direct_code_font() {
    // WordCraft's Code entry has no `\f` (the code font is on the paragraph): the code font
    // stands in for the style's.
    let mut code = Para { kind: Kind::Code, ..Default::default() };
    code.push_text("let x", &Fmt::default());
    let flow = Flow { blocks: vec![FBlock::Para(code)], meta: Meta::default() };
    let rtf = export(&model::to_doc(&flow));
    let runs = text_runs(rtf.as_bytes());
    let f = run_fmt(&runs, "let x");
    assert_eq!((f.size, f.bold, f.code, f.font.as_deref()), (None, false, false, None), "{rtf}");
}

#[test]
fn a_stylesheet_entry_without_a_number_is_normal() {
    // An entry without `\sN` is style 0; it doesn't replace the entry before it.
    let rtf = br"{\rtf1\ansi{\stylesheet{\s2\b\fs26 heading 2;}{\b0\fs30 Normal;}}
\pard\s2\b\fs26 Head\par
\pard\s2\b\fs30 Other\par}";
    let runs = text_runs(rtf);
    let head = run_fmt(&runs, "Head");
    assert_eq!((head.size, head.bold), (None, false));
    let other = run_fmt(&runs, "Other");
    assert_eq!((other.size, other.bold), (Some(15.0), false));
}

#[test]
fn a_heading_size_set_by_hand_survives_in_the_old_formatting() {
    // Reject puts back a size set by hand: it isn't the file's heading size.
    let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}}{\stylesheet{\ql \fs24 Normal;}{\s2\ql \b\fs26 \sbasedon0 \snext0 heading 2;}}
{\*\revtbl {Unknown;}{Alice;}}
\pard\s2 {\b\fs26{\*\oldcprops \fs40\crauth1}Shrunk}{\b\fs40{\*\oldcprops \fs26\crauth1}Grown}\par}";
    let runs = text_runs(rtf);
    let shrunk = run_fmt(&runs, "Shrunk");
    assert_eq!(shrunk.size, None);
    let old = &shrunk.fmt_change.as_deref().expect("change kept").old;
    assert_eq!((old.size, old.bold), (Some(20.0), false));
    let grown = run_fmt(&runs, "Grown");
    assert_eq!(grown.size, Some(20.0));
    let old = &grown.fmt_change.as_deref().expect("change kept").old;
    assert_eq!((old.size, old.bold), (None, false));
}

#[test]
fn pard_inside_oldcprops_keeps_the_words_before_it() {
    let doc = import(br"{\rtf1{\*\revtbl {Unknown;}{Alice;}}{\b{\*\oldcprops \i\crauth1\pard\b0}x}\par}").unwrap();
    let paras = doc_paras(&doc);
    let cp = paras.first().unwrap().props_of_char(0);
    let c = cp.fmt_change.as_deref().expect("change kept");
    assert_eq!(c.old.italic, Some(true));
    assert_ne!(c.old.bold, Some(true));
    assert_eq!(rev_of(&doc, Some(c.rev)).map(|r| r.1).as_deref(), Some("Alice"));
}

#[test]
fn a_section_break_under_a_deletion_has_a_deleted_mark() {
    let rtf = br"{\rtf1{\*\revtbl {Unknown;}{Bob;}}\deleted\revauthdel1 one\sect\pard\plain two\par}";
    let doc = import(rtf).unwrap();
    let paras = doc_paras(&doc);
    assert_eq!(paras.first().map(|p| p.text.as_str()), Some("one"));
    assert_eq!(rev_of(&doc, paras.first().and_then(|p| p.mark.del)).map(|r| r.1).as_deref(), Some("Bob"));
}

#[test]
fn hostile_formatting_revisions_never_panic() {
    let text = |rtf: &[u8]| import(rtf).map(|d| doc_paras(&d).iter().map(|p| p.text.clone()).collect::<Vec<_>>());
    // Old properties without an author: the text is kept and nothing else appears.
    assert_eq!(text(br"{\rtf1{\*\oldcprops \b}x\par}").unwrap().first().map(String::as_str), Some("x"));
    // Huge and negative indices and dates.
    let doc =
        import(br"{\rtf1{\b\crauth2147483647\crdate4294967295{\*\oldcprops \i}x}\pard\prauth-5\prdate4294967295{\*\oldpprops \qc}y\par}").unwrap();
    let paras = doc_paras(&doc);
    let author = |i: Option<u32>| rev_of(&doc, i).map(|r| r.1);
    assert_eq!(paras.first().map(|p| p.text.as_str()), Some("xy"));
    assert_eq!(author(paras.first().and_then(|p| p.props_of_char(0).fmt_change.as_deref().map(|c| c.rev))).as_deref(), Some("Unknown"));
    // Structure inside old properties is ignored: no paragraphs, cells or text come from it.
    assert_eq!(text(br"{\rtf1\pard{\*\oldpprops \pard\qc\par\cell\row\trowd\cellx100 junk}z\par}").unwrap().first().map(String::as_str), Some("z"));
    assert_eq!(text(br"{\rtf1{\b\crauth1{\*\oldcprops {\*\oldcprops \i}\par x}y}\par}").unwrap().first().map(String::as_str), Some("y"));
    // Border words in old properties don't edit the cell border being defined.
    let f = parse(br"{\rtf1\ansi\trowd\clbrdrt\brdrs\brdrw10{\*\oldpprops \brdrdb\brdrw120\brdrcf1}\cellx2000\pard\intbl a\cell\row}").unwrap();
    let FBlock::Table(t) = &f.blocks[0] else { panic!() };
    let top = t.rows[0][0].borders.and_then(|b| b.top).unwrap();
    assert_eq!((top.style, top.width, top.color), (wordcraft_doc::props::BorderStyle::Single, 0.5, None));
    // Unterminated groups and deep nesting.
    for bad in [
        &br"{\rtf1\pard\prauth1\prdate5{\*\oldpprops \pard\qc"[..],
        br"{\rtf1{\crauth1{\*\oldcprops \b",
        br"{\rtf1\crauth\crdate\prauth\prdate{\*\oldcprops}{\*\oldpprops}",
    ] {
        let _ = import(bad);
    }
    let mut deep = String::from("{\\rtf1\\crauth1");
    for _ in 0..20_000 {
        deep.push_str("{\\*\\oldcprops \\b");
    }
    deep.push_str(" x\\par}");
    let _ = import(deep.as_bytes());
}

#[test]
fn sections_keep_their_page_columns_numbering_and_headers() {
    use wordcraft_doc::section::{NumFormat, SectionStart};
    // A4 portrait → landscape restarting at 1 with its own header → continuous two columns
    // → portrait. A `\sectd` inside a footer group is the footer's, not the body's.
    let rtf = br"{\rtf1\ansi\paperw11906\paperh16838\margl1134\margr1134
\sectd\headery708\footery708
{\header \pard One head\par}
{\footer \pard Page {\field{\*\fldinst PAGE}{\fldrslt 1}}\sectd\par}
\pard First\par\sect
\sectd\lndscpsxn\pgwsxn16838\pghsxn11906\headery708\pgnrestart\pgnstarts1\pgnlcrm\titlepg
{\header \pard Two head\par}
{\headerf \pard Two first\par}
{\footer \pard Two foot\sectd\lndscpsxn\par}
\pard Second\par
\trowd\cellx2000\pard\intbl cell\cell\row
\pard{\sect}
\sectd\lndscpsxn\pgwsxn16838\pghsxn11906\headery708\sbknone\pgnrestart\cols2\colsx567\linebetcol
\pard Third\sect
\sectd\headery708\sbkodd
\pard Fourth\par}";
    let d = import(rtf).unwrap();
    let secs = d.sections();
    assert_eq!(secs.len(), 4, "{secs:?}");
    let s: Vec<&wordcraft_doc::SectionProps> = secs.iter().map(|(_, s)| *s).collect();
    // Page size and orientation; the document's margins apply to every section.
    for (i, land) in [(0, false), (1, true), (2, true), (3, false)] {
        assert_eq!(s[i].landscape, land, "section {i}");
        let (w, h) = if land { (841.9, 595.3) } else { (595.3, 841.9) };
        assert!((s[i].page_w - w).abs() < 0.1 && (s[i].page_h - h).abs() < 0.1, "section {i}: {} x {}", s[i].page_w, s[i].page_h);
        assert!((s[i].margin_left - 56.7).abs() < 0.1, "section {i}");
        assert!((s[i].header - 35.4).abs() < 0.1, "section {i}");
    }
    // Break types.
    assert_eq!(
        s.iter().map(|s| s.start).collect::<Vec<_>>(),
        [SectionStart::NextPage, SectionStart::NextPage, SectionStart::Continuous, SectionStart::OddPage]
    );
    // Columns.
    assert_eq!(s[2].columns.count, 2);
    assert!((s[2].columns.space - 28.35).abs() < 0.01 && s[2].columns.separator);
    assert_eq!(s[0].columns.count, 1);
    assert_eq!(s[3].columns.count, 1);
    // Page numbering: restart at 1 (lower roman) in 2 and 3; section 4 carries on.
    assert_eq!(s.iter().map(|s| s.page_num_start).collect::<Vec<_>>(), [None, Some(1), Some(1), None]);
    assert_eq!(s[1].page_num_format, NumFormat::LowerRoman);
    assert_eq!(s[0].page_num_format, NumFormat::Decimal);
    assert!(!s[0].title_page && s[1].title_page && !s[3].title_page);
    // Headers and footers: sections 1 and 2 have their own; 3 and 4 inherit (none set).
    let text = |id: Option<u32>| {
        let p = id.and_then(|id| d.parts.get(&id)).unwrap();
        p.blocks
            .iter()
            .filter_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p.text.clone()) } else { None })
            .collect::<Vec<_>>()
            .join("|")
    };
    assert_eq!(text(s[0].headers.default), "One head");
    assert!(text(s[0].footers.default).starts_with("Page"));
    assert_eq!(text(s[1].headers.default), "Two head");
    assert_eq!(text(s[1].headers.first), "Two first");
    assert_eq!(text(s[1].footers.default), "Two foot");
    assert!(s[0].headers.first.is_none());
    for i in [2, 3] {
        assert_eq!((s[i].headers, s[i].footers), Default::default(), "section {i}");
    }
    // Each section ends on its own paragraph, after the text it holds.
    let body_text = |i: usize| match d.body.get(i).map(|b| &**b) {
        Some(wordcraft_doc::Block::Para(p)) => p.text.clone(),
        _ => "<table>".into(),
    };
    assert_eq!(body_text(secs[0].0 - 1), "First");
    assert_eq!(body_text(secs[1].0 - 1), "<table>");
    assert_eq!(body_text(secs[2].0), "Third");
    assert_eq!(body_text(secs[3].0), "Fourth");
    // A landscape document default doesn't turn a section that gives its own portrait page.
    let d = import(br"{\rtf1\ansi\paperw16838\paperh11906\landscape\sectd a\sect\sectd\pgwsxn11906\pghsxn16838 b\par}").unwrap();
    let secs = d.sections();
    assert_eq!(secs.len(), 2);
    assert!(secs[0].1.landscape && secs[0].1.page_w > secs[0].1.page_h, "{:?}", secs[0].1);
    assert!(!secs[1].1.landscape && secs[1].1.page_w < secs[1].1.page_h, "{:?}", secs[1].1);
}

#[test]
fn export_writes_every_section() {
    use wordcraft_doc::section::{NumFormat, SectionStart};
    use wordcraft_doc::{Block, SectionProps};
    let mut d = wordcraft_doc::Document::from_text("one\ntwo\nthree");
    let s1 = SectionProps { margin_left: 90.0, header: 30.0, footer: 40.0, ..Default::default() };
    let mut s2 = SectionProps { start: SectionStart::Continuous, page_w: 792.0, page_h: 612.0, landscape: true, ..Default::default() };
    s2.columns.count = 2;
    s2.columns.space = 18.0;
    s2.columns.separator = true;
    s2.page_num_start = Some(3);
    s2.page_num_format = NumFormat::LowerRoman;
    let mut s3 = SectionProps { start: SectionStart::OddPage, title_page: true, ..Default::default() };
    s3.page_num_format = NumFormat::UpperLetter;
    for (i, s) in [(0usize, s1.clone()), (1, s2.clone())] {
        if let Some(Block::Para(p)) = d.body.get_mut(i).map(std::sync::Arc::make_mut) {
            p.section = Some(Box::new(s));
        }
    }
    d.last_section = s3.clone();
    let rtf = export(&d);
    assert_eq!(rtf.matches("\\sect\n").count() + rtf.matches("\\sect\\").count(), 2, "{rtf}");
    let back = import(rtf.as_bytes()).unwrap();
    let secs = back.sections();
    assert_eq!(secs.len(), 3, "{rtf}");
    let got: Vec<&SectionProps> = secs.iter().map(|(_, s)| *s).collect();
    for (g, want) in got.iter().zip([&s1, &s2, &s3]) {
        assert_eq!(g.start, want.start);
        assert_eq!(g.landscape, want.landscape);
        assert!((g.page_w - want.page_w).abs() < 0.1 && (g.page_h - want.page_h).abs() < 0.1);
        for (a, b) in [
            (g.margin_left, want.margin_left),
            (g.margin_right, want.margin_right),
            (g.margin_top, want.margin_top),
            (g.margin_bottom, want.margin_bottom),
        ] {
            assert!((a - b).abs() < 0.1);
        }
        assert!((g.header - want.header).abs() < 0.1 && (g.footer - want.footer).abs() < 0.1);
        assert_eq!(g.columns.count, want.columns.count);
        assert_eq!(g.columns.separator, want.columns.separator);
        assert_eq!(g.page_num_start, want.page_num_start);
        assert_eq!(g.page_num_format, want.page_num_format);
        assert_eq!(g.title_page, want.title_page);
    }
    assert!((got[1].columns.space - 18.0).abs() < 0.1);
    // Each section keeps its text.
    assert_eq!(secs.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1, 2]);
    // One section: no section break.
    let one = export(&wordcraft_doc::Document::from_text("a\nb"));
    assert!(!one.contains("\\sect\n") && !one.contains("\\sect\\"), "{one}");
}

#[test]
fn export_keeps_a_single_sections_formatting() {
    use wordcraft_doc::SectionProps;
    let mut d = wordcraft_doc::Document::from_text("a\nb");
    d.last_section = SectionProps { title_page: true, ..Default::default() };
    d.last_section.columns.count = 2;
    d.last_section.columns.space = 20.0;
    let back = import(export(&d).as_bytes()).unwrap();
    assert_eq!(back.sections().len(), 1);
    assert_eq!(back.last_section.columns.count, 2);
    assert!(back.last_section.title_page);
}

#[test]
fn export_keeps_a_section_that_ends_the_body() {
    use wordcraft_doc::{Block, SectionProps};
    let mut d = wordcraft_doc::Document::from_text("a\nb");
    let s1 = SectionProps { landscape: true, page_w: 792.0, page_h: 612.0, ..Default::default() };
    if let Some(Block::Para(p)) = d.body.get_mut(1).map(std::sync::Arc::make_mut) {
        p.section = Some(Box::new(s1));
    }
    d.last_section.columns.count = 3;
    let back = import(export(&d).as_bytes()).unwrap();
    let secs = back.sections();
    assert_eq!(secs.len(), 2);
    assert!(secs[0].1.landscape);
    assert_eq!(secs[1].1.columns.count, 3);
}

#[test]
fn export_writes_no_section_break_inside_a_table() {
    use wordcraft_doc::{Block, SectionProps};
    let mut d = wordcraft_doc::Document::from_text("a");
    let mut t = wordcraft_doc::Table::new(1, 1, 400.0);
    if let Some(Block::Para(p)) =
        t.rows.first_mut().and_then(|r| r.cells.first_mut()).and_then(|c| c.blocks.first_mut()).map(std::sync::Arc::make_mut)
    {
        p.section = Some(Box::new(SectionProps::default()));
    }
    d.body.push(std::sync::Arc::new(Block::Table(t)));
    let rtf = export(&d);
    assert!(!rtf.contains("\\sect\n") && !rtf.contains("\\sect\\") && rtf.matches("\\sectd").count() == 1, "{rtf}");
    assert_eq!(import(rtf.as_bytes()).unwrap().sections().len(), 1);
}

#[test]
fn sections_are_capped_and_a_trailing_sect_adds_no_empty_section() {
    // A file that ends with `\sect` has no empty section after it.
    let d = import(br"{\rtf1\ansi\sectd\sbknone a\sect\sectd\lndscpsxn b\sect}").unwrap();
    let secs = d.sections();
    assert_eq!(secs.len(), 2, "{secs:?}");
    assert!(secs[1].1.landscape && !secs[0].1.landscape);
    // Hostile: a flood of section breaks with headers is capped and never panics.
    let mut rtf = b"{\\rtf1\\ansi".to_vec();
    for _ in 0..(MAX_SECTIONS + 50) {
        rtf.extend_from_slice(b"\\sectd\\cols99999\\colsx-5\\pgnrestart\\pgnstarts-7{\\header h\\par}{\\footerl f\\par}x\\sect");
    }
    rtf.push(b'}');
    let d = import(&rtf).unwrap();
    let secs = d.sections();
    assert_eq!(secs.len(), MAX_SECTIONS);
    // Each section keeps its own header and footer; the ones past the cap replace the last's.
    assert!(d.parts.len() <= 2 * MAX_SECTIONS, "{}", d.parts.len());
    assert!(secs.iter().all(|(_, s)| s.columns.count <= 45 && s.columns.space >= 0.0 && s.page_num_start == Some(0)));
}

#[test]
fn rejects_non_rtf() {
    assert!(parse(b"hello").is_err());
    assert!(parse(b"{\\rtf1").is_ok());
}

#[test]
fn escapes() {
    assert_eq!(esc("a{b}\\c\té😀"), "a\\{b\\}\\\\c\\tab \\u233?\\u-10179?\\u-8704?");
    assert_eq!(esc("col1\u{000E}col2"), "col1\\column col2");
}

#[test]
fn column_break_import() {
    let f = parse(br"{\rtf1\ansi \pard a\column b\par}").unwrap();
    assert_eq!(texts(&f), ["Normal:a\u{000E}b"]);
}

#[test]
fn hyperlink_field_targets() {
    assert_eq!(hyperlink(r#"HYPERLINK "http://example.com" \l "heading""#), Some("http://example.com#heading".into()));
    assert_eq!(hyperlink(r#"HYPERLINK \l "heading""#), Some("#heading".into()));
    assert_eq!(hyperlink(r#"Hyperlink "http://example.com""#), Some("http://example.com".into()));
    assert_eq!(hyperlink(r#"HYPERLINK "http://example.com/path\latest""#), Some("http://example.com/path\\latest".into()));
    assert_eq!(hyperlink(r#"HYPERLINK "http://example.com" \o "Help text""#), Some("http://example.com".into()));
}

#[test]
fn distributed_alignment_round_trips() {
    let rtf = br"{\rtf1\ansi\pard\qd Distributed\par\pard\qj Justified\par}";
    let flow = parse(rtf).unwrap();
    let p0 = match &flow.blocks[0] {
        FBlock::Para(p) => p.align,
        _ => panic!("expected para"),
    };
    let p1 = match &flow.blocks[1] {
        FBlock::Para(p) => p.align,
        _ => panic!("expected para"),
    };
    assert_eq!(p0, Some(Align::Distribute));
    assert_eq!(p1, Some(Align::Justify));

    let doc = import(rtf).unwrap();
    let exported = export(&doc);
    assert!(exported.contains("\\qd"), "expected \\qd in: {exported}");
    assert!(exported.contains("\\qj"), "expected \\qj in: {exported}");
}

#[test]
fn table_cell_vertical_alignment() {
    let rtf = br"{\rtf1\ansi\trowd\clvertalt\cellx1000\clvertalc\cellx2000\clvertalb\cellx3000\intbl top\cell center\cell bottom\cell\row}";
    let doc = import(rtf).unwrap();
    let wordcraft_doc::Block::Table(t) = &*doc.body[0] else { panic!() };
    assert_eq!(t.rows[0].cells[0].props.valign, Some(VAlign::Top));
    assert_eq!(t.rows[0].cells[1].props.valign, Some(VAlign::Center));
    assert_eq!(t.rows[0].cells[2].props.valign, Some(VAlign::Bottom));
    let exported = export(&doc);
    assert!(exported.contains("\\clvertalt"));
    assert!(exported.contains("\\clvertalc"));
    assert!(exported.contains("\\clvertalb"));
}

#[test]
fn rtf_backslash_space_and_special_breaks() {
    let rtf = br"{\rtf1\ansi A\ B\~C\zwbo D\softline E\softpage F\par}";
    let doc = import(rtf).unwrap();
    let p = doc.body.first().unwrap().as_para().unwrap();
    assert_eq!(p.text, "A\u{00A0}B\u{00A0}C\u{200B}D\nE\u{000C}F");
}
