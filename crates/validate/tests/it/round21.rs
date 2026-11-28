//! Round 21 permanent Carbon directory-selection regressions. Independent raw-ZIP builder (stored entries, hand-written
//! local/central/EOCD records) so every name byte, flag and extra field is explicit.

use wordcraft_validate::{Report, Verdict, validate_bytes};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#,
);
const CT_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const CT_FOOTNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";

// ---------- raw zip builder ----------

#[derive(Clone)]
struct Rec {
    name: Vec<u8>,
    flags: u16,
    extra: Vec<u8>,
    body: Vec<u8>,
}

impl Rec {
    fn new(name: impl AsRef<[u8]>, flags: u16, body: impl AsRef<[u8]>) -> Self {
        Self { name: name.as_ref().to_vec(), flags, extra: Vec::new(), body: body.as_ref().to_vec() }
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 { crc >> 1 } else { (crc >> 1) ^ 0xedb8_8320 };
        }
    }
    !crc
}

fn local(r: &Rec) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(0x0403_4b50u32.to_le_bytes());
    out.extend(20u16.to_le_bytes()); // version needed
    out.extend(r.flags.to_le_bytes());
    out.extend(0u16.to_le_bytes()); // stored
    out.extend(0u16.to_le_bytes()); // time
    out.extend(0x21u16.to_le_bytes()); // date (1980-01-01)
    out.extend(crc32(&r.body).to_le_bytes());
    out.extend((r.body.len() as u32).to_le_bytes());
    out.extend((r.body.len() as u32).to_le_bytes());
    out.extend((r.name.len() as u16).to_le_bytes());
    out.extend((r.extra.len() as u16).to_le_bytes());
    out.extend(&r.name);
    out.extend(&r.extra);
    out.extend(&r.body);
    out
}

fn central(r: &Rec, offset: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(0x0201_4b50u32.to_le_bytes());
    out.extend(20u16.to_le_bytes()); // version made by
    out.extend(20u16.to_le_bytes()); // version needed
    out.extend(r.flags.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0x21u16.to_le_bytes());
    out.extend(crc32(&r.body).to_le_bytes());
    out.extend((r.body.len() as u32).to_le_bytes());
    out.extend((r.body.len() as u32).to_le_bytes());
    out.extend((r.name.len() as u16).to_le_bytes());
    out.extend((r.extra.len() as u16).to_le_bytes());
    out.extend(0u16.to_le_bytes()); // comment
    out.extend(0u16.to_le_bytes()); // disk
    out.extend(0u16.to_le_bytes()); // internal attrs
    out.extend(0u32.to_le_bytes()); // external attrs
    out.extend(offset.to_le_bytes());
    out.extend(&r.name);
    out.extend(&r.extra);
    out
}

fn eocd(count: u16, cd_size: u32, cd_off: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(cd_size.to_le_bytes());
    out.extend(cd_off.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

fn assemble(real: &[Rec]) -> (Vec<u8>, Vec<u32>) {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for r in real {
        offsets.push(u32::try_from(out.len()).unwrap());
        out.extend(local(r));
    }
    (out, offsets)
}

fn build(real: &[Rec]) -> Vec<u8> {
    let (mut out, offsets) = assemble(real);
    let cd_off = u32::try_from(out.len()).unwrap();
    let mut cd = Vec::new();
    for (r, o) in real.iter().zip(offsets) {
        cd.extend(central(r, o));
    }
    out.extend(&cd);
    out.extend(eocd(real.len() as u16, cd.len() as u32, cd_off));
    out
}

/// ZIP with a decoy central directory selected only by the ZIP64 record, a sentinel
/// `central directory size` in the 16-bit EOCD, and a real 16-bit count/offset (so the
/// zip crate's `may_be_zip64` is false and the crate parses the real directory).
fn build_decoy(real: &[Rec], decoy: &[Rec]) -> Vec<u8> {
    let (mut out, offsets) = assemble(real);
    let real_cd_off = u32::try_from(out.len()).unwrap();
    let mut real_cd = Vec::new();
    for (r, o) in real.iter().zip(offsets) {
        real_cd.extend(central(r, o));
    }
    out.extend(&real_cd);
    let decoy_cd_off = u32::try_from(out.len()).unwrap();
    let mut decoy_cd = Vec::new();
    for r in decoy {
        decoy_cd.extend(central(r, 0));
    }
    out.extend(&decoy_cd);
    let z64_off = u32::try_from(out.len()).unwrap();
    let mut z = Vec::new();
    z.extend(0x0606_4b50u32.to_le_bytes());
    z.extend(44u64.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend(0u32.to_le_bytes());
    z.extend(0u32.to_le_bytes());
    z.extend((decoy.len() as u64).to_le_bytes());
    z.extend((decoy.len() as u64).to_le_bytes());
    z.extend((decoy_cd.len() as u64).to_le_bytes());
    z.extend((decoy_cd_off as u64).to_le_bytes());
    out.extend(z);
    out.extend(0x0706_4b50u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((z64_off as u64).to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend((real.len() as u16).to_le_bytes());
    out.extend((real.len() as u16).to_le_bytes());
    out.extend(0xFFFF_FFFFu32.to_le_bytes()); // sentinel cd size only
    out.extend(real_cd_off.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

// ---------- part builders ----------

fn ct(main: &str, overrides: &[(&str, &str)]) -> Vec<u8> {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
    );
    s.push_str(&format!(r#"<Override PartName="/{main}" ContentType="{CT_MAIN}"/>"#));
    for (part, ty) in overrides {
        s.push_str(&format!(r#"<Override PartName="/{part}" ContentType="{ty}"/>"#));
    }
    s.push_str("</Types>");
    s.into_bytes()
}

fn rels(children: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{children}</Relationships>"#
    )
    .into_bytes()
}

fn edge(id: &str, kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
}

fn pkg_rels(target: &str) -> Vec<u8> {
    rels(&edge("rId1", "officeDocument", target))
}

fn doc(body: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}</w:body></w:document>"#).into_bytes()
}

fn footnotes(ids: &[u32]) -> Vec<u8> {
    let defs: String = ids.iter().map(|id| format!(r#"<w:footnote w:id="{id}"><w:p/></w:footnote>"#)).collect();
    format!(r#"<?xml version="1.0"?><w:footnotes {NS}>{defs}</w:footnotes>"#).into_bytes()
}

fn empty_footnotes() -> Vec<u8> {
    format!(r#"<?xml version="1.0"?><w:footnotes {NS}/>"#).into_bytes()
}

const NOTE_REF: &str = r#"<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#;

fn base(ct: Vec<u8>, main: &str, body: &str, doc_rels: Option<String>) -> Vec<Rec> {
    vec![
        Rec::new("[Content_Types].xml", 0, ct),
        Rec::new("_rels/.rels", 0, pkg_rels(main)),
        Rec::new(main, 0, doc(body)),
        Rec::new("word/_rels/document.xml.rels", 0, rels(doc_rels.as_deref().unwrap_or(""))),
    ]
}

// ---------- report helpers ----------

fn show(r: &Report) -> String {
    let findings: Vec<String> = r.findings.iter().map(|f| format!("[{} @ {:?}] {}", f.rule, f.part, f.message)).collect();
    format!("verdict={:?}\nfindings={:#?}\nchecks_run={:?}\nnotes={:#?}", r.verdict, findings, r.checks_run, r.coverage_notes)
}

fn count(r: &Report, rule: &str) -> usize {
    r.findings.iter().filter(|f| f.rule == rule).count()
}

fn rules(r: &Report) -> Vec<&str> {
    let mut v: Vec<&str> = r.findings.iter().map(|f| f.rule).collect();
    v.sort_unstable();
    v
}

fn messages(r: &Report, rule: &str) -> Vec<String> {
    let mut v: Vec<String> = r.findings.iter().filter(|f| f.rule == rule).map(|f| f.message.clone()).collect();
    v.sort_unstable();
    v
}

fn ran(r: &Report, rule: &str) -> bool {
    r.checks_run.contains(&rule)
}

fn h6_pair_base() -> Vec<Rec> {
    base(ct("word/document.xml", &[("word/fü.xml", CT_FOOTNOTES)]), "word/document.xml", NOTE_REF, Some(edge("fn", "footnotes", "fü.xml")))
}

// Carbon's three decoy vectors, with exact findings and both entry orders pinned.
#[test]
fn zip64_sentinel_cd_size_must_not_invent_a_decoy_duplicate() {
    for reverse in [false, true] {
        let mut real = base(ct("word/document.xml", &[]), "word/document.xml", "<w:p/>", None);
        real.push(Rec::new("word/x.xml", 0, format!(r#"<w:a xmlns:w="{W_NS}"/>"#)));
        let mut decoy = real.clone();
        decoy.push(Rec::new("word/x.xml", 0, format!(r#"<w:b xmlns:w="{W_NS}"/>"#)));
        if reverse {
            real.reverse();
            decoy.reverse();
        }
        let report = validate_bytes(&build_decoy(&real, &decoy));
        assert_eq!(report.verdict, Verdict::Pass, "reverse={reverse}: {}", show(&report));
        assert!(report.findings.is_empty(), "{}", show(&report));
        assert!(report.coverage_notes.is_empty(), "{}", show(&report));
    }
}

#[test]
fn zip64_sentinel_cd_size_must_not_read_a_different_directory() {
    let ct_xml = format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/dü.xml" ContentType="{CT_MAIN}"/></Types>"#
    );
    let bad = format!(r#"<w:document {NS}><w:body><w:p>"#).into_bytes();
    let good = doc("<w:p/>");
    let mut seen = Vec::new();
    for reverse in [false, true] {
        let mut pair = vec![Rec::new(b"word/d\x81.xml", 0, &bad), Rec::new("word/dü.xml", 0x800, &good)];
        if reverse {
            pair.reverse();
        }
        let mut real = vec![Rec::new("[Content_Types].xml", 0, &ct_xml), Rec::new("_rels/.rels", 0, pkg_rels("word/dü.xml"))];
        real.extend(pair);
        let decoy = vec![
            Rec::new("[Content_Types].xml", 0, &ct_xml),
            Rec::new("_rels/.rels", 0, pkg_rels("word/dü.xml")),
            Rec::new("word/dü.xml", 0x800, &good),
        ];
        let report = validate_bytes(&build_decoy(&real, &decoy));
        assert_eq!(report.verdict, Verdict::Fail, "reverse={reverse}: {}", show(&report));
        assert_eq!(count(&report, "PKG-DUP-ENTRY"), 1, "{}", show(&report));
        assert_eq!(count(&report, "XML-MALFORMED"), usize::from(reverse), "{}", show(&report));
        assert_eq!(report.findings.len(), 1 + usize::from(reverse), "{}", show(&report));
        assert!(!ran(&report, "REF-NOTE"), "{}", show(&report));
        seen.push((messages(&report, "PKG-DUP-ENTRY"), report.checks_run));
    }
    assert_eq!(seen[0], seen[1]);
}

#[test]
fn zip64_sentinel_cd_size_also_hides_footnotes_collapse_in_both_orders() {
    let mut seen = Vec::new();
    for reverse in [false, true] {
        let mut real = h6_pair_base();
        let mut pair = vec![Rec::new("word/fü.xml", 0x800, footnotes(&[1])), Rec::new(b"word/f\x81.xml", 0, empty_footnotes())];
        if reverse {
            pair.reverse();
        }
        real.extend(pair);
        let decoy = vec![
            Rec::new("[Content_Types].xml", 0, ct("word/document.xml", &[("word/fü.xml", CT_FOOTNOTES)])),
            Rec::new("_rels/.rels", 0, pkg_rels("word/document.xml")),
            Rec::new("word/document.xml", 0, doc(NOTE_REF)),
            Rec::new("word/_rels/document.xml.rels", 0, rels(&edge("fn", "footnotes", "fü.xml"))),
            Rec::new("word/fü.xml", 0x800, footnotes(&[1])),
        ];
        let report = validate_bytes(&build_decoy(&real, &decoy));
        assert_eq!(report.verdict, Verdict::Fail, "reverse={reverse}: {}", show(&report));
        assert_eq!(rules(&report), ["PKG-DUP-ENTRY"], "{}", show(&report));
        assert!(!ran(&report, "REF-NOTE"), "{}", show(&report));
        seen.push((messages(&report, "PKG-DUP-ENTRY"), report.checks_run));
    }
    assert_eq!(seen[0], seen[1]);
}

#[test]
fn zip64_wellformed_directory_counts_agree() {
    // Sanity: a genuine ZIP64-flavoured archive (sentinel count, real ZIP64 record agreeing with
    // the actual directory) must be clean: residual 0, no INCOMPLETE.
    let real = base(ct("word/document.xml", &[]), "word/document.xml", "<w:p/>", None);
    let mut zip = build(&real);
    let eocd_at = zip.len() - 22;
    // rewrite 16-bit count to the sentinel and append a ZIP64 record + locator before EOCD
    let cd_off = u32::from_le_bytes(zip[eocd_at + 16..eocd_at + 20].try_into().unwrap());
    let cd_size = u32::from_le_bytes(zip[eocd_at + 12..eocd_at + 16].try_into().unwrap());
    let eocd = zip.split_off(eocd_at);
    let z64_off = zip.len() as u32;
    let mut z = Vec::new();
    z.extend(0x0606_4b50u32.to_le_bytes());
    z.extend(44u64.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend(0u32.to_le_bytes());
    z.extend(0u32.to_le_bytes());
    z.extend((real.len() as u64).to_le_bytes());
    z.extend((real.len() as u64).to_le_bytes());
    z.extend((cd_size as u64).to_le_bytes());
    z.extend((cd_off as u64).to_le_bytes());
    zip.extend(z);
    zip.extend(0x0706_4b50u32.to_le_bytes());
    zip.extend(0u32.to_le_bytes());
    zip.extend((z64_off as u64).to_le_bytes());
    zip.extend(1u32.to_le_bytes());
    zip.extend(&eocd[..10]);
    zip.extend(0xFFFFu16.to_le_bytes());
    zip.extend(&eocd[12..]);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Pass, "{}", show(&report));
    assert!(report.findings.is_empty());
    assert!(report.coverage_notes.is_empty());
}

// The reader accepts trailing data after an EOCD, while the bounded parser requires
// its comment to end at EOF. Nest the reader's directory in the parser EOCD's comment.
fn build_directory_disagreement(real: &[Rec], decoy: &[Rec]) -> Vec<u8> {
    let (mut out, offsets) = assemble(real);
    let decoy_off = u32::try_from(out.len()).unwrap();
    let mut decoy_cd = Vec::new();
    for (r, offset) in decoy.iter().zip(&offsets) {
        decoy_cd.extend(central(r, *offset));
    }
    out.extend(&decoy_cd);
    let mut real_cd = Vec::new();
    for (r, offset) in real.iter().zip(offsets) {
        real_cd.extend(central(r, offset));
    }
    let mut decoy_end = eocd(decoy.len() as u16, decoy_cd.len() as u32, decoy_off);
    decoy_end[20..22].copy_from_slice(&u16::try_from(real_cd.len() + 23).unwrap().to_le_bytes());
    out.extend(decoy_end);
    let real_off = u32::try_from(out.len()).unwrap();
    out.extend(&real_cd);
    out.extend(eocd(real.len() as u16, real_cd.len() as u32, real_off));
    out.push(b'X');
    out
}

#[test]
fn directory_reader_disagreement_suppresses_duplicates_and_absence_both_orders() {
    for reverse in [false, true] {
        for duplicate in [false, true] {
            let mut real = base(ct("word/document.xml", &[]), "word/document.xml", NOTE_REF, None);
            real.extend([
                Rec::new("word/x.xml", 0, format!(r#"<w:a xmlns:w="{W_NS}"/>"#)),
                Rec::new("word/y.xml", 0, format!(r#"<w:b xmlns:w="{W_NS}"/>"#)),
            ]);
            if reverse {
                real.reverse();
            }
            let mut decoy = real.clone();
            let y = decoy.iter_mut().find(|r| r.name == b"word/y.xml").unwrap();
            y.name = if duplicate { b"word/x.xml".to_vec() } else { b"word/z.xml".to_vec() };
            let bytes = build_directory_disagreement(&real, &decoy);
            // Establish the oracle independently: this reader selected the real directory.
            let reader = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
            assert_eq!(reader.len(), real.len());
            assert!(reader.index_for_name("word/y.xml").is_some());
            assert!(reader.index_for_name("word/z.xml").is_none());
            let report = validate_bytes(&bytes);
            assert_eq!(report.verdict, Verdict::Incomplete, "reverse={reverse}, duplicate={duplicate}: {}", show(&report));
            assert!(report.findings.is_empty(), "{}", show(&report));
            assert!(!ran(&report, "PKG-DUP-ENTRY"), "{}", show(&report));
            assert!(!ran(&report, "REF-NOTE"), "{}", show(&report));
            assert!(report.coverage_notes.iter().any(|n| n.contains("decoded names disagree")), "{}", show(&report));
        }
    }
}
