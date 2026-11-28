//! Round 22 permanent regressions adapted from the confirmation reviewer fixtures. Independent builders:
//! a zip-crate builder for ordinary packages, and a hand-written STORED raw builder for
//! directory-selection decoys. Public `validate_bytes` only.

use std::io::{Cursor, Write};
use wordcraft_validate::{Report, Verdict, validate_bytes};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const OD: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const CT_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const CT_FN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
const JOINS: [&str; 3] = ["REF-NOTE", "ID-COMMENT-REF", "REL-REF-MISSING"];

// ------------------------------------------------------------------ zip-crate builder

#[derive(Clone)]
struct Pkg {
    parts: Vec<(Vec<u8>, Vec<u8>)>,
}
impl Pkg {
    fn new() -> Pkg {
        let mut p = Pkg { parts: Vec::new() };
        p.put("[Content_Types].xml", ct(&[("/word/document.xml", CT_MAIN)]));
        p.put("_rels/.rels", rels(&[("rId1", OD, "word/document.xml", false)]));
        p.put("word/document.xml", doc("<w:p><w:r><w:t>x</w:t></w:r></w:p>"));
        p.put("word/_rels/document.xml.rels", rels(&[]));
        p
    }
    fn put(&mut self, name: impl AsRef<[u8]>, bytes: impl Into<Vec<u8>>) {
        let (n, b) = (name.as_ref().to_vec(), bytes.into());
        if let Some(e) = self.parts.iter_mut().find(|e| e.0 == n) {
            e.1 = b;
        } else {
            self.parts.push((n, b));
        }
    }
    fn reversed(&self) -> Pkg {
        let mut p = self.clone();
        p.parts.reverse();
        p
    }
    /// Build with the zip crate (deflate); names are written as UTF-8 strings via placeholders.
    fn build(&self) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, b) in &self.parts {
            w.start_file(String::from_utf8(n.clone()).unwrap(), SimpleFileOptions::default().compression_method(CompressionMethod::Deflated))
                .unwrap();
            w.write_all(b).unwrap();
        }
        w.finish().unwrap().into_inner()
    }
}
fn ct(overrides: &[(&str, &str)]) -> String {
    let mut s = format!(r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="{CT_NS}">"#);
    s += r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#;
    for (p, c) in overrides {
        s += &format!(r#"<Override PartName="{p}" ContentType="{c}"/>"#);
    }
    s + "</Types>"
}
fn rels(items: &[(&str, &str, &str, bool)]) -> String {
    let mut s = format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{PKG_RELS}">"#);
    for (id, ty, t, ext) in items {
        let mode = if *ext { r#" TargetMode="External""# } else { "" };
        s += &format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"{mode}/>"#);
    }
    s + "</Relationships>"
}
fn doc(body: &str) -> String {
    format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#)
}
fn footnotes(ids: &[&str]) -> String {
    let mut s = format!(r#"<?xml version="1.0"?><w:footnotes xmlns:w="{W}">"#);
    for id in ids {
        s += &format!(r#"<w:footnote w:id="{id}"><w:p/></w:footnote>"#);
    }
    s + "</w:footnotes>"
}
fn fnref(id: &str) -> String {
    format!(r#"<w:p><w:r><w:footnoteReference w:id="{id}"/></w:r></w:p>"#)
}
fn rt(k: &str) -> String {
    format!("{R}/{k}")
}
fn with_footnotes(def_ids: &[&str], body: &str) -> Pkg {
    let mut p = Pkg::new();
    p.put("[Content_Types].xml", ct(&[("/word/document.xml", CT_MAIN), ("/word/footnotes.xml", CT_FN)]));
    p.put("word/_rels/document.xml.rels", rels(&[("rIdF", &rt("footnotes"), "footnotes.xml", false)]));
    p.put("word/footnotes.xml", footnotes(def_ids));
    p.put("word/document.xml", doc(body));
    p
}

/// Corrupt the deflate stream of `name` (first 4 data bytes XOR 0x5A), metadata intact.
fn corrupt(zip: &[u8], name: &str) -> Vec<u8> {
    let mut out = zip.to_vec();
    let mut i = 0;
    while i + 30 < out.len() {
        if out[i..i + 4] == [0x50, 0x4B, 0x03, 0x04] {
            let nlen = u16::from_le_bytes([out[i + 26], out[i + 27]]) as usize;
            let elen = u16::from_le_bytes([out[i + 28], out[i + 29]]) as usize;
            if &out[i + 30..i + 30 + nlen] == name.as_bytes() {
                let d = i + 30 + nlen + elen;
                for k in 0..4 {
                    out[d + k] ^= 0x5A;
                }
                return out;
            }
        }
        i += 1;
    }
    panic!("entry {name} not found")
}
// ------------------------------------------------------------------ raw STORED builder

#[derive(Clone)]
struct Rec {
    name: Vec<u8>,
    flags: u16,
    body: Vec<u8>,
}
fn rec(name: &[u8], body: impl Into<Vec<u8>>) -> Rec {
    // Flag 0x800 when the name is non-ASCII valid UTF-8; raw CP437 high bytes stay unflagged.
    let utf8 = name.iter().any(|b| *b >= 0x80) && std::str::from_utf8(name).is_ok();
    Rec { name: name.to_vec(), flags: if utf8 { 0x800 } else { 0 }, body: body.into() }
}
fn crc32(b: &[u8]) -> u32 {
    let mut c = !0u32;
    for &x in b {
        c ^= u32::from(x);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}
fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn le64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}
/// Local headers + data from `base`; returns the absolute offset of each local header.
fn write_locals(out: &mut Vec<u8>, recs: &[Rec]) -> Vec<u64> {
    let mut offs = Vec::new();
    for r in recs {
        offs.push(out.len() as u64);
        le32(out, 0x0403_4B50);
        le16(out, 20);
        le16(out, r.flags);
        le16(out, 0);
        le16(out, 0);
        le16(out, 0x21);
        le32(out, crc32(&r.body));
        le32(out, r.body.len() as u32);
        le32(out, r.body.len() as u32);
        le16(out, r.name.len() as u16);
        le16(out, 0);
        out.extend_from_slice(&r.name);
        out.extend_from_slice(&r.body);
    }
    offs
}
/// Central records for `idx` (indices into recs), local offsets reduced by `delta`.
fn write_cd(out: &mut Vec<u8>, recs: &[Rec], offs: &[u64], idx: &[usize], delta: u64) {
    for &i in idx {
        let r = &recs[i];
        le32(out, 0x0201_4B50);
        le16(out, 45);
        le16(out, 20);
        le16(out, r.flags);
        le16(out, 0);
        le16(out, 0);
        le16(out, 0x21);
        le32(out, crc32(&r.body));
        le32(out, r.body.len() as u32);
        le32(out, r.body.len() as u32);
        le16(out, r.name.len() as u16);
        le16(out, 0);
        le16(out, 0);
        le16(out, 0);
        le16(out, 0);
        le32(out, 0);
        le32(out, (offs[i] - delta) as u32);
        out.extend_from_slice(&r.name);
    }
}
fn write_eocd64(out: &mut Vec<u8>, count: u64, size: u64, off: u64) {
    le32(out, 0x0606_4B50);
    le64(out, 44);
    le16(out, 45);
    le16(out, 45);
    le32(out, 0);
    le32(out, 0);
    le64(out, count);
    le64(out, count);
    le64(out, size);
    le64(out, off);
}
fn write_locator(out: &mut Vec<u8>, eocd64_at: u64) {
    le32(out, 0x0706_4B50);
    le32(out, 0);
    le64(out, eocd64_at);
    le32(out, 1);
}
fn write_eocd(out: &mut Vec<u8>, count: u16, size: u32, off: u32) {
    le32(out, 0x0605_4B50);
    le16(out, 0);
    le16(out, 0);
    le16(out, count);
    le16(out, count);
    le32(out, size);
    le32(out, off);
    le16(out, 0);
}
fn all(n: usize) -> Vec<usize> {
    (0..n).collect()
}
/// Ordinary raw archive.
fn plain(recs: &[Rec]) -> Vec<u8> {
    let mut out = Vec::new();
    let offs = write_locals(&mut out, recs);
    let cd = out.len() as u64;
    write_cd(&mut out, recs, &offs, &all(recs.len()), 0);
    let size = out.len() as u64 - cd;
    write_eocd(&mut out, recs.len() as u16, size as u32, cd as u32);
    out
}
/// My F1-class extension: sentinel count/offset (both parsers take the ZIP64 path). The locator
/// points at a decoy EOCD64 that zip 3.0.0 rejects (it is not adjacent to the locator, so its
/// record size is "inconsistent"); the reader then scans forward to the real EOCD64 and derives
/// archive_offset = real - decoy. The validator reads the locator's target verbatim.
fn eocd64_decoy(recs: &[Rec], decoy: &[usize]) -> Vec<u8> {
    const DELTA: u64 = 56; // size of one EOCD64 record = archive offset the reader will derive
    let mut out = vec![0u8; DELTA as usize];
    let offs = write_locals(&mut out, recs);
    let real = out.len() as u64;
    write_cd(&mut out, recs, &offs, &all(recs.len()), DELTA);
    let rsize = out.len() as u64 - real;
    let dcd = out.len() as u64;
    write_cd(&mut out, recs, &offs, decoy, 0);
    let dsize = out.len() as u64 - dcd;
    let x = out.len() as u64;
    write_eocd64(&mut out, decoy.len() as u64, dsize, dcd);
    write_eocd64(&mut out, recs.len() as u64, rsize, real - DELTA);
    write_locator(&mut out, x);
    write_eocd(&mut out, 0xFFFF, 0xFFFF_FFFF, 0xFFFF_FFFF);
    out
}
// ------------------------------------------------------------------ reporting helpers

fn assert_clean(r: &Report) {
    assert_eq!(r.verdict, Verdict::Pass);
    assert!(r.findings.is_empty());
    assert!(r.coverage_notes.is_empty());
    assert!(r.checks_run.contains(&"PKG-DUP-ENTRY"));
}
fn rules(r: &Report) -> Vec<String> {
    r.findings.iter().map(|f| format!("{}@{}", f.rule, f.part.as_deref().unwrap_or("-"))).collect()
}
fn has(r: &Report, rule: &str) -> bool {
    r.findings.iter().any(|f| f.rule == rule)
}
fn count(r: &Report, rule: &str) -> usize {
    r.findings.iter().filter(|f| f.rule == rule).count()
}
fn joins(r: &Report) -> Vec<&'static str> {
    JOINS.iter().copied().filter(|j| r.checks_run.contains(j)).collect()
}
fn run(name: &str, z: &[u8]) -> Report {
    let r = validate_bytes(z);
    println!("PROBE {name} | {:?} | {:?} | joins_run={:?} | dup_ran={}", r.verdict, rules(&r), joins(&r), r.checks_run.contains(&"PKG-DUP-ENTRY"));
    for n in &r.coverage_notes {
        println!("    note: {}", n.chars().take(170).collect::<String>());
    }
    r
}
fn h6_ct(extra: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><Types xmlns="{CT_NS}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{extra}</Types>"#
    )
}
/// H6 (round 1): footnotes twin `word/fü.xml` (UTF-8 flag) vs `word/f\x81.xml` (CP437 ü).
fn h6_recs(defining_first: bool) -> Vec<Rec> {
    let (a, b) = if defining_first { (footnotes(&["1"]), footnotes(&[])) } else { (footnotes(&[]), footnotes(&["1"])) };
    vec![
        rec(
            b"[Content_Types].xml",
            h6_ct(&format!(
                r#"<Override PartName="/word/document.xml" ContentType="{CT_MAIN}"/><Override PartName="/word/fü.xml" ContentType="{CT_FN}"/>"#
            )),
        ),
        rec(b"_rels/.rels", rels(&[("rId1", OD, "word/document.xml", false)])),
        rec(b"word/document.xml", doc(&fnref("1"))),
        rec(b"word/_rels/document.xml.rels", rels(&[("rF", &rt("footnotes"), "fü.xml", false)])),
        rec("word/fü.xml".as_bytes(), a),
        rec(b"word/f\x81.xml", b),
    ]
}
/// H6b (round 1): main-document twin, one copy malformed.
fn h6b_recs(malformed_first: bool) -> Vec<Rec> {
    let good = doc("<w:p/>").into_bytes();
    let bad = b"<w:document xmlns:w=\"x\"><unclosed>".to_vec();
    let (a, b) = if malformed_first { (bad, good) } else { (good, bad) };
    vec![
        rec(b"[Content_Types].xml", h6_ct(&format!(r#"<Override PartName="/word/dü.xml" ContentType="{CT_MAIN}"/>"#))),
        rec(b"_rels/.rels", rels(&[("rId1", OD, "word/dü.xml", false)])),
        rec("word/dü.xml".as_bytes(), a),
        rec(b"word/d\x81.xml", b),
    ]
}
/// A clean package (body refs footnote 1, defined) — 5 records.
fn clean_recs(rev: bool) -> Vec<Rec> {
    let mut v = vec![
        rec(b"[Content_Types].xml", ct(&[("/word/document.xml", CT_MAIN), ("/word/footnotes.xml", CT_FN)])),
        rec(b"_rels/.rels", rels(&[("rId1", OD, "word/document.xml", false)])),
        rec(b"word/document.xml", doc(&fnref("1"))),
        rec(b"word/_rels/document.xml.rels", rels(&[("rIdF", &rt("footnotes"), "footnotes.xml", false)])),
        rec(b"word/footnotes.xml", footnotes(&["1"])),
    ];
    if rev {
        v.reverse();
    }
    v
}

fn assert_disputed(r: &Report) {
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty(), "{:?}", rules(r));
    assert!(!r.checks_run.contains(&"PKG-DUP-ENTRY"));
    assert!(!r.checks_run.contains(&"REF-NOTE"));
}

fn decoy_case(shape: char, reverse: bool) {
    let (recs, indices) = match shape {
        'a' => (h6b_recs(!reverse), vec![0, 1, 2]),
        'b' => {
            let recs = clean_recs(reverse);
            let mut indices = all(recs.len());
            indices.push(recs.iter().position(|r| r.name == b"word/footnotes.xml").unwrap());
            (recs, indices)
        }
        'c' => (h6_recs(!reverse), vec![0, 1, 2, 3, 4]),
        _ => panic!("unknown fixture"),
    };
    let r = run(&format!("2b-{shape}-reverse={reverse}"), &eocd64_decoy(&recs, &indices));
    assert_disputed(&r);
}

#[test]
fn hidden_main_forward() {
    decoy_case('a', false);
}
#[test]
fn hidden_main_reverse() {
    decoy_case('a', true);
}
#[test]
fn manufactured_duplicate_forward() {
    decoy_case('b', false);
}
#[test]
fn manufactured_duplicate_reverse() {
    decoy_case('b', true);
}
#[test]
fn hidden_notes_forward() {
    decoy_case('c', false);
}
#[test]
fn hidden_notes_reverse() {
    decoy_case('c', true);
}

// Parser EOCD ends exactly at EOF; the reader chooses the later EOCD inside its comment,
// whose own comment ends one byte before EOF. Same decoded names defeat a names-only guard.
fn nested_directory(recs: &[Rec], decoy: &[usize]) -> Vec<u8> {
    let mut out = Vec::new();
    let offs = write_locals(&mut out, recs);
    let decoy_at = out.len() as u32;
    write_cd(&mut out, recs, &offs, decoy, 0);
    let decoy_size = out.len() as u32 - decoy_at;
    let end = out.len();
    write_eocd(&mut out, decoy.len() as u16, decoy_size, decoy_at);
    let real_at = out.len() as u32;
    write_cd(&mut out, recs, &offs, &all(recs.len()), 0);
    let real_size = out.len() as u32 - real_at;
    write_eocd(&mut out, recs.len() as u16, real_size, real_at);
    out.push(b'X');
    let comment_len = (out.len() - end - 22) as u16;
    out[end + 20..end + 22].copy_from_slice(&comment_len.to_le_bytes());
    out
}

fn same_name_recs() -> Vec<Rec> {
    let mut recs = clean_recs(false);
    recs.push(recs.last().unwrap().clone());
    recs
}

#[test]
fn same_name_nested_decoy() {
    let recs = same_name_recs();
    let z = nested_directory(&recs, &[0, 1, 2, 3, 4]);
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.len(), 5);
    assert_eq!(reader.offset(), 0);
    let r = run("N-1 same names", &z);
    assert_disputed(&r);
    assert!(r.coverage_notes.iter().any(|n| n.contains("disagree")));
    assert!(!r.coverage_notes.iter().any(|n| n.contains("could not be parsed")));
}

#[test]
fn same_name_no_decoy_control() {
    let r = run("N-1 no-decoy control", &plain(&same_name_recs()));
    assert_eq!(r.verdict, Verdict::Fail);
    assert_eq!(count(&r, "PKG-DUP-ENTRY"), 1);
    assert_eq!(r.findings.len(), 1);
}

#[test]
fn layout_comment() {
    let mut z = plain(&clean_recs(false));
    let end = z.len() - 22;
    let comment = b"ordinary producer comment";
    z[end + 20..end + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    z.extend(comment);
    let r = run("layout ordinary comment", &z);
    assert_eq!(r.verdict, Verdict::Pass);
    assert!(r.coverage_notes.is_empty());
}

#[test]
fn layout_trailing() {
    let mut z = plain(&clean_recs(false));
    z.extend(b"trailing data");
    let r = run("layout trailing", &z);
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty());
    assert!(r.coverage_notes.iter().any(|n| n.contains("could not be parsed")));
}

#[test]
fn layout_prepended() {
    let mut z = b"prepended stub".to_vec();
    z.extend(plain(&clean_recs(false)));
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.offset(), 14);
    let r = run("layout prepended", &z);
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty());
    assert!(r.coverage_notes.iter().any(|n| n.contains("could not be parsed")));
}

#[test]
fn eocd64_coordinate_witness() {
    let recs = h6_recs(true);
    let z = eocd64_decoy(&recs, &[0, 1, 2, 3, 4]);
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    let mut locals = vec![0u8; 56];
    write_locals(&mut locals, &recs);
    let real_at = locals.len() as u64;
    let mut cd = Vec::new();
    write_cd(&mut cd, &recs, &vec![56; recs.len()], &all(recs.len()), 56);
    let decoy_at = real_at + cd.len() as u64;
    println!(
        "WITNESS reader_start={} reader_offset={} reader_len={} real_at={real_at} real_relative={} parser_decoy_at={decoy_at}",
        reader.central_directory_start(),
        reader.offset(),
        reader.len(),
        real_at - reader.offset()
    );
    assert_eq!(reader.central_directory_start(), 1959);
    assert_eq!(reader.offset(), 56);
    assert_eq!(reader.central_directory_start(), real_at);
    assert_eq!(reader.len(), 5);
    assert!(decoy_at > real_at);
}

// Instance 4: the latest EOCD finds a CDFH signature but that central header is truncated.
// get_metadata retries the earlier, honest EOCD after read_central_header fails.
#[test]
fn reader_retry_earlier_eocd() {
    let mut z = plain(&clean_recs(false));
    let honest = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap().central_directory_start();
    let broken = z.len() as u32;
    z.extend(0x0201_4b50u32.to_le_bytes());
    write_eocd(&mut z, 1, 4, broken);
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.central_directory_start(), honest);
    assert_eq!(reader.len(), 5);
    let r = run("instance 4 retry earlier EOCD", &z);
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty());
}

// Instance 5: the advertised zip32 offset points four bytes before the real CDFH.
// The reader scans forward and derives offset=4; local offsets are reduced by four.
#[test]
fn reader_zip32_forward_scan() {
    let recs = clean_recs(false);
    let mut z = vec![0u8; 4];
    let offsets = write_locals(&mut z, &recs);
    let real_at = z.len() as u32;
    write_cd(&mut z, &recs, &offsets, &all(recs.len()), 4);
    let size = z.len() as u32 - real_at;
    write_eocd(&mut z, recs.len() as u16, size, real_at - 4);
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.central_directory_start(), u64::from(real_at));
    assert_eq!(reader.offset(), 4);
    let r = run("instance 5 zip32 forward", &z);
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty());
}

fn unselected_main(capped: bool, reverse: bool) {
    let p = with_footnotes(&["1"], &fnref("5"));
    let ctl = run("3x readable control", &p.build());
    assert_eq!(ctl.verdict, Verdict::Fail);
    assert!(has(&ctl, "REF-NOTE"));
    let mut q = p.clone();
    if capped {
        let mut items: Vec<_> = (0..20_000).map(|i| (format!("x{i}"), rt("hyperlink"), format!("https://e.invalid/{i}"), true)).collect();
        items.push(("rId1".into(), OD.into(), "word/document.xml".into(), false));
        let refs: Vec<_> = items.iter().map(|(a, b, c, d)| (a.as_str(), b.as_str(), c.as_str(), *d)).collect();
        q.put("_rels/.rels", rels(&refs));
    }
    if reverse {
        q = q.reversed();
    }
    let z = if capped { q.build() } else { corrupt(&q.build(), "_rels/.rels") };
    let r = run(&format!("3x capped={capped} reverse={reverse}"), &z);
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty());
    assert!(!r.checks_run.contains(&"REF-NOTE"));
    assert!(!r.checks_run.contains(&"ID-COMMENT-REF"));
    // The per-part r:id join is complete on this fixture; preserve its recording.
    assert!(r.checks_run.contains(&"REL-REF-MISSING"));
}

#[test]
fn unselected_corrupt_forward() {
    unselected_main(false, false);
}
#[test]
fn unselected_corrupt_reverse() {
    unselected_main(false, true);
}
#[test]
fn unselected_capped_forward() {
    unselected_main(true, false);
}
#[test]
fn unselected_capped_reverse() {
    unselected_main(true, true);
}

#[test]
fn disputed_names_note_is_accurate() {
    let recs = clean_recs(false);
    let mut z = nested_directory(&recs, &[0, 1, 2, 3, 4]);
    let at = z.windows(b"word/footnotes.xml".len()).position(|w| w == b"word/footnotes.xml").unwrap();
    // The first match is a local name; find the copy in the first (decoy) directory.
    let local_end = {
        let mut tmp = Vec::new();
        write_locals(&mut tmp, &recs);
        tmp.len()
    };
    assert!(at < local_end);
    let at = local_end + z[local_end..].windows(b"word/footnotes.xml".len()).position(|w| w == b"word/footnotes.xml").unwrap();
    z[at..at + b"word/footnotes.xml".len()].copy_from_slice(b"word/footnotez.xml");
    let r = run("F21-1 differing names", &z);
    assert_disputed(&r);
    assert!(r.coverage_notes.iter().any(|n| n.contains("decoded names disagree")));
    assert!(!r.coverage_notes.iter().any(|n| n.contains("could not be parsed")));
}

// A complete same-name directory can also be parsed successfully by the bounded parser
// while read_central_header rejects the latest EOCD's split-disk view and retries earlier.
#[test]
fn reader_retry_with_parsed_decoy() {
    let recs = clean_recs(false);
    let mut z = plain(&recs);
    let honest = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap().central_directory_start();
    let mut tmp = Vec::new();
    let offsets = write_locals(&mut tmp, &recs);
    let decoy_at = z.len() as u32;
    write_cd(&mut z, &recs, &offsets, &all(recs.len()), 0);
    let size = z.len() as u32 - decoy_at;
    let end = z.len();
    write_eocd(&mut z, recs.len() as u16, size, decoy_at);
    z[end + 4..end + 6].copy_from_slice(&1u16.to_le_bytes());
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.central_directory_start(), honest);
    assert_eq!(reader.offset(), 0);
    assert_eq!(reader.len(), 5);
    assert_disputed(&run("instance 4 parsed decoy retry", &z));
}

// Equal starts need not imply equal raw counts: the rejected later footer can list
// one more identical-name record than the earlier footer actually used by the reader.
#[test]
fn reader_retry_same_start_different_raw_count() {
    let recs = clean_recs(false);
    let mut z = Vec::new();
    let offsets = write_locals(&mut z, &recs);
    let start = z.len() as u32;
    let mut indices = all(recs.len());
    indices.push(4);
    write_cd(&mut z, &recs, &offsets, &indices, 0);
    let size = z.len() as u32 - start;
    write_eocd(&mut z, recs.len() as u16, size, start);
    let rejected = z.len();
    write_eocd(&mut z, indices.len() as u16, size, start);
    z[rejected + 4..rejected + 6].copy_from_slice(&1u16.to_le_bytes());
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.central_directory_start(), u64::from(start));
    assert_eq!(reader.offset(), 0);
    assert_eq!(reader.len(), 5);
    println!(
        "SAME-START parser_count={} reader_raw_count={} start={start} offset={} len={} names={:?}",
        indices.len(),
        recs.len(),
        reader.offset(),
        reader.len(),
        (0..reader.len()).map(|i| reader.name_for_index(i).unwrap()).collect::<Vec<_>>()
    );
    assert_disputed(&run("instance 4 same start, different raw count", &z));
}

// Both EOCDs are single-disk and parse cleanly. The exact-end parser selects the
// earlier footer (six raw records), while the reader accepts the later footer
// before trailing X (five raw records). All specified anchor observables agree.
#[test]
fn same_start_nested_count_disagreement() {
    let recs = clean_recs(false);
    let mut z = Vec::new();
    let offsets = write_locals(&mut z, &recs);
    let start = z.len() as u32;
    let mut indices = all(recs.len());
    indices.push(4);
    write_cd(&mut z, &recs, &offsets, &indices, 0);
    let size = z.len() as u32 - start;
    let parser_eocd = z.len();
    write_eocd(&mut z, indices.len() as u16, size, start);
    z[parser_eocd + 20..parser_eocd + 22].copy_from_slice(&23u16.to_le_bytes());
    write_eocd(&mut z, recs.len() as u16, size, start);
    z.push(b'X');
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.central_directory_start(), u64::from(start));
    assert_eq!(reader.offset(), 0);
    assert_eq!(reader.len(), 5);
    println!(
        "NESTED-SAME-START parser_count=6 reader_raw_count=5 parser_eocd={parser_eocd} reader_eocd={} start={start} offset=0 len=5 names={:?}",
        parser_eocd + 22,
        (0..reader.len()).map(|i| reader.name_for_index(i).unwrap()).collect::<Vec<_>>()
    );
    assert_disputed(&run("same start nested EOCD count disagreement", &z));
}

// N-3: the real EOCD's legal 22-byte comment ends in a complete empty EOCD.
// The two finder-acceptable tuples disagree; absence under the embedded view is unproved.
#[test]
fn embedded_empty_eocd_comment_is_disputed() {
    let mut z = plain(&clean_recs(false));
    let end = z.len() - 22;
    z[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    write_eocd(&mut z, 0, 0, 0);
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.len(), 0);
    let r = run("N-3 embedded empty EOCD comment", &z);
    assert_disputed(&r);
    assert!(r.coverage_notes.iter().any(|n| n.contains("disagree")));
    assert!(!r.coverage_notes.iter().any(|n| n.contains("could not be parsed")));
}

// Records beyond the selected count are inert unless another acceptable footer selects them.
#[test]
fn records_beyond_declared_count_are_inert() {
    let mut z = plain(&same_name_recs());
    let end = z.len() - 22;
    z[end + 8..end + 12].copy_from_slice(&[5, 0, 5, 0]);
    let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(reader.len(), 5);
    assert_clean(&run("count excludes extra record", &z));
}

#[test]
fn directory_size_does_not_bound_walk() {
    for delta in [-1i32, 1] {
        let mut z = plain(&clean_recs(false));
        let end = z.len() - 22;
        let size = u32::from_le_bytes(z[end + 12..end + 16].try_into().unwrap());
        z[end + 12..end + 16].copy_from_slice(&size.checked_add_signed(delta).unwrap().to_le_bytes());
        assert_eq!(zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap().len(), 5);
        assert_clean(&run(&format!("ignored directory size delta={delta}"), &z));
    }
}

#[test]
fn directory_gap_before_footer_is_inert() {
    let mut z = plain(&clean_recs(false));
    let end = z.len() - 22;
    z.splice(end..end, *b"gap");
    assert_eq!(zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap().len(), 5);
    assert_clean(&run("inert gap before footer", &z));
}

#[test]
fn zip32_disk_count_must_agree_with_total() {
    let mut z = plain(&same_name_recs());
    let end = z.len() - 22;
    z[end + 8..end + 10].copy_from_slice(&5u16.to_le_bytes());
    assert_eq!(zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap().len(), 5);
    assert_disputed(&run("extent on-disk count differs", &z));
}

// The unwalked suffix remains unknown. Preserve proven prefix duplicates even when
// the selected count exceeds the existing 20,000-record budget.
#[test]
fn capped_extent_retains_prefix_duplicate_policy() {
    let recs = clean_recs(false);
    let mut z = Vec::new();
    let offsets = write_locals(&mut z, &recs);
    let start = z.len() as u32;
    let mut indices = all(recs.len());
    indices.resize(20_001, 4);
    write_cd(&mut z, &recs, &offsets, &indices, 0);
    let size = z.len() as u32 - start;
    write_eocd(&mut z, indices.len() as u16, size + 1, start);
    let r = run("extent entry cap prefix", &z);
    assert_eq!(r.verdict, Verdict::Fail);
    assert_eq!(r.findings.len(), 1);
    assert_eq!(count(&r, "PKG-DUP-ENTRY"), 1);
    assert!(r.checks_run.contains(&"PKG-DUP-ENTRY"));
    assert!(r.coverage_notes.iter().any(|n| n.contains("20001 records, more than the 20000 examined")));
}

// Two footer candidates with identical start/count/path differ only in ignored size.
#[test]
fn size_only_candidate_difference_is_same_selection() {
    let mut z = plain(&clean_recs(false));
    let end = z.len() - 22;
    let start = u32::from_le_bytes(z[end + 16..end + 20].try_into().unwrap());
    z[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    write_eocd(&mut z, 5, u32::MAX, start);
    assert_eq!(zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap().len(), 5);
    assert_clean(&run("same selection differing size", &z));
}

// The ZIP64 directory walk shares the ZIP32 count-driven policy. EOCD64 itself
// still has the mandatory record-size adjacency to its locator.
#[test]
fn zip64_directory_size_and_gap_are_inert() {
    for size in [0, 1, u64::MAX] {
        let recs = clean_recs(false);
        let mut z = Vec::new();
        let offsets = write_locals(&mut z, &recs);
        let start = z.len() as u64;
        write_cd(&mut z, &recs, &offsets, &all(recs.len()), 0);
        z.extend_from_slice(b"gap before EOCD64");
        let footer = z.len() as u64;
        write_eocd64(&mut z, 5, size, start);
        write_locator(&mut z, footer);
        write_eocd(&mut z, u16::MAX, u32::MAX, u32::MAX);
        let reader = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
        assert_eq!(reader.len(), 5);
        assert_eq!(reader.offset(), 0);
        assert_eq!(reader.central_directory_start(), start);
        assert_clean(&run(&format!("ZIP64 ignored size={size} and gap"), &z));
    }
}
