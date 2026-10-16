//! Hangul (HWP 5.0, HWPX) import: synthetic documents and hostile input.

use wordcraft_doc::props::{LineSpacing, TabAlign, TabLeader, VMerge};
use wordcraft_doc::{Align, Block, Document};

use crate::hangul_samples;
use crate::{hwp, hwpx, import};

fn paras(doc: &Document) -> Vec<&wordcraft_doc::Paragraph> {
    doc.body.iter().filter_map(|b| b.as_para()).collect()
}

/// What both synthetic documents hold.
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
fn hwp_uncompressed_and_compressed() {
    for compressed in [false, true] {
        let bytes = hangul_samples::hwp(compressed);
        assert!(hwp::is_hwp(&bytes));
        let doc = import("hwp", &bytes).unwrap().unwrap();
        check(&doc);
    }
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
    // Password-protected and distribution documents.
    for (flag, word) in [(2u8, "password"), (4, "distribution")] {
        let mut bytes = hangul_samples::hwp(false);
        // The FileHeader stream's flags byte: find the signature, flags at +36.
        let at = bytes.windows(17).position(|w| w == b"HWP Document File").unwrap();
        bytes[at + 36] = flag;
        let err = hwp::import(&bytes).unwrap_err();
        assert!(err.starts_with("HWP: ") && err.contains(word), "{err}");
    }
    // HWP 3 (Hangul 97) files are not compound files.
    let err = hwp::import(b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05").unwrap_err();
    assert!(err.contains("HWP 3"), "{err}");
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
    assert!(hwp::import(b"not a document").is_err());
    assert!(hwpx::import(b"PK\x03\x04junk").is_err());
    assert!(!hwp::is_hwp(b"not") && !hwpx::is_hwpx(b"PK\x03\x04"));
}

/// Every truncation and many bit flips of the samples read or fail cleanly.
#[test]
fn damaged_samples_never_panic() {
    let samples = [("hwp", hangul_samples::hwp(false)), ("hwp", hangul_samples::hwp(true)), ("hwpx", hangul_samples::hwpx())];
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

/// Damaged records inside a well-formed container: the record streams themselves are hostile.
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
            let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
            let mut header = b"HWP Document File".to_vec();
            header.resize(32, 0);
            header.extend(0x0500_0000u32.to_le_bytes());
            header.extend(0u32.to_le_bytes());
            header.resize(256, 0);
            comp.create_storage("/BodyText").unwrap();
            for (name, data) in [("/FileHeader", &header), ("/DocInfo", &junk), ("/BodyText/Section0", &junk)] {
                std::io::Write::write_all(&mut comp.create_stream(name).unwrap(), data).unwrap();
            }
            let bytes = comp.into_inner().into_inner();
            let _ = hwp::import(&bytes);
            // The same junk as a HWPX section and header.
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

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 64, ..Default::default() })]
    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048)) {
        let _ = hwp::import(&bytes);
        let _ = hwpx::import(&bytes);
        let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        ole.extend(&bytes);
        let _ = hwp::import(&ole);
    }
}
