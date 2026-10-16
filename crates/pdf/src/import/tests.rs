use super::*;
use wordcraft_doc::{Block, StoryRef};

/// Original synthetic PDF, with explicit glyph placement rather than an export-only oracle.
fn pdf(streams: &[&str], extra: &[&str]) -> Vec<u8> {
    let kids = (0..streams.len()).map(|i| format!("{} 0 R", 4 + i * 2)).collect::<Vec<_>>().join(" ");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Count {} /Kids [{kids}] >>", streams.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for (i, content) in streams.iter().enumerate() {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> /XObject << /Im {} 0 R >> >> /Contents {} 0 R >>", 4+streams.len()*2, 5+i*2));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    objects.extend(extra.iter().map(|s| s.to_string()));
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF", offsets.len()).as_bytes());
    bytes
}

#[test]
fn positioned_words_reflow_without_gluing_or_duplicating_stroked_text() {
    let bytes = pdf(
        &[
            "BT /F1 12 Tf 1 0 0 1 72 720 Tm (Hello) Tj 1 0 0 1 108 720 Tm (world) Tj 1 0 0 1 72 705 Tm (next line) Tj 2 Tr 1 0 0 1 72 660 Tm (Bold) Tj 1 Tr 1 0 0 1 72 630 Tm (Outline) Tj ET",
        ],
        &[],
    );
    let (doc, summary) = read(&bytes, None).unwrap();
    let text = doc.plain_text(StoryRef::Body);
    assert!(text.contains("Hello world next line"), "{text:?}");
    assert!(text.contains("Bold") && !text.contains("BBoolldd"), "{text:?}");
    assert!(text.contains("Outline"), "stroke-only text: {text:?}");
    assert_eq!(summary.pages, 1);
    for p in doc.body.iter().filter_map(|b| if let Block::Para(p) = b.as_ref() { Some(p) } else { None }) {
        assert_eq!(p.runs.iter().map(|r| r.len).sum::<usize>(), p.text.len());
    }
}

#[test]
fn pages_are_separated_and_exported_unicode_styles_remain_editable() {
    let mut source = Document::new();
    let mut heading = Paragraph::with_text(
        "Grüße aus Köln",
        CharProps { size: Some(20.0), bold: Some(true), color: Some(TextColor::Rgb(Rgb(200, 0, 0))), ..Default::default() },
    );
    heading.props.page_break_before = Some(true);
    source.body = vec![para_block(Paragraph::with_text("First page", CharProps::default())), para_block(heading)];
    let bytes = crate::export(&source, &Default::default()).unwrap();
    let (doc, summary) = read(&bytes, None).unwrap();
    assert_eq!(summary.pages, 2);
    assert!(doc.plain_text(StoryRef::Body).contains("Grüße aus Köln"));
    let p = doc.body.iter().filter_map(|b| if let Block::Para(p) = b.as_ref() { Some(p) } else { None }).find(|p| p.text.contains("Grüße")).unwrap();
    assert_eq!(p.props.page_break_before, Some(true));
    assert!(p.runs.iter().any(|r| r.props.size.is_some_and(|s| (s - 20.0).abs() < 0.2)));
    assert!(p.runs.iter().any(|r| r.props.color == Some(TextColor::Rgb(Rgb(200, 0, 0)))));
}

#[test]
fn scan_is_an_inline_picture_with_an_explicit_no_ocr_warning() {
    let bytes = pdf(
        &["q 200 0 0 100 72 500 cm /Im Do Q"],
        &["<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 6 >>\nstream\nabcdef\nendstream"],
    );
    let (doc, summary) = read(&bytes, None).unwrap();
    assert_eq!(summary.images, 1);
    assert_eq!(summary.text_characters, 0);
    assert!(summary.warnings.iter().any(|w| w.contains("OCR")));
    let p = doc.body[0].as_ref();
    let Block::Para(p) = p else { panic!("paragraph") };
    let InlineObject::Image { w, h, .. } = &p.objects[0] else { panic!("picture") };
    assert!((*w - 200.0).abs() < 0.1 && (*h - 100.0).abs() < 0.1);
}

#[test]
fn malformed_oversized_and_graphics_only_input_do_not_silently_become_blank_documents() {
    assert!(matches!(read(b"%PDF-invalid", None), Err(ImportError::Invalid)));
    assert!(matches!(read(&vec![0; MAX_BYTES + 1], None), Err(ImportError::Limit(_))));
    let bytes = pdf(&["0 0 100 100 re f"], &[]);
    assert!(matches!(read(&bytes, None), Err(ImportError::NoContent)));
    let bytes = pdf(
        &["q 100 0 0 100 0 0 cm /Im Do Q"],
        &[
            "<< /Type /XObject /Subtype /Image /Width 100000 /Height 100000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 1 >>\nstream\nx\nendstream",
        ],
    );
    assert!(matches!(read(&bytes, None), Err(ImportError::Limit(_))));
}

#[test]
fn content_sniff_is_bounded_and_recognizes_a_leading_wrapper() {
    assert!(is_pdf(b"\n%PDF-1.7\n"));
    assert!(!is_pdf(b"plain text"));
    let mut late = vec![b' '; 1024];
    late.extend_from_slice(b"%PDF-");
    assert!(!is_pdf(&late));
}

#[test]
fn standard_font_weight_and_slant_survive_without_embedded_font_data() {
    let bytes = pdf(&["BT /F1 18 Tf 72 720 Td (Styled text) Tj ET"], &[]);
    let bytes = String::from_utf8(bytes).unwrap().replace("/Helvetica", "/Helvetica-BoldOblique").into_bytes();
    // hayro repairs the shifted xref; the content offsets themselves are unchanged.
    let (doc, _) = read(&bytes, None).unwrap();
    let p = doc.body[0].as_para().unwrap();
    assert_eq!(p.runs[0].props.bold, Some(true));
    assert_eq!(p.runs[0].props.italic, Some(true));
    assert_eq!(p.runs[0].props.font.as_deref(), Some("Helvetica"));
}

#[test]
fn password_protected_pdf_uses_the_supplied_password() {
    // Original synthetic one-line PDF, generated with pypdf AES-128 (password: secret).
    // Stored as bytes in code so the test needs neither Python nor an external PDF fixture.
    let bytes = [
        b"%PDF-1.3\x0a%\xe2\xe3\xcf\xd3\x0a1 0 obj\x0a<<\x0a/Producer <7c8b3847363dadfaf69855f33147039e6".as_slice(),
        b"8d90f94df082e67a8dd5bb0a9ef5c2a>\x0a>>\x0aendobj\x0a2 0 obj\x0a<<\x0a/Type /Pages\x0a/Co".as_slice(),
        b"unt 1\x0a/Kids [ 4 0 R ]\x0a>>\x0aendobj\x0a3 0 obj\x0a<<\x0a/Type /Catalog\x0a/Pages 2 0 R".as_slice(),
        b"\x0a>>\x0aendobj\x0a4 0 obj\x0a<<\x0a/Type /Page\x0a/Resources <<\x0a/Font <<\x0a/F1 5 0 R\x0a>>\x0a".as_slice(),
        b">>\x0a/MediaBox [ 0.0 0.0 612 792 ]\x0a/Parent 2 0 R\x0a/Contents 6 0 R\x0a>>\x0aendo".as_slice(),
        b"bj\x0a5 0 obj\x0a<<\x0a/Type /Font\x0a/Subtype /Type1\x0a/BaseFont /Helvetica\x0a>>\x0aendo".as_slice(),
        b"bj\x0a6 0 obj\x0a<<\x0a/Length 80\x0a>>\x0astream\x0a3E\xc4\xaeDO\x94\xda1\x9b\x11\x16S\xe2\xc5\xb0\xa2\xf7\xe4O\xf4\x12\x08J\xa7I\xc0\xf5B\x1d\x0d\xbc\xa8\xa4\x8d".as_slice(),
        b"\xcb\x0b\x04\x90\xa8e\xc7-{\xa9\xc5B\x223\xdcp&\xda\x98\xa5\xbc2\xc8\xce\x0cS\xdc\xfe\xbd\x11\xf8/\xc8\xf5\xd9\xb0\x12A\xf3\xae\x8at#\xc0\xd1\x0aendstream\x0aendobj\x0a7 0 obj".as_slice(),
        b"\x0a<<\x0a/V 4\x0a/R 4\x0a/Length 128\x0a/P 4294967292\x0a/Filter /Standard\x0a/O <cae12a13".as_slice(),
        b"706437b2a133a2021c2c7f1f1b0692d87066efdbef7b1b00e6c60758>\x0a/U <0e5f158e".as_slice(),
        b"c104f5573a4c910906a6a7c328bf4e5e4e758a4164004e56fffa0108>\x0a/CF <<\x0a/StdC".as_slice(),
        b"F <<\x0a/AuthEvent /DocOpen\x0a/CFM /AESV2\x0a/Length 16\x0a>>\x0a>>\x0a/StmF /StdCF\x0a/St".as_slice(),
        b"rF /StdCF\x0a>>\x0aendobj\x0axref\x0a0 8\x0a0000000000 65535 f \x0a0000000015 00000 n \x0a0".as_slice(),
        b"000000113 00000 n \x0a0000000172 00000 n \x0a0000000221 00000 n \x0a0000000353 ".as_slice(),
        b"00000 n \x0a0000000423 00000 n \x0a0000000553 00000 n \x0atrailer\x0a<<\x0a/Size 8\x0a/R".as_slice(),
        b"oot 3 0 R\x0a/Info 1 0 R\x0a/ID [ <38636631323837343234373839336537616164653".as_slice(),
        b"13234316361323465366634> <38636631323837343234373839336537616164653132".as_slice(),
        b"34316361323465366634> ]\x0a/Encrypt 7 0 R\x0a>>\x0astartxref\x0a860\x0a%%EOF\x0a".as_slice(),
    ].concat();
    assert!(matches!(read(&bytes, None), Err(ImportError::NeedsPassword)));
    assert!(matches!(read(&bytes, Some("wrong")), Err(ImportError::WrongPassword)));
    let (doc, _) = read(&bytes, Some("secret")).unwrap();
    assert!(doc.plain_text(StoryRef::Body).contains("Password protected text"));
}

#[test]
fn quarter_turned_scan_keeps_its_pixel_orientation() {
    let bytes = pdf(
        &["q 200 0 0 100 72 500 cm /Im Do Q"],
        &["<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 6 >>\nstream\nabcdef\nendstream"],
    );
    let bytes = String::from_utf8(bytes).unwrap().replace("/MediaBox", "/Rotate 90 /MediaBox").into_bytes();
    let (doc, _) = read(&bytes, None).unwrap();
    let p = doc.body[0].as_para().unwrap();
    let InlineObject::Image { media, w, h, .. } = &p.objects[0] else { panic!("picture") };
    let image = image::load_from_memory(&doc.media[media]).unwrap().into_rgba8();
    assert_eq!(image.dimensions(), (1, 2));
    assert_eq!(image.get_pixel(0, 0).0, [b'a', b'b', b'c', 255]);
    assert_eq!(image.get_pixel(0, 1).0, [b'd', b'e', b'f', 255]);
    assert!((*w - 100.0).abs() < 0.1 && (*h - 200.0).abs() < 0.1);
}
