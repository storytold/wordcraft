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
