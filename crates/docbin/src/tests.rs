//! Synthetic `.doc` fixtures, written for these tests from the [MS-DOC] specification.
//! Files produced by Word are never committed; every fixture here is built in code.

use std::io::Write;

use super::{DocbinError, is_encrypted, read, read_with_password};

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
    pub(crate) ccp_ftn: u32,
    pub(crate) ccp_hdd: u32,
    pub(crate) ccp_edn: u32,
    /// (pair index, fc, lcb) entries to poke into the RgFcLcb blob.
    pub(crate) pairs: Vec<(usize, u32, u32)>,
}

impl Default for FibSpec {
    fn default() -> Self {
        FibSpec { nfib: 0x00C1, flags: 0x1000, ccp_text: 0, ccp_ftn: 0, ccp_hdd: 0, ccp_edn: 0, pairs: Vec::new() }
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
    put32(&mut v, 0x50, spec.ccp_ftn); // rglw[4] = ccpFtn
    put32(&mut v, 0x54, spec.ccp_hdd); // rglw[5] = ccpHdd
    put32(&mut v, 0x60, spec.ccp_edn); // rglw[8] = ccpEdn
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
    text_doc_with(main, extra, FibSpec { ccp_hdd, ..Default::default() }, &[])
}

/// [`text_doc`] with the FIB knobs given (`ccp_text` is filled in from `main`; `pairs` must be empty) and extra
/// Table-stream data placed after the piece table: `(pair index, bytes)` for each.
fn text_doc_with(main: &[Piece], extra: &[Piece], spec: FibSpec, table_data: &[(usize, &[u8])]) -> Vec<u8> {
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
    let mut pairs = vec![(33, CLX_AT, clx.len() as u32)];
    for (pair, bytes) in table_data {
        pairs.push((*pair, table.len() as u32, bytes.len() as u32));
        table.extend_from_slice(bytes);
    }
    debug_assert!(spec.pairs.is_empty(), "text_doc_with sets the pairs itself");
    let spec = FibSpec { ccp_text, pairs, ..spec };
    word[..fib_bytes(&spec).len()].copy_from_slice(&fib_bytes(&spec));
    cfb_file(&[("WordDocument", &word), ("0Table", &table)])
}

fn body_texts(doc: &wordcraft_doc::Document) -> Vec<String> {
    doc.body.iter().filter_map(|block| block.as_para().map(|paragraph| paragraph.text.clone())).collect()
}

fn assert_body_texts(doc: &wordcraft_doc::Document, expected: &[&str]) {
    assert_eq!(body_texts(doc), expected.iter().map(|text| String::from(*text)).collect::<Vec<_>>());
}

#[test]
fn rejects_non_compound_input_with_the_shared_error_category() {
    assert!(matches!(read(&[0u8; 64]), Err(DocbinError::Container(_))));
    assert!(!is_encrypted(&[0u8; 64]));
}

#[test]
fn reads_compressed_and_utf16_document_text() {
    let fixtures = [
        (Piece { compressed: true, text: "Hello\rworld".into() }, vec!["Hello", "world"]),
        (Piece { compressed: true, text: "Résumé".into() }, vec!["Résumé"]),
        (Piece { compressed: false, text: "日本語".into() }, vec!["日本語"]),
    ];
    for (piece, expected) in fixtures {
        let input = text_doc(std::slice::from_ref(&piece), &[], 0);
        let document = read(&input).expect("shared DOC reader");
        assert_body_texts(&document, &expected);
        assert!(!is_encrypted(&input));
        assert_body_texts(&read_with_password(&input, Some("ignored")).expect("unprotected password ignored"), &expected);
    }
}

fn all_text(doc: &wordcraft_doc::Document) -> String {
    let mut s: String = body_texts(doc).join("\n");
    for part in doc.parts.values() {
        for b in &part.blocks {
            if let Some(p) = b.as_para() {
                s.push('\n');
                s.push_str(&p.text);
            }
        }
    }
    s
}

/// #6359: a document whose stream holds `0123456789END\r` once, with a main text of `body`
/// pieces that all name the digits, then `END\r`; with `note` > 0, a footnote at CP 0 whose
/// story is `note` more such pieces, then `END\r`.
fn aliased_digits_doc(body: u32, note: u32) -> Vec<u8> {
    let digits = (2 * TEXT_FC) | 0x4000_0000;
    let end = (2 * (TEXT_FC + 10)) | 0x4000_0000;
    let (mut cps, mut fcs) = (vec![0u32], Vec::new());
    let mut story = |n: u32| {
        for (len, fc) in std::iter::repeat_n((10, digits), n as usize).chain([(4, end)]) {
            cps.push(cps.last().copied().unwrap_or(0) + len);
            fcs.push(fc);
        }
    };
    story(body);
    if note > 0 {
        story(note);
    }
    let mut plc: Vec<u8> = cps.iter().flat_map(|c| c.to_le_bytes()).collect();
    for fc in &fcs {
        plc.extend_from_slice(&[0, 0]);
        plc.extend_from_slice(&fc.to_le_bytes());
        plc.extend_from_slice(&0u16.to_le_bytes());
    }
    let mut table = vec![0u8; CLX_AT as usize];
    table.push(0x02);
    table.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    table.extend_from_slice(&plc);
    let ccp_text = body * 10 + 4;
    let mut pairs = vec![(33, CLX_AT, (plc.len() + 5) as u32)];
    let ccp_ftn = if note > 0 { note * 10 + 4 } else { 0 };
    if note > 0 {
        let cps = |v: &[u32]| v.iter().flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>();
        let mut refs = cps(&[0, 1]);
        refs.extend_from_slice(&[0, 0]); // the one reference's data
        // PlcffndRef (pair 2), then PlcffndTxt (pair 3): story 0..ccp_ftn, then the limit.
        for (pair, bytes) in [(2, refs), (3, cps(&[0, ccp_ftn, ccp_ftn]))] {
            pairs.push((pair, table.len() as u32, bytes.len() as u32));
            table.extend_from_slice(&bytes);
        }
    }
    let mut word = vec![0u8; TEXT_FC as usize];
    word.extend_from_slice(b"0123456789END\r");
    let fib = fib_bytes(&FibSpec { ccp_text, ccp_ftn, pairs, ..Default::default() });
    word[..fib.len()].copy_from_slice(&fib);
    cfb_file(&[("WordDocument", &word), ("0Table", &table)])
}

/// #6359: the character budget (twice the 1,550-byte stream: 3,100) used to stop the walk and
/// return the text cut short, with no error. A file over it must be refused like the paragraph
/// cap; one under it opens in full.
///
/// The walk now lives in `craft-doc-binary` (this crate only adapts its model), and the pinned
/// revision still stops and returns `Ok` with the text cut short. The fix belongs there; these
/// tests are off until it lands, and must then be un-ignored unchanged.
#[test]
#[ignore = "craft-doc-binary still returns the text cut short at the character budget (#6359)"]
fn text_repeated_past_the_character_budget_is_refused_not_cut() {
    let over = read(&aliased_digits_doc(1000, 0));
    assert!(matches!(&over, Err(DocbinError::Limit(m)) if m.contains("repeats")), "{:?}", over.map(|d| all_text(&d).len()));
    let doc = read(&aliased_digits_doc(300, 0)).expect("opens");
    let body = body_texts(&doc);
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].len(), 3003);
    assert!(body[0].ends_with("789END"), "{:?}", &body[0][2990..]);
}

/// The character budget is one for the whole document: a body and a footnote that each fit
/// but together don't are refused too. Off for the same reason as the test above (#6359).
#[test]
#[ignore = "craft-doc-binary still returns the text cut short at the character budget (#6359)"]
fn the_character_budget_is_shared_by_the_body_and_a_footnote() {
    let doc = read(&aliased_digits_doc(200, 100)).expect("opens");
    assert_eq!(doc.parts.len(), 1);
    assert!(body_texts(&doc).first().is_some_and(|t| t.ends_with("END")));
    let over = read(&aliased_digits_doc(200, 110));
    assert!(matches!(&over, Err(DocbinError::Limit(m)) if m.contains("repeats")), "{:?}", over.map(|d| all_text(&d).len()));
    let over = read(&aliased_digits_doc(210, 100));
    assert!(matches!(&over, Err(DocbinError::Limit(m)) if m.contains("repeats")), "{:?}", over.map(|d| all_text(&d).len()));
}
