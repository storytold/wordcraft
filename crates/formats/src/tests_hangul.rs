//! Hangul HWPX import: synthetic documents and hostile input.

use wordcraft_doc::props::{LineSpacing, TabAlign, TabLeader, VMerge};
use wordcraft_doc::{Align, Block, Document};

use crate::hangul_samples;
use crate::{hwpx, import};

fn paras(doc: &Document) -> Vec<&wordcraft_doc::Paragraph> {
    doc.body.iter().filter_map(|b| b.as_para()).collect()
}

/// What the synthetic document holds.
fn check(doc: &Document) {
    let ps = paras(doc);
    let first = ps.first().unwrap();
    assert_eq!(first.text, "Hello bold");
    let bold: Vec<_> = first.run_ranges().filter(|(_, p)| p.bold == Some(true)).map(|(r, _)| &first.text[r]).collect();
    assert_eq!(bold, ["bold"]);
    let props = first.props_at(0);
    assert_eq!(props.size, Some(12.0));
    assert_eq!(props.font_ea.as_deref(), Some("함초롬바탕"));
    assert_eq!(props.font.as_deref(), Some("Times New Roman"));
    assert_eq!(props.color, Some(wordcraft_doc::TextColor::Rgb(wordcraft_doc::Rgb(255, 0, 0))));
    assert_eq!(first.props.align, Some(Align::Justify));
    assert_eq!(first.props.space_after, Some(5.0));
    // 160 % of the 12 pt text.
    assert_eq!(first.props.line_spacing, Some(LineSpacing::AtLeast(19.2)));
    let centred = ps.get(1).unwrap();
    assert!(centred.text.starts_with("가운데"), "{:?}", centred.text);
    assert!(centred.text.ends_with("한글"));
    assert_eq!(centred.props.align, Some(Align::Center));
    // The table: first row one cell over both columns, borders from the border fill.
    let t = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.grid, [72.0, 72.0]);
    let r0 = &t.rows[0].cells;
    assert_eq!(r0.len(), 1);
    assert_eq!(r0[0].props.span, 2);
    assert_eq!(r0[0].blocks[0].as_para().unwrap().text, "merged");
    assert!(r0[0].props.borders.unwrap().top.unwrap().is_visible());
    let r1: Vec<_> = t.rows[1].cells.iter().map(|c| c.blocks[0].as_para().unwrap().text.clone()).collect();
    assert_eq!(r1, ["A2", "B2"]);
    assert!(t.rows.iter().flat_map(|r| &r.cells).all(|c| c.props.vmerge == VMerge::None));
    // The paragraph that held only the table leaves nothing behind; the next one follows.
    let i = doc.body.iter().position(|b| matches!(**b, Block::Table(_))).unwrap();
    assert_eq!(doc.body[i + 1].as_para().unwrap().text, "After");
    // A5 portrait; the body starts below the header.
    let s = &doc.last_section;
    assert!((s.page_w - 419.54).abs() < 0.01 && (s.page_h - 595.28).abs() < 0.01, "{} × {}", s.page_w, s.page_h);
    assert!((s.margin_top - (42.52 + 28.35)).abs() < 0.01);
    assert!((s.header - 42.52).abs() < 0.01);
}

#[test]
fn hwpx_package() {
    let bytes = hangul_samples::hwpx();
    assert!(hwpx::is_hwpx(&bytes));
    let doc = import("HWPX", &bytes).unwrap().unwrap();
    check(&doc);
    // `hp:tab` is a tab; a plain `hh:tabItem` position is at twice HWPUNIT (20000 = 100 pt).
    let centred = paras(&doc)[1];
    assert_eq!(centred.text, "가운데\t한글");
    let tab = centred.props.tabs.as_ref().unwrap()[0];
    assert_eq!((tab.pos, tab.align, tab.leader), (100.0, TabAlign::Right, TabLeader::Dot));
    // The cell's border fill shades it.
    let t = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows[0].cells[0].props.shading, Some(wordcraft_doc::Rgb(255, 255, 0)));
}

#[test]
fn refused_documents_say_why() {
    // An encrypted HWPX package.
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("mimetype", o).unwrap();
    std::io::Write::write_all(&mut z, b"application/hwp+zip").unwrap();
    z.start_file("META-INF/manifest.xml", o).unwrap();
    std::io::Write::write_all(&mut z, b"<manifest><file-entry><encryption-data/></file-entry></manifest>").unwrap();
    let bytes = z.finish().unwrap().into_inner();
    assert!(hwpx::import(&bytes).unwrap_err().contains("password"));
    // Not Hangul at all.
    assert!(hwpx::import(b"not a document").is_err());
    assert!(hwpx::import(b"PK\x03\x04junk").is_err());
    assert!(!hwpx::is_hwpx(b"not") && !hwpx::is_hwpx(b"PK\x03\x04"));
}

/// Every truncation and many bit flips of the samples read or fail cleanly.
#[test]
fn damaged_samples_never_panic() {
    let samples = [("hwpx", hangul_samples::hwpx())];
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for (ext, bytes) in &samples {
        for cut in (0..bytes.len()).step_by(97) {
            let _ = import(ext, &bytes[..cut]);
        }
        for _ in 0..300 {
            let mut b = bytes.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed as usize) % b.len();
                b[i] ^= 1 << (seed >> 61);
            }
            let _ = import(ext, &b);
        }
    }
}

/// Junk parts inside a well-formed container: the parts themselves are hostile.
#[test]
fn hostile_records_never_panic() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    for len in [0usize, 3, 4, 17, 64, 300, 2048] {
        for _ in 0..40 {
            let junk: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            // Junk as a HWPX section and header.
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let o = zip::write::SimpleFileOptions::default();
            for name in ["Contents/header.xml", "Contents/section0.xml"] {
                z.start_file(name, o).unwrap();
                std::io::Write::write_all(&mut z, &junk).unwrap();
            }
            let _ = hwpx::import(&z.finish().unwrap().into_inner());
        }
    }
}

/// Deeply nested tables and huge declared spans stay bounded.
#[test]
fn deep_and_wide_hwpx() {
    let mut cell = String::from("<hp:p><hp:run><hp:t>x</hp:t></hp:run></hp:p>");
    for _ in 0..200 {
        cell = format!(
            r#"<hp:p><hp:run><hp:tbl><hp:tr><hp:tc><hp:subList>{cell}</hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="99999" rowSpan="4000000000"/><hp:cellSz width="-5" height="1e300"/></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#
        );
    }
    let section = format!(r#"<hs:sec xmlns:hs="s" xmlns:hp="p">{cell}</hs:sec>"#);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, text) in [("mimetype", "application/hwp+zip"), ("Contents/header.xml", "<hh:head/>"), ("Contents/section0.xml", section.as_str())] {
        z.start_file(name, o).unwrap();
        std::io::Write::write_all(&mut z, text.as_bytes()).unwrap();
    }
    let doc = hwpx::import(&z.finish().unwrap().into_inner()).unwrap();
    assert!(doc.body.iter().any(|b| b.as_table().is_some()));
}

/// A ZIP of these entries.
fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, text) in entries {
        z.start_file(*name, o).unwrap();
        std::io::Write::write_all(&mut z, text.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// A HWPX package of one section.
fn package(section: &str) -> Vec<u8> {
    zip_of(&[("mimetype", "application/hwp+zip"), ("Contents/header.xml", "<hh:head/>"), ("Contents/section0.xml", section)])
}

/// A section of one paragraph.
fn section_of(text: &str) -> String {
    format!(r#"<hs:sec xmlns:hs="s" xmlns:hp="p"><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p></hs:sec>"#)
}

/// The elements and text one import builds are bounded: past the budget the import fails
/// cleanly instead of building a tree many times the size of the file.
#[test]
fn xml_tree_budget_stops_the_import() {
    let wide = format!(r#"<hs:sec xmlns:hs="s" xmlns:hp="p">{}</hs:sec>"#, "<a/>".repeat(1000));
    let bytes = package(&wide);
    assert!(hwpx::import(&bytes).is_ok());
    let err = hwpx::import_within(&bytes, hwpx::Budget::new(500, usize::MAX, usize::MAX)).unwrap_err();
    assert!(err.starts_with("HWPX:") && err.contains("too many elements"), "{err}");
    // Text counts too.
    let long = package(&section_of(&"x".repeat(1000)));
    assert!(hwpx::import_within(&long, hwpx::Budget::new(usize::MAX, 500, usize::MAX)).is_err());
    assert!(hwpx::import_within(&long, hwpx::Budget::new(usize::MAX, 2000, usize::MAX)).is_ok());
}

/// A spine naming one section many times (the same idref, or two ids with one href) reads it once.
#[test]
fn repeated_spine_refs_read_a_section_once() {
    let hpf = r#"<opf:package xmlns:opf="o"><opf:manifest><opf:item id="s0" href="Contents/section0.xml"/><opf:item id="s1" href="/Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="s0"/><opf:itemref idref="s0"/><opf:itemref idref="s1"/></opf:spine></opf:package>"#;
    let section = section_of("once");
    let bytes = zip_of(&[
        ("mimetype", "application/hwp+zip"),
        ("Contents/header.xml", "<hh:head/>"),
        ("Contents/content.hpf", hpf),
        ("Contents/section0.xml", &section),
    ]);
    let doc = hwpx::import(&bytes).unwrap();
    assert_eq!(paras(&doc).iter().filter(|p| p.text == "once").count(), 1);
}

/// Every section's inflated XML is charged to one document-wide budget.
#[test]
fn xml_byte_budget_spans_sections() {
    let (a, b) = (section_of("first"), section_of("second"));
    let bytes = zip_of(&[
        ("mimetype", "application/hwp+zip"),
        ("Contents/header.xml", "<hh:head/>"),
        ("Contents/section0.xml", &a),
        ("Contents/section1.xml", &b),
    ]);
    let header = "<hh:head/>".len();
    // Room for the header and either section alone, not both.
    let one = header + a.len().max(b.len());
    let err = hwpx::import_within(&bytes, hwpx::Budget::new(usize::MAX, usize::MAX, one)).unwrap_err();
    assert!(err.contains("too large"), "{err}");
    let doc = hwpx::import_within(&bytes, hwpx::Budget::new(usize::MAX, usize::MAX, header + a.len() + b.len())).unwrap();
    assert_eq!(paras(&doc).len(), 2);
}

/// A table keeps to the rows the file declares and to a grid in proportion to its cells, and a
/// page to Word's 22 inches, whatever the addresses, spans and sizes say.
#[test]
fn table_grid_and_page_stay_bounded() {
    let cell = |row: u32, rows: u32| {
        format!(
            r#"<hp:tc><hp:subList><hp:p><hp:run><hp:t>x</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="{row}"/><hp:cellSpan colSpan="1" rowSpan="{rows}"/><hp:cellSz width="7200" height="1000"/></hp:tc>"#
        )
    };
    let table = |attrs: &str, cells: String| format!(r#"<hp:p><hp:run><hp:tbl {attrs}><hp:tr>{cells}</hp:tr></hp:tbl></hp:run></hp:p>"#);
    let section = format!(
        r#"<hs:sec xmlns:hs="s" xmlns:hp="p"><hp:p><hp:run><hp:secPr><hp:pagePr width="99999999" height="99999999"><hp:margin left="5669" right="5669" top="4252" bottom="4252"/></hp:pagePr></hp:secPr></hp:run></hp:p>{}{}</hs:sec>"#,
        // One cell merged down 30,000 rows, no declared row count: the grid stays small.
        table("", cell(0, 30_000)),
        // A declared row count of 2: a cell merged down 100 rows and a cell below them are cut.
        table(r#"rowCnt="2" colCnt="1""#, format!("{}{}", cell(0, 100), cell(5, 1))),
    );
    let doc = hwpx::import(&package(&section)).unwrap();
    let tables: Vec<_> = doc.body.iter().filter_map(|b| b.as_table()).collect();
    assert_eq!(tables.len(), 2);
    assert!(tables[0].rows.len() <= 4096 + 64, "{} rows", tables[0].rows.len());
    assert_eq!(tables[1].rows.len(), 2);
    assert!(doc.sections().iter().all(|(_, s)| s.page_w <= 1584.0 && s.page_h <= 1584.0));
}

/// A picture keeps to Word's 22 inches whatever size it claims, and a `mimetype` entry is read no
/// further than a real one could be.
#[test]
fn picture_size_and_mimetype_stay_bounded() {
    let mut png = Vec::new();
    image::RgbaImage::new(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let section = r#"<hs:sec xmlns:hs="s" xmlns:hp="p" xmlns:hc="c"><hp:p><hp:run><hp:pic><hp:sz width="99999999" height="99999999"/><hc:img binaryItemIDRef="image1"/></hp:pic></hp:run></hp:p></hs:sec>"#;
    let pkg = |mime: &[u8]| {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        for (name, data) in
            [("mimetype", mime), ("Contents/header.xml", b"<hh:head/>"), ("Contents/section0.xml", section.as_bytes()), ("BinData/image1", &png)]
        {
            z.start_file(name, o).unwrap();
            std::io::Write::write_all(&mut z, data).unwrap();
        }
        z.finish().unwrap().into_inner()
    };
    let doc = hwpx::import(&pkg(b"application/hwp+zip")).unwrap();
    let sizes: Vec<_> = paras(&doc)
        .iter()
        .flat_map(|p| &p.objects)
        .filter_map(|o| if let wordcraft_doc::InlineObject::Image { w, h, .. } = o { Some((*w, *h)) } else { None })
        .collect();
    assert_eq!(sizes, [(1584.0, 1584.0)]);
    // The MIME type followed by a megabyte of padding is not a HWPX package.
    let padded = [b"application/hwp+zip".as_slice(), &vec![b' '; 1 << 20]].concat();
    assert!(!hwpx::is_hwpx(&pkg(&padded)));
}

/// A text box holding a picture keeps its text: the picture is read with the box's paragraphs.
#[test]
fn text_box_with_a_picture_keeps_its_text() {
    let mut png = Vec::new();
    image::RgbaImage::new(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let section = r#"<hs:sec xmlns:hs="s" xmlns:hp="p" xmlns:hc="c"><hp:p><hp:run><hp:t>before</hp:t><hp:rect><hp:drawText><hp:subList><hp:p><hp:run><hp:t>boxed</hp:t><hp:pic><hp:sz width="7200" height="7200"/><hc:img binaryItemIDRef="image1"/></hp:pic></hp:run></hp:p></hp:subList></hp:drawText></hp:rect></hp:run></hp:p></hs:sec>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, data) in [
        ("mimetype", b"application/hwp+zip".as_slice()),
        ("Contents/header.xml", b"<hh:head/>"),
        ("Contents/section0.xml", section.as_bytes()),
        ("BinData/image1", &png),
    ] {
        z.start_file(name, o).unwrap();
        std::io::Write::write_all(&mut z, data).unwrap();
    }
    let doc = hwpx::import(&z.finish().unwrap().into_inner()).unwrap();
    let ps = paras(&doc);
    assert_eq!(ps.iter().map(|p| p.text.trim_matches('\u{FFFC}')).collect::<Vec<_>>(), ["before", "boxed"]);
    assert_eq!(ps[1].objects.len(), 1);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 64, ..Default::default() })]
    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048)) {
        let _ = hwpx::import(&bytes);
    }
}
