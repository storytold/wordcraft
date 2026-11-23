//! Round 23 regressions adapted from the confirmation rerun-2 fixtures.
//! Independent STORED builder; all verdicts go through public `validate_bytes`.

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
/// Carbon's F1 shape: normal 16-bit count/offset, sentinel 16-bit cd_size, and a ZIP64
fn rules(r: &Report) -> Vec<String> {
    r.findings.iter().map(|f| format!("{}@{}", f.rule, f.part.as_deref().unwrap_or("-"))).collect()
}
fn has(r: &Report, rule: &str) -> bool {
    r.findings.iter().any(|f| f.rule == rule)
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

fn disputed(label: &str, r: &Report) -> Result<(), String> {
    let mut bad = Vec::new();
    if r.verdict != Verdict::Incomplete {
        bad.push(format!("verdict {:?}", r.verdict));
    }
    if !r.findings.is_empty() {
        bad.push(format!("findings {:?}", rules(r)));
    }
    for j in ["PKG-DUP-ENTRY", "REF-NOTE", "ID-COMMENT-REF"] {
        if r.checks_run.contains(&j) {
            bad.push(format!("{j} recorded"));
        }
    }
    if bad.is_empty() { Ok(()) } else { Err(format!("{label}: {bad:?}")) }
}
fn retry_out_of_tail(recs: &[Rec], far: bool, dup: bool) -> (Vec<u8>, u32, usize) {
    let mut z = Vec::new();
    let offs = write_locals(&mut z, recs);
    let start = z.len() as u32;
    write_cd(&mut z, recs, &offs, &all(recs.len()), 0);
    let size5 = z.len() as u32 - start;
    let last = &recs[recs.len() - 1];
    let name: Vec<u8> = if dup { last.name.clone() } else { b"word/extra.xml".to_vec() };
    // Extra field: one unknown TLV (0xCAFE) whose payload begins with the embedded EOCD image.
    let mut payload = Vec::new();
    write_eocd(&mut payload, recs.len() as u16, size5, start);
    payload.resize(64, 0);
    let mut extra = Vec::new();
    le16(&mut extra, 0xCAFE);
    le16(&mut extra, payload.len() as u16);
    extra.extend_from_slice(&payload);
    let comment_len: u16 = if far { 0xFFFF } else { 16 };
    let r6 = z.len();
    le32(&mut z, 0x0201_4B50);
    le16(&mut z, 45);
    le16(&mut z, 20);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le16(&mut z, 0x21);
    le32(&mut z, crc32(&last.body));
    le32(&mut z, last.body.len() as u32);
    le32(&mut z, last.body.len() as u32);
    le16(&mut z, name.len() as u16);
    le16(&mut z, extra.len() as u16);
    le16(&mut z, comment_len);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le32(&mut z, 0);
    le32(&mut z, start); // poison: local header "after" the central directory
    z.extend_from_slice(&name);
    z.extend_from_slice(&extra);
    z.extend(std::iter::repeat_n(b' ', usize::from(comment_len)));
    let size6 = z.len() as u32 - start;
    let embedded = r6 + 46 + name.len() + 4;
    write_eocd(&mut z, recs.len() as u16 + 1, size6, start);
    (z, start, embedded)
}

fn retry_case(far: bool, reverse: bool, duplicate: bool) {
    let recs = clean_recs(reverse);
    let (z, start, embedded) = retry_out_of_tail(&recs, far, duplicate);
    let tail_lo = z.len().saturating_sub(22 + 0xFFFF);
    assert_eq!(embedded < tail_lo, far);
    let a = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(a.central_directory_start(), u64::from(start));
    assert_eq!(a.offset(), 0);
    assert_eq!(a.len(), 5);
    assert_eq!(a.file_names().collect::<std::collections::HashSet<_>>(), recs.iter().map(|r| std::str::from_utf8(&r.name).unwrap()).collect());
    println!("reader start={start}, len=5, embedded={embedded}, tail_from={tail_lo}");
    let r = run(&format!("retry far={far} reverse={reverse} duplicate={duplicate}"), &z);
    disputed("retry", &r).unwrap();
    assert!(r.coverage_notes.iter().any(|n| n.contains("selection or decoded names disagree")));
}
#[test]
fn retry_far_forward() {
    retry_case(true, false, true);
}
#[test]
fn retry_far_reverse() {
    retry_case(true, true, true);
}
#[test]
fn retry_near_forward() {
    retry_case(false, false, true);
}
#[test]
fn retry_near_reverse() {
    retry_case(false, true, true);
}
#[test]
fn retry_different_name_forward() {
    retry_case(true, false, false);
}
#[test]
fn retry_different_name_reverse() {
    retry_case(true, true, false);
}
fn assert_clean(r: &Report) {
    assert_eq!(r.verdict, Verdict::Pass);
    assert!(r.findings.is_empty());
    assert!(r.coverage_notes.is_empty());
    assert!(r.checks_run.contains(&"PKG-DUP-ENTRY"));
}
#[test]
fn retry_honest_forward() {
    assert_clean(&run("honest forward", &plain(&clean_recs(false))));
}
#[test]
fn retry_honest_reverse() {
    assert_clean(&run("honest reverse", &plain(&clean_recs(true))));
}
fn stored_nested_case(nested: bool) {
    let inner = plain(&clean_recs(false));
    let body = if nested { inner.clone() } else { vec![b'z'; inner.len()] };
    let mut recs = clean_recs(false);
    recs[0] = rec(
        b"[Content_Types].xml",
        ct(&[("/word/document.xml", CT_MAIN), ("/word/footnotes.xml", CT_FN), ("/word/embeddings/pkg.bin", "application/octet-stream")]),
    );
    recs.push(rec(b"word/embeddings/pkg.bin", body));
    let z = plain(&recs);
    let a = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
    assert_eq!(a.len(), 6);
    assert_eq!(a.offset(), 0);
    assert_eq!(a.central_directory_start(), 4044);
    let r = run(&format!("stored nested={nested}"), &z);
    if nested {
        // The measured ambiguity-retirement run breaks frozen round21/22
        // outcomes. Keep this disclosed section-5 residual while it still binds.
        disputed("stored nested residual", &r).unwrap();
    } else {
        assert_clean(&r);
    }
}
#[test]
fn stored_nested_zip() {
    stored_nested_case(true);
}
#[test]
fn stored_no_signature_control() {
    stored_nested_case(false);
}
fn untyped_case(reverse: bool) {
    let mut p = with_footnotes(&["1"], &fnref("5"));
    p.put("[Content_Types].xml", ct(&[("/word/footnotes.xml", CT_FN)]));
    if reverse {
        p = p.reversed();
    }
    let control = run(&format!("untyped readable reverse={reverse}"), &p.build());
    assert_eq!(control.verdict, Verdict::Fail);
    assert_eq!(rules(&control), ["CT-MAIN-TYPE@word/document.xml", "REF-NOTE@word/document.xml"]);
    let r = run(&format!("untyped corrupt reverse={reverse}"), &corrupt(&p.build(), "_rels/.rels"));
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty());
    for rule in ["REF-NOTE", "ID-COMMENT-REF"] {
        assert!(!r.checks_run.contains(&rule), "{rule} recorded");
    }
    assert!(r.checks_run.contains(&"REL-REF-MISSING"));
    let mut rid = p.clone();
    rid.put("word/document.xml", doc(r#"<w:p><w:hyperlink r:id="rIdGone"><w:r><w:t>x</w:t></w:r></w:hyperlink></w:p>"#));
    let r = run(&format!("untyped r:id reverse={reverse}"), &corrupt(&rid.build(), "_rels/.rels"));
    assert_eq!(r.verdict, Verdict::Fail);
    assert!(has(&r, "REL-REF-MISSING"));
}
#[test]
fn untyped_root_forward() {
    untyped_case(false);
}
#[test]
fn untyped_root_reverse() {
    untyped_case(true);
}

fn untyped_glossary_case(reverse: bool) {
    let mut p = with_footnotes(&["1"], &fnref("5"));
    p.put("[Content_Types].xml", ct(&[("/word/footnotes.xml", CT_FN)]));
    for (name, _) in &mut p.parts {
        if name == b"word/document.xml" {
            *name = b"word/glossary/document.xml".to_vec();
        } else if name == b"word/_rels/document.xml.rels" {
            *name = b"word/glossary/_rels/document.xml.rels".to_vec();
        }
    }
    p.put("_rels/.rels", rels(&[("rId1", OD, "word/glossary/document.xml", false)]));
    p.put("word/glossary/_rels/document.xml.rels", rels(&[("rIdF", &rt("footnotes"), "../footnotes.xml", false)]));
    if reverse {
        p = p.reversed();
    }
    let r = run(&format!("untyped glossary reverse={reverse}"), &corrupt(&p.build(), "_rels/.rels"));
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert!(r.findings.is_empty());
    assert!(!r.checks_run.contains(&"REF-NOTE"));
    assert!(!r.checks_run.contains(&"ID-COMMENT-REF"));
    assert!(r.checks_run.contains(&"REL-REF-MISSING"));
}
#[test]
fn untyped_glossary_forward() {
    untyped_glossary_case(false);
}
#[test]
fn untyped_glossary_reverse() {
    untyped_glossary_case(true);
}

#[test]
fn rejected_extra_fields_cannot_manufacture_duplicates() {
    // Exercise the fallback independently of the local-offset rejection. Each
    // sixth record has a valid local offset but a reader-rejected metadata field;
    // the retry footer remains outside the validator's tail in every case.
    let recs = clean_recs(false);
    let (template, start, embedded) = retry_out_of_tail(&recs, true, true);
    let rejected = embedded - 4 - recs.last().unwrap().name.len() - 46;
    let mut zip64_local = vec![0; 24];
    zip64_local[16..24].copy_from_slice(&u64::from(start).to_le_bytes());
    for (label, tag, payload, method, local) in [
        ("AES without field", 0xCAFE, vec![], 99u16, 0u32),
        ("AES length", 0x9901, vec![0; 6], 0, 0),
        ("AES vendor", 0x9901, vec![1, 0, 0, 0, 1, 0, 0], 0, 0),
        ("AES version", 0x9901, vec![3, 0, 0x41, 0x45, 1, 0, 0], 0, 0),
        ("AES strength", 0x9901, vec![1, 0, 0x41, 0x45, 4, 0, 0], 0, 0),
        ("ZIP64 length", 0x0001, vec![0], 0, u32::MAX),
        ("ZIP64 local offset", 0x0001, zip64_local, 0, 0),
        ("NTFS length", 0x000a, vec![], 0, 0),
        ("timestamp flags", 0x5455, vec![8, 0, 0, 0, 0], 0, 0),
        ("Unicode Comment CRC", 0x6375, vec![1, 0, 0, 0, 0], 0, 0),
        ("Unicode Path CRC", 0x7075, vec![1, 0, 0, 0, 0], 0, 0),
    ] {
        let mut z = template.clone();
        z[rejected + 10..rejected + 12].copy_from_slice(&method.to_le_bytes());
        z[rejected + 42..rejected + 46].copy_from_slice(&local.to_le_bytes());
        let extra_start = embedded - 4;
        let mut field = Vec::new();
        le16(&mut field, tag);
        le16(&mut field, payload.len() as u16);
        field.extend_from_slice(&payload);
        let added = field.len();
        z.splice(extra_start..extra_start, field);
        z[rejected + 30..rejected + 32].copy_from_slice(&(68 + added as u16).to_le_bytes());
        let eocd_cd_size = z.len() - 10;
        let old_cd_size = u32::from_le_bytes(z[eocd_cd_size..eocd_cd_size + 4].try_into().unwrap());
        z[eocd_cd_size..eocd_cd_size + 4].copy_from_slice(&(old_cd_size + added as u32).to_le_bytes());
        assert!(embedded + added < z.len().saturating_sub(22 + 0xFFFF));
        let mut a = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
        assert_eq!(a.central_directory_start(), u64::from(start), "{label}");
        assert_eq!(a.offset(), 0, "{label}");
        assert_eq!(a.len(), 5, "{label}");
        let f = a.by_name("word/footnotes.xml").unwrap();
        assert!(f.central_header_start() < rejected as u64, "{label}: reader did not reject sixth record");
        disputed(label, &run(label, &z)).unwrap();
    }
}

#[test]
fn reader_tolerated_extra_io_remains_accepted() {
    for (label, tag) in [("empty timestamp", 0x5455), ("empty Unicode Comment", 0x6375)] {
        let mut z = plain(&clean_recs(false));
        let (record, after_name) = {
            let mut a = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
            let f = a.by_name("word/footnotes.xml").unwrap();
            let record = f.central_header_start() as usize;
            (record, record + 46 + f.name().len())
        };
        let mut extra = Vec::new();
        le16(&mut extra, tag);
        le16(&mut extra, 0);
        z[record + 30..record + 32].copy_from_slice(&4u16.to_le_bytes());
        z.splice(after_name..after_name, extra);
        let eocd_cd_size = z.len() - 10;
        let old_cd_size = u32::from_le_bytes(z[eocd_cd_size..eocd_cd_size + 4].try_into().unwrap());
        z[eocd_cd_size..eocd_cd_size + 4].copy_from_slice(&(old_cd_size + 4).to_le_bytes());
        assert_clean(&run(label, &z));
    }
}

/// Standard IEEE CRC-32 for Info-ZIP 0x7075 Unicode Path header checks.
fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// CodeRabbit PR#18: distinct raw ZIP names that Unicode-decode to a case collision must fire PKG-CASE-COLLISION.
#[test]
fn unicode_path_case_collision_fires_from_decoded_names() {
    let mut p = Pkg::new();
    p.put("word/x1.xml", "<a/>");
    p.put("word/x2.xml", "<b/>");
    let mut z = p.build();
    for (raw_name, unicode_name) in [("word/x1.xml", "word/Alpha.xml"), ("word/x2.xml", "word/alpha.xml")] {
        let (record, after_name, old_extra_len) = {
            let mut a = zip::ZipArchive::new(Cursor::new(z.as_slice())).unwrap();
            let f = a.by_name(raw_name).unwrap();
            let record = f.central_header_start() as usize;
            let old_extra_len = u16::from_le_bytes(z[record + 30..record + 32].try_into().unwrap());
            (record, record + 46 + f.name().len(), old_extra_len)
        };
        let mut payload = Vec::new();
        payload.push(1);
        payload.extend_from_slice(&crc32_ieee(raw_name.as_bytes()).to_le_bytes());
        payload.extend_from_slice(unicode_name.as_bytes());
        let mut field = Vec::new();
        le16(&mut field, 0x7075);
        le16(&mut field, payload.len() as u16);
        field.extend_from_slice(&payload);
        let added = field.len();
        z[record + 30..record + 32].copy_from_slice(&(old_extra_len + added as u16).to_le_bytes());
        z.splice(after_name..after_name, field);
        let eocd_cd_size = z.len() - 10;
        let old_cd_size = u32::from_le_bytes(z[eocd_cd_size..eocd_cd_size + 4].try_into().unwrap());
        z[eocd_cd_size..eocd_cd_size + 4].copy_from_slice(&(old_cd_size + added as u32).to_le_bytes());
    }
    let report = validate_bytes(&z);
    assert!(report.findings.iter().any(|f| f.rule == "PKG-CASE-COLLISION"), "{report:?}");
}
