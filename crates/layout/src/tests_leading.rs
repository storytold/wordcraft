use wordcraft_doc::props::{CharProps, LineSpacing};
use wordcraft_doc::{Document, Paragraph, para_block};
use wordcraft_fonts::FontDb;

const PROBE_ENV: &str = "WORDCRAFT_AUTO_LEADING_PROBE";
const TEST_NAME: &str = "tests_leading::automatic_metrics_reach_lines";
const PROOF_MARKER: &str = "AUTO_LEADING_PROOF:";
const FAMILY: &str = "Synthetic leading control";
const TOLERANCE: f32 = 0.001;
const UPEM: u16 = 1000;
const ASCENT: i16 = 900;
const DESCENT: i16 = -300;
const GAP: i16 = 200;
const LATIN_PAGE: u32 = 1;
const THAI_PAGE: u32 = 1 << 16;
const CJK_PAGE: u32 = 1 << 17;

struct Mode;
impl Mode {
    const LATIN: &str = "latin";
    const THAI: &str = "thai-metadata";
    const CJK: &str = "cjk-metadata";
    const BOTH: &str = "thai-and-cjk-metadata";
}

// Copy bundled OFL bytes in memory. This changes metadata, not Thai glyphs or native policy.
fn metric_font(font: &[u8], pages: u32) -> Vec<u8> {
    let mut bytes = font.to_vec();
    let count = usize::from(u16::from_be_bytes(bytes[4..6].try_into().unwrap()));
    let offset = |tag: &[u8; 4]| {
        let entry = bytes[12..12 + count * 16].as_chunks::<16>().0.iter().find(|entry| &entry[..4] == tag).unwrap();
        u32::from_be_bytes(entry[8..12].try_into().unwrap()) as usize
    };
    let head = offset(b"head");
    let hhea = offset(b"hhea");
    let os2 = offset(b"OS/2");
    bytes[head + 18..head + 20].copy_from_slice(&UPEM.to_be_bytes());
    bytes[hhea + 4..hhea + 6].copy_from_slice(&ASCENT.to_be_bytes());
    bytes[hhea + 6..hhea + 8].copy_from_slice(&DESCENT.to_be_bytes());
    bytes[hhea + 8..hhea + 10].copy_from_slice(&GAP.to_be_bytes());
    bytes[os2 + 78..os2 + 82].copy_from_slice(&pages.to_be_bytes());
    bytes
}

fn check_metrics(mode: &str) {
    let (pages, automatic) = match mode {
        Mode::LATIN => (LATIN_PAGE, (1100.0, 300.0)),
        Mode::THAI => (THAI_PAGE, (1100.0, 300.0)),
        Mode::CJK => (CJK_PAGE, (1080.0, 480.0)),
        Mode::BOTH => (THAI_PAGE | CJK_PAGE, (1080.0, 480.0)),
        _ => panic!("unknown leading probe"),
    };
    let db = FontDb::global();
    db.set_system_fallback(false);
    let source = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    assert_eq!(db.add_document_fonts(vec![(metric_font(source.data(), pages), FAMILY.into())]), 1);
    let face = wordcraft_fonts::word::resolve(FAMILY, false, false).face;
    assert_eq!(face.family, FAMILY);
    assert!(face.covers('x'));
    assert_eq!(face.is_east_asian(), pages & CJK_PAGE != 0);
    assert_eq!(wordcraft_fonts::word::line_metrics(face.get()), automatic);
    assert_eq!(wordcraft_fonts::word::fixed_line_metrics(face.get()), (1100.0, 300.0));
    for size in [9.0, 14.0, 20.0] {
        for spacing in [LineSpacing::Multiple(1.0), LineSpacing::Exactly(size * 2.0)] {
            let expected = match spacing {
                LineSpacing::Exactly(_) => (1100.0, 300.0),
                _ => automatic,
            };
            let chr = CharProps { font: Some(FAMILY.into()), size: Some(size), ..Default::default() };
            let mut paragraph = Paragraph::new();
            paragraph.mark = chr.clone();
            paragraph.props.line_spacing = Some(spacing);
            paragraph.props.snap_to_grid = Some(false);
            paragraph.insert_text(0, &format!("x{}x", wordcraft_doc::para::LINE_BREAK), &chr).unwrap();
            let mut doc = Document::new();
            doc.styles.default_chr = chr;
            doc.body = vec![para_block(paragraph)];
            let layout = crate::layout(&doc, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default());
            let paragraph = layout
                .pages
                .iter()
                .flat_map(|page| &page.items)
                .find_map(|item| match item {
                    crate::Placed::Lines { para, .. } => Some(para),
                    _ => None,
                })
                .unwrap();
            let style = &paragraph.styles[paragraph.clusters.first().unwrap().style as usize];
            assert_eq!(style.face.id(), face.id(), "the controlled font must supply the metrics");
            let scale = size / f32::from(UPEM);
            assert!((style.ascent - expected.0 as f32 * scale).abs() < TOLERANCE);
            assert!((style.descent - expected.1 as f32 * scale).abs() < TOLERANCE);
            assert_eq!(paragraph.lines.len(), 2);
            let pitch = match spacing {
                LineSpacing::Exactly(pitch) => pitch,
                _ => (expected.0 + expected.1) as f32 * scale,
            };
            for line in &paragraph.lines {
                assert!((line.height - pitch).abs() < TOLERANCE, "{mode}, {size}, {spacing:?}: {line:?}");
            }
            let first = &paragraph.lines[0];
            let second = &paragraph.lines[1];
            assert!((second.baseline - first.baseline - pitch).abs() < TOLERANCE);
        }
    }
}

#[test]
fn automatic_metrics_reach_lines() {
    if let Ok(mode) = std::env::var(PROBE_ENV) {
        check_metrics(&mode);
        println!("{PROOF_MARKER}{mode}");
        return;
    }
    // Global font/metric caches need a separate process for each metadata control.
    for mode in [Mode::LATIN, Mode::THAI, Mode::CJK, Mode::BOTH] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(PROBE_ENV, mode)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(&format!("{PROOF_MARKER}{mode}")), "probe did not complete: {stdout}");
        assert!(output.status.success(), "{mode}: {}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
}
