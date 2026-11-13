//! Unit tests build tiny PDFs in memory: no fixture files, so no asset attribution is needed.

use super::*;

/// A one-page PDF with a blue rectangle. Offsets are computed so the xref table is valid.
fn tiny_pdf(page_w: f32, page_h: f32) -> Vec<u8> {
    let content = b"0 0 1 rg 10 10 100 50 re f\n";
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"endstream");
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w} {page_h}] /Resources << >> /Contents 4 0 R >>").into_bytes(),
        stream,
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

#[test]
fn opens_and_renders_a_pdf_page() {
    let view = PdfView::open(tiny_pdf(200.0, 100.0)).expect("a valid PDF opens");
    assert_eq!(view.page_count(), 1);
    let size = view.page_size(0).expect("one page");
    assert!((size.w - 200.0).abs() < 0.5);
    assert!((size.h - 100.0).abs() < 0.5);
    let raster = view.render(0, 1.0).expect("the page rasterises");
    assert_eq!((raster.width, raster.height), (200, 100));
    assert_eq!(raster.pixels.len(), 200 * 100 * 4);
    // The background is opaque white and the rectangle is opaque blue. PDF coordinates run
    // upward from the bottom, so count colours instead of assuming where the rectangle lands.
    let px = raster.pixels.as_chunks::<4>().0;
    let white = px.iter().filter(|p| **p == [255, 255, 255, 255]).count();
    let blue = px.iter().filter(|p| **p == [0, 0, 255, 255]).count();
    assert!(white > 100, "no white paper: {white}");
    assert!(blue > 100, "the blue rectangle was not drawn: {blue}");
}

#[test]
fn rejects_non_pdf_and_truncated_files() {
    for bytes in [b"not a pdf".to_vec(), b"%PDF-".to_vec(), vec![0u8; 64], tiny_pdf(200.0, 100.0)[..20].to_vec()] {
        // Whatever the parser does, it returns an error instead of panicking.
        let _ = PdfView::open(bytes);
    }
    assert!(matches!(PdfView::open(b"hello".to_vec()), Err(PdfError::NotPdf)));
    assert!(matches!(PdfView::open(vec![b'x'; MAX_PDF_BYTES + 1]), Err(PdfError::TooLarge(_))));
}

#[test]
fn hostile_scales_and_indices_never_panic() {
    let view = PdfView::open(tiny_pdf(200.0, 100.0)).expect("a valid PDF opens");
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, 8.0] {
        let r = view.render(0, scale).expect("every scale renders");
        assert!(r.width >= 1 && r.height >= 1);
        assert!(r.width <= MAX_SIDE && r.height <= MAX_SIDE);
        assert_eq!(r.pixels.len(), (r.width as usize) * (r.height as usize) * 4);
    }
    assert!(matches!(view.render(999, 1.0), Err(PdfError::NoSuchPage(999))));
}

#[test]
fn huge_pages_are_scaled_not_allocated() {
    let view = PdfView::open(tiny_pdf(20_000.0, 20_000.0)).expect("a valid PDF opens");
    let raster = view.render(0, 8.0).expect("the page rasterises");
    assert!(raster.width <= MAX_SIDE && raster.height <= MAX_SIDE);
    assert!(u64::from(raster.width) * u64::from(raster.height) <= MAX_RASTER_PIXELS);
    assert_eq!(raster.pixels.len(), (raster.width as usize) * (raster.height as usize) * 4);
}

/// Assemble numbered indirect objects with a valid xref table.
fn assemble(objects: Vec<Vec<u8>>, trailer_extra: &str) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn stream_obj(content: &[u8]) -> Vec<u8> {
    let mut s = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    s.extend_from_slice(content);
    s.extend_from_slice(b"\nendstream");
    s
}

/// Two pages: page 1 has text ("Hello") and three links — an explicit destination, a named
/// destination via catalog `/Dests`, and an external URI. Page 2 is the jump target.
fn linked_text_pdf() -> Vec<u8> {
    // A minimal /ToUnicode CMap: H, e, l, o map to themselves. hayro reads this for any font
    // type, so the test does not need an embedded font program.
    let cmap = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
4 beginbfchar\n<48> <0048>\n<65> <0065>\n<6C> <006C>\n<6F> <006F>\nendbfchar\n\
endcmap\nend\nend\n";
    let tounicode = stream_obj(cmap);
    assemble(
        vec![
            b"<< /Type /Catalog /Pages 2 0 R /Dests 9 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R /Annots [10 0 R 11 0 R 12 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 6 0 R >>".to_vec(),
            stream_obj(b"BT /F1 12 Tf 20 700 Td (Hello) Tj ET\n"),
            stream_obj(b"0 0 1 rg 10 10 100 50 re f\n"),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 8 0 R >>".to_vec(),
            tounicode,
            b"<< /chapter2 [4 0 R /XYZ 0 700 null] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [10 700 60 720] /Dest [4 0 R /XYZ 0 700 1] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [70 700 130 720] /Dest /chapter2 >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [140 700 200 720] /A << /S /URI /URI (https://example.com) >> >>".to_vec(),
        ],
        "",
    )
}

#[test]
fn link_annotations_resolve_to_pages_and_urls() {
    let view = PdfView::open(linked_text_pdf()).expect("a valid PDF opens");
    assert_eq!(view.page_count(), 2);
    let links = view.links(0);
    assert_eq!(links.len(), 3, "explicit, named and URI links: {links:?}");
    assert!(matches!(links[0].target, LinkTarget::Page { page: 1, .. }), "{:?}", links[0].target);
    assert!(matches!(links[1].target, LinkTarget::Page { page: 1, .. }), "{:?}", links[1].target);
    assert_eq!(links[2].target, LinkTarget::Uri("https://example.com".into()));
    // /Rect [10 700 60 720] in y-up space becomes y = 842-720 .. 842-700.
    assert!(links[0].rect.x0 >= 9.0 && links[0].rect.x1 <= 61.0);
    assert!((links[0].rect.y0 - 122.0).abs() < 1.0, "{}", links[0].rect.y0);
    assert!((links[0].rect.y1 - 142.0).abs() < 1.0, "{}", links[0].rect.y1);
    // Explicit /XYZ top=700 is flipped to 142 points from the page top.
    assert!(matches!(links[0].target, LinkTarget::Page { top: Some(t), .. } if (t - 142.0).abs() < 1.0));
    assert!(matches!(view.link_at(0, 30.0, 132.0), Some(LinkTarget::Page { page: 1, .. })));
    assert!(matches!(view.link_at(0, 170.0, 132.0), Some(LinkTarget::Uri(_))));
    assert!(view.link_at(0, 300.0, 300.0).is_none());
    assert!(view.links(999).is_empty());
}

#[test]
fn text_layer_is_selectable_and_cached() {
    let view = PdfView::open(linked_text_pdf()).expect("a valid PDF opens");
    let chars = view.text(0).expect("page 0 text");
    assert!(!chars.is_empty(), "the interpreter should report the 'Hello' glyphs");
    let text: String = chars.iter().map(|c| c.text.as_str()).collect();
    assert!(text.contains("Hello"), "extracted: {text:?}");
    // All rectangles are finite, non-empty and inside the page.
    assert!(chars.iter().all(|c| c.rect.x0 >= 0.0 && c.rect.y0 >= 0.0 && c.rect.x1 <= 595.0 && c.rect.y1 <= 842.0));
    assert!(chars.iter().all(|c| !c.rect.is_empty()));
    // The cache returns the same allocation on a second call.
    assert!(std::sync::Arc::ptr_eq(&chars, &view.text(0).expect("cached")));
    let copy = view.selection_text((0, 0), (0, chars.len() - 1));
    assert!(copy.contains("Hello"), "copied: {copy:?}");
    assert!(view.selection_text((0, 3), (0, 1)).contains("ll"), "reversed drag should be normalised");
    assert!(view.selection_text((999, 0), (999, 4)).is_empty());
    assert!(view.text(999).is_none());
}

#[test]
fn hostile_link_and_text_inputs_never_panic() {
    let view = PdfView::open(linked_text_pdf()).expect("a valid PDF opens");
    for (p, x, y) in [(usize::MAX, f32::NAN, 0.0), (0, f32::INFINITY, -1.0), (0, f32::MAX, f32::MAX)] {
        let _ = view.link_at(p, x, y);
        let _ = view.links(p);
    }
    let _ = view.selection_text((usize::MAX, usize::MAX), (0, usize::MAX));
    let _ = view.text(usize::MAX);
    // A destination to a page that does not exist is dropped, not clamped into a wrong page.
    let bad = assemble(
        vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Annots [5 0 R 6 0 R] >>".to_vec(),
            stream_obj(b"0 0 1 rg 1 1 2 2 re f\n"),
            b"<< /Type /Annot /Subtype /Link /Rect [1 1 20 20] /Dest [9999 /Fit] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [1 1 20 20] /Dest [0 /Fit] >>".to_vec(),
        ],
        "",
    );
    let view = PdfView::open(bad).expect("a valid PDF opens");
    assert_eq!(view.links(0).len(), 1);
    assert!(matches!(view.links(0)[0].target, LinkTarget::Page { page: 0, .. }));
}
