use super::*;

#[test]
fn escaper_drops_noncharacters() {
    assert_eq!(x("a\u{FFFF}b\u{FFFE}c\u{1}d\te"), "abcd\te");
}

#[test]
fn lengths() {
    assert_eq!(length("1in"), Some(72.0));
    assert_eq!(length("12pt"), Some(12.0));
    assert!((length("2.54cm").unwrap() - 72.0).abs() < 0.01);
    assert_eq!(length("1IN"), Some(72.0));
    assert_eq!(length("12PT"), Some(12.0));
    assert!((length("2.54CM").unwrap() - 72.0).abs() < 0.01);
    assert!((length("25.4MM").unwrap() - 72.0).abs() < 0.01);
    assert_eq!(length("1pc"), Some(12.0));
    assert_eq!(length("10PX"), Some(7.5));
    assert_eq!(length("x"), None);
}

#[test]
fn default_paragraph_style_font_becomes_document_font() {
    // LibreOffice keeps the font in the Standard style or in `style:default-style`; the import
    // used to skip both and the text fell back to the built-in body font.
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Hi</text:p></office:text></office:body></office:document-content>"#
    );
    let standard = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="DejaVu Serif"/></style:default-style><style:style style:name="Standard" style:family="paragraph"><style:text-properties style:font-name="Times New Roman"/></style:style></office:styles></office:document-styles>"#
    );
    let doc = import(&package(&content, Some(&standard))).unwrap();
    assert_eq!(doc.styles.default_chr.font.as_deref(), Some("Times New Roman"));
    let only_default = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="DejaVu Serif"/></style:default-style></office:styles></office:document-styles>"#
    );
    let doc = import(&package(&content, Some(&only_default))).unwrap();
    assert_eq!(doc.styles.default_chr.font.as_deref(), Some("DejaVu Serif"));
}

#[test]
fn rejects_garbage() {
    assert!(parse(b"not a zip").is_err());
}

/// A package holding `entries`, deflated.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        z.start_file(*name, o).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// `bytes` (a package) with the size its central directory declares for `name` set to `size`.
fn declare_size(mut bytes: Vec<u8>, name: &str, size: u32) -> Vec<u8> {
    let at = (0..bytes.len())
        .find(|&i| bytes[i..].starts_with(b"PK\x01\x02") && bytes.get(i + 46..).is_some_and(|r| r.starts_with(name.as_bytes())))
        .unwrap();
    bytes[at + 24..at + 28].copy_from_slice(&size.to_le_bytes());
    bytes
}

fn archive(bytes: &[u8]) -> zip::ZipArchive<Cursor<&[u8]>> {
    zip::ZipArchive::new(Cursor::new(bytes)).unwrap()
}

#[test]
fn pictures_are_inflated_once_and_every_use_is_charged() {
    // Every frame on a picture inflated it again, even once the budget was used up.
    let a = zip_of(&[("p.png", &[1u8; 300])]);
    let b = zip_of(&[("p.png", &[2u8; 300]), ("q.png", &[3u8; 10])]);
    let (mut za, mut zb) = (archive(&a), archive(&b));
    let mut pics = Pictures::new(1000);
    assert_eq!(pics.get(&mut za, "p.png"), Some(vec![1u8; 300]));
    // Kept from the first read: the entry isn't read again (another package's would differ).
    assert_eq!(pics.get(&mut zb, "p.png"), Some(vec![1u8; 300]));
    assert_eq!(pics.get(&mut zb, "p.png"), Some(vec![1u8; 300]));
    assert_eq!(pics.room, 100);
    // A fourth copy doesn't fit; an entry that couldn't be read isn't looked up again.
    assert_eq!(pics.get(&mut za, "p.png"), None);
    assert_eq!(pics.get(&mut za, "q.png"), None);
    assert_eq!(pics.get(&mut zb, "q.png"), None);
    assert_eq!(pics.room, 100);
}

#[test]
fn pictures_over_the_budget_are_not_inflated() {
    // An entry declaring more than is left is refused before inflating, and costs nothing.
    let pkg = zip_of(&[("big.png", &[0u8; 1200]), ("ok.png", &[5u8; 300])]);
    let mut z = archive(&pkg);
    let mut pics = Pictures::new(1000);
    assert_eq!(pics.get(&mut z, "big.png"), None);
    assert_eq!(pics.room, 1000);
    assert_eq!(pics.get(&mut z, "ok.png").map(|d| d.len()), Some(300));
    // One inflating to more than it declares stops a byte past the budget and uses it up:
    // nothing is inflated after it.
    let liar = declare_size(zip_of(&[("liar.png", &[0u8; 5000]), ("ok.png", &[5u8; 10])]), "liar.png", 10);
    let mut z = archive(&liar);
    let mut pics = Pictures::new(1000);
    assert_eq!(pics.get(&mut z, "liar.png"), None);
    assert_eq!(pics.room, 0);
    assert_eq!(pics.get(&mut z, "ok.png"), None);
}

#[test]
fn frames_sharing_a_picture_all_show_it() {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 300]].concat();
    let frames = r#"<draw:frame svg:width="1in" svg:height="1in"><draw:image xlink:href="Pictures/a.png"/></draw:frame>"#.repeat(3);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>{frames}</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = zip_of(&[("content.xml", content.as_bytes()), ("Pictures/a.png", &png)]);
    let flow = parse(&pkg).unwrap();
    let imgs: Vec<&model::Img> =
        paras(&flow).into_iter().flat_map(|p| &p.inlines).filter_map(|i| if let Inline::Image(img) = i { Some(img) } else { None }).collect();
    assert_eq!(imgs.len(), 3);
    assert!(imgs.iter().all(|i| i.data.as_slice() == png.as_slice()));
    assert_eq!(import(&pkg).unwrap().media.len(), 1);
}

#[test]
fn embedded_formula_object_reads_its_mathml() {
    // #3099: LibreOffice saves an equation as a `draw:object` (a formula sub-document whose
    // own `content.xml` is MathML) plus a `draw:image` replacement preview in LibreOffice's
    // own metafile format, which WordCraft can't decode as a picture. The object's MathML
    // must be read instead of silently dropping the frame (blank equation).
    let math = r#"<math xmlns="http://www.w3.org/1998/Math/MathML" display="block"><semantics><mrow><mi>E</mi><mo stretchy="false">=</mo><msup><mi mathvariant="italic">mc</mi><mn>2</mn></msup></mrow><annotation encoding="StarMath 5.0">E=mc^2</annotation></semantics></math>"#;
    let frame = r#"<draw:frame svg:width="0.5in" svg:height="0.2in"><draw:object xlink:href="./Object 1"/><draw:image xlink:href="./ObjectReplacements/Object 1"/></draw:frame>"#;
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>{frame}</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = zip_of(&[
        ("content.xml", content.as_bytes()),
        ("Object 1/content.xml", math.as_bytes()),
        // Not a format `sniff_image` recognizes: proves the equation wins over this fallback.
        ("ObjectReplacements/Object 1", &[0u8; 50]),
    ]);
    let flow = parse(&pkg).unwrap();
    let eqs: Vec<(String, bool)> = paras(&flow)
        .into_iter()
        .flat_map(|p| &p.inlines)
        .filter_map(|i| if let Inline::Equation { linear, display } = i { Some((linear.clone(), *display)) } else { None })
        .collect();
    assert_eq!(eqs, vec![("E=mc^2".to_string(), true)]);
}

#[test]
fn a_trailing_slash_on_the_object_href_still_resolves() {
    // Some ODF producers write the object folder with a trailing slash
    // (`xlink:href="./Object 1/"`); that must not double the slash before `content.xml`.
    let math = r#"<math><mi>x</mi></math>"#;
    let frame = r#"<draw:frame svg:width="1in" svg:height="1in"><draw:object xlink:href="./Object 1/"/></draw:frame>"#;
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>{frame}</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = zip_of(&[("content.xml", content.as_bytes()), ("Object 1/content.xml", math.as_bytes())]);
    let flow = parse(&pkg).unwrap();
    let eqs: Vec<(String, bool)> = paras(&flow)
        .into_iter()
        .flat_map(|p| &p.inlines)
        .filter_map(|i| if let Inline::Equation { linear, display } = i { Some((linear.clone(), *display)) } else { None })
        .collect();
    assert_eq!(eqs, vec![("x".to_string(), false)]);
}

#[test]
fn object_without_mathml_falls_back_to_its_replacement_image() {
    // A non-formula embedded object (or one whose content.xml has no `math:math`) has no
    // equation to read: the frame still shows its `draw:image` replacement, as before.
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 300]].concat();
    let frame = r#"<draw:frame svg:width="1in" svg:height="1in"><draw:object xlink:href="./Object 1"/><draw:image xlink:href="Pictures/a.png"/></draw:frame>"#;
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>{frame}</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = zip_of(&[("content.xml", content.as_bytes()), ("Object 1/content.xml", b"<office:spreadsheet/>"), ("Pictures/a.png", &png)]);
    let flow = parse(&pkg).unwrap();
    let imgs: Vec<&model::Img> =
        paras(&flow).into_iter().flat_map(|p| &p.inlines).filter_map(|i| if let Inline::Image(img) = i { Some(img) } else { None }).collect();
    assert_eq!(imgs.len(), 1);
    assert!(imgs[0].data.as_slice() == png.as_slice());
}

const NS_T: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

/// A package with `content.xml` and, when given, `styles.xml`.
fn package(content: &str, styles: Option<&str>) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in
        [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content)].into_iter().chain(styles.map(|s| ("styles.xml", s)))
    {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn paras(f: &Flow) -> Vec<&Para> {
    f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
}

#[test]
fn common_paragraph_styles_give_text_formatting() {
    // #436: a common paragraph style's text properties reach its paragraphs, directly,
    // through a derived common style and through an automatic style's parent.
    let props = r##"<style:text-properties fo:font-weight="bold" fo:color="#ff0000" fo:font-size="24pt"/>"##;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Emphasis" style:family="paragraph" style:parent-style-name="Standard">{props}</style:style><style:style style:name="Derived" style:family="paragraph" style:parent-style-name="Emphasis"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="Bridge" style:family="paragraph" style:parent-style-name="Emphasis"/></office:automatic-styles><office:body><office:text><text:p>Plain</text:p><text:p text:style-name="Emphasis">Common</text:p><text:p text:style-name="Derived">Derived</text:p><text:p text:style-name="Bridge">Bridge</text:p></office:text></office:body></office:document-content>"#
    );
    let f = parse(&package(&content, Some(&styles))).unwrap();
    let ps = paras(&f);
    assert_eq!(ps.len(), 4);
    let fmt = |p: &Para| p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap();
    assert_eq!(fmt(ps[0]), Fmt::default());
    for p in &ps[1..] {
        let f = fmt(p);
        assert!(f.bold && f.color == Some(Rgb(255, 0, 0)) && f.size == Some(24.0), "{}: {f:?}", p.text());
    }
}

#[test]
fn text_after_a_drawing_shape_stays_in_its_paragraph() {
    // #1654: a shape's own text:p closed the paragraph it is anchored in, and the text after
    // the shape was dropped. Shapes are skipped like text boxes; the paragraph keeps its text.
    let draw =
        r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;
    let content = format!(
        r#"<office:document-content {NS_T} {draw}><office:body><office:text><text:p text:style-name="Heading"><draw:custom-shape svg:width="20mm" svg:height="20mm"><text:p>In the shape</text:p><draw:enhanced-geometry draw:type="rectangle"/></draw:custom-shape>Before <draw:rect><text:p/></draw:rect>middle <draw:g><draw:ellipse><text:p>In the group</text:p></draw:ellipse></draw:g>after</text:p><text:p>Next</text:p></office:text></office:body></office:document-content>"#
    );
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Heading" style:family="paragraph"><style:text-properties fo:font-weight="bold"/></style:style></office:styles></office:document-styles>"#
    );
    let f = parse(&package(&content, Some(&styles))).unwrap();
    let ps = paras(&f);
    assert_eq!(ps.iter().map(|p| p.text()).collect::<Vec<_>>(), ["Before middle after", "Next"]);
    assert!(ps[0].inlines.iter().all(|i| !matches!(i, Inline::Text(_, f) if !f.bold)), "{:?}", ps[0].inlines);
}

#[test]
fn page_size_orientation_and_margins_round_trip() {
    // #810: the page setup written on export is read back on import.
    let mut doc = Document::new();
    doc.last_section.page_w = 841.0;
    doc.last_section.page_h = 595.0;
    doc.last_section.landscape = true;
    doc.last_section.margin_left = 36.0;
    doc.last_section.margin_top = 54.0;
    let back = import(&export(&doc).unwrap()).unwrap();
    let s = &back.last_section;
    assert!(s.landscape);
    assert_eq!((s.page_w, s.page_h), (841.0, 595.0));
    assert_eq!((s.margin_left, s.margin_top), (36.0, 54.0));
}

#[test]
fn page_layout_follows_the_first_master_page() {
    let xml = br#"<office:document-styles><office:automatic-styles>
            <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout>
            <style:page-layout style:name="pm2">
            <style:page-layout-properties fo:page-width="10in" fo:page-height="5in" fo:margin-left="1in" style:print-orientation="landscape"/>
            </style:page-layout>
            </office:automatic-styles><office:master-styles>
            <style:master-page style:name="Standard" style:page-layout-name="pm2"/>
            <style:master-page style:name="Other" style:page-layout-name="pm1"/>
            </office:master-styles></office:document-styles>"#;
    let mut sec = wordcraft_doc::SectionProps::default();
    apply_page_layout(xml, &mut sec);
    assert!(sec.landscape);
    assert_eq!((sec.page_w, sec.page_h, sec.margin_left, sec.margin_top), (720.0, 360.0, 72.0, 72.0));
}

#[test]
fn hostile_page_layout_values_are_clamped() {
    let styles = |w: &str, h: &str, m: &str| {
        format!(
            r#"<office:document-styles><office:automatic-styles><style:page-layout style:name="pm1">
                <style:page-layout-properties fo:page-width="{w}" fo:page-height="{h}" fo:margin-top="{m}"
                fo:margin-bottom="{m}" fo:margin-left="{m}" fo:margin-right="{m}"/></style:page-layout>
                </office:automatic-styles><office:master-styles>
                <style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
                </office:document-styles>"#
        )
    };
    // Huge values: the page is capped at 22 inches and margins at the page's longer side.
    let mut sec = wordcraft_doc::SectionProps::default();
    apply_page_layout(styles("1e30pt", "1e30in", "1e30pt").as_bytes(), &mut sec);
    assert_eq!((sec.page_w, sec.page_h), (1584.0, 1584.0));
    assert_eq!((sec.margin_top, sec.margin_left), (1584.0, 1584.0));
    // Negative, zero, tiny and non-numeric values leave the defaults alone.
    let def = wordcraft_doc::SectionProps::default();
    for (w, h, m) in [("-10in", "-5in", "-1in"), ("0pt", "0pt", "NaNpt"), ("1pt", "2pt", "infpt"), ("NaN", "inf", "x")] {
        let mut sec = wordcraft_doc::SectionProps::default();
        apply_page_layout(styles(w, h, m).as_bytes(), &mut sec);
        assert_eq!((sec.page_w, sec.page_h), (def.page_w, def.page_h), "{w} {h}");
        assert_eq!((sec.margin_top, sec.margin_right), (def.margin_top, def.margin_right), "{m}");
        assert!(sec.page_w.is_finite() && sec.margin_left.is_finite());
    }
}

#[test]
fn table_column_styles_are_the_grid_widths() {
    // A column's `style:column-width` (and `table:number-columns-repeated`) sets its width.
    let content = format!(
        r##"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="Table1.A" style:family="table-column"><style:table-column-properties style:column-width="1in"/></style:style><style:style style:name="Table1.B" style:family="table-column"><style:table-column-properties style:column-width="2in"/></style:style></office:automatic-styles><office:body><office:text><table:table><table:table-column table:style-name="Table1.A"/><table:table-column table:style-name="Table1.B" table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell><table:table-cell><text:p>C</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"##
    );
    let f = parse(&package(&content, None)).unwrap();
    let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
    assert_eq!(t.widths, vec![72.0, 144.0, 144.0]);
}

#[test]
fn table_column_widths_survive_a_round_trip() {
    // A width changed in the document is written and read back; the file's own widths are
    // what the first import shows.
    let content = format!(
        r##"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="Table1.A" style:family="table-column"><style:table-column-properties style:column-width="1in"/></style:style><style:style style:name="Table1.B" style:family="table-column"><style:table-column-properties style:column-width="3in"/></style:style></office:automatic-styles><office:body><office:text><table:table><table:table-column table:style-name="Table1.A"/><table:table-column table:style-name="Table1.B"/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"##
    );
    let grid =
        |doc: &Document| match doc.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t.grid.clone()) } else { None }) {
            Some(g) => g,
            None => panic!("no table"),
        };
    let mut doc = import(&package(&content, None)).unwrap();
    assert_eq!(grid(&doc), vec![72.0, 216.0]);
    let widths = vec![150.0, 100.0];
    for b in doc.body.iter_mut() {
        if let wordcraft_doc::Block::Table(t) = std::sync::Arc::make_mut(b) {
            t.grid = widths.clone();
        }
    }
    let back = import(&export(&doc).unwrap()).unwrap();
    assert_eq!(grid(&back), widths);
}

#[test]
fn table_without_column_widths_keeps_the_equal_split() {
    let content = format!(
        r##"<office:document-content {NS_T}><office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"##
    );
    let f = parse(&package(&content, None)).unwrap();
    let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
    assert!(t.widths.is_empty());
}

#[test]
fn table_cell_style_background_is_shading() {
    // #437: `fo:background-color` on `style:table-cell-properties` is the cell's shading.
    let content = format!(
        r##"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="RedCell" style:family="table-cell"><style:table-cell-properties fo:background-color="#ff0000"/></style:style></office:automatic-styles><office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell table:style-name="RedCell"><text:p>Red</text:p></table:table-cell><table:table-cell><text:p>Plain</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"##
    );
    let f = parse(&package(&content, None)).unwrap();
    let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
    let row: Vec<_> = t.rows[0].iter().map(|c| (c.text(), c.shading)).collect();
    assert_eq!(row, vec![("Red".to_string(), Some(Rgb(255, 0, 0))), ("Plain".to_string(), None)]);
}

#[test]
fn table_cell_style_vertical_alignment() {
    let content = format!(
        r##"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="MidCell" style:family="table-cell"><style:table-cell-properties style:vertical-align="middle"/></style:style><style:style style:name="BotCell" style:family="table-cell"><style:table-cell-properties style:vertical-align="bottom"/></style:style></office:automatic-styles><office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell table:style-name="MidCell"><text:p>Mid</text:p></table:table-cell><table:table-cell table:style-name="BotCell"><text:p>Bot</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"##
    );
    let f = parse(&package(&content, None)).unwrap();
    let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
    assert_eq!(t.rows[0][0].valign, Some(VAlign::Center));
    assert_eq!(t.rows[0][1].valign, Some(VAlign::Bottom));
    let doc = crate::model::to_doc(&f);
    let bytes = export(&doc).unwrap();
    let f2 = parse(&bytes).unwrap();
    let Some(FBlock::Table(t2)) = f2.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table in export") };
    assert_eq!(t2.rows[0][0].valign, Some(VAlign::Center));
    assert_eq!(t2.rows[0][1].valign, Some(VAlign::Bottom));
}

/// #1379: three master pages (A4 upright; A4 wide with page numbering restarting at 1; A4
/// upright again with numbering continuing) and a two-column `text:section` in between.
fn sections_package() -> Vec<u8> {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles>
            <style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" style:print-orientation="portrait" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2.5cm" fo:margin-right="2.5cm"/></style:page-layout>
            <style:page-layout style:name="pW"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape" fo:margin-top="1.5cm" fo:margin-bottom="1.5cm" fo:margin-left="1.5cm" fo:margin-right="1.5cm"/></style:page-layout>
            </office:automatic-styles><office:master-styles>
            <style:master-page style:name="Upright" style:page-layout-name="pU"/>
            <style:master-page style:name="Wide" style:page-layout-name="pW"/>
            <style:master-page style:name="Closing" style:page-layout-name="pU"/>
            </office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
            <style:style style:name="PStart" style:family="paragraph" style:master-page-name="Upright"/>
            <style:style style:name="PWide" style:family="paragraph" style:master-page-name="Wide"><style:paragraph-properties fo:break-before="page" style:page-number="1"/></style:style>
            <style:style style:name="PClose" style:family="paragraph" style:master-page-name="Closing"><style:paragraph-properties fo:break-before="page" style:page-number="auto"/></style:style>
            <style:style style:name="Two" style:family="section"><style:section-properties><style:columns fo:column-count="2" fo:column-gap="1cm"><style:column-sep style:width="0.02cm" style:style="solid"/></style:columns></style:section-properties></style:style>
            </office:automatic-styles><office:body><office:text>
            <text:p text:style-name="PStart">Opening words</text:p><text:p>More opening words</text:p>
            <text:h text:style-name="PWide" text:outline-level="1">Wide part</text:h>
            <table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row></table:table>
            <text:section text:name="Cols" text:style-name="Two"><text:p>Left column text</text:p><text:p>Right column text</text:p></text:section>
            <text:p text:style-name="PClose">Closing words</text:p><text:p>Last line</text:p>
            </office:text></office:body></office:document-content>"#
    );
    package(&content, Some(&styles))
}

fn part_text(d: &Document, id: Option<u32>) -> String {
    d.plain_text(wordcraft_doc::StoryRef::Part(id.expect("part")))
}

#[test]
fn headers_and_footers_follow_their_master_page() {
    // #863: headers and footers live on the master pages in styles.xml.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles>
            <style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout>
            <style:page-layout style:name="pW"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm"/></style:page-layout>
            <style:style style:name="MP1" style:family="paragraph"><style:text-properties fo:font-weight="bold"/></style:style>
            </office:automatic-styles><office:master-styles>
            <style:master-page style:name="Upright" style:page-layout-name="pU">
            <style:header><text:p text:style-name="MP1">ACME <text:span>Letterhead</text:span></text:p></style:header>
            <style:header-left><text:p>Left page</text:p></style:header-left>
            <loext:header-first><text:p>First page</text:p></loext:header-first>
            <style:footer><text:p>Page <text:page-number text:select-page="current">1</text:page-number></text:p></style:footer>
            <style:footer-left style:display="false"><text:p>Hidden</text:p></style:footer-left>
            </style:master-page>
            <style:master-page style:name="Wide" style:page-layout-name="pW"><style:header><text:p>Wide header</text:p></style:header></style:master-page>
            </office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
            <style:style style:name="PWide" style:family="paragraph" style:master-page-name="Wide"/>
            </office:automatic-styles><office:body><office:text>
            <text:p>Upright body</text:p><text:p text:style-name="PWide">Wide body</text:p>
            </office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let secs = d.sections();
    assert_eq!(secs.len(), 2);
    let (up, wide) = (&secs[0].1, &secs[1].1);
    assert_eq!(part_text(&d, up.headers.default), "ACME Letterhead");
    assert_eq!(part_text(&d, up.headers.even), "Left page");
    assert_eq!(part_text(&d, up.headers.first), "First page");
    assert_eq!(part_text(&d, up.footers.default), "Page 1");
    assert!(up.title_page && d.settings.even_odd_headers);
    // The hidden left footer: left pages show the right pages' footer.
    assert_eq!((up.footers.even, up.footers.first), (up.footers.default, None));
    assert_eq!(part_text(&d, wide.headers.default), "Wide header");
    assert_eq!((wide.headers.even, wide.title_page), (wide.headers.default, false));
    // Its master page has no footer: an empty one, not the previous section's.
    assert_eq!(part_text(&d, wide.footers.default), "");
    assert_eq!(d.parts.len(), 6);
    let id = up.headers.default.unwrap();
    let Some(wordcraft_doc::Block::Para(p)) = d.parts[&id].blocks.first().map(|b| &**b) else { panic!("no header paragraph") };
    assert!(p.runs.iter().all(|r| r.props.bold == Some(true)), "{:?}", p.runs);
}

#[test]
fn documents_without_headers_get_none() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Body</text:p></office:text></office:body></office:document-content>"#
    );
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    for d in [import(&package(&content, Some(&styles))).unwrap(), import(&package(&content, None)).unwrap()] {
        assert!(d.parts.is_empty());
        assert_eq!(d.last_section.headers, Default::default());
        assert!(!d.last_section.title_page && !d.settings.even_odd_headers);
    }
}

#[test]
fn deeply_nested_header_content_does_not_panic() {
    let n = 4 * MAX_XML_DEPTH;
    let (open, close) = ("<text:span>".repeat(n), "</text:span>".repeat(n));
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>{open}Deep{close}</text:p></style:header><style:footer><text:p>After</text:p></style:footer></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Body</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert!(d.last_section.headers.default.is_some());
    assert_eq!(part_text(&d, d.last_section.footers.default), "After");
}

#[test]
fn master_pages_and_column_sections_become_sections() {
    use wordcraft_doc::section::SectionStart;
    let doc = import(&sections_package()).unwrap();
    let secs = doc.sections();
    let summary: Vec<_> =
        secs.iter().map(|(_, s)| (s.start, s.landscape, s.page_w.round(), s.page_h.round(), s.columns.count, s.page_num_start)).collect();
    assert_eq!(
        summary,
        [
            (SectionStart::NextPage, false, 595.0, 842.0, 1, None),
            (SectionStart::NextPage, true, 842.0, 595.0, 1, Some(1)),
            (SectionStart::Continuous, true, 842.0, 595.0, 2, None),
            (SectionStart::NextPage, false, 595.0, 842.0, 1, None),
        ]
    );
    // Margins come from each section's page layout; the columns from the section style.
    assert!((secs[0].1.margin_left - 70.87).abs() < 0.01 && (secs[1].1.margin_left - 42.52).abs() < 0.01);
    let cols = &secs[2].1.columns;
    assert!((cols.space - 28.35).abs() < 0.01 && cols.separator, "{cols:?}");
    // Each section ends on its own last paragraph; the wide one ends after its table.
    let text = |i: usize| match doc.body.get(i).map(|b| &**b) {
        Some(wordcraft_doc::Block::Para(p)) => p.plain_text().to_string(),
        _ => String::from("<not a paragraph>"),
    };
    assert_eq!(text(secs[0].0), "More opening words");
    assert!(matches!(doc.body.get(secs[1].0 - 1).map(|b| &**b), Some(wordcraft_doc::Block::Table(_))));
    assert_eq!(text(secs[2].0), "Right column text");
    assert_eq!(text(secs[3].0), "Last line");
}

/// (start, landscape, width, height, columns, restart, margins) of each section, and the
/// text of the paragraph ending it.
fn section_summary(doc: &Document) -> Vec<(wordcraft_doc::section::SectionStart, bool, f32, f32, u32, bool, Option<u32>, [f32; 4], String)> {
    doc.sections()
        .iter()
        .map(|(end, s)| {
            let text = match doc.body.get(*end).map(|b| &**b) {
                Some(wordcraft_doc::Block::Para(p)) => p.plain_text().to_string(),
                _ => String::new(),
            };
            (
                s.start,
                s.landscape,
                s.page_w.round(),
                s.page_h.round(),
                s.columns.count,
                s.columns.separator,
                s.page_num_start,
                [s.margin_top, s.margin_bottom, s.margin_left, s.margin_right].map(f32::round),
                text,
            )
        })
        .collect()
}

#[test]
fn export_writes_sections_that_import_back() {
    // Portrait; landscape on a new page restarting at 1; continuous two columns with a
    // separator; portrait again. Export, import: the same sections.
    let doc = import(&sections_package()).unwrap();
    let before = section_summary(&doc);
    assert_eq!(before.len(), 4, "{before:?}");
    let back = import(&export(&doc).unwrap()).unwrap();
    let after = section_summary(&back);
    assert_eq!(after, before);
    let (a, b) = (&doc.sections(), &back.sections());
    for (x, y) in a.iter().zip(b.iter()) {
        assert!((x.1.columns.space - y.1.columns.space).abs() < 0.1, "{:?} vs {:?}", x.1.columns, y.1.columns);
        assert!((x.1.margin_top - y.1.margin_top).abs() < 0.1 && (x.1.margin_right - y.1.margin_right).abs() < 0.1);
    }
}

#[test]
fn export_starts_a_section_at_a_table_and_keeps_its_restart() {
    // A section that starts with a table names its master page on the table's style.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout><style:page-layout style:name="pW"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Upright" style:page-layout-name="pU"/><style:master-page style:name="Wide" style:page-layout-name="pW"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="WideTable" style:family="table" style:master-page-name="Wide"><style:table-properties style:page-number="5"/></style:style></office:automatic-styles><office:body><office:text><text:p>Intro</text:p><table:table table:style-name="WideTable"><table:table-column/><table:table-row><table:table-cell><text:p>Cell</text:p></table:table-cell></table:table-row></table:table><text:p>After</text:p></office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, Some(&styles))).unwrap();
    let before = section_summary(&doc);
    assert_eq!(before.len(), 2, "{before:?}");
    let back = import(&export(&doc).unwrap()).unwrap();
    assert_eq!(section_summary(&back), before);
    assert_eq!(back.sections()[1].1.page_num_start, Some(5));
}

/// Master pages `Standard` (portrait) and `Cols` (landscape, two columns with a line) for the
/// round-trip tests below.
fn cols_styles() -> String {
    format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2.2cm" fo:margin-left="2.5cm" fo:margin-right="2.6cm"/></style:page-layout><style:page-layout style:name="pC"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape" fo:margin-top="1cm" fo:margin-bottom="1.1cm" fo:margin-left="1.2cm" fo:margin-right="1.3cm"><style:columns fo:column-count="2" fo:column-gap="1cm"><style:column-sep style:width="0.02cm" style:style="solid"/></style:columns></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pU"/><style:master-page style:name="Cols" style:page-layout-name="pC"/></office:master-styles></office:document-styles>"#
    )
}

/// Import `body` (with automatic `styles`) over [`cols_styles`], export, import: both summaries.
fn round_trip(styles: &str, body: &str) -> (Vec<String>, Vec<String>) {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, Some(&cols_styles()))).unwrap();
    let back = import(&export(&doc).unwrap()).unwrap();
    let show = |d: &Document| section_summary(d).iter().map(|s| format!("{s:?}")).collect::<Vec<_>>();
    (show(&doc), show(&back))
}

#[test]
fn export_keeps_a_columned_page_and_the_one_column_part_after_it() {
    let styles = r#"<style:style style:name="PC" style:family="paragraph" style:master-page-name="Cols"/><style:style style:name="One" style:family="section"><style:section-properties><style:columns fo:column-count="1" fo:column-gap="0cm"/></style:section-properties></style:style>"#;
    let body = r#"<text:p>Intro</text:p><text:p text:style-name="PC">Two columns</text:p><text:section text:name="S" text:style-name="One"><text:p>One column</text:p></text:section>"#;
    let (before, after) = round_trip(styles, body);
    assert_eq!(before.len(), 3, "{before:?}");
    assert!(before[1].contains(", 2, true,") && before[2].contains("Continuous") && before[2].contains(", 1, false,"), "{before:?}");
    assert_eq!(after, before);
}

#[test]
fn plain_one_column_text_sections_do_not_make_sections() {
    let styles = r#"<style:style style:name="One" style:family="section"><style:section-properties><style:columns fo:column-count="1" fo:column-gap="0cm"/></style:section-properties></style:style>"#;
    let body = r#"<text:p>A</text:p><text:section text:name="S" text:style-name="One"><text:p>B</text:p></text:section><text:section text:name="T"><text:p>C</text:p></text:section>"#;
    let (before, after) = round_trip(styles, body);
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(after, before);
}

#[test]
fn export_keeps_a_section_that_starts_on_a_list_item() {
    let styles = r#"<style:style style:name="PC" style:family="paragraph" style:master-page-name="Cols"/>"#;
    let body = r#"<text:p>Intro</text:p><text:list><text:list-item><text:p text:style-name="PC">First</text:p></text:list-item><text:list-item><text:p>Second</text:p></text:list-item></text:list><text:p>After</text:p>"#;
    let (before, after) = round_trip(styles, body);
    assert_eq!(before.len(), 2, "{before:?}");
    assert_eq!(after, before);
}

#[test]
fn a_list_cut_at_a_section_boundary_goes_on_numbering() {
    let styles = r#"<style:style style:name="PC" style:family="paragraph" style:master-page-name="Cols"/>"#;
    let body = r#"<text:list><text:list-item><text:p>One</text:p></text:list-item><text:list-item><text:p text:style-name="PC">Two</text:p></text:list-item></text:list>"#;
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, Some(&cols_styles()))).unwrap();
    let bytes = export(&doc).unwrap();
    let xml = String::from_utf8(read_entry(&mut archive(&bytes), "content.xml").unwrap()).unwrap();
    assert_eq!(xml.matches("text:continue-list=\"").count(), 1, "{xml}");
    // The continuation points at the first list's xml:id.
    assert!(xml.contains("xml:id=\"lst1\"") && xml.contains("text:continue-list=\"lst1\""), "{xml}");
    assert_eq!(section_summary(&import(&bytes).unwrap()), section_summary(&doc));
}

#[test]
fn export_keeps_a_page_number_restart_on_the_first_section() {
    let styles = r#"<style:style style:name="PS" style:family="paragraph" style:master-page-name="Standard"><style:paragraph-properties style:page-number="7"/></style:style>"#;
    let body = r#"<text:p text:style-name="PS">First</text:p><text:p>Second</text:p>"#;
    let (before, after) = round_trip(styles, body);
    assert_eq!(before.len(), 1, "{before:?}");
    assert!(before[0].contains("Some(7)"), "{before:?}");
    assert_eq!(after, before);
}

#[test]
fn export_of_a_one_section_document_stays_simple() {
    let doc = import(&sections_package()).unwrap();
    let one = import(&export(&doc).unwrap()).unwrap();
    let mut single = one.clone();
    for b in single.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.section = None;
        }
    }
    let bytes = export(&single).unwrap();
    let mut z = archive(&bytes);
    let styles = String::from_utf8(read_entry(&mut z, "styles.xml").unwrap()).unwrap();
    let content = String::from_utf8(read_entry(&mut z, "content.xml").unwrap()).unwrap();
    assert_eq!(styles.matches("<style:master-page ").count(), 1);
    assert!(styles.contains("style:name=\"Standard\" style:page-layout-name=\"pm1\""));
    assert!(!content.contains("text:section") && !content.contains("master-page-name"), "{content}");
}

#[test]
fn one_master_page_stays_one_section() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pU"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="Standard"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">One</text:p><text:p>Two</text:p></office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(doc.sections().len(), 1);
    assert_eq!(doc.last_section.page_w.round(), 595.0);
}

#[test]
fn hostile_sections_are_capped_and_clamped() {
    // 20,000 master-page paragraphs give at most 10,000 sections; absurd column counts and
    // gaps are clamped; a column section at the very end leaves no empty section after it.
    let mut body = String::new();
    for _ in 0..20_000 {
        body.push_str(r#"<text:p text:style-name="PM">x</text:p>"#);
    }
    body.push_str(r#"<text:section text:style-name="Many"><text:p>cols</text:p></text:section>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="PM" style:family="paragraph" style:master-page-name="Standard"/><style:style style:name="Many" style:family="section"><style:section-properties><style:columns fo:column-count="99999999999" fo:column-gap="1e30in"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pU"/></office:master-styles></office:document-styles>"#
    );
    let doc = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(doc.sections().len(), 10_000);
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="Many" style:family="section"><style:section-properties><style:columns fo:column-count="1000" fo:column-gap="1e30in"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text><text:p>before</text:p><text:section text:style-name="Many"><text:p>cols</text:p></text:section></office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, None)).unwrap();
    let secs = doc.sections();
    assert_eq!(secs.len(), 2);
    assert_eq!(secs[1].1.columns.count, 45);
    assert!(secs[1].1.columns.space <= 1584.0);
}

#[test]
fn master_page_on_a_table_style_starts_a_section() {
    // Review of #1379: a table style can name a master page too (ODF 1.3 §19.509), with its
    // page-number restart on `style:table-properties`. An empty name on a child style means
    // "no page style here", and a page-number restart without a master page starts nothing.
    use wordcraft_doc::section::SectionStart;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="WideParent" style:family="paragraph" style:master-page-name="Wide"/></office:styles><office:automatic-styles>
            <style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"><style:columns fo:column-count="1" fo:column-gap="0cm"/></style:page-layout-properties></style:page-layout>
            <style:page-layout style:name="pW"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape"/></style:page-layout>
            </office:automatic-styles><office:master-styles>
            <style:master-page style:name="Upright" style:page-layout-name="pU"/><style:master-page style:name="Wide" style:page-layout-name="pW"/>
            </office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
            <style:style style:name="NoPage" style:family="paragraph" style:parent-style-name="WideParent" style:master-page-name=""/>
            <style:style style:name="Restart" style:family="paragraph"><style:paragraph-properties fo:break-before="page" style:page-number="3"/></style:style>
            <style:style style:name="WideTable" style:family="table" style:master-page-name="Wide"><style:table-properties style:page-number="1"/></style:style>
            </office:automatic-styles><office:body><office:text>
            <text:p>Intro</text:p><text:p text:style-name="NoPage">Not a new page style</text:p><text:p text:style-name="Restart">Page break only</text:p>
            <table:table table:style-name="WideTable"><table:table-column/><table:table-row><table:table-cell><text:p>Cell</text:p></table:table-cell></table:table-row></table:table>
            <text:p>After</text:p>
            </office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, Some(&styles))).unwrap();
    let secs = doc.sections();
    let summary: Vec<_> = secs.iter().map(|(i, s)| (*i, s.start, s.landscape, s.page_num_start)).collect();
    assert_eq!(summary, [(2, SectionStart::NextPage, false, None), (4, SectionStart::NextPage, true, Some(1))]);
    assert!(matches!(doc.body.get(3).map(|b| &**b), Some(wordcraft_doc::Block::Table(_))));
    // A one-column page layout keeps the default column gap.
    assert_eq!(secs[0].1.columns, wordcraft_doc::section::Columns::default());
}

#[test]
fn keep_with_next_survives_import_and_export() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
            <style:style style:name="Keep" style:family="paragraph"><style:paragraph-properties fo:keep-with-next="always"/></style:style>
            <style:style style:name="Loose" style:family="paragraph" style:parent-style-name="Keep"><style:paragraph-properties fo:keep-with-next="auto"/></style:style>
            </office:automatic-styles><office:body><office:text>
            <text:p text:style-name="Keep">Kept</text:p><text:p text:style-name="Loose">Loose</text:p><text:p>Plain</text:p>
            </office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, None)).unwrap();
    let flags = |d: &Document| d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.keep_next).collect::<Vec<_>>();
    assert_eq!(flags(&doc), [Some(true), None, None]);
    let again = import(&export(&doc).unwrap()).unwrap();
    assert_eq!(flags(&again), [Some(true), None, None]);
}

// --- Tracked changes (ODF 1.3 Part 3 §5.5) -------------------------------------------------

const NS_TC: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/""#;

/// An `.odt` whose body holds `regions` in `text:tracked-changes` and then `body`.
fn tracked(regions: &str, body: &str) -> Vec<u8> {
    let content = format!(
        r#"<office:document-content {NS_TC}><office:body><office:text><text:tracked-changes text:track-changes="true">{regions}</text:tracked-changes>{body}</office:text></office:body></office:document-content>"#
    );
    package(&content, None)
}

fn region(id: &str, kind: &str, author: &str, date: &str, content: &str) -> String {
    format!(
        r#"<text:changed-region text:id="{id}"><text:{kind}><office:change-info><dc:creator>{author}</dc:creator><dc:date>{date}</dc:date><text:p>a remark on the change</text:p></office:change-info>{content}</text:{kind}></text:changed-region>"#
    )
}

/// Each body paragraph as (text, revision kind) runs, plus its mark's revision kind.
type RevRuns = Vec<(Vec<(String, Option<wordcraft_doc::RevisionKind>)>, Option<wordcraft_doc::RevisionKind>)>;

fn rev_runs(doc: &Document) -> RevRuns {
    let kind = |i: Option<u32>| i.and_then(|i| doc.revisions.get(i as usize)).map(|r| r.kind);
    doc.body
        .iter()
        .filter_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None })
        .map(|p| {
            let runs = p.run_ranges().map(|(r, cp)| (p.text[r].to_string(), kind(cp.del).or(kind(cp.ins)))).collect();
            (runs, kind(p.mark.del).or(kind(p.mark.ins)))
        })
        .collect()
}

fn insertion_and_deletion() -> Vec<u8> {
    let regions = region("ct1", "insertion", "Alice", "2024-03-01T10:00:00", "")
        + &region("ct2", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p>old<text:s/></text:p>");
    tracked(
        &regions,
        r#"<text:p>Keep <text:change-start text:change-id="ct1"/>new <text:change-end text:change-id="ct1"/><text:change text:change-id="ct2"/>words.</text:p>"#,
    )
}

#[test]
fn tracked_insertion_and_deletion_import_with_authors_and_dates() {
    use wordcraft_doc::RevisionKind::{Delete, Insert};
    let doc = import(&insertion_and_deletion()).unwrap();
    let s = |v: &str| v.to_string();
    assert_eq!(rev_runs(&doc), vec![(vec![(s("Keep "), None), (s("new "), Some(Insert)), (s("old "), Some(Delete)), (s("words."), None)], None)]);
    let revs: Vec<_> = doc.revisions.iter().map(|r| (r.kind, r.author.as_str(), r.date.as_str())).collect();
    assert_eq!(revs, vec![(Insert, "Alice", "2024-03-01T10:00:00"), (Delete, "Bob", "2024-03-02T11:30:00")]);
    assert!(doc.settings.track_changes, "text:track-changes=\"true\" turns tracking on");
}

#[test]
fn tracked_deletion_across_paragraphs_keeps_the_deleted_paragraph_mark() {
    use wordcraft_doc::RevisionKind::Delete;
    // "First tail¶head rest." with "tail¶head " deleted: the body holds the merged paragraph.
    let regions = region("d1", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p>tail</text:p><text:p>head<text:s/></text:p>");
    let doc = import(&tracked(&regions, r#"<text:p>First <text:change text:change-id="d1"/>rest.</text:p>"#)).unwrap();
    let s = |v: &str| v.to_string();
    assert_eq!(
        rev_runs(&doc),
        vec![(vec![(s("First "), None), (s("tail"), Some(Delete))], Some(Delete)), (vec![(s("head "), Some(Delete)), (s("rest."), None)], None),]
    );
}

#[test]
fn tracked_changes_round_trip_through_export() {
    let regions = region("ct1", "insertion", "Alice", "2024-03-01T10:00:00", "")
        + &region("d1", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p>tail</text:p><text:p></text:p><text:p>head<text:s/></text:p>");
    let body = r#"<text:p>First <text:change text:change-id="d1"/>rest <text:change-start text:change-id="ct1"/>added</text:p><text:p>whole new<text:change-end text:change-id="ct1"/></text:p><text:p>End.</text:p>"#;
    let doc = import(&tracked(&regions, body)).unwrap();
    let back = import(&export(&doc).unwrap()).unwrap();
    assert_eq!(rev_runs(&back), rev_runs(&doc));
    let revs = |d: &Document| d.revisions.iter().map(|r| (r.kind, r.author.clone(), r.date.clone())).collect::<Vec<_>>();
    assert_eq!(revs(&back), revs(&doc));
    assert_eq!(back.settings.track_changes, doc.settings.track_changes);
    // The inserted paragraph mark survives: "added¶" is part of the insertion.
    assert_eq!(rev_runs(&doc)[2].1, Some(wordcraft_doc::RevisionKind::Insert));
}

#[test]
fn an_inserted_ruby_keeps_its_tracked_insertion() {
    // ODF has no ruby: its base text is written as text, and the insertion on the ruby must
    // still be a change range around that text, not plain untracked text.
    use wordcraft_doc::RevisionKind::Insert;
    let props = wordcraft_doc::props::CharProps::default();
    let mut p = wordcraft_doc::Paragraph::with_text("before ", props.clone());
    let n = p.len();
    let mut ins = props.clone();
    ins.ins = Some(0);
    p.insert_object(n, crate::model::ruby_object("漢字".into(), "かんじ".into(), 20.0), &ins).unwrap();
    let mut d = Document::new();
    d.settings.track_changes = true;
    d.revisions.push(wordcraft_doc::Revision::new(Insert, "Alice", "2024-03-01T10:00:00"));
    d.body = vec![wordcraft_doc::para_block(p)];
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(rev_runs(&back), vec![(vec![("before ".to_string(), None), ("漢字".to_string(), Some(Insert))], None)]);
}

#[test]
fn hostile_tracked_changes_are_bounded() {
    // Unknown and duplicate ids, a change-end without a start, nested regions, a deletion
    // inside a deletion, a change mark inside the deleted text and tracked changes inside a paragraph.
    let regions = region("d1", "deletion", "B", "x", r#"<text:p>gone<text:change text:change-id="d1"/></text:p>"#)
        + r#"<text:changed-region text:id="n"><text:changed-region text:id="m"><text:deletion><text:p>inner</text:p></text:deletion></text:changed-region></text:changed-region>"#
        + &region("dd", "deletion", "B", "x", "<text:deletion><text:p>deeper</text:p></text:deletion><text:p>ok</text:p>")
        + &region("d1", "insertion", "C", "y", "");
    let body = r#"<text:p>a<text:change-end text:change-id="zz"/><text:change text:change-id="d1"/><text:change text:change-id="d1"/><text:change-start text:change-id="nope"/>b<text:change text:change-id="m"/><text:change text:change-id="dd"/><text:tracked-changes><text:changed-region text:id="q"/></text:tracked-changes>c</text:p>"#;
    let doc = import(&tracked(&regions, body)).unwrap();
    let text: String = rev_runs(&doc).iter().flat_map(|(r, _)| r.iter().map(|(t, _)| t.clone())).collect();
    assert_eq!(text.matches("gone").count(), 1, "a deletion is placed once: {text}");
    assert!(!text.contains("deeper"), "{text}");
    assert!(text.starts_with('a') && text.ends_with('c'), "{text}");

    // A huge number of regions is capped.
    let many: String = (0..20_000).map(|i| format!(r#"<text:changed-region text:id="r{i}"><text:insertion/></text:changed-region>"#)).collect();
    let starts: String = (0..20_000).map(|i| format!(r#"<text:change-start text:change-id="r{i}"/>x"#)).collect();
    let doc = import(&tracked(&many, &format!("<text:p>{starts}</text:p>"))).unwrap();
    assert!(doc.revisions.len() <= 10_000, "{}", doc.revisions.len());
}

/// Every paragraph (body and table cells, in order) as (text, inserted, deleted) runs plus
/// its mark's (inserted, deleted).
type Marks = Vec<(Vec<(String, bool, bool)>, (bool, bool))>;

fn marks(doc: &Document) -> Marks {
    fn walk(bl: &wordcraft_doc::Blocks, out: &mut Marks) {
        for b in bl {
            match &**b {
                wordcraft_doc::Block::Para(p) => out.push((
                    p.run_ranges().map(|(r, cp)| (p.text[r].to_string(), cp.ins.is_some(), cp.del.is_some())).collect(),
                    (p.mark.ins.is_some(), p.mark.del.is_some()),
                )),
                wordcraft_doc::Block::Table(t) => {
                    for r in &t.rows {
                        for c in &r.cells {
                            walk(&c.blocks, out);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.body, &mut out);
    out
}

#[test]
fn inserted_then_deleted_text_keeps_both_marks() {
    // "new" was inserted by Alice, then Bob deleted "old new": the deletion's content holds
    // the insertion as a change range.
    let regions = region("i1", "insertion", "Alice", "2024-03-01T10:00:00", "")
        + &region(
            "d1",
            "deletion",
            "Bob",
            "2024-03-02T11:30:00",
            r#"<text:p>old <text:change-start text:change-id="i1"/>new<text:change-end text:change-id="i1"/></text:p>"#,
        );
    let doc = import(&tracked(&regions, r#"<text:p>Keep <text:change text:change-id="d1"/>end.</text:p>"#)).unwrap();
    let s = |v: &str| v.to_string();
    let want = vec![(vec![(s("Keep "), false, false), (s("old "), false, true), (s("new"), true, true), (s("end."), false, false)], (false, false))];
    assert_eq!(marks(&doc), want);
    let back = import(&export(&doc).unwrap()).unwrap();
    assert_eq!(marks(&back), want, "both marks survive a save");
    assert_eq!(back.revisions.len(), doc.revisions.len(), "one deletion stays one region and one revision: {:?}", back.revisions);
}

#[test]
fn a_deleted_mark_inside_an_insertion_is_not_inserted() {
    // "A" inserted by Alice, then a paragraph break Bob deleted, then "C" inserted by Alice:
    // the insertion range runs across the deletion point, but the deleted mark wasn't inserted.
    let regions = region("i1", "insertion", "Alice", "2024-03-01T10:00:00", "")
        + &region("d1", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p></text:p><text:p></text:p>");
    let body =
        r#"<text:p><text:change-start text:change-id="i1"/>A<text:change text:change-id="d1"/>C<text:change-end text:change-id="i1"/></text:p>"#;
    let doc = import(&tracked(&regions, body)).unwrap();
    let s = |v: &str| v.to_string();
    let want = vec![(vec![(s("A"), true, false)], (false, true)), (vec![(s("C"), true, false)], (false, false))];
    assert_eq!(marks(&doc), want);
    let back = import(&export(&doc).unwrap()).unwrap();
    assert_eq!(marks(&back), want, "after a save");
}

#[test]
fn insertions_end_before_a_table_and_inside_a_cell() {
    let regions = region("i1", "insertion", "Alice", "2024-03-01T10:00:00", "") + &region("i2", "insertion", "Bob", "2024-03-02T10:00:00", "");
    let table = r#"<table:table><table:table-column/><table:table-row><table:table-cell><text:p>Cell <text:change-start text:change-id="i2"/>added</text:p><text:change-end text:change-id="i2"/></table:table-cell></table:table-row></table:table>"#;
    let body = format!(
        r#"<text:p>Start</text:p><text:p><text:change-start text:change-id="i1"/>New paragraph</text:p><text:change-end text:change-id="i1"/>{table}<text:p>End.</text:p>"#
    );
    let content = format!(
        r#"<office:document-content {NS_TC} xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:text><text:tracked-changes>{regions}</text:tracked-changes>{body}</office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, None)).unwrap();
    let s = |v: &str| v.to_string();
    let want = vec![
        (vec![(s("Start"), false, false)], (false, false)),
        (vec![(s("New paragraph"), true, false)], (true, false)),
        (vec![(s("Cell "), false, false), (s("added"), true, false)], (true, false)),
        (vec![(s("End."), false, false)], (false, false)),
    ];
    assert_eq!(marks(&doc), want);
    let back = import(&export(&doc).unwrap()).unwrap();
    assert_eq!(marks(&back), want);
    let authors = |d: &Document| d.revisions.iter().map(|r| r.author.clone()).collect::<Vec<_>>();
    assert_eq!(authors(&back), authors(&doc));
}

#[test]
fn exported_changes_always_have_a_date_and_keep_paragraph_separators() {
    // A change without a date gets one on save (`dc:date` is required); a U+2029 in deleted
    // text is text, not a paragraph break.
    let regions = r#"<text:changed-region text:id="d1"><text:deletion><office:change-info><dc:creator>Bob</dc:creator></office:change-info><text:p>a&#x2029;b</text:p></text:deletion></text:changed-region>"#;
    let doc = import(&tracked(regions, r#"<text:p>x<text:change text:change-id="d1"/>y</text:p>"#)).unwrap();
    assert_eq!(doc.revisions.first().map(|r| r.date.as_str()), Some(""));
    let back = import(&export(&doc).unwrap()).unwrap();
    assert!(back.revisions.first().is_some_and(|r| r.date.len() >= 19 && r.date.starts_with("20")), "{:?}", back.revisions);
    assert_eq!(marks(&back), marks(&doc));
    assert_eq!(marks(&back).len(), 1);
}

#[test]
fn spaces_after_a_tab_survive_a_round_trip() {
    for src in ["a\t b", "a\t  b", "a\t   éa", "é\t 😀a", "a\t\t b"] {
        let doc = Document::from_text(src);
        let back = import(&export(&doc).unwrap()).unwrap();
        assert_eq!(back.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), src, "{src:?}");
    }
}

#[test]
fn whitespace_in_attributes_is_trimmed() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text>
            <text:h text:outline-level=" 2 ">Heading</text:h>
            <text:p>A<text:s text:c=" 3 "/>B</text:p>
            <table:table>
                <table:table-column/>
                <table:table-row>
                    <table:table-cell table:number-columns-spanned=" 2 " table:number-rows-spanned=" 1 ">
                        <text:p>Cell</text:p>
                    </table:table-cell>
                </table:table-row>
            </table:table>
            </office:text></office:body></office:document-content>"#
    );
    let flow = parse(&package(&content, None)).unwrap();
    let h = match &flow.blocks[0] {
        FBlock::Para(p) => p.kind,
        _ => panic!("expected heading para"),
    };
    assert_eq!(h, Kind::Heading(2));

    let p_text = match &flow.blocks[1] {
        FBlock::Para(p) => p.text(),
        _ => panic!("expected paragraph"),
    };
    assert_eq!(p_text, "A   B");

    let colspan = match &flow.blocks[2] {
        FBlock::Table(t) => t.rows[0][0].colspan,
        _ => panic!("expected table"),
    };
    assert_eq!(colspan, 2);
}

#[test]
fn explicit_spaces_at_paragraph_ends_are_kept() {
    // `text:s` spaces are content; only collapsible XML whitespace is trimmed at the ends.
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p><text:s text:c="4"/>lead</text:p><text:p>trail<text:s text:c="2"/></text:p><text:p>  plain  </text:p><text:p>  <text:s/>x <text:s/> </text:p></office:text></office:body></office:document-content>"#
    );
    let f = parse(&package(&content, None)).unwrap();
    let texts: Vec<String> = paras(&f).iter().map(|p| p.text()).collect();
    assert_eq!(texts, vec!["    lead", "trail  ", "plain", " x  "]);
    let d = Document::from_text("    indented code  ");
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(back.plain_text(wordcraft_doc::StoryRef::Body).trim_end_matches('\n'), "    indented code  ");
    for t in ["trail ", "trail  "] {
        let d = Document::from_text(t);
        let back = import(&export(&d).unwrap()).unwrap();
        assert_eq!(back.plain_text(wordcraft_doc::StoryRef::Body).trim_end_matches('\n'), t);
    }
}

#[test]
fn test_asian_complex_and_extended_text_properties() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
                <style:style style:name="AsianBold" style:family="paragraph"><style:text-properties style:font-weight-asian="bold"/></style:style>
                <style:style style:name="ComplexItalic" style:family="paragraph"><style:text-properties style:font-style-complex="italic"/></style:style>
                <style:style style:name="UlType" style:family="paragraph"><style:text-properties style:text-underline-type="single"/></style:style>
                <style:style style:name="StrikeType" style:family="paragraph"><style:text-properties style:text-line-through-type="double"/></style:style>
                <style:style style:name="CaseAlign" style:family="paragraph"><style:paragraph-properties fo:text-align="CENTER" fo:break-before="PAGE"/><style:text-properties fo:font-weight="BOLD"/></style:style>
            </office:automatic-styles><office:body><office:text>
                <text:p text:style-name="AsianBold">Asian</text:p>
                <text:p text:style-name="ComplexItalic">Complex</text:p>
                <text:p text:style-name="UlType">Underlined</text:p>
                <text:p text:style-name="StrikeType">Struck</text:p>
                <text:p text:style-name="CaseAlign">Centered</text:p>
            </office:text></office:body></office:document-content>"#
    );
    let f = parse(&package(&content, None)).unwrap();
    let ps = paras(&f);
    assert_eq!(ps.len(), 5);

    let fmt = |p: &Para| p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap();

    assert!(fmt(ps[0]).bold);
    assert!(fmt(ps[1]).italic);
    assert!(fmt(ps[2]).underline);
    assert!(fmt(ps[3]).strike);
    assert!(fmt(ps[4]).bold);
    assert_eq!(ps[4].align, Some(Align::Center));
    assert!(ps[4].page_break);
}

#[test]
fn latin_attributes_win_over_asian_and_line_types() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
                <style:style style:name="AsianOnly" style:family="paragraph"><style:text-properties fo:font-weight="normal" style:font-weight-asian="bold" fo:font-style="normal" style:font-style-complex="italic"/></style:style>
                <style:style style:name="NoLines" style:family="paragraph"><style:text-properties style:text-underline-style="none" style:text-underline-type="single" style:text-line-through-style="none" style:text-line-through-type="double"/></style:style>
            </office:automatic-styles><office:body><office:text>
                <text:p text:style-name="AsianOnly">Latin</text:p>
                <text:p text:style-name="NoLines">Plain</text:p>
            </office:text></office:body></office:document-content>"#
    );
    let f = parse(&package(&content, None)).unwrap();
    let ps = paras(&f);
    let fmt = |p: &Para| p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap();
    assert!(!fmt(ps[0]).bold, "Asian bold does not make Latin text bold");
    assert!(!fmt(ps[0]).italic, "complex-script italic does not make Latin text italic");
    assert!(!fmt(ps[1]).underline, "a line type does not override a line style of none");
    assert!(!fmt(ps[1]).strike);
}

#[test]
fn meta_creation_date_timezones_and_fallbacks() {
    let xml_neg = br#"<office:document-meta><office:meta>
            <meta:creation-date>2024-03-05T13:45:00-05:00</meta:creation-date>
            <dc:creator>Modifier</dc:creator>
            <meta:initial-creator>Original Author</meta:initial-creator>
        </office:meta></office:document-meta>"#;
    let m = read_meta(xml_neg);
    assert_eq!(m.created, "2024-03-05T13:45:00-05:00");
    assert_eq!(m.author, "Original Author");

    let xml_neg_frac = br#"<office:document-meta><office:meta>
            <meta:creation-date>2024-03-05T13:45:00.500-05:00</meta:creation-date>
        </office:meta></office:document-meta>"#;
    assert_eq!(read_meta(xml_neg_frac).created, "2024-03-05T13:45:00.500-05:00");

    let xml_local_frac = br#"<office:document-meta><office:meta>
            <meta:creation-date>2024-03-05T13:45:00.123</meta:creation-date>
        </office:meta></office:document-meta>"#;
    assert_eq!(read_meta(xml_local_frac).created, "2024-03-05T13:45:00Z");

    let xml_z_lower = br#"<office:document-meta><office:meta>
            <meta:creation-date>2024-03-05T13:45:00z</meta:creation-date>
        </office:meta></office:document-meta>"#;
    assert_eq!(read_meta(xml_z_lower).created, "2024-03-05T13:45:00z");

    let xml_dc_date_fallback = br#"<office:document-meta><office:meta>
            <dc:date>2024-03-05T13:45:00Z</dc:date>
            <dc:creator>Solo Creator</dc:creator>
        </office:meta></office:document-meta>"#;
    let m2 = read_meta(xml_dc_date_fallback);
    assert_eq!(m2.created, "2024-03-05T13:45:00Z");
    assert_eq!(m2.author, "Solo Creator");
}
