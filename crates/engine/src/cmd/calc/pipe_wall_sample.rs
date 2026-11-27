//! End to end: the guide's data-table examples, built through the commands as a user would.
//!
//! - An ASME B31.3 minimum wall thickness check over a table of pipe sizes, edited, saved to
//!   `.docx`, reopened and edited again.
//! - A materials table with a text column, read with a text `lookup`, a piecewise and a sum,
//!   saved, reopened and edited again.
//! - A flange torque table read with `lookup` and `linterp`.

use serde_json::{Value, json};
use wordcraft_doc::calc::{CalcRegion, CalcState, CalcTable};
use wordcraft_doc::{Block, Document, InlineObject, Pos};

use crate::Session;

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new(Document::new());
    run(&mut s, "document.setText", json!({"text": ""}));
    s
}

/// Inserts a calculation on a line of its own; returns its id.
fn calc(s: &mut Session, source: &str) -> u64 {
    let r = run(s, "insert.calculation", json!({"source": source, "display": true}));
    run(s, "text.newParagraph", json!({}));
    r["id"].as_u64().unwrap_or_else(|| panic!("no id in {r}"))
}

/// Inserts a table holding `rows` (typed cell by cell, Tab between cells), marks it as
/// calculation data, and leaves the caret at the end of the document.
fn data_table(s: &mut Session, rows: &[&[&str]]) {
    let r = run(s, "insert.table", json!({"rows": rows.len(), "cols": rows[0].len()}));
    assert!(r["table"][0].is_u64(), "no table in {r}");
    for (k, text) in rows.iter().flat_map(|r| r.iter()).enumerate() {
        if k > 0 {
            run(s, "text.tab", json!({}));
        }
        run(s, "text.insert", json!({"text": text}));
    }
    let r = run(s, "calc.dataTable", json!({"on": true}));
    assert!(r["id"].is_u64(), "no table id in {r}");
    run(s, "caret.docEnd", json!({}));
}

/// The body index of the only table in the document.
fn table_block(s: &Session) -> usize {
    let tables: Vec<usize> = s.doc.body.iter().enumerate().filter(|(_, b)| matches!(&***b, Block::Table(_))).map(|(i, _)| i).collect();
    assert_eq!(tables.len(), 1, "one table");
    tables[0]
}

fn table_mark(s: &Session) -> Option<CalcTable> {
    match &*s.doc.body[table_block(s)] {
        Block::Table(t) => t.calc.as_deref().cloned(),
        Block::Para(_) => None,
    }
}

fn cell_text(s: &Session, row: usize, col: usize) -> String {
    match &*s.doc.body[table_block(s)] {
        Block::Table(t) => t.rows[row].cells[col]
            .blocks
            .iter()
            .filter_map(|b| match &**b {
                Block::Para(p) => Some(p.text.clone()),
                Block::Table(_) => None,
            })
            .collect(),
        Block::Para(_) => String::new(),
    }
}

/// Replaces the text of cell (`row`, `col`) by deleting it and typing `text`, as a user would.
fn retype_cell(s: &mut Session, row: usize, col: usize, text: &str) {
    let old = cell_text(s, row, col);
    let path = wordcraft_doc::Path(vec![table_block(s) as u32, row as u32, col as u32, 0]);
    s.sel = crate::Selection::caret(Pos { story: wordcraft_doc::StoryRef::Body, path, off: old.len() });
    for _ in old.chars() {
        run(s, "text.backspace", json!({}));
    }
    run(s, "text.insert", json!({"text": text}));
    assert_eq!(cell_text(s, row, col), text);
}

/// (region, linear) of every calculation region in the body, in document order.
fn regions(s: &Session) -> Vec<(CalcRegion, String)> {
    s.doc
        .body
        .iter()
        .flat_map(|b| match &**b {
            Block::Para(p) => p.objects.clone(),
            Block::Table(_) => Vec::new(),
        })
        .filter_map(|o| match o {
            InlineObject::Equation { linear, calc: Some(c), .. } => Some((*c, linear)),
            _ => None,
        })
        .collect()
}

/// The region whose source starts with `source`.
fn region(s: &Session, source: &str) -> (CalcRegion, String) {
    regions(s).into_iter().find(|(r, _)| r.source.starts_with(source)).unwrap_or_else(|| panic!("no region {source}"))
}

/// Asserts the region whose source starts with `source` is current and its display ends `tail`.
fn shows(s: &Session, source: &str, tail: &str) {
    let (r, linear) = region(s, source);
    assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    assert!(linear.ends_with(tail), "{} → {linear}, expected …{tail}", r.source);
}

// ---------------------------------------------------------------------------------------------
// ASME B31.3 minimum wall thickness
// ---------------------------------------------------------------------------------------------

const PIPES: &[&[&str]] =
    &[&["NPS (in)", "D_o (in)", "t_nom (in)"], &["1/2", "0.840", "0.109"], &["3/4", "1.050", "0.113"], &["1", "1.315", "0.133"]];

fn pipe_wall() -> Session {
    let mut s = session();
    data_table(&mut s, PIPES);
    for source in [
        "P := 1000 psi",
        "S := 20000 psi",
        "E := 1",
        "Y := 0.4",
        "t_min := P·D_o/(2·(S·E + P·Y)) =",
        "margin := t_nom - t_min =",
        "min(margin) =",
        "check := \"OK\" if margin ≥ 0 in, \"NG\" otherwise =",
        "n_NG := sum(check == \"NG\") =",
    ] {
        calc(&mut s, source);
    }
    run(&mut s, "text.insert", json!({"text": "Wall thickness at each size: "}));
    run(&mut s, "insert.calcValue", json!({"name": "t_nom"}));
    s
}

fn field_results(s: &Session) -> Vec<String> {
    s.doc
        .body
        .iter()
        .flat_map(|b| match &**b {
            Block::Para(p) => p.objects.clone(),
            Block::Table(_) => Vec::new(),
        })
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect()
}

fn assert_pipe_wall_results(s: &Session) {
    assert_eq!(table_mark(s).map(|c| c.errors), Some(Vec::new()));
    shows(s, "t_min", "=[■(0.021@0.026@0.032)] \"in\"");
    shows(s, "margin", "=[■(0.088@0.087@0.101)] \"in\"");
    shows(s, "min(margin)", "=0.087 \"in\"");
    for (r, linear) in regions(s) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    assert_eq!(field_results(s), ["0.109, 0.113, 0.133 in"]);
}

#[test]
fn pipe_wall_sample() {
    let s = pipe_wall();
    assert_eq!(regions(&s).len(), 9);
    assert_pipe_wall_results(&s);
}

#[test]
fn pipe_wall_sample_cell_edit_recalculates() {
    let mut s = pipe_wall();
    retype_cell(&mut s, 2, 2, "0.120");
    shows(&s, "margin", "=[■(0.088@0.094@0.101)] \"in\"");
    shows(&s, "min(margin)", "=0.088 \"in\"");
    assert_eq!(field_results(&s), ["0.109, 0.12, 0.133 in"]);
    // A cell that isn't a number names its column and row, and everything after it stops.
    retype_cell(&mut s, 2, 2, "n/a");
    let errors = table_mark(&s).map(|c| c.errors).unwrap_or_default();
    assert_eq!(errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), ["t_nom row 2 isn't a number"]);
    assert_eq!(region(&s, "margin").0.state, CalcState::Error("depends on table data with an error".into()));
    assert_eq!(region(&s, "t_min").0.state, CalcState::Fresh, "t_min doesn't use t_nom");
    // A mixed number is a value like any other.
    retype_cell(&mut s, 2, 2, "1-1/2");
    assert_eq!(table_mark(&s).map(|c| c.errors), Some(Vec::new()));
    shows(&s, "min(margin)", "=0.088 \"in\"");
}

#[test]
fn pipe_wall_sample_reopens_live() {
    let s = pipe_wall();
    let bytes = crate::io::save_bytes("pipes.docx", &s.doc).unwrap();
    let mut back = session();
    run(&mut back, "file.open", json!({"path": "pipes.docx", "data": crate::cmd::insert::base64_encode(&bytes)}));
    assert!(!back.status.contains("edited in Word"), "{}", back.status);
    assert_eq!(regions(&back).len(), 9);
    assert_pipe_wall_results(&back);
    for (r, linear) in regions(&s) {
        assert_eq!(region(&back, &r.source).1, linear, "{}", r.source);
    }
    // Live: a cell edit after reopening still flows through.
    retype_cell(&mut back, 2, 2, "0.120");
    shows(&back, "min(margin)", "=0.088 \"in\"");
}

#[test]
fn pipe_wall_check_flips_per_size() {
    let mut s = pipe_wall();
    shows(&s, "check", "=[■(\"OK\"@\"OK\"@\"OK\")]");
    shows(&s, "n_NG", "=0");
    retype_cell(&mut s, 2, 2, "0.020");
    shows(&s, "check", "=[■(\"OK\"@\"NG\"@\"OK\")]");
    shows(&s, "n_NG", "=1");
}

// ---------------------------------------------------------------------------------------------
// Materials: text lookup, a piecewise and a sum
// ---------------------------------------------------------------------------------------------

#[test]
fn materials_sample_reopens_live_with_a_word_edit() {
    let mut s = session();
    data_table(&mut s, &[&["Material (text)", "S (ksi)"], &["A106 B", "20"], &["A312 TP316", "20"]]);
    calc(&mut s, "S_allow := lookup(Material, S, \"A312 TP316\") = ? ksi");
    calc(&mut s, "Y := 0.4 if S_allow ≤ 20 ksi, 0.7 otherwise =");
    calc(&mut s, "Σ_S := ∑(i = 0..length(S) - 1) S[i] = ? ksi");
    shows(&s, "S_allow", "=20 \"ksi\"");
    shows(&s, "Y", "≔{■(0.4&\"if \"S_allow≤20 \"ksi\"@0.7&\"otherwise\")┤=0.4");
    shows(&s, "Σ_S", "≔∑_(i=0)^(length(S)-1)▒〖S_i〗=40 \"ksi\"");
    let bytes = crate::io::save_bytes("materials.docx", &s.doc).unwrap();
    let mut back = session();
    run(&mut back, "file.open", json!({"path": "materials.docx", "data": crate::cmd::insert::base64_encode(&bytes)}));
    assert!(!back.status.contains("edited in Word"), "{}", back.status);
    for (r, linear) in regions(&s) {
        assert_eq!(region(&back, &r.source).1, linear, "{}", r.source);
    }
    retype_cell(&mut back, 2, 1, "16.7");
    shows(&back, "S_allow", "=16.7 \"ksi\"");
    shows(&back, "Y", "=0.4");
    shows(&back, "Σ_S", "=36.7 \"ksi\"");
}

/// Every new display, saved to `.docx` and reopened: and/or/not, `==`, `≠`, quoted text with a
/// space, `if(…)`, a three-case brace and a parenthesised sum body read back unchanged and live.
#[test]
fn conditions_text_and_sums_reopen_unchanged() {
    let mut s = session();
    for source in [
        "a := 2",
        "b := (a > 1 and a < 3) or not (a == 2) =",
        "c := if(a ≠ 2, \"x y\", \"z\") =",
        "d := ∑(i = 1..3) (i + a) =",
        "f_2 := 1 if a < 0, 2 if a < 1, 3 otherwise =",
    ] {
        calc(&mut s, source);
    }
    shows(&s, "b", "≔a>1∧a<3∨¬(a==2)=1");
    shows(&s, "c", "≔if(a≠2,\"x y\",\"z\")=\"z\"");
    shows(&s, "d", "≔∑_(i=1)^3▒〖(i+a)〗=12");
    shows(&s, "f_2", "≔{■(1&\"if \"a<0@2&\"if \"a<1@3&\"otherwise\")┤=3");
    let bytes = crate::io::save_bytes("conditions.docx", &s.doc).unwrap();
    let mut back = session();
    run(&mut back, "file.open", json!({"path": "conditions.docx", "data": crate::cmd::insert::base64_encode(&bytes)}));
    assert!(!back.status.contains("edited in Word"), "{}", back.status);
    assert_eq!(regions(&back).len(), 5);
    for (r, linear) in regions(&s) {
        let (b, back_linear) = region(&back, &r.source);
        assert_eq!(b.state, CalcState::Fresh, "{}", r.source);
        assert_eq!(back_linear, linear, "{}", r.source);
    }
}

// ---------------------------------------------------------------------------------------------
// Flange torque by pipe size
// ---------------------------------------------------------------------------------------------

#[test]
fn flange_torque_sample() {
    let mut s = session();
    data_table(&mut s, &[&["NPS (in)", "T_flange (ft·lbf)"], &["1/2", "30"], &["3/4", "45"], &["1", "60"], &["1-1/2", "90"]]);
    calc(&mut s, "lookup(NPS, T_flange, 1.5 in) = ? ft·lbf");
    calc(&mut s, "linterp(NPS, T_flange, 1.25 in) = ? ft·lbf");
    calc(&mut s, "linterp(NPS, T_flange, 1.6 in) =");
    assert_eq!(table_mark(&s).map(|c| c.errors), Some(Vec::new()));
    shows(&s, "lookup", "=90 \"ft·lbf\"");
    shows(&s, "linterp(NPS, T_flange, 1.25", "=75 \"ft·lbf\"");
    let (r, _) = region(&s, "linterp(NPS, T_flange, 1.6");
    assert_eq!(r.state, CalcState::Error("1.6 in is outside NPS (0.5 in to 1.5 in)".into()));
}
