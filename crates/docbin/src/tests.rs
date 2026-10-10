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

// ---------------------------------------------------------------------------------------------
// D2: formatting — fixtures with a stylesheet, font table and FKP pages.

/// Twips→FC units inside a piece: 2 per character.
fn fc_of(cp: u32) -> u32 {
    TEXT_FC + 2 * cp
}

/// A grpprl of Prls given as (opcode, operand) pairs.
fn grpprl(prls: &[(u16, &[u8])]) -> Vec<u8> {
    let mut v = Vec::new();
    for (op, operand) in prls {
        v.extend_from_slice(&op.to_le_bytes());
        v.extend_from_slice(operand);
    }
    v
}

/// One STD record (Word 97 shape, cbSTDBaseInFile = 10).
fn std_bytes(stk: u8, istd_base: u16, name: &str, papx: &[u8], chpx: &[u8]) -> Vec<u8> {
    let cupx = if stk == 1 { 2 } else { 1 };
    let mut base = vec![0u8; 10];
    // sti 0xFFF (user-defined) | flags
    base[0] = 0xFF;
    base[1] = 0x0F;
    // stk(4) | istdNext(12) = none (0xFFF)
    base[2] = stk | 0xF0;
    base[3] = 0xFF;
    // cupx(4) | istdBase(12)
    let w2 = (cupx as u16) | (istd_base << 4);
    base[4..6].copy_from_slice(&w2.to_le_bytes());
    // bchUpe filled below; grfstd = 0
    let mut v = base;
    // xstzName: cch + UTF-16 + terminator
    let units: Vec<u16> = name.chars().map(|c| c as u16).collect();
    v.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for u in &units {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&0u16.to_le_bytes());
    if stk == 1 {
        // LPUpxPapx: cbUpx + UpxPapx (istd u16 + grpprl), padded to even.
        let papx_upx = [0u16.to_le_bytes().as_slice(), papx].concat();
        v.extend_from_slice(&(papx_upx.len() as u16).to_le_bytes());
        v.extend_from_slice(&papx_upx);
        if papx_upx.len() % 2 == 1 {
            v.push(0);
        }
    }
    // LPUpxChpx: cbUpx + raw grpprl.
    v.extend_from_slice(&(chpx.len() as u16).to_le_bytes());
    v.extend_from_slice(chpx);
    if chpx.len() % 2 == 1 {
        v.push(0);
    }
    let cb = v.len() as u16;
    let mut out = cb.to_le_bytes().to_vec();
    out.extend_from_slice(&v);
    // bchUpe (offset 6 in the Stdf) equals cbStd.
    out[6 + 2] = cb as u8;
    out[7 + 2] = (cb >> 8) as u8;
    out
}

/// An STSH with the given styles.
fn stsh_bytes(styles: &[Vec<u8>]) -> Vec<u8> {
    let stshi = [(styles.len() as u16).to_le_bytes(), 10u16.to_le_bytes()].concat(); // cstd, cbSTDBaseInFile
    let mut v = (stshi.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&stshi);
    for s in styles {
        v.extend_from_slice(s);
    }
    v
}

/// A non-extended SttbfFfn of font names.
fn ffn_bytes(names: &[&str]) -> Vec<u8> {
    let mut v = (names.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&0u16.to_le_bytes()); // cbExtra
    for n in names {
        v.extend_from_slice(&[0u8; 40]); // FFN header
        for c in n.chars() {
            v.extend_from_slice(&(c as u16).to_le_bytes());
        }
        v.extend_from_slice(&0u16.to_le_bytes()); // NUL
    }
    v
}

/// A ChpxFkp page mapping run boundaries (in CPs, ends exclusive) to grpprls.
fn chpx_fkp(runs: &[(u32, Vec<u8>)], end_cp: u32) -> Vec<u8> {
    let mut page = vec![0u8; 512];
    let n = runs.len();
    for (i, r) in runs.iter().enumerate() {
        page[i * 4..i * 4 + 4].copy_from_slice(&fc_of(r.0).to_le_bytes());
    }
    page[n * 4..n * 4 + 4].copy_from_slice(&fc_of(end_cp).to_le_bytes());
    let rgb_at = (n + 1) * 4;
    let mut data_at = rgb_at + n + 1;
    for (i, r) in runs.iter().enumerate() {
        if r.1.is_empty() {
            page[rgb_at + i] = 0;
            continue;
        }
        // Offsets are half-bytes; align the data to even offsets.
        data_at += data_at & 1;
        let half = (data_at / 2) as u8;
        page[rgb_at + i] = half;
        page[data_at] = r.1.len() as u8;
        page[data_at + 1..data_at + 1 + r.1.len()].copy_from_slice(&r.1);
        data_at += 1 + r.1.len();
    }
    page[511] = n as u8;
    page
}

/// A PapxFkp page mapping paragraph-mark FCs (as "FC after the mark") to PAPX data
/// (istd + grpprl).
fn papx_fkp(entries: &[(u32, u16, Vec<u8>)]) -> Vec<u8> {
    let mut page = vec![0u8; 512];
    let n = entries.len();
    for (i, e) in entries.iter().enumerate() {
        page[i * 4..i * 4 + 4].copy_from_slice(&fc_of(e.0).to_le_bytes());
    }
    page[n * 4..n * 4 + 4].copy_from_slice(&fc_of(0x3FF0_0000).to_le_bytes()); // far end
    let rgbx_at = (n + 1) * 4;
    let mut data_at = rgbx_at + n * 13;
    for (i, e) in entries.iter().enumerate() {
        data_at += data_at & 1;
        let half = (data_at / 2) as u8;
        page[rgbx_at + i * 13] = half;
        let grp = [e.1.to_le_bytes().as_slice(), e.2.as_slice()].concat();
        let cb = (grp.len() + 1) as u8 / 2 * 2 / 2; // 2×cb-1 = grp.len()+even
        let cb = ((grp.len() + 2) / 2) as u8;
        page[data_at] = cb;
        page[data_at + 1..data_at + 1 + grp.len()].copy_from_slice(&grp);
        data_at += 1 + grp.len() + ((1 + grp.len()) & 1);
    }
    page[511] = n as u8;
    page
}

/// A document with one uncompressed text piece, a stylesheet, a font table and formatting
/// pages, assembled the way Word lays out the streams.
struct RichSpec {
    text: &'static str,
    /// Styles in istd order, (stk, base, name, papx, chpx).
    styles: Vec<(u8, u16, &'static str, Vec<u8>, Vec<u8>)>,
    fonts: Vec<&'static str>,
    /// Character runs (start CP, grpprl); text length ends the last.
    chpx_runs: Vec<(u32, Vec<u8>)>,
    /// Paragraph entries (mark CP, istd, grpprl).
    papx_entries: Vec<(u32, u16, Vec<u8>)>,
}

fn rich_doc(spec: &RichSpec) -> Vec<u8> {
    let text: String = spec.text.into();
    let ccp = text.chars().count() as u32;
    let _ = &text;
    let plc = plcpcd(&[Piece { compressed: false, text: text.clone() }], TEXT_FC);
    let mut clx = vec![0x02u8];
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);

    // WordDocument: FIB | padding | text | FKP pages (512-aligned).
    let chpx_page_at = ((TEXT_FC as usize + text.len() * 2 + 511) / 512) * 512;
    let papx_page_at = chpx_page_at + 512;
    let mut word = vec![0u8; TEXT_FC as usize];
    word.extend_from_slice(&piece_bytes(&Piece { compressed: false, text: text.clone() }));
    word.resize(chpx_page_at, 0);
    word.extend_from_slice(&chpx_fkp(&spec.chpx_runs, ccp));
    word.extend_from_slice(&papx_fkp(&spec.papx_entries));

    // Table: STSH | FFN | Clx | bin tables.
    let styles: Vec<Vec<u8>> = spec.styles.iter().map(|s| std_bytes(s.0, s.1, s.2, &s.3, &s.4)).collect();
    let stsh = stsh_bytes(&styles);
    let ffn = ffn_bytes(&spec.fonts);
    let clx_at = 0x40;
    let bins_at = clx_at + clx.len();
    let plcf_chpx =
        [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((chpx_page_at / 512) as u32).to_le_bytes().as_slice()].concat();
    let plcf_papx =
        [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((papx_page_at / 512) as u32).to_le_bytes().as_slice()].concat();
    let mut table = vec![0u8; clx_at];
    table.extend_from_slice(&clx);
    let chpx_bins_at = table.len();
    table.extend_from_slice(&plcf_chpx);
    let papx_bins_at = table.len();
    table.extend_from_slice(&plcf_papx);
    table.extend(std::iter::repeat(0).take(stsh.len() + ffn.len()));
    let stsh_at = table.len() - stsh.len() - ffn.len();
    let ffn_at = stsh_at + stsh.len();
    table[stsh_at..stsh_at + stsh.len()].copy_from_slice(&stsh);
    table[ffn_at..ffn_at + ffn.len()].copy_from_slice(&ffn);
    let _ = bins_at;

    let spec_fib = FibSpec {
        ccp_text: ccp,
        pairs: vec![
            (33, clx_at as u32, clx.len() as u32),
            (1, stsh_at as u32, stsh.len() as u32),
            (15, ffn_at as u32, ffn.len() as u32),
            (12, chpx_bins_at as u32, plcf_chpx.len() as u32),
            (13, papx_bins_at as u32, plcf_papx.len() as u32),
        ],
        ..Default::default()
    };
    let fib = fib_bytes(&spec_fib);
    word[..fib.len()].copy_from_slice(&fib);
    cfb_file(&[("WordDocument", &word), ("0Table", &table)])
}

fn para0(doc: &wordcraft_doc::Document) -> wordcraft_doc::Paragraph {
    doc.body.first().and_then(|b| b.as_para().map(|p| p.clone())).unwrap_or_default()
}

#[test]
fn character_formatting_applied() {
    let spec = RichSpec {
        text: "plain bold red\r",
        styles: vec![(1, 0, "Normal", Vec::new(), Vec::new())],
        fonts: vec!["Times New Roman", "Arial"],
        chpx_runs: vec![
            (0, Vec::new()),
            (6, grpprl(&[(0x0835, &[0x01])])),                     // bold
            (11, grpprl(&[(0x0835, &[0x01]), (0x2A42, &[0x06])])), // bold + red (ico 6)
        ],
        papx_entries: vec![(16, 0, Vec::new())],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let p = para0(&doc);
    assert_eq!(p.text, "plain bold red");
    // runs: plain(0..6) bold(6..11) bold+red(11..16)
    assert!(p.runs.get(1).is_some_and(|r| r.props.bold == Some(true)), "run 1 bold: {:?}", p.runs);
    assert!(p.runs.get(2).is_some_and(|r| r.props.bold == Some(true)), "run 2 bold+red: {:?}", p.runs);
    assert!(p.runs.get(2).is_some_and(|r| matches!(r.props.color, Some(wordcraft_doc::props::TextColor::Rgb(_)))), "run 2 red");
    assert!(p.runs.first().is_some_and(|r| r.props.bold.is_none()));
}

#[test]
fn font_and_size_from_style_and_direct() {
    let spec = RichSpec {
        text: "Hello\r",
        styles: vec![
            (1, 0, "Normal", Vec::new(), grpprl(&[(0x4A43, &24u16.to_le_bytes())])), // 12pt
            (2, 0, "Emphasis", Vec::new(), grpprl(&[(0x4A4F, &1u16.to_le_bytes())])), // Arial
        ],
        fonts: vec!["Times New Roman", "Arial"],
        chpx_runs: vec![(0, grpprl(&[(0x4A4F, &1u16.to_le_bytes())]))],
        papx_entries: vec![(5, 0, Vec::new())],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    assert!(doc.styles.styles.iter().any(|s| s.name == "Normal" && s.chr.size == Some(12.0)), "Normal 12pt");
    let p = para0(&doc);
    assert!(p.runs.first().is_some_and(|r| r.props.font.as_deref() == Some("Arial")), "run font: {:?}", p.runs);
}

#[test]
fn paragraph_props_and_heading_style() {
    let spec = RichSpec {
        text: "centered\rplain\r",
        styles: vec![
            (1, 0, "Normal", Vec::new(), Vec::new()),
            (1, 0, "heading 1", grpprl(&[(0x2461, &[0x01, 0x00])]), Vec::new()), // centered
        ],
        fonts: vec!["Times New Roman"],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![(8, 1, Vec::new()), (15, 0, grpprl(&[(0x845E, &720u16.to_le_bytes())]))],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let heading = doc.styles.styles.iter().find(|s| s.name == "Heading 1").expect("style");
    assert_eq!(heading.para.align, Some(wordcraft_doc::props::Align::Center));
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    assert_eq!(p0.props.style.as_deref(), Some("Heading 1"));
    assert_eq!(heading.para.outline_level, Some(0));
    let p1 = doc.body.get(1).and_then(|b| b.as_para()).expect("para");
    assert_eq!(p1.props.indent_left, Some(36.0)); // 720 twips = 36pt
    assert_eq!(p1.props.style.as_deref(), Some("Normal"));
}

/// Local corpus (gitignored `plan/word/fixtures`): real files must open with text and styles.
#[test]
fn corpus_files_open() {
    let Some(dir) = std::env::var_os("WORDCRAFT_DOCBIN_FIXTURES") else { return };
    let mut checked = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("doc")) {
                let bytes = std::fs::read(&p).expect("read");
                let doc = super::read(&bytes).expect("opens");
                assert!(!doc.styles.styles.is_empty(), "{}: no styles", p.display());
                let text = doc.plain_text(wordcraft_doc::StoryRef::Body);
                assert!(text.chars().count() > 3, "{}: no text", p.display());
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no .doc files found in {}", dir.display());
}
