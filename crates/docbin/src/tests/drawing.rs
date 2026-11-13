//! Pictures and drawings: OfficeArt records ([MS-ODRAW]) hand-built for these tests from the
//! specification, with tiny pictures generated in code. No file produced by Word is used.

use proptest::prelude::*;
use wordcraft_doc::fill::Fill;
use wordcraft_doc::para::{Anchor, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::props::Rgb;

use super::{StructSpec, grpprl, plc, read, struct_doc, structured_spec};
use crate::officeart;

/// One OfficeArt record: header (recVer, recInstance, recType, recLen) + body.
fn rec(ver: u16, inst: u16, typ: u16, body: &[u8]) -> Vec<u8> {
    let vi = (inst << 4) | (ver & 0xF);
    [vi.to_le_bytes().as_slice(), &typ.to_le_bytes(), &(body.len() as u32).to_le_bytes(), body].concat()
}

fn container(typ: u16, kids: &[Vec<u8>]) -> Vec<u8> {
    rec(0xF, 0, typ, &kids.concat())
}

/// `OfficeArtFSP`: shape type, spid, flags.
fn fsp(spt: u16, spid: u32, flags: u32) -> Vec<u8> {
    rec(2, spt, 0xF00A, &[spid.to_le_bytes(), flags.to_le_bytes()].concat())
}

/// `OfficeArtFOPT` with simple properties and complex ones (data appended in order).
fn fopt(simple: &[(u16, u32)], complex: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (pid, op) in simple {
        body.extend_from_slice(&pid.to_le_bytes());
        body.extend_from_slice(&op.to_le_bytes());
    }
    for (pid, data) in complex {
        body.extend_from_slice(&(pid | 0x8000).to_le_bytes());
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    }
    for (_, data) in complex {
        body.extend_from_slice(data);
    }
    rec(3, (simple.len() + complex.len()) as u16, 0xF00B, &body)
}

/// A shape container with the given records.
fn sp(kids: &[Vec<u8>]) -> Vec<u8> {
    container(0xF004, kids)
}

/// A real 2×2 PNG.
fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// `OfficeArtBlipPNG` (one UID, tag) and the `OfficeArtFBSE` embedding it.
fn fbse_png(png: &[u8]) -> Vec<u8> {
    let blip = rec(0, 0x6E0, 0xF01E, &[[0u8; 16].as_slice(), &[0xFF], png].concat());
    let mut body = vec![6u8, 6];
    body.extend_from_slice(&[0u8; 16]); // rgbUid
    body.extend_from_slice(&0u16.to_le_bytes()); // tag
    body.extend_from_slice(&(blip.len() as u32).to_le_bytes()); // size
    body.extend_from_slice(&1u32.to_le_bytes()); // cRef
    body.extend_from_slice(&0u32.to_le_bytes()); // foDelay
    body.extend_from_slice(&[0, 0, 0, 0]); // unused1, cbName, unused2, unused3
    body.extend_from_slice(&blip);
    rec(2, 6, 0xF007, &body)
}

/// UTF-16LE, NUL-terminated (a complex string property).
fn wz(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect()
}

/// A `Spa` (26 bytes): spid, bounds in twips, flags, cTxbx.
fn spa(spid: u32, [l, t, r, b]: [i32; 4], flags: u16) -> Vec<u8> {
    [spid.to_le_bytes().as_slice(), &l.to_le_bytes(), &t.to_le_bytes(), &r.to_le_bytes(), &b.to_le_bytes(), &flags.to_le_bytes(), &0u32.to_le_bytes()]
        .concat()
}

/// `Spa` flags from bx, by, wr and fBelowText.
fn spa_flags(bx: u16, by: u16, wr: u16, below: bool) -> u16 {
    (bx << 1) | (by << 3) | (wr << 5) | (u16::from(below) << 14)
}

/// `OfficeArtContent`: a DggContainer holding a BLIP store, then one main-document drawing.
fn art_content(fbses: &[Vec<u8>], shapes: &[Vec<u8>]) -> Vec<u8> {
    let bstore = rec(0xF, fbses.len() as u16, 0xF001, &fbses.concat());
    let dgg = container(0xF000, &[bstore]);
    let patriarch = sp(&[fsp(0, 1024, 0b101)]);
    let spgr = container(0xF003, &[[patriarch].as_slice(), shapes].concat());
    let dg = container(0xF002, &[rec(0, 1, 0xF008, &[0u8; 8]), spgr]);
    [dgg, vec![0u8], dg].concat()
}

/// A PICF for an inline OfficeArt picture: `MM_SHAPE`, 1 × 0.5 inch goal size at 100 %.
fn picf_shape(rest: &[u8]) -> Vec<u8> {
    let mut picf = vec![0u8; 0x44];
    picf[0..4].copy_from_slice(&((0x44 + rest.len()) as u32).to_le_bytes());
    picf[4..6].copy_from_slice(&0x44u16.to_le_bytes());
    picf[6..8].copy_from_slice(&0x0064u16.to_le_bytes()); // mfpf.mm = MM_SHAPE
    picf[28..30].copy_from_slice(&1440i16.to_le_bytes());
    picf[30..32].copy_from_slice(&720i16.to_le_bytes());
    picf[32..34].copy_from_slice(&1000u16.to_le_bytes());
    picf[34..36].copy_from_slice(&1000u16.to_le_bytes());
    [picf.as_slice(), rest].concat()
}

#[test]
fn inline_officeart_picture_with_crop_and_alt_text() {
    let png = tiny_png();
    // A picture frame whose pib names the first FBSE that follows it, a quarter cropped
    // off the left, with alt text.
    let shape = sp(&[fsp(75, 1025, 0xA00), fopt(&[(0x0102, 0x4000), (0x0104, 1)], &[(0x0381, wz("Logo"))])]);
    let data = picf_shape(&[shape, fbse_png(&png)].concat());
    let text = "Pic\u{1}!\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec { text, data_stream: data, chpx_runs: vec![(0, grpprl(&[(0x6A03, &0u32.to_le_bytes())]))], ..structured_spec(text, ccp) };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.text, "Pic\u{FFFC}!");
    let Some(InlineObject::Image { media, w, h, alt, crop, float, .. }) = p0.objects.first() else { panic!("no image: {:?}", p0.objects) };
    assert_eq!((*w, *h), (54.0, 36.0)); // 72 pt less a quarter, × 36 pt
    assert_eq!(*crop, [0.25, 0.0, 0.0, 0.0]);
    assert_eq!(alt, "Logo");
    assert_eq!(float.wrap, Wrap::Inline);
    assert_eq!(doc.media.get(media).map(|b| b.as_slice()), Some(png.as_slice()));
}

#[test]
fn floating_picture_from_the_blip_store_keeps_position_and_wrap() {
    let png = tiny_png();
    let pic = sp(&[fsp(75, 1025, 0xA00), fopt(&[(0x0104, 1)], &[])]);
    let art = art_content(&[fbse_png(&png)], &[pic]);
    // Two inches by one, 1" from the page's left edge and 0.5" below the top margin; square wrap.
    let spas = plc(&[1, 3], &spa(1025, [1440, 720, 4320, 2160], spa_flags(1, 0, 2, false)));
    let text = "A\u{8}B\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec { text, extra_pairs: vec![(40, spas), (50, art)], ..structured_spec(text, ccp) };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.text, "A\u{FFFC}B");
    let Some(InlineObject::Image { media, w, h, float, .. }) = p0.objects.first() else { panic!("no image: {:?}", p0.objects) };
    assert_eq!((*w, *h), (144.0, 72.0));
    assert_eq!((float.wrap, float.h_rel, float.v_rel, float.x, float.y), (Wrap::Square, Anchor::Page, Anchor::Margin, 72.0, 36.0));
    assert_eq!(doc.media.get(media).map(|b| b.as_slice()), Some(png.as_slice()));
}

#[test]
fn shapes_keep_kind_colours_wrap_and_text_box_text() {
    // A red ellipse with a 2 pt blue outline behind the text, and an unfilled text box.
    let ellipse = sp(&[fsp(3, 1026, 0xA00), fopt(&[(0x0181, 0x0000_00FF), (0x01C0, 0x00FF_0000), (0x01CB, 25_400)], &[])]);
    let tb = sp(&[fsp(202, 1027, 0xA00), fopt(&[(0x01BF, 1 << 20)], &[]), rec(0, 0, 0xF00D, &0x0001_0000u32.to_le_bytes())]);
    let art = art_content(&[], &[ellipse, tb]);
    let spas =
        plc(&[1, 2, 4], &[spa(1026, [0, 0, 1440, 1440], spa_flags(2, 2, 3, true)), spa(1027, [0, 0, 2880, 720], spa_flags(0, 0, 1, false))].concat());
    // The text box story: an FTXBXS whose lid is the shape's spid (cTxbx, cReusable, fReusable,
    // reserved, lid, txidUndo).
    let ftxbxs =
        [1i32.to_le_bytes().as_slice(), &0i32.to_le_bytes(), &0i16.to_le_bytes(), &0i32.to_le_bytes(), &1027u32.to_le_bytes(), &0i32.to_le_bytes()]
            .concat();
    let txbx = plc(&[0, 7], &ftxbxs);
    let text = "A\u{8}\u{8}B\r";
    let ccp = text.chars().count() as u32;
    // The text box story is stored as the subdocument right after the main text; the endnote
    // slot of the fixture writes it there and the FIB is then patched to call it the text box
    // subdocument (ccpEdn = 0, ccpTxbx = 7).
    let spec =
        StructSpec { text, endnote_stories: vec!["Inside\r"], extra_pairs: vec![(40, spas), (50, art), (56, txbx)], ..structured_spec(text, ccp) };
    let file = patch_word(&struct_doc(&spec), &[(0x60, 0), (0x64, 7)]);
    let doc = read(&file).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.objects.len(), 2, "{:?}", p0.objects);
    let Some(InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, .. }) = p0.objects.first() else { panic!("{:?}", p0.objects) };
    assert_eq!((*kind, *w, *h), (ShapeKind::Ellipse, 72.0, 72.0));
    assert_eq!((fill.clone(), *stroke, *stroke_width), (Some(Fill::solid(Rgb(255, 0, 0))), Some(Rgb(0, 0, 255)), 2.0));
    assert_eq!((float.wrap, float.h_rel, float.v_rel), (Wrap::BehindText, Anchor::Column, Anchor::Paragraph));
    let Some(InlineObject::Shape { kind, fill, float, story, .. }) = p0.objects.get(1) else { panic!("{:?}", p0.objects) };
    assert_eq!((*kind, fill.clone(), float.wrap), (ShapeKind::TextBox, None, Wrap::TopAndBottom));
    let part = story.and_then(|id| doc.parts.get(&id)).expect("text box story part");
    assert_eq!(part.kind, wordcraft_doc::PartKind::TextBox);
    assert_eq!(part.blocks.first().and_then(|b| b.as_para()).map(|p| p.text.as_str()), Some("Inside"));
}

#[test]
fn text_boxes_pointing_at_the_same_story_text_read_it_once() {
    // Two text boxes whose PlcftxbxTxt stories are both [0, 7) (the table steps back to CP 0
    // in between): each was read into its own part, so a hostile table could copy one story
    // thousands of times. The second story is dropped and its box stays empty.
    let tb = |spid: u32| sp(&[fsp(202, spid, 0xA00), fopt(&[(0x01BF, 1 << 20)], &[]), rec(0, 0, 0xF00D, &0x0001_0000u32.to_le_bytes())]);
    let art = art_content(&[], &[tb(1027), tb(1028)]);
    let flags = spa_flags(0, 0, 1, false);
    let spas = plc(&[1, 2, 4], &[spa(1027, [0, 0, 2880, 720], flags), spa(1028, [0, 0, 2880, 720], flags)].concat());
    // FTXBXS: cTxbx, cReusable, fReusable, reserved, lid, txidUndo.
    let ftxbxs = |lid: u32| {
        [1i32.to_le_bytes().as_slice(), &0i32.to_le_bytes(), &0i16.to_le_bytes(), &0i32.to_le_bytes(), &lid.to_le_bytes(), &0i32.to_le_bytes()]
            .concat()
    };
    let txbx = plc(&[0, 7, 0, 7], &[ftxbxs(1027), ftxbxs(1), ftxbxs(1028)].concat());
    let text = "A\u{8}\u{8}B\r";
    let ccp = text.chars().count() as u32;
    // As above: the endnote slot holds the text box subdocument, then the FIB is patched.
    let spec =
        StructSpec { text, endnote_stories: vec!["Inside\r"], extra_pairs: vec![(40, spas), (50, art), (56, txbx)], ..structured_spec(text, ccp) };
    let doc = read(&patch_word(&struct_doc(&spec), &[(0x60, 0), (0x64, 7)])).expect("opens");
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    let stories: Vec<Option<u32>> =
        p0.objects.iter().filter_map(|o| if let InlineObject::Shape { story, .. } = o { Some(*story) } else { None }).collect();
    let [Some(first), None] = stories[..] else { panic!("stories: {stories:?}") };
    let part = doc.parts.get(&first).expect("text box story part");
    assert_eq!(part.blocks.first().and_then(|b| b.as_para()).map(|p| p.text.as_str()), Some("Inside"));
    assert_eq!(doc.parts.len(), 1);
}

/// Overwrite little-endian u32s in the WordDocument stream of a compound file.
fn patch_word(file: &[u8], at: &[(usize, u32)]) -> Vec<u8> {
    use std::io::Read;
    let mut comp = cfb::CompoundFile::open(std::io::Cursor::new(file.to_vec())).unwrap();
    let mut streams = Vec::new();
    for name in ["WordDocument", "0Table", "Data"] {
        if let Ok(mut s) = comp.open_stream(format!("/{name}")) {
            let mut v = Vec::new();
            s.read_to_end(&mut v).unwrap();
            if name == "WordDocument" {
                for (o, x) in at {
                    v[*o..*o + 4].copy_from_slice(&x.to_le_bytes());
                }
            }
            streams.push((name, v));
        }
    }
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, v)| (*n, v.as_slice())).collect();
    super::cfb_file(&refs)
}

#[test]
fn dib_becomes_png_and_compressed_metafiles_inflate() {
    // A 2×1 24-bit DIB (BITMAPINFOHEADER, rows padded to 4 bytes).
    let mut dib = Vec::new();
    for v in [40u32, 2, 1] {
        dib.extend_from_slice(&v.to_le_bytes());
    }
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&24u16.to_le_bytes());
    dib.extend_from_slice(&[0u8; 24]);
    dib.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
    let r = rec(0, 0x7A8, 0xF01F, &[[0u8; 16].as_slice(), &[0xFF], &dib].concat());
    let (dib_rec, _) = officeart::record(&r).unwrap();
    let blip = officeart::blip(&dib_rec, 1 << 20).expect("DIB decodes");
    assert_eq!(blip.ext, "png");
    let img = image::load_from_memory(&blip.bytes).unwrap().to_rgb8();
    assert_eq!((img.width(), img.height(), img.get_pixel(0, 0).0), (2, 1, [255, 0, 0]));

    // A deflated WMF stays a WMF (the metafile renderer draws it).
    let wmf: Vec<u8> = [1u16, 9, 0x300].iter().flat_map(|v| v.to_le_bytes()).chain([0u8; 12]).collect();
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&wmf, 6);
    let mut hdr = Vec::new();
    hdr.extend_from_slice(&(wmf.len() as u32).to_le_bytes());
    hdr.extend_from_slice(&[0u8; 24]); // rcBounds, ptSize
    hdr.extend_from_slice(&(packed.len() as u32).to_le_bytes());
    hdr.extend_from_slice(&[0x00, 0xFE]); // compression = deflate, filter
    let r = rec(0, 0x216, 0xF01B, &[[0u8; 16].as_slice(), &hdr, &packed].concat());
    let (wmf_rec, _) = officeart::record(&r).unwrap();
    let blip = officeart::blip(&wmf_rec, 1 << 20).expect("WMF inflates");
    assert_eq!((blip.ext, blip.bytes), ("wmf", wmf));
}

#[test]
fn deeply_nested_groups_and_lying_lengths_are_bounded() {
    // 2000 nested group containers: the walk stops at its depth cap (no stack overflow).
    let mut g = sp(&[fsp(1, 2000, 0)]);
    for _ in 0..2_000 {
        g = container(0xF003, &[sp(&[fsp(0, 2001, 1)]), g]);
    }
    let _ = officeart::content(&art_content(&[], &[g]));
    // A metafile claiming 4 GB uncompressed with garbage data, and an FBSE pointing far out.
    let mut hdr = vec![0xFFu8; 4];
    hdr.extend_from_slice(&[0u8; 24]);
    hdr.extend_from_slice(&u32::MAX.to_le_bytes());
    hdr.extend_from_slice(&[0, 0]);
    let r = rec(0, 0x3D4, 0xF01A, &[[0u8; 16].as_slice(), &hdr, &[0x78, 0x9C, 1, 2, 3]].concat());
    let (emf, _) = officeart::record(&r).unwrap();
    assert!(officeart::blip(&emf, 1 << 20).is_none());
    let mut fbse = vec![0u8; 36];
    fbse[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(officeart::fbse_blip(&fbse, &[0u8; 64], 1 << 20).is_none());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn junk_drawings_never_panic(art in proptest::collection::vec(any::<u8>(), 0..2048), spas in proptest::collection::vec(any::<u8>(), 0..512), data in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let text = "A\u{8}\u{1}\u{8}B\r";
        let ccp = text.chars().count() as u32;
        let picf = picf_shape(&data);
        let spec = StructSpec {
            text,
            data_stream: picf,
            chpx_runs: vec![(0, grpprl(&[(0x6A03, &0u32.to_le_bytes())]))],
            extra_pairs: vec![(40, spas), (50, art.clone()), (56, art)],
            ..structured_spec(text, ccp)
        };
        let _ = read(&struct_doc(&spec));
    }
}

// A 1×1 24-bit DIB, including its padded four-byte row.
fn one_pixel_dib() -> Vec<u8> {
    let mut dib = [40u32, 1, 1].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&24u16.to_le_bytes());
    dib.extend_from_slice(&[0u8; 24]);
    dib.extend_from_slice(&[30, 60, 90, 0]);
    dib
}

#[test]
fn dib_conversion_caps_header_and_encoded_bytes() {
    let dib = one_pixel_dib();
    let record = rec(0, 0x7A8, officeart::rt::BLIP_DIB, &[[0u8; 16].as_slice(), &[0xFF], &dib].concat());
    let (record, _) = officeart::record(&record).unwrap();
    let bmp_size = 14 + dib.len();
    assert!(officeart::blip(&record, bmp_size - 1).is_none(), "the BMP header also needs room");
    let fallback = officeart::blip(&record, bmp_size).expect("the BMP fallback fits exactly");
    assert_eq!((fallback.ext, fallback.placeholder, fallback.bytes.len()), ("bmp", true, bmp_size));
    let img = image::load_from_memory(&fallback.bytes).unwrap().to_rgb8();
    assert_eq!((img.width(), img.height(), img.get_pixel(0, 0).0), (1, 1, [90, 60, 30]));
    let normal = officeart::blip(&record, 512).expect("normal conversion fits");
    assert_eq!((normal.ext, normal.placeholder), ("png", false));
    assert!(normal.bytes.len() > bmp_size && normal.bytes.len() <= 512);
    assert_eq!(image::load_from_memory(&normal.bytes).unwrap().width(), 1);
}
