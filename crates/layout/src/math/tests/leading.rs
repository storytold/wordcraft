use super::rc;
use serde_json::{Value, json};
use wordcraft_doc::math::{MNode, MRun, Math, MathProps};
use wordcraft_doc::props::{CharProps, LineSpacing};
use wordcraft_doc::{Document, InlineObject, Paragraph, para_block};
use wordcraft_fonts::FontDb;

const PROBE_ENV: &str = "WORDCRAFT_MATH_LEADING_PROBE";
const OUTPUT_PREFIX: &str = "MATH_LEADING_METRICS:";
const TEST_NAME: &str = "math::tests::leading::leading_reaches_paragraph";
const TOLERANCE: f64 = 0.001;
const MATH_HEADER_BYTES: usize = 10;
const MATH_VALUE_COUNT: usize = 51;
const MATH_PREFIX_BYTES: usize = 8;
const MATH_VALUE_BYTES: usize = 4;
const MATH_TAIL_BYTES: usize = 2;

struct ProbeMode;
impl ProbeMode {
    const ZERO: &str = "zero";
    const POSITIVE: &str = "positive";
}

// Adapt the existing fonts::math test's sfnt construction; no font file is written.
fn with_leading(font: &[u8], units: i16) -> Vec<u8> {
    let count = u16::from_be_bytes(font[4..6].try_into().unwrap()) as usize;
    let mut directory: Vec<[u8; 16]> = (0..count).map(|i| font[12 + i * 16..28 + i * 16].try_into().unwrap()).collect();
    assert!(directory.iter().all(|row| &row[..4] != b"MATH"));
    let mut table = vec![0u8; MATH_HEADER_BYTES + MATH_PREFIX_BYTES + MATH_VALUE_COUNT * MATH_VALUE_BYTES + MATH_TAIL_BYTES];
    table[..2].copy_from_slice(&1u16.to_be_bytes());
    table[4..6].copy_from_slice(&u16::try_from(MATH_HEADER_BYTES).unwrap().to_be_bytes());
    let leading = MATH_HEADER_BYTES + MATH_PREFIX_BYTES;
    table[leading..leading + 2].copy_from_slice(&units.to_be_bytes());
    for row in &mut directory {
        let offset = u32::from_be_bytes(row[8..12].try_into().unwrap()) + 16;
        row[8..12].copy_from_slice(&offset.to_be_bytes());
    }
    let mut entry = [0u8; 16];
    entry[..4].copy_from_slice(b"MATH");
    entry[8..12].copy_from_slice(&u32::try_from(font.len() + 16).unwrap().to_be_bytes());
    entry[12..16].copy_from_slice(&u32::try_from(table.len()).unwrap().to_be_bytes());
    directory.push(entry);
    directory.sort_by_key(|row| <[u8; 4]>::try_from(&row[..4]).unwrap());
    let mut out = font[..12].to_vec();
    out[4..6].copy_from_slice(&u16::try_from(count + 1).unwrap().to_be_bytes());
    for row in directory {
        out.extend_from_slice(&row);
    }
    out.extend_from_slice(&font[12 + count * 16..]);
    out.extend_from_slice(&table);
    out
}

fn metrics(mode: &str) -> Vec<Value> {
    let db = FontDb::global();
    let source = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let units = match mode {
        ProbeMode::ZERO => 0,
        ProbeMode::POSITIVE => i16::try_from(source.units_per_em() as u32).unwrap(),
        _ => panic!("unknown math-leading probe"),
    };
    assert_eq!(db.add_document_fonts(vec![(with_leading(source.data(), units), "Cambria Math".into())]), 1);
    let (face, table) = wordcraft_fonts::math::resolve_math("Cambria Math");
    assert_eq!(face.family, "Cambria Math");
    assert_eq!(table.unwrap().get(wordcraft_fonts::math::C::MathLeading), f32::from(units));
    let math = Math { nodes: vec![MNode::Run(MRun { nor: true, ..MRun::new("a") })], ..Default::default() };
    let mut records = Vec::new();
    for display in [false, true] {
        for factor in [0.75, 1.0, 1.5] {
            let size = rc().size * factor;
            let props = MathProps::default();
            let mut style = rc();
            style.font = wordcraft_fonts::DEFAULT_FAMILY.into();
            style.size = size;
            style.size_cs = size;
            let equation = super::super::layout_equation(&math, "", &style, display, 400.0, &props, &mut 0);
            let chr = CharProps { size: Some(size), font: Some(style.font.clone()), ..Default::default() };
            let mut paragraph = Paragraph::new();
            paragraph.mark = chr.clone();
            paragraph.props.line_spacing = Some(LineSpacing::Multiple(1.0));
            paragraph.insert_object(0, InlineObject::Equation { linear: "a".into(), display, math: math.clone() }, &chr).unwrap();
            let mut doc = Document::new();
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
            let embedded = &paragraph.maths.first().unwrap().1;
            assert!((embedded.ascent - equation.ascent).abs() < TOLERANCE as f32);
            assert!((embedded.descent - equation.descent).abs() < TOLERANCE as f32);
            let cluster = paragraph.clusters.first().unwrap();
            let text = &paragraph.styles[cluster.style as usize];
            let ascent = equation.ascent.max(text.ascent);
            let descent = equation.descent.max(text.descent);
            assert!((cluster.obj_h - ascent).abs() < TOLERANCE as f32);
            let line = paragraph.lines.first().unwrap();
            assert!((line.height - ascent - descent).abs() < TOLERANCE as f32);
            records.push(json!({"ascent": equation.ascent, "descent": equation.descent, "width": equation.width,
                "items": format!("{:?}", equation.items), "text_ascent": text.ascent, "line_height": line.height,
                "leading": f32::from(units) * size / face.upem as f32}));
        }
    }
    records
}

fn run_probe(mode: &str) -> Vec<Value> {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(PROBE_ENV, mode)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    let json = text.lines().find_map(|line| line.split_once(OUTPUT_PREFIX).map(|(_, value)| value)).expect("probe metrics");
    serde_json::from_str(json).unwrap()
}

#[test]
fn leading_reaches_paragraph() {
    if let Ok(mode) = std::env::var(PROBE_ENV) {
        println!("{OUTPUT_PREFIX}{}", serde_json::to_string(&metrics(&mode)).unwrap());
        return;
    }
    // The font database is process-wide; each controlled table has its own test process.
    let zero = run_probe(ProbeMode::ZERO);
    let positive = run_probe(ProbeMode::POSITIVE);
    assert_eq!(zero.len(), positive.len());
    assert!(!zero.is_empty());
    for (zero, positive) in zero.iter().zip(&positive) {
        let value = |row: &Value, key: &str| row[key].as_f64().unwrap();
        let leading = value(positive, "leading");
        assert!(leading > 0.0);
        assert!((value(positive, "ascent") - value(zero, "ascent") - leading).abs() < TOLERANCE);
        assert_eq!(positive["descent"], zero["descent"]);
        assert_eq!(positive["width"], zero["width"]);
        assert_eq!(positive["items"], zero["items"]);
        let baseline = value(zero, "ascent");
        let strut = value(zero, "text_ascent");
        let expected = (baseline + leading).max(strut) - baseline.max(strut);
        assert!(expected > 0.0);
        assert!((value(positive, "line_height") - value(zero, "line_height") - expected).abs() < TOLERANCE);
    }
}
