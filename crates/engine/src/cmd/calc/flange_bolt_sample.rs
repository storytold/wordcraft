//! End to end: a flange bolt-stress calculation as an engineer would write it, built through the
//! commands, saved to `.docx`, reopened, and edited.

use serde_json::{Value, json};
use wordcraft_doc::calc::{CalcRegion, CalcState};
use wordcraft_doc::{Block, Document, InlineObject};

use crate::Session;

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn calc(s: &mut Session, source: &str) -> u64 {
    let r = run(s, "insert.calculation", json!({"source": source, "display": true}));
    run(s, "text.newParagraph", json!({}));
    r["id"].as_u64().unwrap_or_else(|| panic!("no id in {r}"))
}

/// Every equation and field in the body, in document order.
fn objects(doc: &Document) -> Vec<InlineObject> {
    doc.body
        .iter()
        .flat_map(|b| match &**b {
            Block::Para(p) => p.objects.clone(),
            Block::Table(_) => Vec::new(),
        })
        .collect()
}

fn regions(s: &Session) -> Vec<(CalcRegion, String)> {
    objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, calc: Some(c), .. } => Some((*c, linear)),
            _ => None,
        })
        .collect()
}

fn linear_of(s: &Session, id: u64) -> String {
    regions(s).into_iter().find(|(r, _)| r.id == id).unwrap_or_else(|| panic!("no region {id}")).1
}

fn field_results(s: &Session) -> Vec<String> {
    objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect()
}

fn reopen(s: &Session) -> Session {
    let bytes = crate::io::save_bytes("flange.docx", &s.doc).unwrap();
    Session::new(crate::io::open_bytes("flange.docx", &bytes).unwrap())
}

struct Sample {
    s: Session,
    preload: u64,
    stress: u64,
    total: u64,
}

fn build() -> Sample {
    let mut s = Session::new(Document::new());
    run(&mut s, "document.setText", json!({"text": ""}));
    calc(&mut s, "n_b := 8");
    calc(&mut s, "d_b := 0.75 in");
    let preload = calc(&mut s, "F_i := 20 kip");
    calc(&mut s, "σ_allow := 45 ksi");
    calc(&mut s, "A_t(d) := π/4·d^2");
    let stress = calc(&mut s, "σ_b := F_i/A_t(d_b) = ? ksi");
    let total = calc(&mut s, "F_total := n_b·F_i = ? kip");
    run(&mut s, "text.insert", json!({"text": "Each bolt carries "}));
    run(&mut s, "insert.calcValue", json!({"name": "σ_b", "unit": "ksi"}));
    run(&mut s, "text.insert", json!({"text": " of stress."}));
    Sample { s, preload, stress, total }
}

fn stress_ksi(preload_kip: f64) -> f64 {
    preload_kip / (std::f64::consts::FRAC_PI_4 * 0.75 * 0.75)
}

/// The number printed before the unit in a displayed result such as `σ_b=45.271 "ksi"`.
fn shown(linear: &str) -> f64 {
    let after = linear.rsplit('=').next().unwrap_or("");
    let num: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    num.parse().unwrap_or_else(|_| panic!("no number in {linear}"))
}

#[test]
fn bolt_stress_values_are_right() {
    let Sample { s, stress, total, .. } = build();
    assert!((stress_ksi(20.0) - 45.27).abs() < 0.01);
    let sigma = linear_of(&s, stress);
    assert!((shown(&sigma) - 45.271).abs() < 0.001, "{sigma}");
    assert!(sigma.contains("ksi"), "{sigma}");
    let sum = linear_of(&s, total);
    assert!((shown(&sum) - 160.0).abs() < 1e-9, "{sum}");
    assert!(sum.contains("kip"), "{sum}");
    assert_eq!(field_results(&s), ["45.271 ksi"]);
    for (r, linear) in regions(&s) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
}

#[test]
fn saved_docx_reopens_live_and_equal() {
    let Sample { s, stress, total, .. } = build();
    let back = reopen(&s);
    assert_eq!(regions(&back).len(), 7);
    for (r, linear) in regions(&back) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    assert_eq!(linear_of(&back, stress), linear_of(&s, stress));
    assert_eq!(linear_of(&back, total), linear_of(&s, total));
    assert_eq!(field_results(&back), field_results(&s));
    // Live: a change after reopening still flows through.
    let mut back = back;
    let preload = regions(&back).into_iter().find(|(r, _)| r.source.starts_with("F_i")).unwrap().0.id;
    run(&mut back, "calc.setSource", json!({"id": preload, "source": "F_i := 18 kip"}));
    assert!((shown(&linear_of(&back, stress)) - stress_ksi(18.0)).abs() < 0.001);
}

#[test]
fn changing_the_preload_updates_every_result_and_the_sentence() {
    let Sample { mut s, preload, stress, total } = build();
    run(&mut s, "calc.setSource", json!({"id": preload, "source": "F_i := 18 kip"}));
    let sigma = linear_of(&s, stress);
    assert!((shown(&sigma) - 40.744).abs() < 0.001, "{sigma}");
    let sum = linear_of(&s, total);
    assert!((shown(&sum) - 144.0).abs() < 1e-9, "{sum}");
    assert_eq!(field_results(&s), ["40.744 ksi"]);
    // And the same after a save and reopen.
    let back = reopen(&s);
    assert_eq!(linear_of(&back, stress), sigma);
    assert_eq!(field_results(&back), ["40.744 ksi"]);
}
