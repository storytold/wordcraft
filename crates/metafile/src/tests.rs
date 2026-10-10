//! Hand-built WMF and EMF buffers: each test writes the records it needs and checks the parsed model.

use crate::canvas::Canvas;
use crate::dib::Rgba;
use crate::{Error, Item, MAX_BYTES, MAX_ITEMS, Seg, is_metafile, parse};

fn h16(v: i32) -> Vec<u8> {
    (v as i16).to_le_bytes().to_vec()
}

fn h32(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn e32(v: i32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn cat(parts: Vec<Vec<u8>>) -> Vec<u8> {
    parts.concat()
}

/// One WMF record: size in words, function, parameters (padded to a word boundary).
fn wrec(func: u16, params: &[u8]) -> Vec<u8> {
    let mut p = params.to_vec();
    if !p.len().is_multiple_of(2) {
        p.push(0);
    }
    let words = ((6 + p.len()) / 2) as u32;
    let mut v = words.to_le_bytes().to_vec();
    v.extend(func.to_le_bytes());
    v.extend(p);
    v
}

/// A WMF file: optional placeable header (bbox left, top, right, bottom in logical units, 1440 per inch),
/// the standard header, the records and an end-of-file record.
fn wmf(records: &[Vec<u8>], placeable: Option<[i32; 4]>) -> Vec<u8> {
    let body: Vec<u8> = records.concat();
    let eof = wrec(0, &[]);
    let mut f = Vec::new();
    if let Some(bb) = placeable {
        f.extend(0x9AC6_CDD7u32.to_le_bytes());
        f.extend(0u16.to_le_bytes());
        for v in bb {
            f.extend((v as i16).to_le_bytes());
        }
        f.extend(1440u16.to_le_bytes());
        f.extend(0u32.to_le_bytes());
        f.extend(0u16.to_le_bytes());
    }
    let total = (f.len() + 18 + body.len() + eof.len()) / 2;
    f.extend(1u16.to_le_bytes());
    f.extend(9u16.to_le_bytes());
    f.extend(0x300u16.to_le_bytes());
    f.extend((total as u32).to_le_bytes());
    f.extend(16u16.to_le_bytes());
    f.extend(0u32.to_le_bytes());
    f.extend(0u16.to_le_bytes());
    f.extend(body);
    f.extend(eof);
    f
}

/// One EMF record: type, size in bytes (payload padded to 4), payload.
fn erec(typ: u32, payload: &[u8]) -> Vec<u8> {
    let mut p = payload.to_vec();
    while !p.len().is_multiple_of(4) {
        p.push(0);
    }
    let mut v = typ.to_le_bytes().to_vec();
    v.extend(((8 + p.len()) as u32).to_le_bytes());
    v.extend(p);
    v
}

/// An EMF file with an 88-byte header (bounds and frame in 0.01 mm), the records and an EOF record.
fn emf(records: &[Vec<u8>], bounds: [i32; 4], frame: [i32; 4]) -> Vec<u8> {
    let body: Vec<u8> = records.concat();
    let eof = erec(14, &[0; 12]);
    let total = 88 + body.len() + eof.len();
    let mut h = vec![0u8; 88];
    h[0..4].copy_from_slice(&1u32.to_le_bytes());
    h[4..8].copy_from_slice(&88u32.to_le_bytes());
    h[8..24].copy_from_slice(&bounds.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    h[24..40].copy_from_slice(&frame.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    h[40..44].copy_from_slice(&0x464D_4520u32.to_le_bytes());
    h[44..48].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    h[48..52].copy_from_slice(&(total as u32).to_le_bytes());
    h[52..56].copy_from_slice(&((records.len() + 1) as u32).to_le_bytes());
    h[56..58].copy_from_slice(&1u16.to_le_bytes());
    // 1000 x 500 device pixels for 250 x 125 mm: 0.04 device units per hundredth of a millimetre.
    h[72..76].copy_from_slice(&1000i32.to_le_bytes());
    h[76..80].copy_from_slice(&500i32.to_le_bytes());
    h[80..84].copy_from_slice(&250i32.to_le_bytes());
    h[84..88].copy_from_slice(&125i32.to_le_bytes());
    let mut out = h;
    out.extend(body);
    out.extend(eof);
    out
}

/// A BITMAPINFOHEADER (40 bytes).
fn info_header(w: i32, h: i32, bpp: u16, size_image: u32) -> Vec<u8> {
    cat(vec![h32(40), e32(w), e32(h), h16(1), h16(i32::from(bpp)), h32(0), h32(size_image), e32(0), e32(0), h32(0), h32(0)])
}

/// A 2x2 24-bit bottom-up DIB. File rows are stored bottom first: the bottom image row is blue, green;
/// the top row is red, white. Rows are padded to 4 bytes (8 bytes per row).
fn dib24_bottom_up() -> Vec<u8> {
    let mut v = info_header(2, 2, 24, 16);
    v.extend([255, 0, 0, 0, 255, 0, 0, 0]);
    v.extend([0, 0, 255, 255, 255, 255, 0, 0]);
    v
}

/// Top-down RGBA of [`dib24_bottom_up`]: top-left red, top-right white, bottom-left blue, bottom-right green.
const DIB24_RGBA: [u8; 16] = [255, 0, 0, 255, 255, 255, 255, 255, 0, 0, 255, 255, 0, 255, 0, 255];

fn path_segs(item: &Item) -> (&Vec<Seg>, Option<[u8; 4]>, Option<([u8; 4], f32)>, bool) {
    match item {
        Item::Path { segs, fill, stroke, even_odd } => (segs, *fill, *stroke, *even_odd),
        Item::Bitmap { .. } => panic!("expected a path"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn wmf_rectangle_with_solid_brush() {
    let recs = [
        wrec(0x02FC, &cat(vec![h16(0), h32(0x0000_00FF), h16(0)])),
        wrec(0x012D, &h16(0)),
        wrec(0x041B, &cat(vec![h16(70), h16(110), h16(20), h16(10)])),
    ];
    let p = parse(&wmf(&recs, None)).unwrap();
    assert_eq!((p.width, p.height), (100.0, 50.0));
    assert_eq!(p.items.len(), 1);
    let (segs, fill, stroke, even_odd) = path_segs(&p.items[0]);
    assert_eq!(segs, &vec![Seg::Move(0.0, 0.0), Seg::Line(100.0, 0.0), Seg::Line(100.0, 50.0), Seg::Line(0.0, 50.0), Seg::Close]);
    assert_eq!(fill, Some([255, 0, 0, 255]));
    assert_eq!(stroke, Some(([0, 0, 0, 255], 0.0)));
    assert!(even_odd);
}

#[test]
fn wmf_polyline_with_pen() {
    let recs = [
        wrec(0x02FA, &cat(vec![h16(0), h16(4), h16(0), h32(0x0000_FF00)])),
        wrec(0x012D, &h16(0)),
        wrec(0x0325, &cat(vec![h16(3), h16(0), h16(0), h16(50), h16(0), h16(50), h16(40)])),
    ];
    let p = parse(&wmf(&recs, None)).unwrap();
    assert_eq!((p.width, p.height), (50.0, 40.0));
    let (segs, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(segs, &vec![Seg::Move(0.0, 0.0), Seg::Line(50.0, 0.0), Seg::Line(50.0, 40.0)]);
    assert_eq!(fill, None);
    assert_eq!(stroke, Some(([0, 255, 0, 255], 4.0)));
}

#[test]
fn wmf_polygon_fill_rule_follows_polyfillmode() {
    let square = cat(vec![h16(4), h16(0), h16(0), h16(10), h16(0), h16(10), h16(10), h16(0), h16(10)]);
    let winding = [wrec(0x0106, &h16(2)), wrec(0x0324, &square)];
    let p = parse(&wmf(&winding, None)).unwrap();
    assert!(!path_segs(&p.items[0]).3, "WINDING fills use the nonzero rule");
    let alternate = [wrec(0x0324, &square)];
    let p = parse(&wmf(&alternate, None)).unwrap();
    let (segs, fill, _, even_odd) = path_segs(&p.items[0]);
    assert!(even_odd, "default ALTERNATE is even-odd");
    assert_eq!(fill, Some([255, 255, 255, 255]));
    assert_eq!(segs.last(), Some(&Seg::Close));
}

#[test]
fn wmf_stretchdib_flips_bottom_up_rows() {
    let params = cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(0), h16(10), h16(20), h16(0), h16(0), dib24_bottom_up()]);
    let p = parse(&wmf(&[wrec(0x0F43, &params)], None)).unwrap();
    assert_eq!((p.width, p.height), (20.0, 10.0));
    let Item::Bitmap { rect, width, height, rgba } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rect, &[0.0, 0.0, 20.0, 10.0]);
    assert_eq!((*width, *height), (2, 2));
    assert_eq!(rgba.as_slice(), DIB24_RGBA);
}

#[test]
fn wmf_mapping_window_and_viewport() {
    let recs = [
        wrec(0x0103, &h16(8)),
        wrec(0x020B, &cat(vec![h16(100), h16(100)])),
        wrec(0x020C, &cat(vec![h16(50), h16(50)])),
        wrec(0x020E, &cat(vec![h16(100), h16(100)])),
        wrec(0x0214, &cat(vec![h16(100), h16(100)])),
        wrec(0x0213, &cat(vec![h16(150), h16(150)])),
    ];
    let p = parse(&wmf(&recs, None)).unwrap();
    assert_eq!((p.width, p.height), (100.0, 100.0));
    let (segs, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(segs, &vec![Seg::Move(0.0, 0.0), Seg::Line(100.0, 100.0)]);
    assert_eq!(fill, None);
    assert_eq!(stroke, Some(([0, 0, 0, 255], 0.0)));
}

#[test]
fn wmf_placeable_header_sets_frame_in_points() {
    let p = parse(&wmf(&[wrec(0x041B, &cat(vec![h16(1440), h16(2880), h16(0), h16(0)]))], Some([0, 0, 2880, 1440]))).unwrap();
    assert_eq!((p.width, p.height), (144.0, 72.0));
}

#[test]
fn emf_stretchdibits_flips_and_fits_frame() {
    let mut payload = vec![0u8; 16];
    payload.extend(cat(vec![
        e32(0),
        e32(0),
        e32(0),
        e32(0),
        e32(2),
        e32(2),
        e32(80),
        e32(40),
        e32(120),
        e32(16),
        e32(0),
        e32(0x00CC_0020),
        e32(100),
        e32(50),
    ]));
    payload.extend(info_header(2, 2, 24, 16));
    payload.extend(dib24_bottom_up().get(40..).unwrap_or_default());
    let rec = erec(81, &payload);
    // A 25 x 12.5 mm frame is 70.87 x 35.43 points; the bitmap fills the whole frame.
    let file = emf(&[rec], [0, 0, 100, 50], [0, 0, 2500, 1250]);
    let p = parse(&file).unwrap();
    let (w, h) = (2500.0 * 72.0 / 2540.0, 1250.0 * 72.0 / 2540.0);
    assert!(close(p.width, w as f32) && close(p.height, h as f32));
    let Item::Bitmap { rect, width, height, rgba } = &p.items[0] else { panic!("expected a bitmap") };
    assert!(rect.iter().zip([0.0, 0.0, w, h]).all(|(a, b)| close(*a, b as f32)));
    assert_eq!((*width, *height), (2, 2));
    assert_eq!(rgba.as_slice(), DIB24_RGBA);
}

#[test]
fn emf_world_transform_scales_geometry() {
    let xf = cat([2.0f32, 0.0, 0.0, 2.0, 0.0, 0.0].iter().map(|v| v.to_le_bytes().to_vec()).collect());
    let recs = [erec(35, &xf), erec(27, &cat(vec![e32(0), e32(0)])), erec(54, &cat(vec![e32(10), e32(5)]))];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    assert_eq!((p.width, p.height), (20.0, 10.0));
    let (segs, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(segs, &vec![Seg::Move(0.0, 0.0), Seg::Line(20.0, 10.0)]);
    assert_eq!(fill, None);
    assert_eq!(stroke, Some(([0, 0, 0, 255], 0.0)));
}

#[test]
fn emf_ellipse_is_four_cubics_back_to_start() {
    let rect = cat(vec![e32(0), e32(0), e32(40), e32(20)]);
    let p = parse(&emf(&[erec(42, &rect)], [0; 4], [0; 4])).unwrap();
    let (segs, fill, _, _) = path_segs(&p.items[0]);
    assert_eq!(segs.len(), 6);
    assert_eq!(segs[0], Seg::Move(40.0, 10.0));
    assert!(matches!(segs[4], Seg::Cubic(_, _, _, _, x, y) if close(x, 40.0) && close(y, 10.0)));
    assert_eq!(segs[5], Seg::Close);
    assert_eq!(fill, Some([255, 255, 255, 255]));
}

#[test]
fn emf_path_bracket_strokeandfill_uses_stock_black_brush() {
    let bounds = vec![0u8; 16];
    let recs = [
        erec(59, &[]),
        erec(27, &cat(vec![e32(0), e32(0)])),
        erec(54, &cat(vec![e32(10), e32(0)])),
        erec(54, &cat(vec![e32(10), e32(10)])),
        erec(61, &[]),
        erec(60, &[]),
        erec(37, &h32(0x8000_0004)),
        erec(63, &bounds),
    ];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    assert_eq!(p.items.len(), 1);
    let (segs, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(segs, &vec![Seg::Move(0.0, 0.0), Seg::Line(10.0, 0.0), Seg::Line(10.0, 10.0), Seg::Close]);
    assert_eq!(fill, Some([0, 0, 0, 255]));
    assert_eq!(stroke, Some(([0, 0, 0, 255], 0.0)));
}

#[test]
fn item_cap_stops_the_parse_and_keeps_the_rest() {
    let rect = wrec(0x041B, &cat(vec![h16(10), h16(10), h16(0), h16(0)]));
    let recs: Vec<Vec<u8>> = (0..MAX_ITEMS + 10).map(|_| rect.clone()).collect();
    let p = parse(&wmf(&recs, None)).unwrap();
    assert_eq!(p.items.len(), MAX_ITEMS);
}

#[test]
fn detection_and_garbage() {
    let w = wmf(&[wrec(0x041B, &cat(vec![h16(1), h16(1), h16(0), h16(0)]))], None);
    let e = emf(&[erec(42, &cat(vec![e32(0), e32(0), e32(4), e32(4)]))], [0; 4], [0; 4]);
    assert!(is_metafile(&w));
    assert!(is_metafile(&e));
    assert!(!is_metafile(b"hello world, not a metafile"));
    assert_eq!(parse(b"hello world, not a metafile").unwrap_err(), Error::NotMetafile);
    assert!(parse(&[]).is_err());
}

/// Fixed-seed LCG (Knuth's constants): the fuzz inputs are reproducible.
struct Lcg(u64);

impl Lcg {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

#[test]
fn truncated_random_and_mutated_inputs_never_panic() {
    let rect = wrec(0x041B, &cat(vec![h16(10), h16(10), h16(0), h16(0)]));
    let wfile = wmf(
        &[
            wrec(0x02FC, &cat(vec![h16(0), h32(0xFF), h16(0)])),
            wrec(0x012D, &h16(0)),
            rect,
            wrec(0x0325, &cat(vec![h16(2), h16(0), h16(0), h16(5), h16(5)])),
        ],
        Some([0, 0, 20, 10]),
    );
    let mut dib_rec = cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(0), h16(10), h16(20), h16(0), h16(0)]);
    dib_rec.extend(dib24_bottom_up());
    let mut payload = vec![0u8; 16];
    payload.extend(cat(vec![e32(0), e32(0), e32(0), e32(0), e32(2), e32(2), e32(80), e32(40), e32(120), e32(16), e32(0), e32(0), e32(20), e32(10)]));
    payload.extend(info_header(2, 2, 24, 16));
    payload.extend(dib24_bottom_up().get(40..).unwrap_or_default());
    let efile = emf(
        &[
            erec(59, &[]),
            erec(27, &cat(vec![e32(0), e32(0)])),
            erec(54, &cat(vec![e32(9), e32(9)])),
            erec(61, &[]),
            erec(60, &[]),
            erec(63, &[0; 16]),
            erec(81, &payload),
            erec(86, &cat(vec![h32(4), h16(0), h16(0), h16(9), h16(9), h16(0), h16(9)])),
        ],
        [0, 0, 20, 10],
        [0, 0, 2540, 1270],
    );
    for file in [&wfile, &efile] {
        for n in 0..=file.len() {
            let prefix = &file[..n];
            let _ = parse(prefix);
            let _ = is_metafile(prefix);
        }
    }
    let mut rng = Lcg(0x2545_F491_4F6C_DD1D);
    for _ in 0..3000 {
        let len = (rng.rnd() % 400) as usize;
        let buf: Vec<u8> = (0..len).map(|_| rng.rnd() as u8).collect();
        let _ = parse(&buf);
    }
    for i in 0..1500usize {
        let mut buf = if i.is_multiple_of(2) { efile.clone() } else { wfile.clone() };
        for _ in 0..8 {
            let at = rng.rnd() as usize % buf.len();
            buf[at] = rng.rnd() as u8;
        }
        let _ = parse(&buf);
    }
    for i in 0..500usize {
        let mut buf = if i.is_multiple_of(2) { efile.clone() } else { wfile.clone() };
        let at = rng.rnd() as usize % buf.len();
        buf[at..].iter_mut().for_each(|b| *b = 0xFF);
        let _ = parse(&buf);
    }
}

#[test]
fn wmf_stock_null_brush_by_16_bit_handle_has_no_fill() {
    // SELECTOBJECT with 0x8005 is the NULL_BRUSH stock object (0x8000 | 5), so the rectangle is outline only.
    let recs = [wrec(0x012D, &h16(0x8005u16 as i32)), wrec(0x041B, &cat(vec![h16(70), h16(110), h16(20), h16(10)]))];
    let p = parse(&wmf(&recs, None)).unwrap();
    let (_, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(fill, None);
    assert_eq!(stroke, Some(([0, 0, 0, 255], 0.0)));
}

#[test]
fn emf_frame_comes_from_rclframe_not_device_bounds() {
    // Bounds cover only the left half of the 25 x 12.5 mm frame (device units are 0.04 per 0.01 mm), so a
    // rectangle from x = 0 to 50 px ends at half the picture width.
    let rec = erec(43, &cat(vec![e32(0), e32(0), e32(50), e32(25)]));
    let p = parse(&emf(&[rec], [0, 0, 50, 25], [0, 0, 2500, 1250])).unwrap();
    let w = 2500.0 * 72.0 / 2540.0;
    assert!(close(p.width, w as f32) && close(p.height, (1250.0 * 72.0 / 2540.0) as f32));
    let (segs, ..) = path_segs(&p.items[0]);
    assert!(matches!(segs[1], Seg::Line(x, _) if close(x, (w / 2.0) as f32)), "{segs:?}");
}

#[test]
fn emf_bounds_stand_in_when_the_frame_is_empty() {
    let rec = erec(43, &cat(vec![e32(0), e32(0), e32(50), e32(25)]));
    let p = parse(&emf(&[rec], [0, 0, 50, 25], [0; 4])).unwrap();
    assert_eq!((p.width, p.height), (50.0, 25.0));
}

#[test]
fn wmf_placeable_bounds_go_through_the_window_mapping() {
    // Logical bounds 200 x 100 at 1440 per inch are 10 x 5 points; the window is 2:1 onto the viewport, so
    // the frame is 100 x 50 device units and the rectangle fills it.
    let recs = [
        wrec(0x0103, &h16(8)),
        wrec(0x020C, &cat(vec![h16(100), h16(200)])),
        wrec(0x020E, &cat(vec![h16(50), h16(100)])),
        wrec(0x041B, &cat(vec![h16(100), h16(200), h16(0), h16(0)])),
    ];
    let p = parse(&wmf(&recs, Some([0, 0, 200, 100]))).unwrap();
    assert_eq!((p.width, p.height), (10.0, 5.0));
    let (segs, ..) = path_segs(&p.items[0]);
    assert!(segs.iter().any(|s| matches!(s, Seg::Line(x, y) if close(*x, 10.0) && close(*y, 5.0))), "{segs:?}");
}

#[test]
fn wmf_stretchdib_clipped_source_keeps_its_place() {
    // Source x from -1 for 2 columns: only column 0 is in the bitmap, so it fills the right half of the
    // destination and the picture keeps the original 20-unit width scale.
    let params = cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(-1), h16(10), h16(20), h16(0), h16(0), dib24_bottom_up()]);
    let p = parse(&wmf(&[wrec(0x0F43, &params)], Some([0, 0, 20, 10]))).unwrap();
    let Item::Bitmap { rect, width, height, rgba } = &p.items[0] else { panic!("expected a bitmap") };
    // Placeable units are points: 20 units at 1440 per inch are 1 point, so the right column is 0.5 pt wide.
    assert_eq!(rect, &[0.5, 0.0, 0.5, 0.5]);
    assert_eq!((*width, *height), (1, 2));
    assert_eq!(rgba.as_slice(), [255, 0, 0, 255, 0, 0, 255, 255]);
}

#[test]
fn wmf_negative_destination_width_mirrors_the_bitmap() {
    let params = cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(0), h16(10), h16(-20), h16(0), h16(0), dib24_bottom_up()]);
    // Frame from the placeable bounds (-20, 0)-(0, 10): the mirrored bitmap fills it.
    let p = parse(&wmf(&[wrec(0x0F43, &params)], Some([-20, 0, 0, 10]))).unwrap();
    let Item::Bitmap { rect, rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rect, &[0.0, 0.0, 1.0, 0.5]);
    assert_eq!(&rgba[..8], [255, 255, 255, 255, 255, 0, 0, 255]);
}

#[test]
fn emf_blackness_bitblt_fills_black_without_a_source() {
    let mut payload = vec![0u8; 16];
    payload.extend(cat(vec![e32(10), e32(20), e32(30), e32(40), e32(0x0000_0042)]));
    let p = parse(&emf(&[erec(76, &payload)], [0; 4], [0; 4])).unwrap();
    let (segs, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(fill, Some([0, 0, 0, 255]));
    assert_eq!(stroke, None);
    assert_eq!(segs.len(), 5);
}

#[test]
fn emf_savedc_past_the_stack_cap_stops_the_parse() {
    let mut recs: Vec<Vec<u8>> = (0..=crate::MAX_STACK).map(|_| erec(33, &[])).collect();
    recs.push(erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)])));
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    assert!(p.items.is_empty(), "records after the cap must not be drawn");
}

#[test]
fn canvas_stops_at_the_byte_budget() {
    // Two 128 MB bitmaps fit the budget; the third would exceed it and stops the canvas.
    let mut cv = Canvas::new();
    let big = || Rgba { w: 1, h: 1, bottom_up: false, px: vec![0u8; MAX_BYTES / 2] };
    cv.bitmap([0.0, 0.0, 1.0, 1.0], big());
    cv.bitmap([0.0, 0.0, 1.0, 1.0], big());
    assert!(!cv.full());
    cv.bitmap([0.0, 0.0, 1.0, 1.0], big());
    assert!(cv.full());
    assert_eq!(cv.items.len(), 2);
}

#[test]
fn oversized_dib_header_size_is_rejected_without_overflow() {
    let mut info = info_header(2, 2, 24, 16);
    info[0..4].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    let params = cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(0), h16(10), h16(20), h16(0), h16(0), info]);
    let p = parse(&wmf(&[wrec(0x0F43, &params)], None)).unwrap();
    assert!(p.items.is_empty());
}

/// An EMF record of `len` bytes (header included) with u32 fields and byte blobs written at absolute
/// record offsets, so each test places a field where the spec puts it.
fn erec_at(typ: u32, len: usize, fields: &[(usize, u32)], blobs: &[(usize, &[u8])]) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0..4].copy_from_slice(&typ.to_le_bytes());
    v[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    for (off, val) in fields {
        v[*off..*off + 4].copy_from_slice(&val.to_le_bytes());
    }
    for (off, b) in blobs {
        v[*off..*off + b.len()].copy_from_slice(b);
    }
    v
}

/// A BITBLT-style record (BITBLT, STRETCHBLT, ALPHABLEND layout) with its DIB at 112: the bmi and bits
/// offsets and lengths go at 84 and 92. `fields` and `blobs` add the other fields.
fn dib_rec(typ: u32, fields: &[(usize, u32)], blobs: &[(usize, &[u8])], info: &[u8], bits: &[u8]) -> Vec<u8> {
    let at = 112;
    let bits_at = at + info.len();
    let mut f = fields.to_vec();
    f.extend([(84, at as u32), (88, info.len() as u32), (92, bits_at as u32), (96, bits.len() as u32)]);
    let mut b: Vec<(usize, &[u8])> = blobs.to_vec();
    b.extend([(at, info), (bits_at, bits)]);
    erec_at(typ, bits_at + bits.len(), &f, &b)
}

/// BITBLT (76) with destination `dst` = [x, y, w, h] and a 40-byte header DIB.
fn bitblt_rec(rop: u32, dst: [u32; 4], info: &[u8], bits: &[u8]) -> Vec<u8> {
    let fields = [(24, dst[0]), (28, dst[1]), (32, dst[2]), (36, dst[3]), (40, rop)];
    dib_rec(76, &fields, &[], info, bits)
}

#[test]
fn wmf_font_takes_an_object_slot_so_handles_line_up() {
    // The font is handle 0, so the pen created after it is handle 1.
    let recs = [
        wrec(0x02FB, &[0u8; 18]),
        wrec(0x02FA, &cat(vec![h16(0), h16(4), h16(0), h32(0x0000_FF00)])),
        wrec(0x012D, &h16(1)),
        wrec(0x0325, &cat(vec![h16(2), h16(0), h16(0), h16(50), h16(0), h16(50)])),
    ];
    let p = parse(&wmf(&recs, None)).unwrap();
    let (_, _, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(stroke, Some(([0, 255, 0, 255], 4.0)));
}

#[test]
fn emf_polybezierto16_continues_the_open_path() {
    // BEGINPATH, MOVETO a, LINETO b, POLYBEZIERTO16, CLOSEFIGURE, FILLPATH: one closed figure from a.
    let mut bezier = vec![0u8; 16];
    bezier.extend(cat(vec![e32(3), h16(10), h16(10), h16(20), h16(10), h16(20), h16(0)]));
    let recs = [
        erec(59, &[]),
        erec(27, &cat(vec![e32(0), e32(0)])),
        erec(54, &cat(vec![e32(10), e32(0)])),
        erec(88, &bezier),
        erec(61, &[]),
        erec(62, &[0; 16]),
    ];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    assert_eq!(p.items.len(), 1);
    let (segs, fill, _, _) = path_segs(&p.items[0]);
    assert_eq!(segs, &vec![Seg::Move(0.0, 0.0), Seg::Line(10.0, 0.0), Seg::Cubic(10.0, 10.0, 20.0, 10.0, 20.0, 0.0), Seg::Close]);
    assert_eq!(fill, Some([255, 255, 255, 255]));
}

#[test]
fn emf_polybezier_starts_its_own_figure_and_keeps_the_current_point() {
    // POLYBEZIER (2) starts at its first point; the LINETO after it still starts at the MOVETO point.
    let mut bezier = vec![0u8; 16];
    bezier.extend(cat(vec![e32(4), e32(0), e32(0), e32(5), e32(5), e32(10), e32(5), e32(10), e32(0)]));
    let recs = [erec(27, &cat(vec![e32(3), e32(3)])), erec(2, &bezier), erec(54, &cat(vec![e32(50), e32(50)]))];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    assert_eq!(p.items.len(), 2);
    let (segs, ..) = path_segs(&p.items[0]);
    assert_eq!(segs[0], Seg::Move(0.0, 0.0));
    let (segs, ..) = path_segs(&p.items[1]);
    assert_eq!(segs, &vec![Seg::Move(3.0, 3.0), Seg::Line(50.0, 50.0)]);
}

#[test]
fn hairline_pen_stays_zero_under_an_anisotropic_window() {
    // Window 2000 x 2000 onto a 1 x 1 viewport: a cosmetic pen must not become one device unit wide.
    let recs = [
        wrec(0x0103, &h16(8)),
        wrec(0x020C, &cat(vec![h16(2000), h16(2000)])),
        wrec(0x020E, &cat(vec![h16(1), h16(1)])),
        wrec(0x0214, &cat(vec![h16(0), h16(0)])),
        wrec(0x0213, &cat(vec![h16(2000), h16(2000)])),
    ];
    let p = parse(&wmf(&recs, None)).unwrap();
    assert_eq!((p.width, p.height), (1.0, 1.0));
    let (segs, _, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(segs.last(), Some(&Seg::Line(1.0, 1.0)));
    assert_eq!(stroke, Some(([0, 0, 0, 255], 0.0)));
}

#[test]
fn null_pen_is_recognised_under_extra_style_bits() {
    // Pen style 0x105: PS_NULL (5) with a flag above the style nibble still draws no outline.
    let pen = erec_at(38, 28, &[(8, 0), (12, 0x105), (16, 1), (24, 0)], &[]);
    let recs = [pen, erec(37, &h32(0)), erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)]))];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    let (_, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(fill, Some([255, 255, 255, 255]));
    assert_eq!(stroke, None);
}

#[test]
fn metric_map_mode_flips_y_and_scales_to_device_pixels_emf() {
    // LOMETRIC is 0.1 mm per unit; the header maps 4 device pixels to 1 mm. Points go y-up, so the
    // drawing lies below its origin and the picture frame moves up to its top.
    let recs = [erec(17, &h32(2)), erec(27, &cat(vec![e32(0), e32(0)])), erec(54, &cat(vec![e32(100), e32(50)]))];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    let (segs, ..) = path_segs(&p.items[0]);
    assert!(close_seg(&segs[0], Seg::Move(0.0, 20.0)) && close_seg(&segs[1], Seg::Line(40.0, 0.0)), "{segs:?}");
}

#[test]
fn metric_map_mode_uses_96_dpi_for_wmf() {
    let recs = [wrec(0x0103, &h16(2)), wrec(0x0214, &cat(vec![h16(0), h16(0)])), wrec(0x0213, &cat(vec![h16(50), h16(100)]))];
    let p = parse(&wmf(&recs, None)).unwrap();
    let px_mm = 96.0 / 25.4;
    let (segs, ..) = path_segs(&p.items[0]);
    let (x, y) = (100.0 * 0.1 * px_mm, 50.0 * 0.1 * px_mm);
    assert!(close_seg(&segs[0], Seg::Move(0.0, y as f32)) && close_seg(&segs[1], Seg::Line(x as f32, 0.0)), "{segs:?}");
}

/// Segment equality within [`close`] tolerance.
fn close_seg(a: &Seg, b: Seg) -> bool {
    match (*a, b) {
        (Seg::Move(x, y), Seg::Move(u, v)) | (Seg::Line(x, y), Seg::Line(u, v)) => close(x, u) && close(y, v),
        _ => false,
    }
}

#[test]
fn isotropic_mode_uses_one_scale_and_keeps_axis_signs() {
    // Window 100 x 50; viewport 400 x -100: the scale is min(4, 2) = 2 on both axes, y flipped.
    let recs = [
        wrec(0x0103, &h16(7)),
        wrec(0x020C, &cat(vec![h16(50), h16(100)])),
        wrec(0x020E, &cat(vec![h16(-100), h16(400)])),
        wrec(0x0214, &cat(vec![h16(0), h16(0)])),
        wrec(0x0213, &cat(vec![h16(25), h16(50)])),
    ];
    let p = parse(&wmf(&recs, None)).unwrap();
    let (segs, ..) = path_segs(&p.items[0]);
    assert!(close_seg(&segs[0], Seg::Move(0.0, 100.0)) && close_seg(&segs[1], Seg::Line(100.0, 50.0)), "{segs:?}");
}

#[test]
fn modify_world_with_an_unknown_mode_keeps_the_transform() {
    let scale2 = cat([2.0f32, 0.0, 0.0, 2.0, 0.0, 0.0].iter().map(|v| v.to_le_bytes().to_vec()).collect());
    let identity = cat([1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0].iter().map(|v| v.to_le_bytes().to_vec()).collect());
    let recs =
        [erec(35, &scale2), erec(36, &cat(vec![identity, h32(99)])), erec(27, &cat(vec![e32(0), e32(0)])), erec(54, &cat(vec![e32(10), e32(5)]))];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    let (segs, ..) = path_segs(&p.items[0]);
    assert_eq!(segs, &vec![Seg::Move(0.0, 0.0), Seg::Line(20.0, 10.0)]);
}

#[test]
fn round_rect_accepts_reversed_corners() {
    let normal = crate::geom::round_rect(0.0, 0.0, 10.0, 20.0, 4.0, 4.0);
    assert_eq!(crate::geom::round_rect(10.0, 20.0, 0.0, 0.0, 4.0, 4.0), normal);
    assert_eq!(crate::geom::round_rect(10.0, 0.0, 0.0, 20.0, 4.0, 4.0), normal);
}

#[test]
fn stroke_width_is_clamped_to_the_picture_diagonal() {
    // A 1440-unit placeable picture is 72 points square (diagonal 101.8); a 5000-unit pen would be 250.
    let recs = [
        wrec(0x02FA, &cat(vec![h16(0), h16(5000), h16(0), h32(0)])),
        wrec(0x012D, &h16(0)),
        wrec(0x041B, &cat(vec![h16(1440), h16(1440), h16(0), h16(0)])),
    ];
    let p = parse(&wmf(&recs, Some([0, 0, 1440, 1440]))).unwrap();
    assert_eq!((p.width, p.height), (72.0, 72.0));
    let (_, _, stroke, _) = path_segs(&p.items[0]);
    assert!(close(stroke.unwrap_or(([0; 4], 0.0)).1, 72.0 * 2.0f32.sqrt()), "{stroke:?}");
}

/// A WMF rectangle from (0, 0) to (1, 1), so a picture's frame starts at the origin.
fn origin_rect() -> Vec<u8> {
    wrec(0x041B, &cat(vec![h16(1), h16(1), h16(0), h16(0)]))
}

#[test]
fn wmf_dibbitblt_without_a_dib_is_a_pattern_fill() {
    // The 12-word form: rop(4), ySrc, xSrc, reserved, height, width, yDest, xDest; PATCOPY fills with the brush.
    let params = cat(vec![h32(0x00F0_0021), h16(0), h16(0), h16(0), h16(10), h16(20), h16(5), h16(7)]);
    let p = parse(&wmf(&[origin_rect(), wrec(0x0940, &params)], None)).unwrap();
    let (segs, fill, _, _) = path_segs(&p.items[1]);
    assert_eq!(fill, Some([255, 255, 255, 255]));
    assert_eq!(segs, &vec![Seg::Move(7.0, 5.0), Seg::Line(27.0, 5.0), Seg::Line(27.0, 15.0), Seg::Line(7.0, 15.0), Seg::Close]);
}

#[test]
fn wmf_dibstretchblt_without_a_dib_is_a_rop_fill() {
    // The 14-word form: rop(4), srcH, srcW, ySrc, xSrc, reserved, destH, destW, yDest, xDest; BLACKNESS fills black.
    let params = cat(vec![h32(0x0000_0042), h16(2), h16(2), h16(0), h16(0), h16(0), h16(10), h16(20), h16(5), h16(7)]);
    let p = parse(&wmf(&[origin_rect(), wrec(0x0B41, &params)], None)).unwrap();
    let (segs, fill, stroke, _) = path_segs(&p.items[1]);
    assert_eq!((fill, stroke), (Some([0, 0, 0, 255]), None));
    assert_eq!(segs, &vec![Seg::Move(7.0, 5.0), Seg::Line(27.0, 5.0), Seg::Line(27.0, 15.0), Seg::Line(7.0, 15.0), Seg::Close]);
}

#[test]
fn wmf_dibstretchblt_stretches_the_dib_into_its_destination() {
    // rop(4), srcH, srcW, ySrc, xSrc, destH, destW, yDest, xDest, then the DIB (no reserved word).
    let mut params = cat(vec![h32(0x00CC_0020), h16(2), h16(2), h16(0), h16(0), h16(10), h16(20), h16(6), h16(5)]);
    params.extend(dib24_bottom_up());
    let p = parse(&wmf(&[origin_rect(), wrec(0x0B41, &params)], None)).unwrap();
    let Item::Bitmap { rect, rgba, .. } = &p.items[1] else { panic!("expected a bitmap") };
    assert_eq!(rect, &[5.0, 6.0, 20.0, 10.0]);
    assert_eq!(rgba.as_slice(), DIB24_RGBA);
}

#[test]
fn wmf_negative_destination_height_mirrors_the_rows() {
    let params = cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(0), h16(-10), h16(20), h16(0), h16(0), dib24_bottom_up()]);
    let p = parse(&wmf(&[wrec(0x0F43, &params)], None)).unwrap();
    let Item::Bitmap { rect, rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rect, &[0.0, 0.0, 20.0, 10.0]);
    // Blue, green on top; red, white below.
    assert_eq!(rgba.as_slice(), [0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255]);
}

#[test]
fn wmf_dibbitblt_copies_the_dib_at_its_destination() {
    // The rectangle fixes the picture origin at (0, 0); the DIB lands at x = 5, y = 6.
    let mut params = cat(vec![h32(0x00CC_0020), h16(0), h16(0), h16(2), h16(2), h16(6), h16(5)]);
    params.extend(dib24_bottom_up());
    let recs = [wrec(0x041B, &cat(vec![h16(10), h16(10), h16(0), h16(0)])), wrec(0x0940, &params)];
    let p = parse(&wmf(&recs, None)).unwrap();
    let Item::Bitmap { rect, width, height, rgba } = &p.items[1] else { panic!("expected a bitmap") };
    assert_eq!(rect, &[5.0, 6.0, 2.0, 2.0]);
    assert_eq!((*width, *height), (2, 2));
    assert_eq!(rgba.as_slice(), DIB24_RGBA);
}

#[test]
fn emf_setdibitstodevice_draws_the_image_at_its_own_size() {
    let dib = dib24_bottom_up();
    let (info, bits) = (&dib[..40], &dib[40..]);
    // Destination x/y at 24, src x/y at 32, size at 40, bmi at 48 (offset, length), bits at 56, scan start
    // 68 and scan count 72.
    let fields = [(24, 10), (28, 20), (40, 2), (44, 2), (48, 112), (52, 40), (56, 152), (60, 16), (72, 2)];
    let rec = erec_at(80, 168, &fields, &[(112, info), (152, bits)]);
    let p = parse(&emf(&[rec], [0; 4], [0; 4])).unwrap();
    let Item::Bitmap { rect, rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rect, &[0.0, 0.0, 2.0, 2.0]);
    assert_eq!(rgba.as_slice(), DIB24_RGBA);
}

#[test]
fn emf_setdibitstodevice_source_counts_from_the_start_scan() {
    // ySrc 1 with the bits starting at scan 1 is source row 0 of the bits: the bottom row (blue, green),
    // drawn cxSrc x cySrc = 2 x 1 at the destination.
    let dib = dib24_bottom_up();
    let (info, bits) = (&dib[..40], &dib[40..]);
    let fields = [(24, 10), (28, 20), (36, 1), (40, 2), (44, 1), (48, 112), (52, 40), (56, 152), (60, 16), (68, 1), (72, 2)];
    let rec = erec_at(80, 168, &fields, &[(112, info), (152, bits)]);
    let r = erec(43, &cat(vec![e32(0), e32(0), e32(1), e32(1)]));
    let p = parse(&emf(&[r, rec], [0; 4], [0; 4])).unwrap();
    let Item::Bitmap { rect, rgba, .. } = &p.items[1] else { panic!("expected a bitmap") };
    assert_eq!(rect, &[10.0, 20.0, 2.0, 1.0]);
    assert_eq!(rgba.as_slice(), [0, 0, 255, 255, 0, 255, 0, 255]);
}

#[test]
fn emf_alphablend_uses_constant_and_per_pixel_alpha() {
    // Constant alpha 128 (BLENDFUNCTION bytes at 40: op, flags, constant, format) over a 24-bit DIB.
    let blend = [0u8, 0, 128, 0];
    let dib = dib24_bottom_up();
    let rec = dib_rec(114, &[(24, 0), (28, 0), (32, 2), (36, 2), (44, 0), (48, 0), (100, 2), (104, 2)], &[(40, &blend[..])], &dib[..40], &dib[40..]);
    let p = parse(&emf(&[rec], [0; 4], [0; 4])).unwrap();
    let Item::Bitmap { rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    let alpha: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|px| px[3]).collect();
    assert_eq!(alpha, vec![128; 4]);
    // Per-pixel alpha (AC_SRC_ALPHA) over a 32-bit premultiplied DIB: BGRA 0, 0, 128, 128 is straight red at
    // half alpha, and the constant 128 halves the alpha again.
    let one_px = |constant: u8, bgra: [u8; 4]| {
        let blend = [0u8, 0, constant, 1];
        let info = info_header(1, 1, 32, 4);
        let rec = dib_rec(114, &[(24, 0), (28, 0), (32, 1), (36, 1), (44, 0), (48, 0), (100, 1), (104, 1)], &[(40, &blend[..])], &info, &bgra);
        let p = parse(&emf(&[rec], [0; 4], [0; 4])).unwrap();
        let Item::Bitmap { rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
        rgba.clone()
    };
    assert_eq!(one_px(255, [0, 0, 128, 128]), [255, 0, 0, 128]);
    assert_eq!(one_px(128, [0, 0, 128, 128]), [255, 0, 0, 64]);
    assert_eq!(one_px(255, [10, 20, 64, 128]), [128, 40, 20, 128], "rounded to the nearest value");
}

#[test]
fn emf_srcand_mask_draws_all_but_its_white_pixels() {
    // SRCAND over white paper is the source itself; white keeps the destination, so it is transparent.
    let dib = dib24_bottom_up();
    let rec = bitblt_rec(0x0088_00C6, [0, 0, 2, 2], &dib[..40], &dib[40..]);
    let p = parse(&emf(&[rec], [0; 4], [0; 4])).unwrap();
    let Item::Bitmap { rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rgba.as_slice(), [255, 0, 0, 255, 255, 255, 255, 0, 0, 0, 255, 255, 0, 255, 0, 255]);
}

#[test]
fn emf_srcpaint_makes_black_pixels_transparent() {
    // A 2 x 1 DIB: black, then red. SRCPAINT leaves black transparent; SRCCOPY keeps it opaque.
    let info = info_header(2, 1, 24, 8);
    let bits = [0u8, 0, 0, 0, 0, 255, 0, 0];
    let p = parse(&emf(&[bitblt_rec(0x00EE_0086, [0, 0, 2, 1], &info, &bits)], [0; 4], [0; 4])).unwrap();
    let Item::Bitmap { rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rgba.as_slice(), [0, 0, 0, 0, 255, 0, 0, 255]);
    let p = parse(&emf(&[bitblt_rec(0x00CC_0020, [0, 0, 2, 1], &info, &bits)], [0; 4], [0; 4])).unwrap();
    let Item::Bitmap { rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rgba.as_slice(), [0, 0, 0, 255, 255, 0, 0, 255]);
}

#[test]
fn wmf_dib_palette_entries_are_skipped_before_the_bits() {
    // A 24-bit DIB with biClrUsed = 2 carries two RGBQUADs (8 bytes) before its pixel bits.
    let mut info = info_header(2, 2, 24, 16);
    info[32..36].copy_from_slice(&2u32.to_le_bytes());
    let mut dib = info;
    dib.extend([9u8; 8]);
    dib.extend(&dib24_bottom_up()[40..]);
    let params = cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(0), h16(10), h16(20), h16(0), h16(0), dib]);
    let p = parse(&wmf(&[wrec(0x0F43, &params)], None)).unwrap();
    let Item::Bitmap { rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!(rgba.as_slice(), DIB24_RGBA);
}

#[test]
fn emf_point_counts_past_the_budget_are_rejected() {
    // A count of 4 billion points in a record that cannot hold them: nothing is drawn and nothing is allocated.
    let mut poly = vec![0u8; 16];
    poly.extend(cat(vec![h32(u32::MAX), h32(0), h32(0)]));
    let mut groups = vec![0u8; 16];
    groups.extend(cat(vec![h32(u32::MAX), h32(u32::MAX), h32(1)]));
    let p = parse(&emf(&[erec(4, &poly), erec(7, &groups)], [0; 4], [0; 4])).unwrap();
    assert!(p.items.is_empty());
}

#[test]
fn emf_polypolygon_counts_must_fit_the_record() {
    // A polygon count within the point budget in a record too short to hold the counts draws nothing.
    let mut short = vec![0u8; 16];
    short.extend(cat(vec![h32(crate::MAX_PATH_OPS as u32), h32(3), h32(3)]));
    // POLYPOLYGON16: two triangles of three points each.
    let mut two = vec![0u8; 16];
    two.extend(cat(vec![h32(2), h32(6), h32(3), h32(3)]));
    two.extend(cat([0, 0, 10, 0, 0, 10, 20, 0, 30, 0, 20, 10].iter().map(|v| h16(*v)).collect()));
    let p = parse(&emf(&[erec(91, &short), erec(91, &two)], [0; 4], [0; 4])).unwrap();
    assert_eq!(p.items.len(), 1);
    let (segs, ..) = path_segs(&p.items[0]);
    assert_eq!(segs.iter().filter(|s| matches!(s, Seg::Move(..))).count(), 2, "{segs:?}");
}

/// EMR_EXTCREATEPEN into slot 1 with LogPenEx style, width and brush style, then a selected rectangle.
fn ext_pen_rect(style: u32, width: u32, brush: u32) -> Option<([u8; 4], f32)> {
    let pen = erec_at(95, 52, &[(8, 1), (28, style), (32, width), (36, brush), (40, 0x0000_FF00)], &[]);
    let recs = [pen, erec(37, &h32(1)), erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)]))];
    let p = parse(&emf(&recs, [0; 4], [0; 4])).unwrap();
    path_segs(&p.items[0]).2
}

#[test]
fn emf_ext_pen_cosmetic_is_a_hairline_and_null_brush_draws_nothing() {
    // PS_COSMETIC widths are device pixels: a hairline, not one logical unit.
    assert_eq!(ext_pen_rect(0, 1, 0), Some(([0, 255, 0, 255], 0.0)));
    // PS_GEOMETRIC (0x10000) keeps its logical width.
    assert_eq!(ext_pen_rect(0x0001_0000, 4, 0), Some(([0, 255, 0, 255], 4.0)));
    // A BS_NULL brush draws no line.
    assert_eq!(ext_pen_rect(0x0001_0000, 4, 1), None);
}

#[test]
fn flip_tolerates_a_short_pixel_buffer() {
    let mut img = Rgba { w: 2, h: 2, bottom_up: false, px: vec![1u8; 7] };
    crate::dib::flip(&mut img, true, true);
    assert_eq!(img.px.len(), 7);
    // Three rows held of four: only the rows present swap (0 with 2).
    let mut img = Rgba { w: 1, h: 4, bottom_up: false, px: (0u8..12).collect() };
    crate::dib::flip(&mut img, false, true);
    assert_eq!(img.px, [8, 9, 10, 11, 4, 5, 6, 7, 0, 1, 2, 3]);
}

#[test]
fn place_fits_the_frame_scales_strokes_and_drops_far_paths() {
    let pic = crate::Picture {
        width: 100.0,
        height: 50.0,
        items: vec![
            Item::Path { segs: vec![Seg::Move(0.0, 0.0), Seg::Line(10.0, 10.0)], fill: None, stroke: Some(([0, 0, 0, 255], 4.0)), even_odd: true },
            Item::Path { segs: vec![Seg::Move(0.0, 0.0), Seg::Line(2e6, 0.0)], fill: None, stroke: None, even_odd: true },
        ],
    };
    let placed = pic.place(kurbo::Rect::new(0.0, 0.0, 200.0, 100.0), [0.0; 4]);
    assert_eq!(placed.len(), 1);
    let crate::PlacedItem::Path { stroke, .. } = &placed[0] else { panic!("expected a path") };
    assert_eq!(stroke.map(|s| s.1), Some(8.0));
}

#[test]
fn place_keeps_only_finite_bounded_bitmaps_with_area() {
    let bitmap = |rect: [f32; 4]| Item::Bitmap { rect, width: 1, height: 1, rgba: vec![0; 4] };
    let pic = crate::Picture {
        width: 100.0,
        height: 50.0,
        items: vec![
            bitmap([f32::NAN, 0.0, 10.0, 10.0]),
            bitmap([0.0, 0.0, 0.0, 10.0]),
            bitmap([0.0, 0.0, 2e6, 10.0]),
            bitmap([0.0, 0.0, f32::INFINITY, 10.0]),
            bitmap([10.0, 5.0, 10.0, 10.0]),
        ],
    };
    let placed = pic.place(kurbo::Rect::new(0.0, 0.0, 200.0, 100.0), [0.0; 4]);
    assert_eq!(placed.len(), 1);
    let crate::PlacedItem::Bitmap { index, rect } = &placed[0] else { panic!("expected a bitmap") };
    assert_eq!((*index, *rect), (4, kurbo::Rect::new(20.0, 10.0, 40.0, 30.0)));
}

#[test]
fn take_pixels_returns_only_drawable_bitmaps() {
    let mut ok = Item::Bitmap { rect: [0.0; 4], width: 2, height: 1, rgba: vec![7; 8] };
    assert_eq!(ok.take_pixels(), Some((2, 1, vec![7; 8])));
    assert!(matches!(&ok, Item::Bitmap { rgba, .. } if rgba.is_empty()), "the pixels move out");
    let mut short = Item::Bitmap { rect: [0.0; 4], width: 2, height: 2, rgba: vec![0; 8] };
    assert_eq!(short.take_pixels(), None);
    let side = crate::MAX_BITMAP_SIDE + 1;
    let mut wide = Item::Bitmap { rect: [0.0; 4], width: side, height: 1, rgba: vec![0; side as usize * 4] };
    assert_eq!(wide.take_pixels(), None);
    let mut path = Item::Path { segs: Vec::new(), fill: None, stroke: None, even_odd: false };
    assert_eq!(path.take_pixels(), None);
    let pic = |width: f32, height: f32| crate::Picture { width, height, items: Vec::new() };
    assert!(pic(1.0, 2.0).has_size());
    assert!(!pic(0.0, 2.0).has_size() && !pic(f32::NAN, 2.0).has_size() && !pic(1.0, f32::INFINITY).has_size());
}
