//! Synthetic `.doc` fixtures, written for these tests from the [MS-DOC] specification.
//! Files produced by Word are never committed; every fixture here is built in code.

use std::io::Write;

use proptest::prelude::*;

use super::read;

/// Offset in the WordDocument stream where test text pieces are placed (past the FIB).
const TEXT_FC: u32 = 0x600;
/// Offset in the Table stream where the test Clx is placed.
const CLX_AT: u32 = 0x40;

/// Build an in-memory compound file holding the given root streams.
pub(crate) fn cfb_file(streams: &[(&str, &[u8])]) -> Vec<u8> {
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).expect("cfb create");
    for (name, data) in streams {
        let mut s = comp.create_stream(format!("/{name}")).expect("cfb stream");
        s.write_all(data).expect("cfb write");
    }
    comp.into_inner().into_inner()
}

/// Knobs of the minimal FIB these tests build (Word 97 shape, `cbRgFcLcb` = 0x5D).
pub(crate) struct FibSpec {
    pub(crate) nfib: u16,
    pub(crate) flags: u16,
    pub(crate) ccp_text: u32,
    pub(crate) ccp_hdd: u32,
    /// (pair index, fc, lcb) entries to poke into the RgFcLcb blob.
    pub(crate) pairs: Vec<(usize, u32, u32)>,
}

impl Default for FibSpec {
    fn default() -> Self {
        FibSpec { nfib: 0x00C1, flags: 0x1000, ccp_text: 0, ccp_hdd: 0, pairs: Vec::new() }
    }
}

/// A minimal Word 97 FIB: FibBase + csw/rgw + cslw/rglw + 0x5D fc/lcb pairs + cswNew = 0.
pub(crate) fn fib_bytes(spec: &FibSpec) -> Vec<u8> {
    let mut v = vec![0u8; 0x9A + 0x5D * 8 + 2];
    let put16 = |v: &mut Vec<u8>, at: usize, x: u16| {
        v[at..at + 2].copy_from_slice(&x.to_le_bytes());
    };
    let put32 = |v: &mut Vec<u8>, at: usize, x: u32| {
        v[at..at + 4].copy_from_slice(&x.to_le_bytes());
    };
    put16(&mut v, 0x00, 0xA5EC);
    put16(&mut v, 0x02, spec.nfib);
    put16(&mut v, 0x0A, spec.flags);
    put16(&mut v, 0x0C, 0x00BF);
    put16(&mut v, 0x20, 0x000E);
    put16(&mut v, 0x3E, 0x0016);
    put32(&mut v, 0x40, 0x1000); // cbMac
    put32(&mut v, 0x4C, spec.ccp_text); // rglw[3] = ccpText
    put32(&mut v, 0x50, spec.ccp_hdd); // rglw[5] = ccpHdd
    put16(&mut v, 0x98, 0x005D);
    for (i, fc, lcb) in &spec.pairs {
        put32(&mut v, 0x9A + i * 8, *fc);
        put32(&mut v, 0x9A + i * 8 + 4, *lcb);
    }
    v
}

/// One test piece: text stored either uncompressed (UTF-16LE) or compressed (Windows-1252).
#[derive(Clone)]
pub(crate) struct Piece {
    pub(crate) compressed: bool,
    pub(crate) text: String,
}

/// Build a PlcPcd (aCp[n+1] + aPcd[n]) for pieces stored at consecutive offsets from
/// `first_fc` (compressed pieces start at `fc = 2 × byte offset` per FcCompressed).
fn plcpcd(pieces: &[Piece], first_fc: u32) -> Vec<u8> {
    let mut v = Vec::new();
    let mut cp = 0u32;
    v.extend_from_slice(&cp.to_le_bytes());
    for p in pieces {
        cp += p.text.chars().count() as u32;
        v.extend_from_slice(&cp.to_le_bytes());
    }
    let mut off = 0u32;
    for p in pieces {
        // Bytes this piece occupies in the text region: one per character compressed, two
        // uncompressed (count chars, not UTF-8 bytes).
        let len = if p.compressed { p.text.chars().count() } else { p.text.chars().count() * 2 } as u32;
        let fc = if p.compressed { 2 * (first_fc + off) } else { first_fc + off };
        let fc_raw = if p.compressed { fc | 0x4000_0000 } else { fc };
        v.extend_from_slice(&[0, 0]); // fNoParaLast & friends
        v.extend_from_slice(&fc_raw.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes()); // prm
        off += len;
    }
    v
}

fn piece_bytes(p: &Piece) -> Vec<u8> {
    if p.compressed {
        // Tests only use cp1252-encodable chars in compressed pieces.
        p.text.chars().map(|c| c as u8).collect()
    } else {
        let units: Vec<u16> = p.text.chars().map(|c| c as u16).collect();
        units.iter().flat_map(|u| u.to_le_bytes()).collect()
    }
}

/// A document whose main text is the given pieces (extra subdocument text may follow).
fn text_doc(main: &[Piece], extra: &[Piece], ccp_hdd: u32) -> Vec<u8> {
    let all: Vec<Piece> = main.iter().chain(extra.iter()).cloned().collect();
    let ccp_text: u32 = main.iter().map(|p| p.text.chars().count() as u32).sum();
    let mut word = vec![0u8; TEXT_FC as usize];
    for p in &all {
        word.extend_from_slice(&piece_bytes(p));
    }
    let plc = plcpcd(&all, TEXT_FC);
    let mut clx = vec![0x02u8];
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);
    let mut table = vec![0u8; CLX_AT as usize];
    table.extend_from_slice(&clx);
    let spec = FibSpec { ccp_text, ccp_hdd, pairs: vec![(33, CLX_AT, clx.len() as u32)], ..Default::default() };
    word[..fib_bytes(&spec).len()].copy_from_slice(&fib_bytes(&spec));
    cfb_file(&[("WordDocument", &word), ("0Table", &table)])
}

fn body_texts(doc: &wordcraft_doc::Document) -> Vec<String> {
    doc.body.iter().filter_map(|b| b.as_para().map(|p| p.text.clone())).collect()
}

#[test]
fn not_a_compound_file() {
    assert!(matches!(read(&[0u8; 64]), Err(super::DocbinError::Container(_))));
}

#[test]
fn cfb_without_word_stream() {
    let f = cfb_file(&[("0Table", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(_))));
}

#[test]
fn word_stream_too_short() {
    let f = cfb_file(&[("WordDocument", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(_))));
}

#[test]
fn wrong_magic_rejected() {
    let mut fib = fib_bytes(&FibSpec::default());
    fib[0] = 0x34;
    fib[1] = 0x12;
    let f = cfb_file(&[("WordDocument", &fib)]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(m)) if m.contains("magic")));
}

#[test]
fn word_6_95_rejected() {
    let mut fib = fib_bytes(&FibSpec::default());
    fib[0] = 0xDC;
    fib[1] = 0xA5;
    let f = cfb_file(&[("WordDocument", &fib)]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(m)) if m.contains("6.0/95")));
}

#[test]
fn encrypted_rejected() {
    let spec = FibSpec { flags: 0x1100, ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec))]);
    assert!(matches!(read(&f), Err(super::DocbinError::Encrypted)));
}

#[test]
fn wrong_csw_rejected() {
    let mut fib = fib_bytes(&FibSpec::default());
    fib[0x20] = 0x0D;
    fib[0x21] = 0x00;
    let f = cfb_file(&[("WordDocument", &fib)]);
    assert!(matches!(read(&f), Err(super::DocbinError::NotWord(m)) if m.contains("csw")));
}

#[test]
fn missing_clx_rejected() {
    let spec = FibSpec { ccp_text: 3, ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec)), ("0Table", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::Malformed(m)) if m.contains("piece table")));
}

#[test]
fn unicode_text_extracted() {
    let f = text_doc(&[Piece { compressed: false, text: "Héllo Wörld\rSecond€\r".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["Héllo Wörld", "Second€"]);
}

#[test]
fn compressed_cp1252_text_extracted() {
    let f = text_doc(&[Piece { compressed: true, text: "caf\u{E9} 100\u{BD}\r".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["café 100½"]);
}

#[test]
fn multiple_pieces_and_encodings() {
    let f = text_doc(
        &[
            Piece { compressed: false, text: "plain ".into() },
            Piece { compressed: true, text: "mixed\u{E9} ".into() },
            Piece { compressed: false, text: "end\r".into() },
        ],
        &[],
        0,
    );
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["plain mixedé end"]);
}

#[test]
fn special_characters_mapped() {
    let raw = "a\tb\u{B}c\u{C}d\u{E}e\u{1E}f\u{1F}g\r\u{13}field\u{14}result\u{15}\r";
    let f = text_doc(&[Piece { compressed: false, text: raw.into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["a\tb\nc\u{C}d\u{E}e\u{2011}f\u{AD}g", "fieldresult"]);
}

#[test]
fn empty_paragraphs_kept() {
    let f = text_doc(&[Piece { compressed: false, text: "a\r\rb\r".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["a", "", "b"]);
}

#[test]
fn empty_document_gets_a_paragraph() {
    let f = text_doc(&[Piece { compressed: false, text: "".into() }], &[], 0);
    let doc = read(&f).expect("opens");
    assert_eq!(doc.body.len(), 1);
}

#[test]
fn header_text_stays_out_of_body() {
    let f = text_doc(
        &[Piece { compressed: false, text: "body\r".into() }],
        &[Piece { compressed: false, text: "header\u{D}".into() }],
        8, // ccpHdd covers the extra piece
    );
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["body"]);
}

#[test]
fn clx_outside_table_rejected() {
    let spec = FibSpec { pairs: vec![(33, 0x1000, 0x40)], ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec)), ("0Table", &[0u8; 16])]);
    assert!(matches!(read(&f), Err(super::DocbinError::Malformed(_))));
}

#[test]
fn garbage_plcpcd_rejected() {
    let mut table = vec![0u8; CLX_AT as usize];
    table.extend_from_slice(&[0x02, 0x05, 0x00, 0x00, 0x00, 1, 2, 3, 4]); // lcb 5: not ≡ 1 mod 12
    let spec = FibSpec { pairs: vec![(33, CLX_AT, 9)], ..Default::default() };
    let f = cfb_file(&[("WordDocument", &fib_bytes(&spec)), ("0Table", &table)]);
    assert!(matches!(read(&f), Err(super::DocbinError::Malformed(m)) if m.contains("PlcPcd")));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn junk_bytes_never_panic(b in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = read(&b);
    }

    #[test]
    fn truncated_file_never_panics(n in 0..4096usize) {
        let f = text_doc(
            &[Piece { compressed: false, text: "Hello\rworld\r\u{E9}x\r".into() }],
            &[],
            0,
        );
        let n = n.min(f.len());
        let _ = read(&f[..n]);
    }

    #[test]
    fn flipped_bytes_never_panic(pos in 0..4096usize, bit in 0..8usize) {
        let mut f = text_doc(
            &[Piece { compressed: false, text: "Hello\rworld\r\u{E9}x\r".into() }],
            &[],
            0,
        );
        let at = pos.min(f.len() - 1);
        if let Some(b) = f.get_mut(at) {
            *b ^= 1 << (bit & 7);
        }
        let _ = read(&f);
    }
}
