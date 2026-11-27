//! The recalculation pass over a whole document: collection order, unique names, dependency
//! order, cycles, quoted values and the displayed result.

use craft_units::Formatted;
use std::sync::Arc;
use std::time::Instant;
use wordcraft_calc::{
    CalcError, calc_unchanged, check_new_name, check_quote_index, check_quote_unit, data_tables, defined_columns, defined_names, mark_stale, recalc,
    render,
};
use wordcraft_doc::calc::{CalcRegion, CalcSettings, CalcState, CalcTable, ColumnError, GMeans, QuoteSpec, calc_field_instr};
use wordcraft_doc::math::{MNode, MRun, MSty, Math};
use wordcraft_doc::props::VMerge;
use wordcraft_doc::{Block, Cell, CharProps, Document, InlineObject, Paragraph, Rgb, Row, Table, para_block};

const ERROR_RED: Rgb = Rgb(0xC0, 0x00, 0x00);

fn calc_eq(id: u64, source: &str) -> InlineObject {
    InlineObject::Equation {
        linear: String::new(),
        display: false,
        math: Math::default(),
        calc: Some(Box::new(CalcRegion {
            result_format: None,
            worksheet_order: None,
            id,
            source: source.into(),
            unit: None,
            precision: None,
            numbered: false,
            state: CalcState::Fresh,
        })),
    }
}

fn field(instr: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: "stale".into(), locked: true, dirty: false, code: false, form: String::new() }
}

fn para(objs: Vec<InlineObject>) -> Paragraph {
    para_with(objs, &CharProps::default())
}

fn para_with(objs: Vec<InlineObject>, props: &CharProps) -> Paragraph {
    let mut p = Paragraph::with_text("Text ", CharProps::default());
    for o in objs {
        let off = p.text.len();
        p.insert_object(off, o, props).unwrap();
    }
    p
}

fn doc_of(blocks: Vec<Arc<Block>>) -> Document {
    let mut d = Document::new();
    d.body = blocks;
    d
}

/// One region per paragraph, ids 1, 2, 3…
fn doc_with(sources: &[&str]) -> Document {
    doc_of(sources.iter().enumerate().map(|(i, s)| para_block(para(vec![calc_eq(i as u64 + 1, s)]))).collect())
}

/// Every equation and field in the body, tables included, in document order.
fn objects(doc: &Document) -> Vec<InlineObject> {
    fn walk(blocks: &[Arc<Block>], out: &mut Vec<InlineObject>) {
        for b in blocks {
            match &**b {
                Block::Para(p) => out.extend(p.objects.iter().cloned()),
                Block::Table(t) => {
                    for row in &t.rows {
                        for cell in &row.cells {
                            walk(&cell.blocks, out);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.body, &mut out);
    out
}

/// (region, linear, math) of every calculation region in the body.
fn regions(doc: &Document) -> Vec<(CalcRegion, String, Math)> {
    objects(doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, math, calc: Some(c), .. } => Some((*c, linear, math)),
            _ => None,
        })
        .collect()
}

fn region(doc: &Document, id: u64) -> (CalcRegion, String, Math) {
    regions(doc).into_iter().find(|(r, ..)| r.id == id).unwrap()
}

fn field_results(doc: &Document) -> Vec<String> {
    objects(doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect()
}

/// Every run in a math tree, at any depth.
fn runs(nodes: &[MNode]) -> Vec<MRun> {
    let mut out = Vec::new();
    let mut todo: Vec<&MNode> = nodes.iter().collect();
    while let Some(n) = todo.pop() {
        match n {
            MNode::Run(r) => out.push(r.clone()),
            MNode::Frac { num, den, .. } => todo.extend(num.iter().chain(den.iter())),
            MNode::Script { base, sub, sup, .. } => todo.extend(base.iter().chain(sub.iter()).chain(sup.iter())),
            MNode::Rad { deg, e, .. } => todo.extend(deg.iter().chain(e.iter())),
            MNode::Delim { elems, .. } => todo.extend(elems.iter().flatten()),
            MNode::Func { name, e } => todo.extend(name.iter().chain(e.iter())),
            MNode::Matrix { rows, .. } => todo.extend(rows.iter().flatten().flatten()),
            _ => {}
        }
    }
    out
}

/// Change the region `id` (and its displayed text) in a document built by [`doc_with`].
fn edit_region(d: &mut Document, id: u64, f: impl Fn(&mut CalcRegion, &mut String)) {
    for b in &mut d.body {
        if let Block::Para(p) = Arc::make_mut(b) {
            for o in &mut p.objects {
                if let InlineObject::Equation { linear, calc: Some(c), .. } = o
                    && c.id == id
                {
                    f(c, linear);
                }
            }
        }
    }
}

/// The code of a quoted value of `name`.
fn quote(name: &str, unit: Option<&str>, precision: Option<u8>, index: Option<(u32, Option<u32>)>) -> String {
    calc_field_instr(&QuoteSpec { name: name.into(), unit: unit.map(str::to_string), precision, index })
}

fn no_page(_: u64) -> Option<u32> {
    None
}

fn err(e: CalcError) -> CalcState {
    CalcState::Error(e.to_string())
}

#[test]
fn recalculates_in_order() {
    let mut d = doc_with(&["P := 1500 lbf", "A_b := 2.5 in^2", "σ := P/A_b ="]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, 3);
    assert_eq!(report.errors, 0);
    let (r, linear, _) = region(&d, 3);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=600 \"psi\""), "{linear}");
    assert_eq!(linear, "σ≔P/A_b=600 \"psi\"");
    let (_, p_linear, p_math) = region(&d, 1);
    assert_eq!(p_linear, "P≔1500 \"lbf\"");
    // The unit is an upright run of its own, without the quotes of the linear form.
    let p_runs = runs(&p_math.nodes);
    assert!(p_runs.iter().any(|r| r.text.trim() == "lbf" && r.sty == Some(MSty::Plain)), "{p_runs:?}");
    assert!(p_runs.iter().all(|r| !r.text.contains('"')), "{p_runs:?}");
}

#[test]
fn use_before_define_works() {
    let mut d = doc_with(&["σ := P/A_b =", "P := 1500 lbf", "A_b := 2.5 in^2"]);
    recalc(&mut d, &no_page);
    let (r, linear, _) = region(&d, 1);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=600 \"psi\""), "{linear}");
}

#[test]
fn duplicate_definition_is_error() {
    let mut d = doc_with(&["x:=1", "x =", "x:=2"]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.errors, 1);
    assert_eq!(region(&d, 1).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 3).0.state, CalcState::Error("x is already defined elsewhere in the document — choose a unique name".into()));
    let (r, linear, _) = region(&d, 2);
    assert_eq!(r.state, CalcState::Fresh);
    assert_eq!(linear, "x=1");
}

#[test]
fn duplicate_message_uses_page_lookup() {
    let mut d = doc_with(&["x:=1", "x:=2"]);
    recalc(&mut d, &|id| (id == 1).then_some(4));
    assert_eq!(region(&d, 2).0.state, CalcState::Error("x is already defined on page 4 — choose a unique name".into()));
}

#[test]
fn circular_definitions_error() {
    let mut d = doc_with(&["a := b + 1", "b := a/2", "c := a ="]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.errors, 3);
    assert_eq!(region(&d, 1).0.state, CalcState::Error("a, b depend on each other".into()));
    assert_eq!(region(&d, 2).0.state, CalcState::Error("a, b depend on each other".into()));
    assert_eq!(region(&d, 3).0.state, err(CalcError::Upstream));
}

#[test]
fn self_reference_and_mutual_functions_are_circular() {
    let mut d = doc_with(&["x := x + 1", "f_a(t) := f_b(t)", "f_b(t) := f_a(t)", "y := 2 =", "z := f_a(1) + y ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::Circular("x".into())));
    assert_eq!(region(&d, 2).0.state, err(CalcError::Circular("f_a, f_b".into())));
    assert_eq!(region(&d, 3).0.state, err(CalcError::Circular("f_a, f_b".into())));
    assert_eq!(region(&d, 4).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 5).0.state, err(CalcError::Upstream));
}

#[test]
fn a_calculation_between_two_cycles_is_upstream_not_circular() {
    let mut d = doc_with(&["a := b", "b := a", "m_id := a + 1", "c := d + m_id", "d := c"]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, err(CalcError::Upstream));
    assert_eq!(region(&d, 4).0.state, err(CalcError::Circular("c, d".into())));
}

#[test]
fn pasted_unit_name_definition_is_error() {
    let mut d = doc_with(&["A := 2 in^2", "2 A ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::ReservedName("A".into())));
    // The document meant its own A, so the ampere would be a wrong number: the use fails too.
    let (r, linear, _) = region(&d, 2);
    assert_eq!(r.state, err(CalcError::Upstream));
    assert_eq!(linear, "2 \"A\" ⚠");
}

/// A definition refused because its name is a unit, function or constant (a document from before
/// the name was reserved) fails every calculation using the name, wherever it is, rather than
/// letting them quietly compute with the unit.
#[test]
fn a_name_refused_as_a_unit_function_or_constant_fails_its_uses() {
    let cases: &[(&str, &str, &str)] = &[
        ("day := 7", "x := 2·day =", "day"),
        ("ton := 2000", "W_t := 3·ton =", "ton"),
        ("hr := 0.5", "y := hr·2 =", "hr"),
        ("m := 5", "y := m·2 =", "m"),
        ("m := 5", "y := 3 m =", "m"),
        ("pi := 3", "y := 2·pi =", "pi"),
        ("g_0 := 32", "y := 2·g_0 =", "g_0"),
        // A function's name, the same way: the use points at the failed definition.
        ("slope := 0.02", "y := slope·100 ft =", "slope"),
        ("max := 10", "y := max(1, 2) =", "max"),
        // The solver's words, the same way: a use is never a solve.
        ("Find := 1", "y := Find(x) =", "Find"),
        ("Find := 1", "Find(x) =", "Find"),
        ("Minerr := 1", "y := 2·Minerr(x) =", "Minerr"),
        ("Given := 1", "y := Given + 1 =", "Given"),
        ("root := 2", "y := root(x^2 − 2, x) =", "root"),
    ];
    for &(define, using, name) in cases {
        for sources in [[define, using], [using, define]] {
            let mut d = doc_with(&sources);
            let report = recalc(&mut d, &no_page);
            let (at_define, at_use) = if sources[0] == define { (1, 2) } else { (2, 1) };
            assert_eq!(region(&d, at_define).0.state, err(CalcError::ReservedName(name.into())), "{sources:?}");
            let (r, linear, _) = region(&d, at_use);
            assert_eq!(r.state, err(CalcError::Upstream), "{sources:?}: {linear}");
            // The error mark, and no result.
            assert!(linear.ends_with(" ⚠") && !linear.contains('='), "{sources:?}: {linear}");
            assert_eq!(report.errors, 2, "{sources:?}");
        }
    }
    // A block's result too, when the document defines its Find.
    let mut d = doc_with(&["Find := 1", "x := 1", "Given", "x^2 == 2", "x_s := Find(x) ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 5).0.state, err(CalcError::Upstream));
    // The name stays refused, and is no variable to quote.
    let d = doc_with(&["day := 7", "x := 2·day"]);
    assert_eq!(check_new_name(&d, "day", None), Err(CalcError::ReservedName("day".into())));
    assert!(defined_names(&d).iter().all(|(n, _)| n != "day"), "{:?}", defined_names(&d));
}

#[test]
fn a_column_heading_refused_as_a_unit_fails_its_uses() {
    let mut d = doc_of(vec![table_block(data_table(1, &[&["day (hr)"], &["1"], &["2"]])), para_block(para(vec![calc_eq(8, "x := 2·day =")]))]);
    recalc(&mut d, &no_page);
    let message = CalcError::ReservedName("day".into()).to_string();
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(0), message }]);
    let (r, linear, _) = region(&d, 8);
    assert_eq!(r.state, err(CalcError::UpstreamTable), "{linear}");
}

#[test]
fn quoted_value_updates() {
    let mut d = doc_of(vec![
        para_block(para(vec![field("WCCALC σ")])),
        para_block(para(vec![calc_eq(1, "P := 1500 lbf"), calc_eq(2, "A_b := 2.5 in^2")])),
        para_block(para(vec![calc_eq(3, "σ := P/A_b")])),
        para_block(para(vec![field("WCCALC σ"), field(&quote("σ", Some("ksi"), Some(2), None))])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["600 psi", "600 psi", "0.6 ksi"]);
}

#[test]
fn quoted_value_without_a_valid_value_is_a_question_mark() {
    let mut d = doc_of(vec![
        para_block(para(vec![calc_eq(1, "x := 1/0"), calc_eq(2, "f(t) := t"), calc_eq(3, "len := 2 in")])),
        para_block(para(vec![
            field("WCCALC x"),
            field("WCCALC nope"),
            field("WCCALC m"),
            field("WCCALC f"),
            field("WCCALC len \\u lbf"),
            field("WCCALC len \\u frob"),
            field("WCCALC len"),
            field("PAGE"),
        ])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["?", "?", "?", "?", "?", "?", "2 in", "stale"]);
}

#[test]
fn quoted_value_clears_dirty_flag() {
    let mut d = doc_of(vec![para_block(para(vec![
        calc_eq(1, "x := 3"),
        InlineObject::Field { instr: "WCCALC x".into(), result: "3".into(), locked: true, dirty: true, code: false, form: String::new() },
    ]))]);
    recalc(&mut d, &no_page);
    assert!(matches!(&objects(&d)[1], InlineObject::Field { result, dirty: false, locked: true, .. } if result == "3"));
}

#[test]
fn check_new_name_reports() {
    let d = doc_with(&["P := 1500 lbf", "Q_x := 2"]);
    let p_id = 1;
    assert_eq!(check_new_name(&d, "P", None), Err(CalcError::Duplicate { name: "P".into(), page: None }));
    assert_eq!(check_new_name(&d, "P", Some(p_id)), Ok(()));
    assert_eq!(check_new_name(&d, "m", None), Err(CalcError::ReservedName("m".into())));
    assert_eq!(check_new_name(&d, "Q", None), Ok(()));
}

#[test]
fn defined_names_lists_owners_sorted() {
    let d = doc_with(&["b := 1", "a := 2", "b := 3", "m := 1", "a + b =", "f(x) := x"]);
    // A function has no single value to quote, so it isn't listed.
    assert_eq!(defined_names(&d), [("a".to_string(), 2), ("b".to_string(), 1)]);
    // Its name is still taken.
    assert_eq!(check_new_name(&d, "f", None), Err(CalcError::Duplicate { name: "f".into(), page: None }));
}

#[test]
fn names_use_the_ids_recalculation_gives() {
    // A pasted copy kept the id 7 of the region above it.
    let mut d = doc_with(&["Q := 1", "P := 2"]);
    edit_region(&mut d, 2, |c, _| c.id = 7);
    edit_region(&mut d, 1, |c, _| c.id = 7);
    // Renaming the first region (id 7) to P clashes with the pasted region, which recalculation
    // gives id 8.
    assert_eq!(check_new_name(&d, "P", Some(7)), Err(CalcError::Duplicate { name: "P".into(), page: None }));
    assert_eq!(check_new_name(&d, "P", Some(8)), Ok(()));
    assert_eq!(defined_names(&d), [("P".to_string(), 8), ("Q".to_string(), 7)]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 8).0.source, "P := 2");
}

#[test]
fn a_paragraph_with_regions_and_fields_updates_both() {
    let mut d = doc_of(vec![para_block(para(vec![calc_eq(1, "x := 3 ="), field("WCCALC x")]))]);
    let before = match &*d.body[0] {
        Block::Para(p) => p.rev,
        Block::Table(_) => unreachable!(),
    };
    recalc(&mut d, &no_page);
    let Block::Para(p) = &*d.body[0] else { unreachable!() };
    assert_ne!(p.rev, before);
    assert_eq!(field_results(&d), ["3"]);
    assert_eq!(region(&d, 1).1, "x≔3=3");
}

#[test]
fn table_cells_in_row_order() {
    let cell = |id: u64, src: &str| Cell { blocks: vec![para_block(para(vec![calc_eq(id, src)]))], ..Cell::empty() };
    let table = Table {
        rows: vec![
            Row { cells: vec![cell(1, "a := 10"), cell(2, "b := 1")], ..Default::default() },
            Row { cells: vec![cell(3, "b := 2"), cell(4, "c := a + b =")], ..Default::default() },
        ],
        grid: vec![100.0, 100.0],
        ..Default::default()
    };
    let mut d = doc_of(vec![Arc::new(Block::Table(table)), para_block(para(vec![calc_eq(5, "b =")]))]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, 5);
    assert_eq!(region(&d, 2).0.state, CalcState::Fresh);
    assert!(matches!(region(&d, 3).0.state, CalcState::Error(m) if m.contains("already defined")));
    assert_eq!(region(&d, 4).1, "c≔a+b=11");
    assert_eq!(region(&d, 5).1, "b=1");
}

#[test]
fn duplicate_ids_are_reassigned() {
    let mut d = doc_of(vec![para_block(para(vec![calc_eq(7, "x := 1"), calc_eq(3, "y := 2")])), para_block(para(vec![calc_eq(7, "z := 3")]))]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.reassigned_ids, 1);
    let ids: Vec<u64> = regions(&d).iter().map(|(r, ..)| r.id).collect();
    assert_eq!(ids, [7, 3, 8]);
    assert_eq!(region(&d, 8).0.source, "z := 3");
    // A second pass finds nothing to fix.
    assert_eq!(recalc(&mut d, &no_page).reassigned_ids, 0);
}

#[test]
fn duplicate_ids_at_the_top_of_the_range_get_unused_ids() {
    let mut d = doc_with(&["x := 1", "y := 2", "z := 3"]);
    for id in 1..=3 {
        edit_region(&mut d, id, |c, _| c.id = u64::MAX);
    }
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.reassigned_ids, 2);
    let mut ids: Vec<u64> = regions(&d).iter().map(|(r, ..)| r.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "{ids:?}");
}

#[test]
fn tracked_deleted_region_is_ignored() {
    let mut d = Document::new();
    d.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "me", "2026-11-14T00:00:00Z"));
    let deleted = CharProps { del: Some(0), ..Default::default() };
    let mut gone = calc_eq(1, "x := 1");
    if let InlineObject::Equation { linear, .. } = &mut gone {
        *linear = "kept as is".into();
    }
    d.body = vec![para_block(para_with(vec![gone, field("WCCALC x")], &deleted)), para_block(para(vec![calc_eq(2, "x := 2"), calc_eq(3, "x =")]))];
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, 2);
    assert_eq!(report.errors, 0);
    let (r, linear, _) = region(&d, 1);
    assert_eq!((r.state, linear.as_str()), (CalcState::Fresh, "kept as is"));
    assert_eq!(region(&d, 2).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 3).1, "x=2");
    assert_eq!(field_results(&d), ["stale"], "a deleted quoted value is left alone");
}

#[test]
fn plain_equations_untouched() {
    let plain = InlineObject::Equation {
        linear: "a=b/c".into(),
        display: true,
        math: Math { nodes: wordcraft_doc::math::parse_linear("a=b/c"), omml: "<m:oMath/>".into(), ..Default::default() },
        calc: None,
    };
    let mut d = doc_of(vec![para_block(para(vec![plain.clone(), calc_eq(1, "x := 1 =")])), para_block(para(vec![plain.clone()]))]);
    let before = format!("{:?}", d.body[1]);
    recalc(&mut d, &no_page);
    assert_eq!(objects(&d)[0], plain);
    assert_eq!(format!("{:?}", objects(&d)[0]), format!("{plain:?}"));
    assert_eq!(format!("{:?}", d.body[1]), before);
}

#[test]
fn unchanged_paragraphs_are_not_copied() {
    let mut d = doc_with(&["x := 1 =", "y := 2"]);
    recalc(&mut d, &no_page);
    let snapshot = d.body.clone();
    recalc(&mut d, &no_page);
    for (a, b) in snapshot.iter().zip(&d.body) {
        assert!(Arc::ptr_eq(a, b), "a recalculation that changes nothing copies nothing");
    }
}

#[test]
fn edited_outside_region_keeps_its_display_marked_and_defines_nothing() {
    let mut d = doc_with(&["x := 5", "y := x + 1 =", "y + 1 =", "x := 7", "z := 2 ="]);
    edit_region(&mut d, 1, |c, linear| {
        *linear = "x≔6".into();
        c.state = CalcState::EditedOutside;
    });
    if let Block::Para(p) = Arc::make_mut(&mut d.body[4]) {
        let end = p.text.len();
        p.insert_object(end, field(&quote("x", None, None, None)), &CharProps::default()).unwrap();
        let end = p.text.len();
        p.insert_object(end, field(&quote("y", None, None, None)), &CharProps::default()).unwrap();
    }
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.edited, 1);
    let (r, linear, math) = region(&d, 1);
    assert_eq!((r.state, linear.as_str()), (CalcState::EditedOutside, "x≔6 ⚠"));
    assert!(runs(&math.nodes).iter().any(|r| r.text == "⚠" && r.color == Some(render::EDITED_AMBER)), "marked in amber");
    assert!(runs(&math.nodes).iter().any(|r| r.text.contains('6')), "Word's equation is kept");
    // Its dependents, near and far, show no number.
    let upstream = err(CalcError::UpstreamEdited);
    assert_eq!(region(&d, 2).0.state, upstream);
    assert_eq!(region(&d, 3).0.state, upstream);
    assert!(!region(&d, 2).1.contains('6') && region(&d, 2).1.ends_with('⚠'), "{}", region(&d, 2).1);
    // It still owns x: a later definition is a duplicate, not a silent replacement.
    assert!(matches!(region(&d, 4).0.state, CalcState::Error(m) if m.contains("already defined")));
    assert_eq!(region(&d, 5).0.state, CalcState::Fresh);
    let quoted: Vec<String> = objects(&d)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(quoted, vec!["?", "?"]);
    // A second pass marks it once.
    let snapshot = d.body.clone();
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).1, "x≔6 ⚠");
    assert!(snapshot.iter().zip(&d.body).all(|(a, b)| Arc::ptr_eq(a, b)), "nothing changes the second time");
}

#[test]
fn errors_show_a_red_marker_and_no_number() {
    let mut d = doc_with(&["y := q + 1 =", "z := 1 +"]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.errors, 2);
    let (r, linear, math) = region(&d, 1);
    assert_eq!(r.state, err(CalcError::Undefined("q".into())));
    assert_eq!(linear, "y≔q+1 ⚠");
    assert!(runs(&math.nodes).iter().any(|r| r.text.contains('⚠') && r.color == Some(ERROR_RED)));
    // A parse error shows the source as typed.
    let (r, linear, math) = region(&d, 2);
    assert!(matches!(r.state, CalcState::Error(_)));
    assert_eq!(linear, "z := 1 + ⚠");
    assert!(runs(&math.nodes).iter().any(|r| r.text.contains('⚠') && r.color == Some(ERROR_RED)));
}

#[test]
fn region_unit_and_precision_apply() {
    let mut d = doc_with(&["P := 1500 lbf", "A_b := 2.5 in^2", "σ := P/A_b =", "σ = ? ksi", "P/3 ="]);
    edit_region(&mut d, 3, |c, _| c.unit = Some("ksi".into()));
    edit_region(&mut d, 5, |c, _| c.precision = Some(1));
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).1, "σ≔P/A_b=0.6 \"ksi\"");
    assert_eq!(region(&d, 4).1, "σ=0.6 \"ksi\"");
    assert_eq!(region(&d, 5).1, "P/3=500 \"lbf\"");
}

#[test]
fn document_precision_is_clamped() {
    let mut d = doc_with(&["half := 0.5 =", "third := 2/3 ="]);
    d.calc_settings = Some(CalcSettings { precision: 40, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).1, "half≔0.5=0.5");
    assert_eq!(region(&d, 2).1, "third≔2/3=0.666666666666667");
}

#[test]
fn region_unit_of_the_wrong_dimension_is_an_error() {
    let mut d = doc_with(&["x := 2 in ="]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0])
        && let Some(InlineObject::Equation { calc: Some(c), .. }) = p.objects.first_mut()
    {
        c.unit = Some("lbf".into());
    }
    recalc(&mut d, &no_page);
    let (r, linear, _) = region(&d, 1);
    assert!(matches!(r.state, CalcState::Error(_)));
    assert!(!linear.contains('='), "{linear}");
}

#[test]
fn si_settings_show_si_units() {
    let mut d = doc_with(&["F := 1 kN ="]);
    d.calc_settings = Some(CalcSettings { system: wordcraft_doc::calc::UnitSystem::Si, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).1, "F≔1 \"kN\"=1000 \"N\"");
}

#[test]
fn mark_stale_greys_results_with_approx() {
    let mut d = doc_with(&["P := 1500 lbf", "A_b := 2.5 in^2", "σ := P/A_b =", "q =", "big := 6000000 ="]);
    recalc(&mut d, &no_page);
    mark_stale(&mut d);
    let (r, linear, math) = region(&d, 3);
    assert_eq!(r.state, CalcState::Stale);
    assert_eq!(linear, "σ≔P/A_b≈600 \"psi\"");
    assert!(!linear.contains('='));
    let grey: Vec<MRun> = runs(&math.nodes).into_iter().filter(|r| r.color.is_some_and(|c| c != ERROR_RED)).collect();
    assert!(grey.iter().any(|r| r.text.contains('≈')), "{grey:?}");
    assert!(grey.iter().any(|r| r.text.contains("600")), "{grey:?}");
    // A definition with no shown value goes stale but has no number to mark.
    assert_eq!(region(&d, 1).0.state, CalcState::Stale);
    // Errors stay errors.
    assert!(matches!(region(&d, 4).0.state, CalcState::Error(_)));
    // Exponential results survive the round trip through the display.
    let (_, big, _) = region(&d, 5);
    assert!(big.contains('≈') && big.contains("6×〖10〗^6") && !big.contains('='), "{big}");
    // Recalculating makes them current again.
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 3).1, "σ≔P/A_b=600 \"psi\"");
}

#[test]
fn mark_stale_marks_quoted_values_once() {
    let mut d = doc_of(vec![
        para_block(para(vec![calc_eq(1, "P := 1500 lbf"), calc_eq(2, "A_b := 2.5 in^2"), calc_eq(3, "σ := P/A_b")])),
        para_block(para(vec![field("WCCALC σ"), field("WCCALC nope")])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["600 psi", "?"]);
    mark_stale(&mut d);
    assert_eq!(field_results(&d), ["≈ 600 psi", "?"]);
    mark_stale(&mut d);
    assert_eq!(field_results(&d), ["≈ 600 psi", "?"]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["600 psi", "?"]);
}

#[test]
fn mark_stale_without_a_readable_result_shows_a_grey_question_mark() {
    let mut d = doc_with(&["x := 2 ="]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0])
        && let Some(InlineObject::Equation { linear, .. }) = p.objects.first_mut()
    {
        *linear = "garbage".into();
    }
    mark_stale(&mut d);
    let (r, linear, _) = region(&d, 1);
    assert_eq!(r.state, CalcState::Stale);
    assert_eq!(linear, "x≔2≈?");
}

#[test]
fn display_renders_from_the_syntax_tree() {
    let f = |number: &str, unit: &str| Formatted { number: number.into(), unit: unit.into() };
    assert_eq!(render::display("σ := P/A_b", None, &CalcState::Fresh), "σ≔P/A_b");
    assert_eq!(render::display("σ := P/A_b =", Some(&f("600", "psi")), &CalcState::Fresh), "σ≔P/A_b=600 \"psi\"");
    assert_eq!(render::display("x =", Some(&f("1", "")), &CalcState::Fresh), "x=1");
    assert_eq!(render::display("σ := P/A_b =", Some(&f("600", "psi")), &CalcState::Error("bad".into())), "σ≔P/A_b ⚠");
    assert_eq!(render::display("σ := P/A_b =", Some(&f("600", "psi")), &CalcState::Stale), "σ≔P/A_b≈600 \"psi\"");
    assert_eq!(render::display("x := 1 +", None, &CalcState::Error("bad".into())), "x := 1 + ⚠");
    // √x² parses as (√x)²: the display says so.
    assert_eq!(render::display("y := √x²", None, &CalcState::Fresh), "y≔(√x)^2");
    assert_eq!(render::display("y := √(x+1)", None, &CalcState::Fresh), "y≔√(x+1)");
    assert_eq!(render::display("A_b := 2.5 in^2", None, &CalcState::Fresh), "A_b≔2.5 \"in\"^2");
    assert_eq!(render::display("M := 5 kN*m", None, &CalcState::Fresh), "M≔5 \"kN\"·\"m\"");
    assert_eq!(render::display("f(x, y) := x^2 + y", None, &CalcState::Fresh), "f(x,y)≔x^2+y");
    assert_eq!(render::display("f_a(t) := t^2", None, &CalcState::Fresh), "f_a(t)≔t^2");
    assert_eq!(render::display("y := f_a(2) =", Some(&f("4", "")), &CalcState::Fresh), "y≔f_a(2)=4");
    assert_eq!(render::display("z := a*b/c", None, &CalcState::Fresh), "z≔(a·b)/c");
    assert_eq!(render::display("z := a/(b*c)", None, &CalcState::Fresh), "z≔a/(b·c)");
    assert_eq!(render::display("z := a - (b - c)", None, &CalcState::Fresh), "z≔a-(b-c)");
    assert_eq!(render::display("z := (a + b)*c", None, &CalcState::Fresh), "z≔(a+b)·c");
    assert_eq!(render::display("z := -(a + b)", None, &CalcState::Fresh), "z≔-(a+b)");
    assert_eq!(render::display("z := (-a)^2", None, &CalcState::Fresh), "z≔(-a)^2");
    assert_eq!(render::display("z := -a^2", None, &CalcState::Fresh), "z≔-a^2");
    assert_eq!(render::display("z := (a^b)^c", None, &CalcState::Fresh), "z≔(a^b)^c");
    assert_eq!(render::display("z := a^b^c", None, &CalcState::Fresh), "z≔a^(b^c)");
    assert_eq!(render::display("z := x^(-1)", None, &CalcState::Fresh), "z≔x^(-1)");
    assert_eq!(render::display("z := x^0.5", None, &CalcState::Fresh), "z≔x^(0.5)");
    assert_eq!(render::display("z := |a - b|", None, &CalcState::Fresh), "z≔|a-b|");
    assert_eq!(render::display("z := sin(θ) + max(a, b)", None, &CalcState::Fresh), "z≔sin(θ)+max((a,b))");
    assert_eq!(render::display("z := 1e300", None, &CalcState::Fresh), "z≔1×〖10〗^300");
    assert_eq!(render::display("z := 1.5e-7 m", None, &CalcState::Fresh), "z≔1.5×〖10〗^(-7) \"m\"");
    assert_eq!(
        render::display("z := 1.5×10^-5 m =", Some(&f("1.5×10^-5", "m")), &CalcState::Fresh).rsplit('=').next(),
        Some("1.5×〖10〗^(-5) \"m\"")
    );
}

#[test]
fn vectors_indexes_and_ranges_display_as_matrices_and_subscripts() {
    let shown = |src: &str| render::display(src, None, &CalcState::Fresh);
    assert_eq!(shown("D := [0.840, 1.050] in"), "D≔[■(0.84@1.05)] \"in\"");
    assert_eq!(shown("M := [1, 2; 3, 4]"), "M≔[■(1&2@3&4)]");
    assert_eq!(shown("x := D[2]"), "x≔D_2");
    assert_eq!(shown("y := M[i, j]"), "y≔M_(i,j)");
    assert_eq!(shown("z := D_o[i]"), "z≔〖D_o〗_i");
    assert_eq!(shown("i := 0..5"), "i≔0..5");
    assert_eq!(shown("t[i] := D[i]"), "t_i≔D_i");
    // Elements, indexes and range ends keep their own structure.
    assert_eq!(shown("v := [2 in, 50 mm]"), "v≔[■(2 \"in\"@50 \"mm\")]");
    assert_eq!(shown("v := [a + b, -c]"), "v≔[■(a+b@-c)]");
    assert_eq!(shown("x := D[i + 1]"), "x≔D_(i+1)");
    assert_eq!(shown("x := D[i_0]"), "x≔D_(i_0)");
    assert_eq!(shown("x := D[i]^2"), "x≔(D_i)^2");
    assert_eq!(shown("x := 2 D[0]"), "x≔2·D_0");
    assert_eq!(shown("x := 1, 1.5..3"), "x≔1,1.5..3");
    assert_eq!(shown("x := 0 in, 0.5 in..1 in"), "x≔0 \"in\",0.5 \"in\"..1 \"in\"");
    assert_eq!(shown("i := 0..n - 1"), "i≔0..n-1");
    assert_eq!(shown("i := 0..(n - 1)"), "i≔0..n-1");
    assert_eq!(shown("x := 1, 1 + 0.5..a/b"), "x≔1,1+0.5..(a/b)");
    // The value before `..` is kept whole when it ends in a script, so the dots stay out of it.
    assert_eq!(shown("x := D[0]..5"), "x≔〖D_0〗..5");
    assert_eq!(shown("x := 10^2..10^3"), "x≔〖〖10〗^2〗..〖10〗^3");
    assert_eq!(shown("x := D[0], D[1]..D[5]"), "x≔D_0,〖D_1〗..D_5");
    assert_eq!(shown("x := 1, -x^2..5"), "x≔1,〖-x^2〗..5");
    assert_eq!(shown("x := a_1..5"), "x≔〖a_1〗..5");
    assert_eq!(shown("t_o[k] := k"), "〖t_o〗_k≔k");
    assert_eq!(shown("t[i_0] := 1"), "t_(i_0)≔1");
}

#[test]
fn a_literal_is_a_bracketed_matrix_and_an_index_a_subscript_in_the_math() {
    // The evaluator doesn't take these yet; the math is drawn from the source all the same.
    let mut d = doc_with(&["D := [0.840, 1.050] in", "z := D_o[i]"]);
    recalc(&mut d, &no_page);
    let (_, _, math) = region(&d, 1);
    let bracket = math
        .nodes
        .iter()
        .position(|n| matches!(n, MNode::Delim { elems, .. } if matches!(elems.as_slice(), [e] if matches!(e.as_slice(), [MNode::Matrix { .. }]))));
    let bracket = bracket.unwrap_or_else(|| panic!("{:#?}", math.nodes));
    // The unit after the bracket is upright and set apart from it.
    match math.nodes.get(bracket + 1) {
        Some(MNode::Run(r)) => assert!(r.text == "\u{2009}in" && r.sty == Some(MSty::Plain), "{r:?}"),
        other => panic!("{other:?}"),
    }
    // `〖D_o〗_i`: the index lowers the whole subscripted name.
    let (_, _, math) = region(&d, 2);
    let script = math.nodes.iter().find_map(|n| match n {
        MNode::Script { base, sub, .. } => Some((base, sub)),
        _ => None,
    });
    let Some((base, sub)) = script else { panic!("{:#?}", math.nodes) };
    assert!(matches!(base.as_slice(), [MNode::Script { .. }]), "{base:#?}");
    assert!(matches!(sub.as_slice(), [MNode::Run(r)] if r.text == "i"), "{sub:#?}");
}

/// Every sub- and superscript in `nodes`, at any depth.
fn scripts(nodes: &[MNode]) -> Vec<(Vec<MNode>, Vec<MNode>)> {
    let mut out = Vec::new();
    let mut todo: Vec<&MNode> = nodes.iter().collect();
    while let Some(n) = todo.pop() {
        match n {
            MNode::Script { base, sub, sup, .. } => {
                out.push((sub.clone(), sup.clone()));
                todo.extend(base.iter().chain(sub.iter()).chain(sup.iter()));
            }
            MNode::Frac { num, den, .. } => todo.extend(num.iter().chain(den.iter())),
            MNode::Rad { deg, e, .. } => todo.extend(deg.iter().chain(e.iter())),
            MNode::Delim { elems, .. } => todo.extend(elems.iter().flatten()),
            MNode::Func { name, e } => todo.extend(name.iter().chain(e.iter())),
            _ => {}
        }
    }
    out
}

#[test]
fn a_range_keeps_its_dots_out_of_scripts_in_the_math() {
    let sources = ["x := D[0]..5", "x := 10^2..10^3", "x := D[0], D[1]..D[5]", "x := a_1..5", "x := 1e20..1e21"];
    let mut d = doc_with(&sources);
    recalc(&mut d, &no_page);
    for (k, src) in sources.iter().enumerate() {
        let (_, linear, math) = region(&d, k as u64 + 1);
        let all = scripts(&math.nodes);
        assert!(!all.is_empty(), "{src}: no scripts in {:#?}", math.nodes);
        for (sub, sup) in all {
            let text: String = runs(&sub).iter().chain(runs(&sup).iter()).map(|r| r.text.clone()).collect();
            assert!(!text.contains('.'), "{src} ({linear}): a script holds {text:?}");
        }
        assert!(runs(&math.nodes).iter().any(|r| r.text.contains("..")), "{src}: no `..` run in {:#?}", math.nodes);
    }
}

/// Every fraction's denominator in `nodes`, at any depth.
fn denominators(nodes: &[MNode]) -> Vec<Vec<MNode>> {
    let mut out = Vec::new();
    let mut todo: Vec<&MNode> = nodes.iter().collect();
    while let Some(n) = todo.pop() {
        match n {
            MNode::Frac { num, den, .. } => {
                out.push(den.clone());
                todo.extend(num.iter().chain(den.iter()));
            }
            MNode::Script { base, sub, sup, .. } => todo.extend(base.iter().chain(sub.iter()).chain(sup.iter())),
            MNode::Rad { deg, e, .. } => todo.extend(deg.iter().chain(e.iter())),
            MNode::Delim { elems, .. } => todo.extend(elems.iter().flatten()),
            MNode::Func { name, e } => todo.extend(name.iter().chain(e.iter())),
            MNode::Matrix { rows, .. } => todo.extend(rows.iter().flatten().flatten()),
            _ => {}
        }
    }
    out
}

#[test]
fn a_range_keeps_its_dots_out_of_fractions_in_the_math() {
    let sources = [("x := 1 + 1/2..5", "x≔〖1+1/2〗..5"), ("y := 1 + a/b..5", "y≔〖1+a/b〗..5"), ("w := 0, 1 + a/2..5", "w≔0,〖1+a/2〗..5")];
    let mut d = doc_with(&["a := 1", "b := 2"].into_iter().chain(sources.map(|(src, _)| src)).collect::<Vec<_>>());
    recalc(&mut d, &no_page);
    for (k, (src, want)) in sources.iter().enumerate() {
        let (_, linear, math) = region(&d, k as u64 + 3);
        let dens = denominators(&math.nodes);
        assert!(!dens.is_empty(), "{src}: no fraction in {:#?}", math.nodes);
        for den in dens {
            let text: String = runs(&den).iter().map(|r| r.text.clone()).collect();
            assert!(!text.contains(".."), "{src} ({linear}): a denominator holds {text:?}");
        }
        assert!(runs(&math.nodes).iter().any(|r| r.text.contains("..")), "{src}: no `..` run in {:#?}", math.nodes);
        assert_eq!(linear, *want, "{src}");
    }
}

/// A fraction before a range's `,` or after its `..` keeps the comma and dots out of it too.
#[test]
fn a_range_keeps_its_comma_out_of_fractions_in_the_math() {
    let sources = ["p := 1 + 1/2, 2..5", "q := 1/2, 1..5", "r := 0..1 + 1/2 =", "s := 0, 1/2..1"];
    let mut d = doc_with(&sources);
    recalc(&mut d, &no_page);
    for (k, src) in sources.iter().enumerate() {
        let (_, linear, math) = region(&d, k as u64 + 1);
        let dens = denominators(&math.nodes);
        assert!(!dens.is_empty(), "{src}: no fraction in {:#?}", math.nodes);
        for den in dens {
            let text: String = runs(&den).iter().map(|r| r.text.clone()).collect();
            assert!(!text.contains(',') && !text.contains(".."), "{src} ({linear}): a denominator holds {text:?}");
        }
    }
}

#[test]
fn a_subscripted_function_keeps_its_arguments_out_of_the_subscript() {
    let mut d = doc_with(&["f_a(t) := t^2"]);
    recalc(&mut d, &no_page);
    let (_, _, math) = region(&d, 1);
    match math.nodes.first() {
        Some(MNode::Script { sub, .. }) => assert_eq!(runs(sub).iter().map(|r| r.text.as_str()).collect::<String>(), "a"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(math.nodes.get(1), Some(MNode::Delim { .. })), "{:?}", math.nodes);
}

/// The math tree shown for `z := <expr>` (after its `z≔` run).
fn tree(expr: &str) -> Vec<MNode> {
    let mut d = doc_with(&[&format!("z := {expr}")]);
    recalc(&mut d, &no_page);
    let (_, _, math) = region(&d, 1);
    let mut nodes = math.nodes;
    match nodes.first_mut() {
        Some(MNode::Run(r)) if r.text == "z≔" => {
            nodes.remove(0);
        }
        Some(MNode::Run(r)) if r.text.starts_with("z≔") => r.text = r.text.replacen("z≔", "", 1),
        other => panic!("{other:?}"),
    }
    // Drop the error marker of an undefined name.
    if matches!(nodes.last(), Some(MNode::Run(r)) if r.text == "⚠") {
        nodes.pop();
    }
    nodes
}

fn text_of(nodes: &[MNode]) -> String {
    let mut out = String::new();
    for n in nodes {
        match n {
            MNode::Run(r) => out.push_str(&r.text),
            MNode::Delim { beg, end, elems, .. } => {
                out.extend(*beg);
                out.push_str(&elems.iter().map(|e| text_of(e)).collect::<Vec<_>>().join("|"));
                out.extend(*end);
            }
            MNode::Func { name, e } => out.push_str(&format!("[{}: {}]", text_of(name), text_of(e))),
            MNode::Script { base, sup, sub, .. } => out.push_str(&format!("{{{}}}_{{{}}}^{{{}}}", text_of(base), text_of(sub), text_of(sup))),
            MNode::Frac { num, den, .. } => out.push_str(&format!("{{{}}}/{{{}}}", text_of(num), text_of(den))),
            MNode::Rad { e, .. } => out.push_str(&format!("√{{{}}}", text_of(e))),
            other => out.push_str(&format!("{other:?}")),
        }
    }
    out
}

#[test]
fn function_calls_show_the_structure_that_was_evaluated() {
    // A built-in UnicodeMath function keeps its argument in brackets when it is not a lone atom.
    assert_eq!(text_of(&tree("sin(θ+1)")), "[sin: (θ+1)]");
    assert_eq!(text_of(&tree("sin(θ)")), "[sin: θ]");
    // What follows the call is outside it.
    let max = tree("max(a, b) + c");
    assert!(matches!(max.first(), Some(MNode::Func { .. })), "{max:?}");
    assert_eq!(text_of(&max), "[max: (a,b)]+c");
    // A call used as a base is raised whole, not just its argument.
    assert_eq!(text_of(&tree("cos(θ)^2")), "{([cos: θ])}_{}^{2}");
    assert_eq!(text_of(&tree("sin(θ+1)^2")), "{([sin: (θ+1)])}_{}^{2}");
    assert_eq!(text_of(&tree("f(x)^2")), "{(f(x))}_{}^{2}");
    // A call as a numerator is the whole numerator.
    assert_eq!(text_of(&tree("sin(θ)/2")), "{[sin: θ]}/{2}");
    // Names holding a function name are never split into a letter and a function.
    let asin = tree("asin(x) + atan2(y, x)");
    assert_eq!(text_of(&asin), "asin(x)+atan2(y,x)");
    assert!(runs(&asin).iter().any(|r| r.text == "asin" && r.sty == Some(MSty::Plain)), "{asin:?}");
    assert_eq!(text_of(&tree("xmax(t) + 1")), "xmax(t)+1");
    assert_eq!(text_of(&tree("f_max(t)")), "{f}_{max}^{}(t)");
    assert_eq!(text_of(&tree("σ_max + len")), "{σ}_{max}^{}+len");
}

#[test]
fn a_number_base_is_raised_whole() {
    assert_eq!(text_of(&tree("25^2")), "{25}_{}^{2}");
    assert_eq!(text_of(&tree("1.5^2")), "{1.5}_{}^{2}");
    assert_eq!(text_of(&tree("2^3")), "{2}_{}^{3}");
    assert_eq!(text_of(&tree("1e300")), "1×{10}_{}^{300}");
    // A result shown in exponential form.
    let mut d = doc_with(&["big := 6000000 ="]);
    recalc(&mut d, &no_page);
    let (_, linear, math) = region(&d, 1);
    assert_eq!(linear, "big≔6000000=6×〖10〗^6");
    let shown: String = text_of(&math.nodes);
    assert!(shown.ends_with("=6×{10}_{}^{6}"), "{shown}");
    // It reads back for the stale display.
    mark_stale(&mut d);
    assert_eq!(region(&d, 1).1, "big≔6000000≈6×〖10〗^6");
}

#[test]
fn names_with_several_or_empty_subscripts() {
    assert_eq!(text_of(&tree("a_b_c")), "{a}_{{b}_{c}^{}}^{}");
    assert_eq!(render::display("z := a_b_c", None, &CalcState::Fresh), "z≔a_(b_(c))");
    // Empty parts show as nothing, never a dangling `_`.
    assert_eq!(text_of(&tree("a__b + 1")), "{a}_{b}^{}+1");
    assert_eq!(render::display("x_ := 3", None, &CalcState::Fresh), "x≔3");
    // Such a name is refused (`check_name`), but its display stays safe: nothing is lowered.
    let mut d = doc_with(&["x_ := 3 ="]);
    recalc(&mut d, &no_page);
    let (_, _, math) = region(&d, 1);
    assert_eq!(text_of(&math.nodes), "x≔3⚠");
}

#[test]
fn names_with_an_empty_subscript_are_refused() {
    let message = |n: &str| CalcState::Error(format!("{n} has an empty subscript — write a name like a_b"));
    let mut d = doc_with(&["a__b := 1", "x_ := 3", "f(t_) := t_", "a_b := 2 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, message("a__b"));
    assert_eq!(region(&d, 2).0.state, message("x_"));
    assert_eq!(region(&d, 3).0.state, message("t_"));
    assert_eq!(region(&d, 4).0.state, CalcState::Fresh);
    let syntax = |n: &str| Err(CalcError::Syntax(format!("{n} has an empty subscript — write a name like a_b")));
    assert_eq!(check_new_name(&d, "a__b", None), syntax("a__b"));
    assert_eq!(check_new_name(&d, "x_", None), syntax("x_"));
    assert_eq!(check_new_name(&d, "_x", None), syntax("_x"));
    assert_eq!(check_new_name(&d, "a_c", None), Ok(()));
}

#[test]
fn display_of_a_deep_expression_does_not_overflow() {
    let sum = vec!["1"; 1000].join("+");
    let src = format!("z := {sum}");
    let shown =
        std::thread::Builder::new().stack_size(256 * 1024).spawn(move || render::display(&src, None, &CalcState::Fresh)).unwrap().join().unwrap();
    assert!(shown.starts_with("z≔1+1+"));
}

#[test]
fn hostile_documents_return_a_report() {
    let long = format!("x := {}", "1+".repeat(60_000));
    let mut sources: Vec<String> = vec![long, "y := 10^400 =".into(), "√-1 =".into(), String::new(), "=".into(), ":=".into()];
    // A 3,000-long cycle.
    for i in 0..3000 {
        sources.push(format!("q_{i} := q_{} + 1", (i + 1) % 3000));
    }
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let mut d = doc_with(&refs);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, refs.len());
    assert_eq!(report.errors, refs.len());
    let (r, ..) = region(&d, 7);
    match r.state {
        CalcState::Error(m) => assert!(m.len() < 200 && m.ends_with("depend on each other"), "{m}"),
        other => panic!("{other:?}"),
    }
}

fn chained_doc(n: usize) -> Document {
    let mut sources = vec!["x_0 := 1 in".to_string()];
    for i in 1..n {
        sources.push(format!("x_{i} := x_{} + 1 in =", i - 1));
    }
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    doc_with(&refs)
}

#[test]
fn chained_regions_recalculate() {
    let mut d = chained_doc(50);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 50).1, "x_49≔x_48+1 \"in\"=50 \"in\"");
}

#[test]
#[ignore = "timing: run with --release -- --ignored perf_2000_regions"]
fn perf_2000_regions() {
    let mut d = chained_doc(2000);
    recalc(&mut d, &no_page);
    // An undo snapshot shares the blocks, so a recalculation pays for what it copies.
    let _snapshot = d.clone();
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0])
        && let Some(InlineObject::Equation { calc: Some(c), .. }) = p.objects.first_mut()
    {
        c.source = "x_0 := 2 in".into();
    }
    let start = Instant::now();
    let report = recalc(&mut d, &no_page);
    let took = start.elapsed();
    println!("perf_2000_regions: {took:?}");
    assert_eq!(report.regions, 2000);
    assert_eq!(report.errors, 0);
    assert_eq!(region(&d, 2000).1, "x_1999≔x_1998+1 \"in\"=2001 \"in\"");
    assert!(took.as_millis() < 100, "recalc took {took:?}");
}

/// `n` regions defining the reserved word `or` (a document from before it was one), then `n`
/// using `not` and `and`.
fn reserved_word_doc(defining: usize, using: usize) -> Document {
    let mut sources = vec!["or := 1"; defining];
    sources.extend(std::iter::repeat_n("not 1 > 2 and 1 < 2 =", using));
    doc_with(&sources)
}

#[test]
#[ignore = "timing: run with --release -- --ignored perf_reserved_words"]
fn perf_reserved_words() {
    // Each region is checked against the words defined as names: each word once, however many
    // regions define it, so 20,000 definitions of `or` cost what one does.
    let n = 20_000;
    let time = |defining: usize| {
        let mut d = reserved_word_doc(defining, n);
        let start = Instant::now();
        let report = recalc(&mut d, &no_page);
        let took = start.elapsed();
        assert_eq!(report.regions, defining + n);
        (took, d)
    };
    let (one, _) = time(1);
    let (many, d) = time(n);
    println!("perf_reserved_words: one definition {one:?}, {n} definitions {many:?}");
    assert_eq!(region(&d, 1).0.state, err(CalcError::Keyword("or".into())));
    assert!(matches!(region(&d, n as u64 + 1).0.state, CalcState::Fresh), "{:?}", region(&d, n as u64 + 1).0.state);
    // The extra 20,000 regions cost their own work, not 20,000 scans each.
    assert!(many < one * 2 + std::time::Duration::from_millis(30), "{n} definitions took {many:?}, one took {one:?}");
}

#[test]
fn one_budget_bounds_the_whole_pass() {
    // Each f_i calls the one before twice: f_24(1) needs ~33 million steps, far over one
    // statement's limit, and twenty of them would cost seconds if each had a limit of its own.
    let mut sources: Vec<String> = vec!["f_0(x) := x".into()];
    sources.extend((1..=24).map(|i| format!("f_{i}(x) := f_{}(x) + f_{}(x)", i - 1, i - 1)));
    sources.extend((0..20).map(|i| format!("y_{i} := f_24(1) =")));
    sources.push("z := 2 =".into());
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let mut d = doc_with(&refs);
    let started = Instant::now();
    let report = recalc(&mut d, &no_page);
    let elapsed = started.elapsed();
    assert_eq!(report.errors, 20, "every y");
    let budget = err(CalcError::TooMuchWork);
    let states: Vec<CalcState> = regions(&d).into_iter().map(|(r, _, _)| r.state).collect();
    // Each y spends its own million steps until the pass's five million are gone.
    assert_eq!(states[25..29], vec![err(CalcError::TooComplex); 4]);
    assert_eq!(states[29..45], vec![budget; 16]);
    // z needs nothing, so it ran first and is correct.
    assert_eq!(states[45], CalcState::Fresh);
    for (r, linear, _) in regions(&d).into_iter().skip(25).take(20) {
        assert!(!linear.contains('='), "{}: no number: {linear}", r.source);
    }
    // Generous: a debug build on a slow machine. Without the shared budget it is ~4× this work.
    assert!(elapsed.as_secs() < 20, "{elapsed:?}");
}

#[test]
fn unchanged_calculations_need_no_pass() {
    let mut d = doc_with(&["x := 1 =", "y := x + 1 ="]);
    recalc(&mut d, &no_page);
    // Typing ordinary text.
    let mut typed = d.clone();
    if let Block::Para(p) = Arc::make_mut(&mut typed.body[0]) {
        p.insert_text(0, "More ", &CharProps::default()).unwrap();
    }
    assert!(calc_unchanged(&d, &typed));
    // A source, a unit, the settings, a region's order: each needs a pass.
    let mut changed = d.clone();
    edit_region(&mut changed, 1, |c, _| c.source = "x := 2 =".into());
    assert!(!calc_unchanged(&d, &changed));
    let mut changed = d.clone();
    edit_region(&mut changed, 2, |c, _| c.unit = Some("in".into()));
    assert!(!calc_unchanged(&d, &changed));
    let mut changed = d.clone();
    changed.calc_settings = Some(CalcSettings { precision: 1, ..CalcSettings::default() });
    assert!(!calc_unchanged(&d, &changed));
    let mut changed = d.clone();
    changed.body.swap(0, 1);
    assert!(!calc_unchanged(&d, &changed));
    // A display changed by something else is drawn again.
    let mut changed = d.clone();
    edit_region(&mut changed, 2, |_, linear| *linear = "y≔x+1=99".into());
    assert!(!calc_unchanged(&d, &changed));
    // In automatic mode a stale result (left by a pass that failed) is retried.
    let mut stale = d.clone();
    mark_stale(&mut stale);
    assert!(!calc_unchanged(&stale, &stale.clone()));
    stale.calc_settings = Some(CalcSettings { manual: true, ..CalcSettings::default() });
    assert!(calc_unchanged(&stale, &stale.clone()));
}

#[test]
fn quoted_values_in_exponential_form_use_superscripts() {
    let mut d = doc_with(&["E := 29000000 psi", "α := 0.0000065", "n := -2.5e9"]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0]) {
        for instr in [quote("E", None, None, None), quote("α", None, None, None), quote("n", None, None, None)] {
            let end = p.text.len();
            p.insert_object(end, field(&instr), &CharProps::default()).unwrap();
        }
    }
    recalc(&mut d, &no_page);
    let quoted: Vec<String> = objects(&d)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(quoted, ["2.9×10⁷ psi", "6.5×10⁻⁶", "-2.5×10⁹"]);
    assert!(quoted.iter().all(|q| !q.contains('^')), "no markup in a sentence");
}

#[test]
fn a_temperature_difference_never_shows_on_an_offset_scale() {
    let mut d = doc_with(&["T_1 := 70 °F", "T_2 := 100 °F", "dT := T_2 - T_1", "dT =", "T_1 = ? °C"]);
    // The region's own unit, as Insert › Calculation's options would set it.
    edit_region(&mut d, 4, |c, _| c.unit = Some("°F".into()));
    if let Block::Para(p) = Arc::make_mut(&mut d.body[4]) {
        for instr in [quote("dT", Some("°C"), None, None), quote("dT", Some("Δ°C"), None, None), quote("T_1", Some("°C"), None, None)] {
            let end = p.text.len();
            p.insert_object(end, field(&instr), &CharProps::default()).unwrap();
        }
    }
    recalc(&mut d, &no_page);
    let (r, linear, _) = region(&d, 4);
    assert_eq!(r.state, err(CalcError::NotAbsolute("°F".into())), "{linear}");
    assert!(!linear.contains('='), "{linear}");
    assert_eq!(region(&d, 5).1, "T_1=21.111 \"°C\"");
    let quoted: Vec<String> = objects(&d)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(quoted, ["?", "16.667 Δ°C", "21.111 °C"]);
}

#[test]
fn body_regions_reach_every_depth_with_recalculation_ids() {
    use wordcraft_calc::body_regions;
    // A region 20 tables deep (deeper than ordinary paragraph walks go), and a copy of id 1.
    let mut block = para_block(para(vec![calc_eq(1, "deep := 2 =")]));
    for _ in 0..20 {
        let cell = Cell { blocks: vec![block], ..Cell::empty() };
        block = Arc::new(Block::Table(Table { rows: vec![Row { cells: vec![cell], ..Default::default() }], ..Default::default() }));
    }
    let mut d = doc_of(vec![para_block(para(vec![calc_eq(1, "top := deep + 1 =")])), block]);
    let found = body_regions(&d);
    assert_eq!(found.len(), 2);
    assert_eq!(found[1].0.len(), 1 + 3 * 20, "the path goes all the way down");
    assert_eq!((found[0].2.id, found[1].2.id), (1, 2), "the copy gets the id recalculation gives it");
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).1, "top≔deep+1=3");
    assert_eq!(region(&d, 2).1, "deep≔2=2");
    // The offset is the region's object character.
    let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path(found[1].0.clone())).unwrap();
    assert!(matches!(p.object_at(found[1].1), Some(InlineObject::Equation { calc: Some(c), .. }) if c.id == 2));
}

#[test]
fn g_follows_the_document_setting() {
    let mut d = doc_with(&["F_w := 5 kg·g = ? N", "m_1 := 250 gm = ? kg"]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 1).1.ends_with("=49.033 \"N\""), "Mathcad's g is the default: {}", region(&d, 1).1);
    assert!(region(&d, 2).1.ends_with("=0.25 \"kg\""), "{}", region(&d, 2).1);
    d.calc_settings = Some(CalcSettings { g: wordcraft_doc::calc::GMeans::Gram, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    assert!(matches!(region(&d, 1).0.state, CalcState::Error(_)), "kg·g is kg² as grams: no longer a force");
    assert!(region(&d, 2).1.ends_with("=0.25 \"kg\""), "gm is a gram either way");
}

#[test]
fn indexes_follow_the_document_origin() {
    let mut d = doc_with(&["D := [0.840, 1.050] in", "x := D[1] = ? in"]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 2).1.ends_with("=1.05 \"in\""), "Mathcad counts from 0 by default: {}", region(&d, 2).1);
    d.calc_settings = Some(CalcSettings { origin: 1, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    assert!(region(&d, 2).1.ends_with("=0.84 \"in\""), "{}", region(&d, 2).1);
}

/// The bracketed matrix of a result (the last one in the tree): its rows of cell texts, and the
/// runs after it.
fn result_matrix(nodes: &[MNode]) -> (Vec<Vec<String>>, Vec<MRun>) {
    let at = nodes.iter().rposition(|n| matches!(n, MNode::Delim { beg: Some('['), .. })).unwrap_or_else(|| panic!("{nodes:#?}"));
    let MNode::Delim { elems, .. } = &nodes[at] else { panic!("{nodes:#?}") };
    let [MNode::Matrix { rows, .. }] = elems[0].as_slice() else { panic!("{elems:#?}") };
    let cells = rows.iter().map(|row| row.iter().map(|cell| runs(cell).iter().rev().map(|r| r.text.clone()).collect()).collect()).collect();
    let after = nodes[at + 1..].iter().map(|n| if let MNode::Run(r) = n { r.clone() } else { panic!("{n:?}") }).collect();
    (cells, after)
}

#[test]
fn vector_result_shows_as_a_matrix() {
    let mut d = doc_with(&["D := [0.840, 1.050] in", "D ="]);
    recalc(&mut d, &no_page);
    let (r, linear, math) = region(&d, 2);
    assert_eq!(r.state, CalcState::Fresh);
    assert_eq!(linear, "D=[■(0.84@1.05)] \"in\"");
    let (cells, after) = result_matrix(&math.nodes);
    assert_eq!(cells, [["0.84"], ["1.05"]]);
    // The shared unit is shown once, upright, set apart from the bracket.
    assert_eq!(after.len(), 1, "{after:?}");
    assert!(after[0].text == "\u{2009}in" && after[0].sty == Some(MSty::Plain), "{after:?}");
}

#[test]
fn display_unit_applies_to_every_element() {
    let mut d = doc_with(&["D := [0.840, 1.050] in", "D = ? mm", "D = ? lbf"]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 2).1, "D=[■(21.336@26.67)] \"mm\"");
    let (r, linear, _) = region(&d, 3);
    let (length, force) = (craft_units::parse_unit("in").unwrap().dim, craft_units::parse_unit("lbf").unwrap().dim);
    let mismatch = CalcError::Unit(craft_units::UnitError::Mismatch { left: length, right: force });
    assert_eq!(r.state, err(CalcError::Element { at: "0".into(), error: Box::new(mismatch) }), "{linear}");
    assert!(!linear.contains("=["), "{linear}");
}

#[test]
fn mixed_dimensions_show_each_unit() {
    let mut d = doc_with(&["[2 in, 3 lbf] ="]);
    recalc(&mut d, &no_page);
    let (r, linear, math) = region(&d, 1);
    assert_eq!(r.state, CalcState::Fresh);
    assert_eq!(linear, "[■(2 \"in\"@3 \"lbf\")]=[■(2 \"in\"@3 \"lbf\")]");
    let (cells, after) = result_matrix(&math.nodes);
    assert_eq!(cells, [["2\u{2009}in"], ["3\u{2009}lbf"]]);
    assert!(after.is_empty(), "no unit after the bracket: {after:?}");
}

#[test]
fn offset_unit_is_checked_per_element() {
    let mut d = doc_with(&["T := [20 °C, 30 °C]", "T = ? °F", "T - [10 °C, 10 °C] = ? °C"]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 2).1, "T=[■(68@86)] \"°F\"");
    // A difference of absolute temperatures is a difference: never shown on an offset scale.
    let (r, linear, _) = region(&d, 3);
    assert_eq!(r.state, err(CalcError::Element { at: "0".into(), error: Box::new(CalcError::NotAbsolute("°C".into())) }), "{linear}");
}

#[test]
fn long_vectors_are_visibly_cut() {
    let mut d = doc_with(&["x := 0..59 =", "len := 0 in, 1 in..59 in =", "y := 1..50 ="]);
    recalc(&mut d, &no_page);
    let first: Vec<String> = (0..50).map(|k| k.to_string()).collect();
    let (r, linear, math) = region(&d, 1);
    assert_eq!(r.state, CalcState::Fresh);
    assert_eq!(linear, format!("x≔0..59=[■({}@⋮)] \"(60 rows)\"", first.join("@")));
    assert!(linear.ends_with("@⋮)] \"(60 rows)\""), "{linear}");
    // The cut shows in the equation too: a ⋮ row, then the count.
    let (cells, after) = result_matrix(&math.nodes);
    assert_eq!(cells.len(), 51);
    assert_eq!(cells[49], ["49"]);
    assert_eq!(cells[50], ["⋮"]);
    assert_eq!(after.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["\u{2009}(60 rows)"]);
    assert!(after.iter().all(|r| r.sty == Some(MSty::Plain)), "{after:?}");
    // A shared unit comes before the count.
    let (_, linear, math) = region(&d, 2);
    assert!(linear.ends_with("@49@⋮)] \"in\" \"(60 rows)\""), "{linear}");
    let (_, after) = result_matrix(&math.nodes);
    assert_eq!(after.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["\u{2009}in", "\u{2009}(60 rows)"]);
    // Fifty rows show in full.
    let (_, linear, math) = region(&d, 3);
    assert!(linear.ends_with("@49@50)]") && !linear.contains('⋮'), "{linear}");
    assert_eq!(result_matrix(&math.nodes).0.len(), 50);
}

#[test]
fn stale_vector_keeps_its_values() {
    let sources =
        ["D := [0.840, 1.050] in =", "x := 0..59 =", "len := 0 in, 1 in..59 in =", "[2 in, 3 lbf] =", "M := [1, 2; 3, 4] =", "S := [1e-9, 2e9] ="];
    let mut d = doc_with(&sources);
    recalc(&mut d, &no_page);
    let fresh: Vec<String> = regions(&d).into_iter().map(|(_, linear, _)| linear).collect();
    mark_stale(&mut d);
    for (k, (r, linear, math)) in regions(&d).into_iter().enumerate() {
        assert_eq!(r.state, CalcState::Stale);
        let (head, tail) = fresh[k].rsplit_once('=').unwrap();
        assert_eq!(linear, format!("{head}≈{tail}"), "{}", sources[k]);
        // Every element is drawn grey, the cut and count included.
        let (cells, after) = result_matrix(&math.nodes);
        let (fresh_cells, _) = {
            let mut f = doc_with(&[sources[k]]);
            recalc(&mut f, &no_page);
            result_matrix(&region(&f, 1).2.nodes)
        };
        assert_eq!(cells, fresh_cells, "{}", sources[k]);
        let at = math.nodes.iter().rposition(|n| matches!(n, MNode::Delim { beg: Some('['), .. })).unwrap();
        let shown = runs(&math.nodes[at..]);
        assert!(shown.iter().all(|r| r.color == Some(Rgb(0x80, 0x80, 0x80))), "{}: {shown:?}", sources[k]);
        assert!(after.iter().all(|r| r.color == Some(Rgb(0x80, 0x80, 0x80))));
    }
    recalc(&mut d, &no_page);
    assert_eq!(regions(&d).into_iter().map(|(_, linear, _)| linear).collect::<Vec<_>>(), fresh);
}

#[test]
fn quoted_vector_is_a_list() {
    let mut d = doc_with(&["D := [0.840, 1.050] in", "M := [1, 2; 3, 4]", "x := 0..59", "v := [2 in, 3 lbf]", "len := 0 in, 1 in..59 in"]);
    let codes = [
        quote("D", None, None, None),
        quote("D", None, None, Some((1, None))),
        quote("M", None, None, None),
        quote("x", None, None, None),
        quote("D", None, None, Some((9, None))),
        quote("M", None, None, Some((1, Some(0)))),
        quote("M", None, None, Some((1, None))),
        quote("D", Some("mm"), None, None),
        quote("D", Some("lbf"), None, None),
        quote("v", None, None, None),
        quote("len", None, None, None),
        quote("D", Some("mm"), Some(1), Some((0, None))),
        quote("D", None, None, Some((1, Some(0)))),
        quote("D", None, None, Some((0, Some(1)))),
    ];
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0]) {
        for instr in &codes {
            let end = p.text.len();
            p.insert_object(end, field(instr), &CharProps::default()).unwrap();
        }
    }
    recalc(&mut d, &no_page);
    let to_49: Vec<String> = (0..50).map(|k| k.to_string()).collect();
    assert_eq!(
        field_results(&d),
        [
            "0.84, 1.05 in".to_string(),
            "1.05 in".into(),
            "1, 2; 3, 4".into(),
            format!("{}, … (60 values)", to_49.join(", ")),
            "?".into(),
            "3".into(),
            "?".into(),
            "21.336, 26.67 mm".into(),
            "?".into(),
            "2 in, 3 lbf".into(),
            format!("{} in, … (60 values)", to_49.join(", ")),
            "21.3 mm".into(),
            "1.05 in".into(),
            "?".into(),
        ]
    );
    // Indexes follow the document's origin.
    d.calc_settings = Some(CalcSettings { origin: 1, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    let results = field_results(&d);
    assert_eq!((results[1].as_str(), results[5].as_str(), results[11].as_str()), ("0.84 in", "?", "?"));
    // A unit for a quoted vector must suit every element.
    assert_eq!(check_quote_unit(&d, "D", "mm"), Ok(()));
    assert!(matches!(check_quote_unit(&d, "D", "lbf"), Err(CalcError::Element { .. })));
    assert!(matches!(check_quote_unit(&d, "v", "in"), Err(CalcError::Element { at, .. }) if at == "2"));
}

#[test]
fn a_vector_whose_display_fails_defines_nothing() {
    let mut d = doc_with(&["D := [1, 2] in = ? lbf", "y := D[0] ="]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[1]) {
        let end = p.text.len();
        p.insert_object(end, field("WCCALC D"), &CharProps::default()).unwrap();
    }
    recalc(&mut d, &no_page);
    assert!(matches!(region(&d, 1).0.state, CalcState::Error(_)));
    assert_eq!(region(&d, 2).0.state, err(CalcError::Upstream));
    assert_eq!(field_results(&d), ["?"]);
}

#[test]
fn defined_names_lists_vectors() {
    let d = doc_with(&["D := [1, 2]", "i := 0..3", "t[i] := 2·i", "n := 1", "f(x) := x"]);
    assert_eq!(defined_names(&d), [("D".to_string(), 1), ("i".to_string(), 2), ("n".to_string(), 4), ("t".to_string(), 3)]);
}

#[test]
fn wide_results_are_visibly_cut() {
    let mut d = doc_with(&["i := 0..9999", "r := transpose(i) =", "a := 0..59", "M := mmul(a, transpose(a)) =", "w := transpose(a) ="]);
    recalc(&mut d, &no_page);
    // A row of 10,000 shows its first 50, a ⋯, and how many there are; no more cells are built.
    let (r, linear, math) = region(&d, 2);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    assert!(linear.ends_with("&48&49&⋯)] \"(10000 columns)\""), "{linear}");
    let (cells, after) = result_matrix(&math.nodes);
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].len(), 51);
    assert_eq!(cells[0][50], "⋯");
    assert_eq!(after.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["\u{2009}(10000 columns)"]);
    // Cut both ways: the dots meet in a corner, and the size names both.
    let (r, linear, math) = region(&d, 4);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    assert!(linear.ends_with("&⋮&⋮&⋱)] \"(60 rows × 60 columns)\""), "{}", &linear[linear.len() - 60..]);
    let (cells, after) = result_matrix(&math.nodes);
    assert_eq!(cells.len(), 51);
    assert!(cells.iter().all(|row| row.len() == 51));
    assert_eq!((cells[0][50].as_str(), cells[50][0].as_str(), cells[50][50].as_str()), ("⋯", "⋮", "⋱"));
    assert_eq!(cells[49][49], (49 * 49).to_string());
    assert_eq!(after.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["\u{2009}(60 rows × 60 columns)"]);
    // Going stale keeps every form.
    let fresh: Vec<String> = regions(&d).into_iter().map(|(_, linear, _)| linear).collect();
    mark_stale(&mut d);
    for (k, (_, linear, _)) in regions(&d).into_iter().enumerate() {
        if let Some((head, tail)) = fresh[k].rsplit_once('=') {
            assert_eq!(linear, format!("{head}≈{tail}"));
        }
    }
    // A quoted row lists its first 50 values.
    let mut d = doc_with(&["i := 0..9999", "r := transpose(i)"]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[1]) {
        let end = p.text.len();
        p.insert_object(end, field("WCCALC r"), &CharProps::default()).unwrap();
    }
    recalc(&mut d, &no_page);
    let to_49: Vec<String> = (0..50).map(|k| k.to_string()).collect();
    assert_eq!(field_results(&d), [format!("{}, … (10000 values)", to_49.join(", "))]);
}

/// The text of every cell of the (first) bracketed matrix in a source's math tree, row by row.
fn literal_cells(nodes: &[MNode]) -> Vec<Vec<String>> {
    let at = nodes.iter().position(|n| matches!(n, MNode::Delim { beg: Some('['), .. })).unwrap_or_else(|| panic!("{nodes:#?}"));
    result_matrix(&nodes[..=at]).0
}

#[test]
fn a_long_literal_keeps_every_element() {
    let long: Vec<String> = (0..200).map(|k| k.to_string()).collect();
    let square: Vec<String> = (0..20).map(|r| (0..20).map(|c| format!("{}.5 in", r * 20 + c)).collect::<Vec<_>>().join(", ")).collect();
    let mut d = doc_with(&[
        &format!("D := [{}]", long.join(", ")),
        &format!("M := [{}]", square.join("; ")),
        "v := 2·[a + b, -c] in",
        "a := 1",
        "b := 2",
        "c := 3",
    ]);
    recalc(&mut d, &no_page);
    let (_, linear, math) = region(&d, 1);
    assert!(linear.starts_with("D≔[■(0@1@2@") && linear.ends_with("@198@199)]"), "{linear}");
    let cells = literal_cells(&math.nodes);
    assert_eq!(cells, long.iter().map(|k| vec![k.clone()]).collect::<Vec<_>>());
    let (_, _, math) = region(&d, 2);
    let cells = literal_cells(&math.nodes);
    assert_eq!(cells.len(), 20);
    for (r, row) in cells.iter().enumerate() {
        assert_eq!(row.len(), 20);
        for (c, cell) in row.iter().enumerate() {
            assert_eq!(cell, &format!("{}.5\u{2009}in", r * 20 + c), "[{r}, {c}]");
        }
    }
    // As long a literal as a source can hold, of elements as long as they get: the linear parser
    // alone would stop partway through it.
    let small: Vec<String> = (0..1200).map(|k| format!("{}.5e-{} m", k % 10, k % 9 + 7)).collect();
    let mut big = doc_with(&[&format!("E := [{}]", small.join(", "))]);
    recalc(&mut big, &no_page);
    let (r, linear, math) = region(&big, 1);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    let cells = literal_cells(&math.nodes);
    assert_eq!(cells.len(), 1200);
    let mut one = doc_with(&[small[1199].as_str()].map(|e| format!("E := [{e}]")).each_ref().map(String::as_str));
    recalc(&mut one, &no_page);
    assert_eq!(cells[1199], literal_cells(&region(&one, 1).2.nodes)[0]);
    // A wide one too: 40 × 40 elements that each take the parser some work.
    let rows: Vec<String> = (0..40).map(|r| (0..40).map(|c| format!("{}.5e-{}", (r + c) % 10, c % 9 + 7)).collect::<Vec<_>>().join(", ")).collect();
    let mut wide = doc_with(&[&format!("G := [{}]", rows.join("; "))]);
    recalc(&mut wide, &no_page);
    let (r, linear, math) = region(&wide, 1);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    let cells = literal_cells(&math.nodes);
    assert_eq!(cells.len(), 40);
    assert!(cells.iter().all(|row| row.len() == 40), "{:?}", cells.iter().map(Vec::len).collect::<Vec<_>>());
    // A literal among other things keeps its place, and the unit after it stands apart.
    let (_, linear, math) = region(&d, 3);
    assert_eq!(linear, "v≔2·[■(a+b@-c)] \"in\"");
    assert_eq!(literal_cells(&math.nodes), [["a+b"], ["-c"]]);
    let unit = math.nodes.last();
    assert!(matches!(unit, Some(MNode::Run(r)) if r.text == "\u{2009}in" && r.sty == Some(MSty::Plain)), "{unit:?}");
}

#[test]
fn a_definition_that_cannot_be_shown_defines_nothing() {
    // 10^308 m is 3.9×10^309 in: no number to show in US customary units.
    let mut d = doc_with(&["x := 1e308 m =", "y := x =", "v := [1 m, 1e308 m] =", "z := v[0] ="]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[1]) {
        for code in ["WCCALC x", "WCCALC v"] {
            let end = p.text.len();
            p.insert_object(end, field(code), &CharProps::default()).unwrap();
        }
    }
    recalc(&mut d, &no_page);
    let non_finite = CalcError::Unit(craft_units::UnitError::NonFinite);
    assert_eq!(region(&d, 1).0.state, err(non_finite.clone()));
    assert_eq!(region(&d, 2).0.state, err(CalcError::Upstream));
    // The element that can't be shown is named.
    assert_eq!(region(&d, 3).0.state, err(CalcError::Element { at: "1".into(), error: Box::new(non_finite) }));
    assert_eq!(region(&d, 4).0.state, err(CalcError::Upstream));
    assert_eq!(field_results(&d), ["?", "?"]);
}

#[test]
fn a_long_source_keeps_all_its_text() {
    // As long as sources get, in shapes the linear parser works hardest on: every name reaches the
    // math tree.
    let sources = [
        format!("x := {}a", "a + ".repeat(1000)),
        format!("x := {}a", "(a + b)/(c + d) + ".repeat(800)),
        format!("x := {}a", "a_b^2 + ".repeat(1000)),
        format!("x := {}a", "√(a + 1) + ".repeat(1000)),
        format!("x := {}a", "[a, b; a, b] + ".repeat(700)),
    ];
    let names = ["a", "b", "c", "d"];
    let defined: Vec<String> = names.iter().map(|n| format!("{n} := 1")).collect();
    for src in &sources {
        let mut all: Vec<&str> = defined.iter().map(String::as_str).collect();
        all.push(src);
        let mut d = doc_with(&all);
        recalc(&mut d, &no_page);
        let (r, linear, math) = region(&d, 5);
        assert!(!matches!(r.state, CalcState::Error(ref e) if e.contains("too complex")), "{}: {:?}", &src[..40], r.state);
        let text: String = runs(&math.nodes).iter().map(|r| r.text.as_str()).collect();
        for n in names {
            let want = linear.matches(n).count();
            assert_eq!(text.matches(n).count(), want, "{n} in {}", &src[..40]);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Data tables
// ---------------------------------------------------------------------------------------------

/// A table marked as calculation data with id `id`, one cell per text, one paragraph per cell.
fn data_table(id: u64, rows: &[&[&str]]) -> Table {
    Table {
        rows: rows.iter().map(|r| Row { cells: r.iter().map(|t| Cell::with_text(t)).collect(), ..Default::default() }).collect(),
        grid: vec![100.0; rows.first().map_or(1, |r| r.len())],
        calc: Some(Box::new(CalcTable { id, errors: Vec::new() })),
        ..Default::default()
    }
}

fn table_block(t: Table) -> Arc<Block> {
    Arc::new(Block::Table(t))
}

/// The table at body index `at`.
fn table_at(d: &Document, at: usize) -> &Table {
    match &*d.body[at] {
        Block::Table(t) => t,
        Block::Para(_) => panic!("block {at} isn't a table"),
    }
}

fn table_mut(d: &mut Document, at: usize) -> &mut Table {
    match Arc::make_mut(&mut d.body[at]) {
        Block::Table(t) => t,
        Block::Para(_) => panic!("block {at} isn't a table"),
    }
}

/// The calculation data of the table at body index `at`.
fn table_calc(d: &Document, at: usize) -> CalcTable {
    *table_at(d, at).calc.clone().unwrap()
}

/// Replace the text of cell (`r`, `c`) of the table at body index `at`.
fn set_cell(d: &mut Document, at: usize, r: usize, c: usize, text: &str) {
    table_mut(d, at).rows[r].cells[c].blocks = vec![para_block(Paragraph::with_text(text, CharProps::default()))];
}

fn cell_text(d: &Document, at: usize, r: usize, c: usize) -> String {
    match &*table_at(d, at).rows[r].cells[c].blocks[0] {
        Block::Para(p) => p.text.clone(),
        Block::Table(_) => panic!("a nested table"),
    }
}

const PIPES: &[&[&str]] =
    &[&["NPS (in)", "D_o (in)", "t_nom (in)"], &["1/2", "0.840", "0.109"], &["3/4", "1.050", "0.113"], &["1", "1.315", "0.133"]];

/// The B31.3 example: the pipe table (block 0), then regions 1 to 7 (blocks 1 to 7). The margin
/// is `margin`, not `m`: `m` is the metre.
fn pipe_doc(rows: &[&[&str]]) -> Document {
    let sources =
        ["P := 1000 psi", "S := 20000 psi", "E := 1", "Y := 0.4", "t_min := P·D_o/(2·(S·E + P·Y)) =", "margin := t_nom - t_min =", "min(margin) ="];
    let mut blocks = vec![table_block(data_table(1, rows))];
    blocks.extend(sources.iter().enumerate().map(|(i, s)| para_block(para(vec![calc_eq(i as u64 + 1, s)]))));
    doc_of(blocks)
}

#[test]
fn table_columns_define_vectors() {
    let mut d = pipe_doc(PIPES);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.errors, 0, "{:?}", regions(&d).iter().map(|(r, ..)| r.state.clone()).collect::<Vec<_>>());
    assert!(region(&d, 5).1.ends_with("=[■(0.021@0.026@0.032)] \"in\""), "{}", region(&d, 5).1);
    assert!(region(&d, 6).1.ends_with("=[■(0.088@0.087@0.101)] \"in\""), "{}", region(&d, 6).1);
    assert!(region(&d, 7).1.ends_with("=0.087 \"in\""), "{}", region(&d, 7).1);
    assert_eq!(table_calc(&d, 0), CalcTable { id: 1, errors: Vec::new() });
    // A table whose errors don't change is left alone (and shared, for undo).
    let before = d.body[0].clone();
    recalc(&mut d, &no_page);
    assert!(Arc::ptr_eq(&before, &d.body[0]));
}

#[test]
fn editing_a_cell_recalculates() {
    let mut d = pipe_doc(PIPES);
    recalc(&mut d, &no_page);
    set_cell(&mut d, 0, 2, 2, "0.120");
    recalc(&mut d, &no_page);
    assert!(region(&d, 7).1.ends_with("=0.088 \"in\""), "{}", region(&d, 7).1);
}

#[test]
fn bad_cell_names_column_and_row() {
    let rows: &[&[&str]] = &[PIPES[0], PIPES[1], &["3/4", "", "0.113"], PIPES[3]];
    let mut d = pipe_doc(rows);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 5).0.state, CalcState::Error("depends on table data with an error".into()));
    // Through another calculation too.
    assert_eq!(region(&d, 6).0.state, err(CalcError::UpstreamTable));
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(1), message: "D_o row 2 isn't a number".into() }]);
    assert_eq!(cell_text(&d, 0, 2, 1), "", "the table's text is never written");
    // Fixing the cell clears the error.
    set_cell(&mut d, 0, 2, 1, "1.050");
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, []);
    assert_eq!(region(&d, 5).0.state, CalcState::Fresh);
}

#[test]
fn cell_unit_must_match_heading() {
    let wrong: &[&[&str]] = &[&["D_o (in)"], &["50 lbf"]];
    let mut d = doc_of(vec![table_block(data_table(1, wrong)), para_block(para(vec![calc_eq(1, "D_o[0] = ? in")]))]);
    recalc(&mut d, &no_page);
    let errors = table_calc(&d, 0).errors;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].col, Some(0));
    assert!(errors[0].message.starts_with("D_o row 1") && errors[0].message.contains("lbf"), "{errors:?}");
    assert_eq!(region(&d, 1).0.state, err(CalcError::UpstreamTable));

    let metric: &[&[&str]] = &[&["D_o (in)"], &["21.3 mm"]];
    let mut d = doc_of(vec![table_block(data_table(1, metric)), para_block(para(vec![calc_eq(1, "D_o[0] = ? in")]))]);
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, []);
    assert_eq!(region(&d, 1).1, "〖D_o〗_0=0.839 \"in\"");
}

#[test]
fn table_g_unit_follows_the_g_setting() {
    // `m (g)` would be refused whatever g means: m is the metre. `mass` is free.
    let heading: &[&[&str]] = &[&["mass (g)"], &["5"]];
    let cell: &[&[&str]] = &[&["mass_2 (kg)"], &["5 g"]];
    let doc = || {
        doc_of(vec![
            table_block(data_table(1, heading)),
            table_block(data_table(2, cell)),
            para_block(para(vec![calc_eq(1, "mass[0] = ? mg"), calc_eq(2, "mass_2[0] = ? mg")])),
        ])
    };
    // g is gravity (the default): both are errors naming gm.
    let mut d = doc();
    recalc(&mut d, &no_page);
    for at in [0, 1] {
        let errors = table_calc(&d, at).errors;
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].col, Some(0));
        assert!(errors[0].message.contains("gm"), "{errors:?}");
    }
    assert_eq!(region(&d, 1).0.state, err(CalcError::UpstreamTable));
    // g is the gram: both read as grams.
    let mut d = doc();
    d.calc_settings = Some(CalcSettings { g: GMeans::Gram, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, []);
    assert_eq!(table_calc(&d, 1).errors, []);
    assert_eq!(region(&d, 1).1, "mass_0=5000 \"mg\"");
    assert_eq!(region(&d, 2).1, "〖mass_2〗_0=5000 \"mg\"");
}

#[test]
fn merged_cells_are_an_error() {
    let merged = ColumnError { col: None, message: "a calculation data table can't have merged cells".into() };
    // Across: a data cell spans two columns.
    let mut d = pipe_doc(PIPES);
    {
        let t = table_mut(&mut d, 0);
        t.rows[2].cells.remove(2);
        t.rows[2].cells[1].props.span = 2;
    }
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, std::slice::from_ref(&merged));
    assert_eq!(region(&d, 5).0.state, err(CalcError::UpstreamTable));
    // Down: two cells of a column merged.
    let mut d = pipe_doc(PIPES);
    {
        let t = table_mut(&mut d, 0);
        t.rows[1].cells[0].props.vmerge = VMerge::Restart;
        t.rows[2].cells[0].props.vmerge = VMerge::Continue;
    }
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, [merged]);
    assert_eq!(region(&d, 5).0.state, err(CalcError::UpstreamTable));
}

#[test]
fn duplicate_names_follow_document_order() {
    let rows: &[&[&str]] = &[&["D_o (in)", "t (in)"], &["0.840", "0.109"]];
    // The table comes first: it owns D_o.
    let mut d = doc_of(vec![table_block(data_table(1, rows)), para_block(para(vec![calc_eq(1, "D_o := 1 in")]))]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::Duplicate { name: "D_o".into(), page: None }));
    assert_eq!(table_calc(&d, 0).errors, []);
    // The region comes first: the column is in error, the other column still reads.
    let mut d = doc_of(vec![
        para_block(para(vec![calc_eq(1, "D_o := 1 in")])),
        table_block(data_table(1, rows)),
        para_block(para(vec![calc_eq(2, "t[0] = ? in")])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, CalcState::Fresh);
    let duplicate = CalcError::Duplicate { name: "D_o".into(), page: None }.to_string();
    assert_eq!(table_calc(&d, 1).errors, [ColumnError { col: Some(0), message: duplicate }]);
    assert_eq!(region(&d, 2).1, "t_0=0.109 \"in\"");
    // A name a function has is taken too.
    let rows: &[&[&str]] = &[&["f"], &["1"]];
    let mut d = doc_of(vec![para_block(para(vec![calc_eq(1, "f(x) := 2·x")])), table_block(data_table(1, rows))]);
    recalc(&mut d, &no_page);
    let duplicate = CalcError::Duplicate { name: "f".into(), page: None }.to_string();
    assert_eq!(table_calc(&d, 1).errors, [ColumnError { col: Some(0), message: duplicate }]);
    // A new calculation can't take a column's name either.
    let mut d = doc_of(vec![table_block(data_table(1, rows))]);
    recalc(&mut d, &no_page);
    assert_eq!(check_new_name(&d, "f", None), Err(CalcError::Duplicate { name: "f".into(), page: None }));
}

#[test]
fn copied_data_table_is_a_duplicate() {
    let rows: &[&[&str]] = &[&["D_o (in)", "t (in)"], &["0.840", "0.109"]];
    let mut d =
        doc_of(vec![table_block(data_table(4, rows)), table_block(data_table(4, rows)), para_block(para(vec![calc_eq(1, "x := D_o + t =")]))]);
    assert_eq!(data_tables(&d), [(vec![0], 4), (vec![1], 5)]);
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0), CalcTable { id: 4, errors: Vec::new() });
    let copy = table_calc(&d, 1);
    assert_eq!(copy.id, 5, "the copy gets the next id above the largest");
    let duplicate = |name: &str| CalcError::Duplicate { name: name.into(), page: None }.to_string();
    assert_eq!(copy.errors, [ColumnError { col: Some(0), message: duplicate("D_o") }, ColumnError { col: Some(1), message: duplicate("t") }]);
    assert_eq!(region(&d, 1).0.state, CalcState::Fresh);
    assert!(region(&d, 1).1.ends_with("=[■(0.949)] \"in\""), "{}", region(&d, 1).1);
    // Nothing left to fix.
    assert_eq!(data_tables(&d), [(vec![0], 4), (vec![1], 5)]);
}

#[test]
fn tracked_deleted_cells_are_skipped() {
    let mut d = doc_of(vec![table_block(data_table(1, &[&["D_o (in)"], &["0.840"]])), para_block(para(vec![calc_eq(1, "D_o[0] = ? in")]))]);
    d.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "me", "2026-11-15T00:00:00Z"));
    let deleted = CharProps { del: Some(0), ..Default::default() };
    let mut p = Paragraph::with_text("0.840", deleted.clone());
    p.insert_text(p.text.len(), "0.850", &CharProps::default()).unwrap();
    table_mut(&mut d, 0).rows[1].cells[0].blocks = vec![para_block(p)];
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, []);
    assert_eq!(region(&d, 1).1, "〖D_o〗_0=0.85 \"in\"");
    // A whole table in a tracked deletion defines nothing.
    for row in &mut table_mut(&mut d, 0).rows {
        for cell in &mut row.cells {
            let text = match &*cell.blocks[0] {
                Block::Para(p) => p.final_text(),
                Block::Table(_) => unreachable!(),
            };
            cell.blocks = vec![para_block(Paragraph::with_text(&text, deleted.clone()))];
        }
    }
    assert_eq!(data_tables(&d), []);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::Undefined("D_o".into())));
    // Its id isn't reused: a pasted copy of a live table skips it.
    table_mut(&mut d, 0).calc = Some(Box::new(CalcTable { id: 7, errors: Vec::new() }));
    let live: &[&[&str]] = &[&["D_o (in)"], &["0.840"]];
    d.body.push(table_block(data_table(6, live)));
    d.body.push(table_block(data_table(6, live)));
    assert_eq!(data_tables(&d), [(vec![2], 6), (vec![3], 8)]);
}

#[test]
fn typing_outside_tables_skips_the_pass() {
    let mut d = pipe_doc(PIPES);
    recalc(&mut d, &no_page);
    let mut typed = d.clone();
    if let Block::Para(p) = Arc::make_mut(&mut typed.body[1]) {
        p.insert_text(0, "More ", &CharProps::default()).unwrap();
    }
    assert!(calc_unchanged(&d, &typed));
    let mut cell = d.clone();
    set_cell(&mut cell, 0, 2, 2, "0.120");
    assert!(!calc_unchanged(&d, &cell));
    let mut heading = d.clone();
    set_cell(&mut heading, 0, 0, 2, "t_n (in)");
    assert!(!calc_unchanged(&d, &heading));
    // Moving a calculation from after the table to before it changes who owns a name.
    let mut moved = d.clone();
    moved.body.swap(0, 1);
    assert!(!calc_unchanged(&d, &moved));
}

#[test]
fn heading_with_a_bad_unit_still_owns_its_name() {
    let rows: &[&[&str]] = &[&["D_o (furlongs)", "t (in)"], &["1", "2"]];
    let mut d = doc_of(vec![
        table_block(data_table(1, rows)),
        para_block(para(vec![calc_eq(1, "x := D_o =")])),
        para_block(para(vec![calc_eq(2, "D_o := 1 in")])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, CalcState::Error("depends on table data with an error".into()));
    assert_eq!(region(&d, 2).0.state, err(CalcError::Duplicate { name: "D_o".into(), page: None }));
    let errors = table_calc(&d, 0).errors;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].col, Some(0));
    assert!(errors[0].message.contains("furlongs"), "{errors:?}");
    // A heading holding an object, or text in two paragraphs, still names its column.
    let mut t = data_table(1, &[&["E_h", "F_h (in)"], &["1", "2"]]);
    let mut head = Paragraph::with_text("E_h ", CharProps::default());
    head.insert_object(head.text.len(), field("PAGE"), &CharProps::default()).unwrap();
    t.rows[0].cells[0].blocks = vec![para_block(head)];
    t.rows[0].cells[1].blocks =
        vec![para_block(Paragraph::with_text("F_h", CharProps::default())), para_block(Paragraph::with_text("(in)", CharProps::default()))];
    let mut d = doc_of(vec![table_block(t), para_block(para(vec![calc_eq(1, "E_h + F_h =")])), para_block(para(vec![calc_eq(2, "F_h := 1")]))]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::UpstreamTable));
    assert_eq!(region(&d, 2).0.state, err(CalcError::Duplicate { name: "F_h".into(), page: None }));
    let cols: Vec<Option<u32>> = table_calc(&d, 0).errors.iter().map(|e| e.col).collect();
    assert_eq!(cols, [Some(0), Some(1)]);
}

#[test]
fn a_second_heading_row_is_an_error() {
    let mut d = pipe_doc(PIPES);
    table_mut(&mut d, 0).rows[0].props.header = true;
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, [], "the first row may repeat as a heading row");
    table_mut(&mut d, 0).rows[2].props.header = true;
    recalc(&mut d, &no_page);
    let error = ColumnError { col: None, message: "only the first row of a calculation data table can be a heading row".into() };
    assert_eq!(table_calc(&d, 0).errors, [error]);
    assert_eq!(region(&d, 5).0.state, err(CalcError::UpstreamTable));
}

#[test]
fn a_split_data_table_stays_data_in_both_parts() {
    let mut head = data_table(1, PIPES);
    let tail = head.split_off(2);
    assert_eq!(tail.calc, head.calc);
    let mut d = doc_of(vec![table_block(head), table_block(tail), para_block(para(vec![calc_eq(1, "x := D_o =")]))]);
    assert_eq!(data_tables(&d), [(vec![0], 1), (vec![1], 2)]);
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0), CalcTable { id: 1, errors: Vec::new() });
    // The tail's first row is read as headings: every column is in error, never silently short.
    let tail = table_calc(&d, 1);
    assert_eq!(tail.id, 2);
    let cols: Vec<Option<u32>> = tail.errors.iter().map(|e| e.col).collect();
    assert_eq!(cols, [Some(0), Some(1), Some(2)], "{:?}", tail.errors);
}

#[test]
fn tracked_deleted_rows_are_skipped() {
    let mut d = pipe_doc(PIPES);
    d.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "me", "2026-11-15T00:00:00Z"));
    let deleted = CharProps { del: Some(0), ..Default::default() };
    // Row 2 deleted with tracking on: all of its text is marked deleted.
    for (c, text) in PIPES[2].iter().enumerate() {
        table_mut(&mut d, 0).rows[2].cells[c].blocks = vec![para_block(Paragraph::with_text(text, deleted.clone()))];
    }
    // Row 3 has only some text deleted: it reads what is left.
    let mut p = Paragraph::with_text("1.315", CharProps::default());
    p.insert_text(p.text.len(), "9", &deleted).unwrap();
    table_mut(&mut d, 0).rows[3].cells[1].blocks = vec![para_block(p)];
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, []);
    assert!(region(&d, 5).1.ends_with("=[■(0.021@0.032)] \"in\""), "{}", region(&d, 5).1);
    assert!(region(&d, 6).1.ends_with("=[■(0.088@0.101)] \"in\""), "{}", region(&d, 6).1);
    // A row with one cell's text all deleted but the rest live isn't deleted: the cell is blank.
    let mut d = pipe_doc(PIPES);
    table_mut(&mut d, 0).rows[2].cells[1].blocks = vec![para_block(Paragraph::with_text("1.050", deleted))];
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(1), message: "D_o row 2 isn't a number".into() }]);
}

#[test]
fn data_cells_hold_numbers_only() {
    // A calculation in a data cell: an error for its column, though the calculation still works.
    let mut t = data_table(1, &[&["a", "b"], &["1", "2"]]);
    t.rows[1].cells[1].blocks = vec![para_block(para(vec![calc_eq(1, "z := 3 =")]))];
    // A table nested in a data cell.
    let mut u = data_table(2, &[&["c"], &["1"]]);
    u.rows[1].cells[0].blocks.push(Arc::new(Block::Table(Table::new(1, 1, 100.0))));
    let mut d = doc_of(vec![table_block(t), table_block(u), para_block(para(vec![calc_eq(2, "a + z =")]))]);
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(1), message: "b row 1 holds an object — a data cell holds a number only".into() }]);
    assert_eq!(table_calc(&d, 1).errors, [ColumnError { col: Some(0), message: "c row 1 holds a table — a data cell holds a number only".into() }]);
    assert_eq!(region(&d, 1).0.state, CalcState::Fresh);
    assert!(region(&d, 2).1.ends_with("=[■(4)]"), "{}", region(&d, 2).1);
    // A column of more than 10,000 values.
    let values = vec!["1"; 10_001];
    let mut rows: Vec<&[&str]> = vec![&["x"]];
    rows.extend(values.iter().map(std::slice::from_ref));
    let mut d = doc_of(vec![table_block(data_table(1, &rows)), para_block(para(vec![calc_eq(1, "x =")]))]);
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(0), message: CalcError::TooLarge.to_string() }]);
    assert_eq!(region(&d, 1).0.state, err(CalcError::UpstreamTable));
}

#[test]
#[ignore = "timing: run with --release -- --ignored perf_1000_element_vector_chain"]
fn perf_1000_element_vector_chain() {
    let values: Vec<String> = (0..1000).map(|i| format!("{}.5", i % 37)).collect();
    let mut rows: Vec<Vec<&str>> = vec![vec!["x (in)"]];
    rows.extend(values.iter().map(|v| vec![v.as_str()]));
    let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
    let mut sources = vec!["y_0 := x".to_string()];
    sources.extend((1..200).map(|k| format!("y_{k} := y_{}·1.001 + x", k - 1)));
    let mut blocks = vec![table_block(data_table(1, &rows))];
    blocks.extend(sources.iter().enumerate().map(|(i, s)| para_block(para(vec![calc_eq(i as u64 + 1, s)]))));
    let mut d = doc_of(blocks);
    recalc(&mut d, &no_page);
    let _snapshot = d.clone();
    set_cell(&mut d, 0, 1, 0, "2 in");
    let start = Instant::now();
    let report = recalc(&mut d, &no_page);
    let took = start.elapsed();
    println!("perf_1000_element_vector_chain: {took:?}");
    assert_eq!(report.regions, 200);
    assert_eq!(report.errors, 0);
    assert_eq!(table_calc(&d, 0).errors, []);
    assert!(took.as_millis() < 100, "recalc took {took:?}");
}

#[test]
fn columns_are_listed_by_their_table_id() {
    let mut d = pipe_doc(PIPES);
    // The table's id repeats a region's: the two are id spaces of their own.
    assert_eq!(table_calc(&d, 0).id, 1);
    let columns = defined_columns(&d);
    assert_eq!(columns, [("D_o".to_string(), 1, 1), ("NPS".to_string(), 1, 0), ("t_nom".to_string(), 1, 2)]);
    assert!(defined_names(&d).iter().all(|(n, _)| !columns.iter().any(|(c, ..)| c == n)), "regions only");
    // A column in error still owns its name; a column whose name a region owns first doesn't.
    set_cell(&mut d, 0, 2, 2, "abc");
    let mut blocks = vec![para_block(para(vec![calc_eq(9, "NPS := 1")]))];
    blocks.extend(d.body.iter().cloned());
    let d = doc_of(blocks);
    assert_eq!(defined_columns(&d), [("D_o".to_string(), 1, 1), ("t_nom".to_string(), 1, 2)]);
}

#[test]
fn a_quoted_index_must_pick_an_element() {
    let mut d = pipe_doc(PIPES);
    d.body.push(para_block(para(vec![calc_eq(8, "M := [1, 2; 3, 4]"), calc_eq(9, "bad := (")])));
    assert_eq!(check_quote_index(&d, "D_o", (2, None)), Ok(()));
    assert_eq!(check_quote_index(&d, "M", (1, Some(0))), Ok(()));
    assert_eq!(check_quote_index(&d, "P", (0, None)), Err(CalcError::IndexRange("P isn't a vector".into())));
    let outside = check_quote_index(&d, "D_o", (3, None)).unwrap_err().to_string();
    assert_eq!(outside, "index 3 is outside D_o (0 to 2)");
    assert!(check_quote_index(&d, "M", (1, None)).unwrap_err().to_string().contains("M is a matrix"));
    // Counted from the document's origin.
    d.calc_settings = Some(CalcSettings { origin: 1, ..CalcSettings::default() });
    assert_eq!(check_quote_index(&d, "D_o", (3, None)), Ok(()));
    assert!(check_quote_index(&d, "D_o", (0, None)).is_err());
    // No value now: checked when it has one.
    assert_eq!(check_quote_index(&d, "bad", (0, None)), Ok(()));
}

#[test]
fn text_results_show_and_quote() {
    let mut d = doc_with(&["c := \"OK\" =", "v := [\"OK\", \"NG\", \"OK\"] ="]);
    d.body.push(para_block(para(vec![
        field(&quote("c", None, None, None)),
        field(&quote("v", None, None, None)),
        field(&quote("v", None, None, Some((1, None)))),
    ])));
    recalc(&mut d, &no_page);
    assert!(region(&d, 1).1.ends_with("=\"OK\""));
    assert!(region(&d, 2).1.ends_with("=[■(\"OK\"@\"NG\"@\"OK\")]"), "{}", region(&d, 2).1);
    assert_eq!(field_results(&d), ["OK", "OK, NG, OK", "NG"]);
    assert!(check_quote_unit(&d, "c", "in").unwrap_err().to_string().contains("can't be used with a unit"));
    // Stale: the text result is read back and shown with ≈.
    mark_stale(&mut d);
    assert!(region(&d, 1).1.ends_with("≈\"OK\""));

    // A text with `=` and `&` in it reads back too.
    let mut d = doc_with(&["c := \"t = 0.2 & up\" ="]);
    recalc(&mut d, &no_page);
    mark_stale(&mut d);
    assert!(region(&d, 1).1.ends_with("≈\"t = 0.2 & up\""), "{}", region(&d, 1).1);
}

#[test]
fn a_reserved_word_name_fails_alone() {
    let mut d = doc_with(&["a := 1", "or := 2", "b := a + 1 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 2).0.state, CalcState::Error("or is a reserved word — choose another name".into()));
    assert_eq!(region(&d, 3).0.state, CalcState::Fresh);
}

#[test]
fn a_use_of_a_word_defined_as_a_name_fails_with_the_word() {
    let word = |w: &str| err(CalcError::Keyword(w.into()));
    // `or` and `not` defined as names (before they were keywords): every use as an operator fails.
    let mut d = doc_with(&["or := 2", "not := 3", "a := 1", "b := 0", "x := a or b =", "y := not b =", "z := a and b =", "w := a + b ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 5).0.state, word("or"));
    assert_eq!(region(&d, 6).0.state, word("not"));
    // `and` is not defined as a name here.
    assert_eq!(region(&d, 7).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 8).0.state, CalcState::Fresh);
    // Anything built on a failed calculation is upstream, as ever.
    let mut d = doc_with(&["or := 2", "a := 1", "b := 0", "x := a or b", "c := x + 1 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 4).0.state, word("or"));
    assert_eq!(region(&d, 5).0.state, err(CalcError::Upstream));
    // The same document with no such definition computes.
    let mut d = doc_with(&["a := 1", "b := 0", "x := a or b =", "y := not b ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, CalcState::Fresh);
    assert!(region(&d, 3).1.ends_with("=1"), "{}", region(&d, 3).1);
    assert_eq!(region(&d, 4).0.state, CalcState::Fresh);
    assert!(region(&d, 4).1.ends_with("=1"), "{}", region(&d, 4).1);
}

#[test]
fn a_function_parameter_that_is_a_word_fails_its_uses() {
    let mut d = doc_with(&["f(or) := or + 1", "a := 1", "x := a or 0 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::Keyword("or".into())));
    assert_eq!(region(&d, 3).0.state, err(CalcError::Keyword("or".into())));
}

#[test]
fn a_data_table_heading_that_is_a_word_fails_its_uses() {
    let mut d = doc_of(vec![table_block(data_table(1, &[&["not", "x"], &["1", "2"]]))]);
    d.body.push(Arc::new(Block::Para(para(vec![calc_eq(1, "y := not 0 =")]))));
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::Keyword("not".into())));
}

#[test]
fn a_use_of_if_or_otherwise_defined_as_a_name_fails_with_the_word() {
    let word = |w: &str| err(CalcError::Keyword(w.into()));
    let mut d = doc_with(&["if := 2", "a := 1", "x := 1 if a > 0, 2 otherwise =", "y := if(a > 0, 1, 2) =", "z := a + 1 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, word("if"));
    assert_eq!(region(&d, 4).0.state, word("if"));
    assert_eq!(region(&d, 5).0.state, CalcState::Fresh);
    let mut d = doc_with(&["otherwise := 2", "a := 1", "x := 1 if a > 0, 2 otherwise =", "y := 1 if a > 0 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, word("otherwise"));
    assert_eq!(region(&d, 4).0.state, CalcState::Fresh);
}

#[test]
fn piecewise_definitions_recalculate() {
    let mut d = doc_with(&["T := 1000 °F", "Y := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F, 0.7 otherwise =", "Z := 1 if T < 0 °F ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 2).0.state, CalcState::Fresh);
    assert!(region(&d, 2).1.ends_with(")┤=0.7"), "{}", region(&d, 2).1);
    assert_eq!(region(&d, 3).0.state, CalcState::Error("no case applies (T = 1000 °F)".into()));
    // Stale, the result is read back from after the brace.
    mark_stale(&mut d);
    assert!(region(&d, 2).1.ends_with(")┤≈0.7"), "{}", region(&d, 2).1);
    // A text result, with `==` in a condition and `=` inside a text, reads back too.
    let mut d = doc_with(&["T := 1000 °F", "c := \"a = 1\" if T == 1000 °F, \"b = 2\" otherwise ="]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 2).1.ends_with(")┤=\"a = 1\""), "{}", region(&d, 2).1);
    mark_stale(&mut d);
    assert!(region(&d, 2).1.ends_with(")┤≈\"a = 1\""), "{}", region(&d, 2).1);
    // Every name in every branch is a dependency: a failure upstream fails it, picked or not.
    let mut d = doc_with(&["a := 1 in + 1 lbf", "Y := 1 if 1 > 0, a otherwise ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 2).0.state, err(CalcError::Upstream));
}

#[test]
#[ignore = "timing: run with --release -- --ignored perf_1000_element_piecewise"]
fn perf_1000_element_piecewise() {
    let values: Vec<String> = (0..1000).map(|i| format!("{}.5", i % 37)).collect();
    let mut rows: Vec<Vec<&str>> = vec![vec!["x (in)"]];
    rows.extend(values.iter().map(|v| vec![v.as_str()]));
    let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
    let mut sources = vec!["check := \"OK\" if x ≥ 10 in, \"NG\" otherwise".to_string(), "n_NG := sum(check == \"NG\") =".to_string()];
    // Branches that fail for the elements that don't pick them: a negative root, and a division by
    // zero at x = 0.5 in.
    sources.extend((0..50).map(|k| format!("y_{k} := sqrt(x/(1 in) - {k}) if x ≥ {k} in, 1/(x/(1 in) - 0.5) if x > 0.5 in, 0 otherwise")));
    // A function with no `otherwise`, used only where it has a case: its other elements fail with
    // a no-case message that nothing reads.
    sources.push("neg(v) := 5 if v < 1 in".to_string());
    sources.extend((0..20).map(|k| format!("n_{k} := neg(x) if x < 1 in, {k} otherwise")));
    let mut blocks = vec![table_block(data_table(1, &rows))];
    blocks.extend(sources.iter().enumerate().map(|(i, s)| para_block(para(vec![calc_eq(i as u64 + 1, s)]))));
    let mut d = doc_of(blocks);
    recalc(&mut d, &no_page);
    let _snapshot = d.clone();
    set_cell(&mut d, 0, 1, 0, "2 in");
    let start = Instant::now();
    let report = recalc(&mut d, &no_page);
    let took = start.elapsed();
    println!("perf_1000_element_piecewise: {took:?}");
    assert_eq!(report.regions, 73);
    assert_eq!(report.errors, 0, "{:?}", regions(&d).iter().map(|r| r.0.state.clone()).collect::<Vec<_>>());
    assert!(region(&d, 2).1.ends_with("=271"), "{}", region(&d, 2).1);
    assert!(took.as_millis() < 100, "recalc took {took:?}");
}

/// The plan's sample shape: a pass/fail check and its count over a 1,000-row column, and 50
/// element-wise piecewise regions. (The column is `w`: `m` is the metre.)
#[test]
#[ignore = "timing: run with --release -- --ignored perf_1000_element_piecewise_regions"]
fn perf_1000_element_piecewise_regions() {
    let values: Vec<String> = (0..1000).map(|i| format!("{}.5", i % 37)).collect();
    let mut rows: Vec<Vec<&str>> = vec![vec!["w (in)"]];
    rows.extend(values.iter().map(|v| vec![v.as_str()]));
    let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
    let mut sources = vec!["check := \"OK\" if w ≥ 10 in, \"NG\" otherwise".to_string(), "n := sum(check == \"NG\") =".to_string()];
    sources.extend((0..50).map(|k| format!("y_{k} := w·1.001 if w > {k} in, w otherwise")));
    let mut blocks = vec![table_block(data_table(1, &rows))];
    blocks.extend(sources.iter().enumerate().map(|(i, s)| para_block(para(vec![calc_eq(i as u64 + 1, s)]))));
    let mut d = doc_of(blocks);
    recalc(&mut d, &no_page);
    let _snapshot = d.clone();
    set_cell(&mut d, 0, 1, 0, "2 in");
    let start = Instant::now();
    let report = recalc(&mut d, &no_page);
    let took = start.elapsed();
    println!("perf_1000_element_piecewise_regions: {took:?}");
    assert_eq!(report.regions, 52);
    assert_eq!(report.errors, 0, "{:?}", regions(&d).iter().map(|r| r.0.state.clone()).collect::<Vec<_>>());
    assert!(region(&d, 2).1.ends_with("=271"), "{}", region(&d, 2).1);
    assert!(took.as_millis() < 100, "recalc took {took:?}");
}

#[test]
fn a_sum_index_defined_elsewhere_is_reported() {
    let mut d = doc_with(&["tot := ∑(i = 1..3) i =", "i := 0..5"]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, CalcState::Error("the sum's index i is already defined — choose another name".into()));
}

#[test]
fn a_sum_calculates_and_shows_in_a_document() {
    let mut d = doc_with(&["a := [1, 2, 3]", "n := 2", "tot := ∑(i = 0..n) a[i] =", "∑(i = 1..4) i^2 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 3).1, "tot≔∑_(i=0)^n▒〖a_i〗=6");
    assert_eq!(region(&d, 4).1, "∑_(i=1)^4▒〖i^2〗=30");
    // The sum is a real ∑ in the tree.
    assert!(region(&d, 4).2.nodes.iter().any(|n| matches!(n, MNode::Nary { chr: '∑', .. })));
}

const MATERIALS: &[&[&str]] = &[&["Material (text)", "Sa (ksi)"], &["A106 B", "20"], &["A312 TP316", "20"], &["A333 6", "20"]];

#[test]
fn text_columns_feed_lookups() {
    let mut d = doc_of(vec![
        table_block(data_table(1, MATERIALS)),
        para_block(para(vec![calc_eq(8, "S_allow := lookup(Material, Sa, \"A106 B\") =")])),
        para_block(para(vec![calc_eq(9, "grade := lookup(Sa, Material, 20 ksi) =")])),
        para_block(para(vec![calc_eq(10, "bad := lookup(Material, Sa, \"A106 C\") =")])),
    ]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 8).1.ends_with("=20000 \"psi\""), "{}", region(&d, 8).1);
    assert!(matches!(region(&d, 9).0.state, CalcState::Error(ref m) if m.contains("matches more than one value in Sa")));
    assert_eq!(region(&d, 10).0.state, CalcState::Error("\"A106 C\" isn't in Material".into()));
    set_cell(&mut d, 0, 2, 0, "");
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(0), message: "Material row 2 is empty".into() }]);
    set_cell(&mut d, 0, 2, 0, "1/2\" pipe");
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors[0].message, "Material row 2 has a quote mark \", which text can't hold");
    // A number in a text column is text; a numeric column stays strict.
    set_cell(&mut d, 0, 2, 0, "6");
    set_cell(&mut d, 0, 3, 1, "twenty");
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), ["Sa row 3 isn't a number"]);
}

#[test]
fn a_fahrenheit_column_compares_as_temperatures() {
    // Review Focus 2.
    let mut d = doc_of(vec![
        table_block(data_table(1, &[&["T (°F)"], &["800"], &["950"]])),
        para_block(para(vec![calc_eq(8, "Y := 0.4 if T ≤ 900 °F, 0.7 otherwise =")])),
    ]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 8).1.ends_with("=[■(0.4@0.7)]"), "{}", region(&d, 8).1);
}

#[test]
fn text_lookups_follow_the_kinds_and_refuse_a_mismatch() {
    let mut d = doc_of(vec![
        table_block(data_table(1, MATERIALS)),
        para_block(para(vec![calc_eq(8, "a := lookup(Material, Sa, [\"A333  6\", \"A106 B\"]) =")])),
        para_block(para(vec![calc_eq(9, "b := lookup(Material, Sa, 20 ksi) =")])),
        para_block(para(vec![calc_eq(10, "c := lookup(Sa, Material, \"A106 B\") =")])),
        para_block(para(vec![calc_eq(11, "pair := lookup(Material, Material, [\"A106 B\", \"A333 6\"]) =")])),
    ]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 8).1.ends_with("=[■(20000@20000)] \"psi\""), "{}", region(&d, 8).1);
    assert_eq!(region(&d, 9).0.state, CalcState::Error("can't compare \"A106 B\" with a number".into()));
    assert_eq!(region(&d, 10).0.state, CalcState::Error("can't compare \"A106 B\" with a number".into()));
    assert_eq!(region(&d, 11).0.state, CalcState::Fresh);
    assert!(region(&d, 11).1.ends_with("=[■(\"A106 B\"@\"A333 6\")]"), "{}", region(&d, 11).1);
}

#[test]
fn numbers_look_up_texts_and_equal_text_keys_are_duplicates() {
    const GRADES: &[&[&str]] = &[&["Grade (text)", "P (psi)"], &["A106 B", "100"], &["A106  B", "200"], &["A333 6", "300"]];
    let mut d = doc_of(vec![
        table_block(data_table(1, GRADES)),
        para_block(para(vec![calc_eq(8, "grade_a := lookup(P, Grade, 300 psi) =")])),
        para_block(para(vec![calc_eq(9, "p_dup := lookup(Grade, P, \"A106 B\") =")])),
        para_block(para(vec![calc_eq(10, "p_ok := lookup(Grade, P, \"A333  6\") =")])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 8).0.state, CalcState::Fresh);
    assert!(region(&d, 8).1.ends_with("=\"A333 6\""), "{}", region(&d, 8).1);
    // `A106 B` and `A106  B` are the same text, so which row is meant is a question.
    assert_eq!(region(&d, 9).0.state, CalcState::Error("\"A106 B\" matches more than one value in Grade".into()));
    assert_eq!(region(&d, 10).0.state, CalcState::Fresh);
    assert!(region(&d, 10).1.ends_with("=300 \"psi\""), "{}", region(&d, 10).1);
}

#[test]
fn an_h_heading_hints_hr() {
    let mut d = doc_of(vec![table_block(data_table(1, &[&["Time (h)", "Flow (gpm)"], &["1", "2"]]))]);
    recalc(&mut d, &no_page);
    let message = "unknown unit `h` — for hours, use hr".to_string();
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(0), message }]);
    // A cell's own unit, the same way.
    let mut d = doc_of(vec![table_block(data_table(1, &[&["Time (hr)"], &["1 h"]]))]);
    recalc(&mut d, &no_page);
    let message = "Time row 1: unknown unit `h` — for hours, use hr".to_string();
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(0), message }]);
}

#[test]
fn a_quote_mark_of_any_kind_is_refused_in_a_text_cell() {
    let mut d = doc_of(vec![table_block(data_table(1, MATERIALS))]);
    for mark in ["\"", "“", "”"] {
        set_cell(&mut d, 0, 2, 0, &format!("1/2{mark} pipe"));
        recalc(&mut d, &no_page);
        let message = format!("Material row 2 has a quote mark {mark}, which text can't hold");
        assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(0), message }]);
    }
}

/// The shown result of each calculation in `sources` (its text after the last `=`), in a document
/// with `system` units.
fn shown_in(system: wordcraft_doc::calc::UnitSystem, sources: &[&str]) -> Vec<String> {
    let mut d = doc_with(sources);
    d.calc_settings = Some(CalcSettings { system, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    (1..=sources.len() as u64).map(|id| region(&d, id).1).collect()
}

#[test]
fn percent_shows_as_a_unit() {
    let shown = shown_in(wordcraft_doc::calc::UnitSystem::UsCustomary, &["x := 0.25", "x = ? %", "y := 5 % * 200 ="]);
    assert_eq!(shown[1], "x=25 \"%\"");
    assert_eq!(shown[2], "y≔5 \"%\"·200=10");
}

#[test]
fn hour_is_hr() {
    let shown = shown_in(wordcraft_doc::calc::UnitSystem::UsCustomary, &["q := 3412.14 Btu/hr", "q = ? watt"]);
    assert_eq!(shown[1], "q=1000 \"watt\"");
}

#[test]
fn typed_units_show_as_typed() {
    let shown = shown_in(
        wordcraft_doc::calc::UnitSystem::UsCustomary,
        &["q := 1 kW", "q = ? BTU/hr", "q = ? Btu/hr", "t := 90 s", "t = ? s", "t = ? sec", "t = ? min"],
    );
    assert_eq!(shown[1], "q=3412.142 \"BTU/hr\"");
    assert_eq!(shown[2], "q=3412.142 \"Btu/hr\"");
    assert_eq!(shown[4], "t=90 \"s\"");
    assert_eq!(shown[5], "t=90 \"sec\"");
    assert_eq!(shown[6], "t=1.5 \"min\"");
}

#[test]
fn si_results_spell_units() {
    let shown = shown_in(wordcraft_doc::calc::UnitSystem::Si, &["1 kJ/sec =", "9.8 m/sec^2 =", "5 s ="]);
    let result = |text: &str| text.rsplit('=').next().unwrap_or_default().to_string();
    assert_eq!(result(&shown[0]), "1000 \"watt\"");
    assert_eq!(result(&shown[1]), "9.8 \"m/sec²\"");
    assert_eq!(result(&shown[2]), "5 \"sec\"");
}

#[test]
fn percent_displays_upright_as_a_unit() {
    assert_eq!(render::display("x := 5 %", None, &CalcState::Fresh), "x≔5 \"%\"");
}

// ---------------------------------------------------------------------------------------------
// Solve blocks
// ---------------------------------------------------------------------------------------------

/// Whether `a` agrees with Mathcad's `b` to three significant figures: `|a − b| ≤ 0.5·10^(⌊log10|b|⌋ − 2)`.
/// The crate's own `sig3` is test-only and so out of reach of an integration test: this is it.
fn sig3(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.5 * 10f64.powi(b.abs().log10().floor() as i32 - 2)
}

/// A document of `sources` (one region per paragraph, ids from 1) recalculated at full precision.
fn solved_doc(sources: &[&str]) -> Document {
    let mut d = doc_with(sources);
    d.calc_settings = Some(CalcSettings { precision: 15, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    d
}

/// The number region `id` shows (its text after the last `=`, before any unit), with an exponent
/// `×〖10〗^(…)` read as one.
fn shown_number(d: &Document, id: u64) -> f64 {
    let (r, linear, _) = region(d, id);
    assert_eq!(r.state, CalcState::Fresh, "region {id}: {linear}");
    let result = linear.rsplit('=').next().unwrap_or_default();
    let number = result.split(" \"").next().unwrap_or_default();
    let plain: String = number.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '×' | '^')).collect();
    let value = match plain.split_once("×10^") {
        Some((mantissa, exp)) => format!("{mantissa}e{exp}"),
        None => plain,
    };
    value.parse().unwrap_or_else(|e| panic!("region {id} shows {linear:?}: {e}"))
}

/// A region that shows nothing: current, with no `=` result after it (an equation's own `==` aside).
fn shows_nothing(d: &Document, id: u64) {
    let (r, linear, _) = region(d, id);
    assert_eq!(r.state, CalcState::Fresh, "region {id}: {linear}");
    assert!(!linear.replace("==", "").contains('='), "region {id}: {linear}");
}

fn syntax(message: &str) -> CalcState {
    err(CalcError::Syntax(message.into()))
}

#[test]
fn simple_block() {
    for closing in ["x_s := Find(x) =", "Find(x) ="] {
        let d = solved_doc(&["x := 1", "Given", "x^2 == 2", closing]);
        let x = shown_number(&d, 4);
        assert!(sig3(x, 2f64.sqrt()), "{closing}: {x}");
        // The equation holds at the answer.
        assert!((x * x - 2.0).abs() <= 1e-9, "{closing}: {x}");
        shows_nothing(&d, 2);
        shows_nothing(&d, 3);
        assert_eq!(region(&d, 2).1, "Given");
        assert!(region(&d, 4).1.contains("Find(x)"), "{}", region(&d, 4).1);
    }
}

const UNCLOSED: &str = "this Given has no Find or Minerr below it to end its block";
const NEEDS_GIVEN: &str = "an equation needs a Given above it";
const INVALID_MEMBER: &str = "a Given block accepts only starting values for its unknowns followed by equations";

#[test]
fn block_errors() {
    // An unclosed Given: an error on it alone. What follows works as if it weren't there, and its
    // equation shows nothing, as a block's does.
    let d = solved_doc(&["x := 1", "Given", "x^2 == 2"]);
    assert_eq!(region(&d, 2).0.state, syntax(UNCLOSED));
    shows_nothing(&d, 3);
    // A second Given leaves the first block unended and starts its own, unended too.
    let d = solved_doc(&["x := 1", "Given", "x^2 == 2", "y := 3", "Given", "z := y + 1 ="]);
    assert_eq!(region(&d, 2).0.state, syntax(UNCLOSED));
    shows_nothing(&d, 3);
    assert_eq!(region(&d, 4).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 5).0.state, syntax(UNCLOSED));
    assert_eq!(shown_number(&d, 6), 4.0);
    let d = solved_doc(&["x := 1", "Given", "x^2 == 2", "y := 3", "z := y + 1 ="]);
    assert_eq!(region(&d, 4).0.state, CalcState::Fresh);
    assert_eq!(shown_number(&d, 5), 4.0);
    // A block with no equation.
    let d = solved_doc(&["x := 1", "Given", "x_s := Find(x)", "y := x_s + 1 ="]);
    assert_eq!(region(&d, 3).0.state, syntax("a Given block needs at least one equation"));
    assert_eq!(region(&d, 4).0.state, err(CalcError::Upstream));
    shows_nothing(&d, 2);
    // A stray Find, in each form that would end a block.
    for (stray, using) in [
        ("x_s := Find(x)", "y := x_s + 1 ="),
        ("f(a) := Find(x)", "y := f(2) ="),
        ("[a; b] := Minerr(x, x)", "y := a + 1 ="),
        ("Find(x) =", "y := 1 ="),
    ] {
        let d = solved_doc(&["x := 1", stray, using]);
        let kind = if stray.contains("Find") { "Find" } else { "Minerr" };
        assert_eq!(region(&d, 2).0.state, syntax(&format!("{kind} must end a Given block")), "{stray}");
        if !stray.starts_with("Find") {
            assert_eq!(region(&d, 3).0.state, err(CalcError::Upstream), "{stray}");
        }
    }
    // An equation with no Given above it, and the Find after it.
    let d = solved_doc(&["x := 1", "x^2 == 2", "x_s := Find(x)", "y := x_s + 1 ="]);
    assert_eq!(region(&d, 2).0.state, syntax(NEEDS_GIVEN));
    assert_eq!(region(&d, 3).0.state, syntax("Find must end a Given block"));
    assert_eq!(region(&d, 4).0.state, err(CalcError::Upstream));
    // A definition inside a block, or an unknown that isn't a name: the block's result fails, and
    // so does every use of it.
    for (inside, closing, at, message) in [
        ("y := 2", "x_s := Find(x)", 4, INVALID_MEMBER),
        ("x =", "x_s := Find(x)", 4, INVALID_MEMBER),
        ("x == 1.5", "x_s := Find(2·x)", 5, "Find's arguments are the unknowns, like Find(x, y)"),
        ("x == 1.5", "x_s := Minerr()", 5, "Minerr's arguments are the unknowns, like Minerr(x, y)"),
    ] {
        let d = solved_doc(&["x := 1", "Given", "x^2 == 2", inside, closing, "z := x_s + 1 ="]);
        assert_eq!(region(&d, at).0.state, syntax(message), "{inside} / {closing}");
        if at == 4 {
            assert_eq!(region(&d, 5).0.state, err(CalcError::Upstream), "{inside}");
        }
        assert_eq!(region(&d, 6).0.state, err(CalcError::Upstream), "{inside} / {closing}");
        shows_nothing(&d, 2);
        shows_nothing(&d, 3);
    }
    // A second Given before the first block's Find: the first is unended, and the second block,
    // with no equation of its own, fails, and so does every use of it.
    let d = solved_doc(&["x := 1", "Given", "x^2 == 2", "Given", "x_s := Find(x)", "z := x_s + 1 ="]);
    assert_eq!(region(&d, 2).0.state, syntax(UNCLOSED));
    shows_nothing(&d, 3);
    shows_nothing(&d, 4);
    assert_eq!(region(&d, 5).0.state, syntax("a Given block needs at least one equation"));
    assert_eq!(region(&d, 6).0.state, err(CalcError::Upstream));
    // After a block, a new one may start.
    let d = solved_doc(&["x := 1", "Given", "x^2 == 4", "x_a := Find(x) =", "Given", "x^2 == 9", "x_b := Find(x) ="]);
    let (a, b) = (shown_number(&d, 4), shown_number(&d, 7));
    assert!(sig3(a, 2.0) && sig3(b, 3.0), "{a}, {b}");
    assert!((a * a - 4.0).abs() <= 1e-9 && (b * b - 9.0).abs() <= 1e-9, "{a}, {b}");
}

#[test]
fn deleted_given() {
    let mut d = Document::new();
    d.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "me", "2026-11-17T00:00:00Z"));
    let deleted = CharProps { del: Some(0), ..Default::default() };
    d.body = vec![
        para_block(para(vec![calc_eq(1, "x := 1")])),
        para_block(para_with(vec![calc_eq(2, "Given")], &deleted)),
        para_block(para(vec![calc_eq(3, "x^2 == 2")])),
        para_block(para(vec![calc_eq(4, "x^2 == 2")])),
        para_block(para(vec![calc_eq(5, "x_s := Find(x)")])),
        para_block(para(vec![calc_eq(6, "x^2 == a")])),
        para_block(para(vec![calc_eq(7, "f(a) := Find(x)")])),
        para_block(para(vec![calc_eq(8, "f(2) =")])),
    ];
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, syntax(NEEDS_GIVEN));
    assert_eq!(region(&d, 4).0.state, syntax(NEEDS_GIVEN));
    assert_eq!(region(&d, 5).0.state, syntax("Find must end a Given block"));
    // A block used as a function too: never defined, so its use is upstream.
    assert_eq!(region(&d, 6).0.state, syntax(NEEDS_GIVEN));
    assert_eq!(region(&d, 7).0.state, syntax("Find must end a Given block"));
    assert_eq!(region(&d, 8).0.state, err(CalcError::Upstream));
    // The deleted Given is left as it was.
    assert_eq!(region(&d, 2).0.state, CalcState::Fresh);
    // A deleted Given inside a live block is no second Given, and a deleted equation joins nothing.
    let mut d = Document::new();
    d.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "me", "2026-11-17T00:00:00Z"));
    d.body = vec![
        para_block(para(vec![calc_eq(1, "x := 1")])),
        para_block(para(vec![calc_eq(2, "Given")])),
        para_block(para_with(vec![calc_eq(3, "Given")], &deleted)),
        para_block(para_with(vec![calc_eq(4, "x^2 == 9")], &deleted)),
        para_block(para(vec![calc_eq(5, "x^2 == 2")])),
        para_block(para(vec![calc_eq(6, "x_s := Find(x)")])),
        para_block(para(vec![calc_eq(7, "x_s =")])),
    ];
    d.calc_settings = Some(CalcSettings { precision: 15, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    let x = shown_number(&d, 7);
    assert!(sig3(x, 2f64.sqrt()) && (x * x - 2.0).abs() <= 1e-9, "{x}");
    // A block whose every equation is deleted has none.
    let mut d = Document::new();
    d.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "me", "2026-11-17T00:00:00Z"));
    d.body = vec![
        para_block(para(vec![calc_eq(1, "x := 1")])),
        para_block(para(vec![calc_eq(2, "Given")])),
        para_block(para_with(vec![calc_eq(3, "x^2 == 2")], &deleted)),
        para_block(para_with(vec![calc_eq(4, "x^2 == 3")], &deleted)),
        para_block(para(vec![calc_eq(5, "x_s := Find(x)")])),
        para_block(para(vec![calc_eq(6, "x_s =")])),
    ];
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 5).0.state, syntax("a Given block needs at least one equation"));
    assert_eq!(region(&d, 6).0.state, err(CalcError::Upstream));
    assert_eq!(region(&d, 2).0.state, CalcState::Fresh);
}

#[test]
fn guess_shows_guess() {
    let d = solved_doc(&["f := 0.02", "Given", "f^2 == 0.0009", "f_D := Find(f) =", "f =", "f_D ="]);
    let f_d = shown_number(&d, 4);
    assert!(sig3(f_d, 0.03), "{f_d}");
    assert!((f_d * f_d - 0.0009).abs() <= 1e-9 * 0.0009, "{f_d}");
    assert_eq!(shown_number(&d, 5), 0.02);
    assert!(sig3(shown_number(&d, 6), 0.03));
}

#[test]
fn define_many_block() {
    let d = solved_doc(&["x := 1", "y := 1", "Given", "x + y == 3", "x − y == 1", "[a; b] := Find(x, y)", "a =", "b =", "c := a·b ="]);
    let (a, b) = (shown_number(&d, 7), shown_number(&d, 8));
    assert!(sig3(a, 2.0) && sig3(b, 1.0), "{a}, {b}");
    // Both equations hold.
    assert!((a + b - 3.0).abs() <= 1e-9 && (a - b - 1.0).abs() <= 1e-9, "{a}, {b}");
    assert!(sig3(shown_number(&d, 9), 2.0));
    // Each name is a variable to quote.
    let names: Vec<String> = defined_names(&d).into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"a".to_string()) && names.contains(&"b".to_string()), "{names:?}");
    // A second definition of one of them is a duplicate, in either order.
    let d = solved_doc(&["x := 1", "y := 1", "Given", "x + y == 3", "x − y == 1", "[a; b] := Find(x, y)", "a := 5"]);
    assert_eq!(region(&d, 7).0.state, err(CalcError::Duplicate { name: "a".into(), page: None }));
    let d = solved_doc(&["b := 5", "x := 1", "y := 1", "Given", "x + y == 3", "x − y == 1", "[a; b] := Find(x, y)", "a ="]);
    assert_eq!(region(&d, 7).0.state, err(CalcError::Duplicate { name: "b".into(), page: None }));
    // The failed definition still owns `a`: its use is upstream, not undefined.
    assert_eq!(region(&d, 8).0.state, err(CalcError::Upstream));
    // A cycle names each of them.
    let d = solved_doc(&["[a; b] := [c; 1]", "c := a + 1"]);
    assert_eq!(region(&d, 1).0.state, err(CalcError::Circular("a, b, c".into())));
}

#[test]
fn edited_member() {
    let mut d = doc_with(&["x := 1", "Given", "x^2 == 2", "x_s := Find(x) =", "x_s + 1 ="]);
    edit_region(&mut d, 3, |c, linear| {
        *linear = "x^2=3".into();
        c.state = CalcState::EditedOutside;
    });
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, CalcState::EditedOutside);
    assert_eq!(region(&d, 4).0.state, err(CalcError::UpstreamEdited));
    assert_eq!(region(&d, 5).0.state, err(CalcError::UpstreamEdited));
    // So does an edited Given.
    let mut d = doc_with(&["x := 1", "Given", "x^2 == 2", "x_s := Find(x) ="]);
    edit_region(&mut d, 2, |c, _| c.state = CalcState::EditedOutside);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 4).0.state, err(CalcError::UpstreamEdited));
}

#[test]
fn minerr_document() {
    let d = solved_doc(&["x := 0", "Given", "x == 1", "x == 3", "x_m := Minerr(x) ="]);
    // The closest fit to both equations: their errors are equal and opposite.
    let x = shown_number(&d, 5);
    assert!((x - 2.0).abs() <= 1e-9, "{x}");
    assert!(((x - 1.0) + (x - 3.0)).abs() <= 1e-9, "{x}");
    shows_nothing(&d, 3);
    shows_nothing(&d, 4);
}

/// `Fluid Flow Analysis - 3 Reservoir Problem.xmcd`: three reservoirs joined at one point.
#[test]
#[allow(clippy::excessive_precision, reason = "Mathcad's saved results are transcribed with all their digits")]
fn three_reservoir_document() {
    let d = solved_doc(&[
        "z_1 := 838 ft",
        "z_3 := 850 ft",
        "z_4 := 805 ft",
        "f_p := 0.03",
        "L_12 := 800 ft",
        "L_24 := 600 ft",
        "L_23 := 700 ft",
        "d_12 := 1 ft",
        "d_24 := 1.2 ft",
        "d_23 := 2.1 ft",
        "A_12 := π·d_12^2/4",
        "A_24 := π·d_24^2/4",
        "A_23 := π·d_23^2/4",
        "V_12 := 3 ft/sec",
        "V_23 := 5 ft/sec",
        "V_24 := 5 ft/sec",
        "Given",
        "z_1 − f_p·L_12/d_12·V_12·|V_12|/(2·g_0) − f_p·L_24/d_24·V_24·|V_24|/(2·g_0) == z_4",
        "z_1 − f_p·L_12/d_12·V_12·|V_12|/(2·g_0) + f_p·L_23/d_23·V_23·|V_23|/(2·g_0) == z_3",
        "A_12·V_12 + A_23·V_23 == A_24·V_24",
        "[V_12s; V_23s; V_24s] := Find(V_12, V_23, V_24)",
        "A_12·V_12s = ? gpm",
        "A_24·V_24s = ? gpm",
        "A_23·V_23s = ? gpm",
        // Each equation, its two sides subtracted at the answer.
        "z_1 − f_p·L_12/d_12·V_12s·|V_12s|/(2·g_0) − f_p·L_24/d_24·V_24s·|V_24s|/(2·g_0) − z_4 = ? ft",
        "z_1 − f_p·L_12/d_12·V_12s·|V_12s|/(2·g_0) + f_p·L_23/d_23·V_23s·|V_23s|/(2·g_0) − z_3 = ? ft",
        "A_12·V_12s + A_23·V_23s − A_24·V_24s = ? gpm",
        "V_12 = ? ft/sec",
    ]);
    // Mathcad's saved results.
    let (q12, q24, q23) = (shown_number(&d, 22), shown_number(&d, 23), shown_number(&d, 24));
    assert!(sig3(q12, -1589.5412631603367), "{q12}");
    assert!(sig3(q24, 6697.8072260949511), "{q24}");
    assert!(sig3(q23, 8287.3484892554116), "{q23}");
    for id in [25, 26] {
        assert!(shown_number(&d, id).abs() <= 1e-9 * 838.0, "equation {id}: {}", shown_number(&d, id));
    }
    assert!(shown_number(&d, 27).abs() <= 1e-9 * q23.abs(), "{}", shown_number(&d, 27));
    // The guess is still the guess.
    assert_eq!(shown_number(&d, 28), 3.0);
}

/// `Mathcad Verification Simple Pump Model.xmcd`: a pump's curve against the system's, with the
/// Colebrook friction factor as a block used as a function inside the block that finds the
/// operating point.
#[test]
#[allow(clippy::excessive_precision, reason = "Mathcad's saved results are transcribed with all their digits")]
fn pump_operating_point() {
    let d = solved_doc(&[
        "ρ_w := 999.106 kg/m^3",
        "μ_w := 0.00113 Pa·sec",
        "D_1 := 7.981 in",
        "D_2 := 7.981 in",
        "A_1 := π·D_1^2/4",
        "A_2 := π·D_2^2/4",
        "L_1 := 100 m",
        "L_2 := 200 m",
        "ε := 0.00004572 m",
        "K_1 := 2.8 + 0.5 + 0.2",
        "K_2 := 2.8 + 1.0 + 2·0.2",
        "z_1 := 10 m",
        "z_2 := 30 m",
        "g_c := 32.174 ft/sec^2",
        "flowrate := [0; 1000; 2000; 3000; 3500; 4000] gpm",
        "head := [230; 228.5; 221; 200.5; 183.5; 157] ft",
        "Head_pump(Q) := interp(cspline(flowrate, head), flowrate, head, Q)",
        "Rn(V, D) := D·V·ρ_w/μ_w",
        "f_t := 0.1",
        "Given",
        "1/√f_t == −2·log(ε_r/3.7 + 2.51/(R·√f_t))",
        "f_turb(R, ε_r) := Find(f_t)",
        "f_lam(R) := 64/R",
        "friction(R, ε_r) := f_turb(R, ε_r) if R > 2300, f_lam(R) otherwise",
        "h_L(Q) := friction(Rn(Q/A_2, D_2), ε/D_2)·L_2/D_2·(Q/A_2)^2/(2·g_c) + friction(Rn(Q/A_1, D_1), ε/D_1)·L_1/D_1·(Q/A_1)^2/(2·g_c)",
        "h_1(Q) := K_1·(Q/A_1)^2/(2·g_c)",
        "h_2(Q) := K_2·(Q/A_2)^2/(2·g_c)",
        "Head_sys(Q) := z_2 − z_1 + h_L(Q) + h_1(Q) + h_2(Q)",
        "Q := 1000 gpm",
        "Given",
        "Head_pump(Q) == Head_sys(Q)",
        "Q_op := Find(Q)",
        "Q_op = ? gpm",
        "Head_pump(Q_op) = ? ft",
        "friction(Rn(Q_op/A_1, D_1), ε/D_1) =",
        "h_1(Q_op) = ? ft",
        // The equation, its two sides subtracted at the answer.
        "Head_pump(Q_op) − Head_sys(Q_op) = ? ft",
    ]);
    // Mathcad's saved results.
    let q = shown_number(&d, 33);
    assert!(sig3(q, 2726.5945130250111), "{q}");
    let head = shown_number(&d, 34);
    assert!(sig3(head, 207.60972913844549), "{head}");
    let f = shown_number(&d, 35);
    assert!(sig3(f, 0.014988971068715593), "{f}");
    let h1 = shown_number(&d, 36);
    assert!(sig3(h1, 16.631219949906445), "{h1}");
    let off = shown_number(&d, 37);
    assert!(off.abs() <= 1e-9 * head, "{off}");
}

/// The final review's probes: each is no solution, and shows the no-solution error rather than a
/// number.
#[test]
fn a_non_solution_is_never_shown() {
    let no_solution = |d: &Document, id: u64| match &region(d, id).0.state {
        CalcState::Error(m) => assert!(m.starts_with("no solution found near the guesses — the equations are still off by "), "region {id}: {m}"),
        other => panic!("region {id}: {other:?} ({})", region(d, id).1),
    };
    let d = solved_doc(&["x := 30", "Given", "exp(x) + 1 == 0.5", "x_s := Find(x) ="]);
    no_solution(&d, 4);
    let d = solved_doc(&["x := 1e6", "Given", "x^2 == −1", "x_s := Find(x) ="]);
    no_solution(&d, 4);
    for guess in ["x := 1e6", "x := 1e8"] {
        let d = solved_doc(&[guess, "root(x^2 + 1, x) ="]);
        no_solution(&d, 2);
    }
    // From 1e3 the miss is all of the equation's size, and the message says so.
    let d = solved_doc(&["x := 1e3", "root(x^2 + 1, x) ="]);
    assert_eq!(region(&d, 2).0.state, syntax("no solution found near the guesses — the equations are still off by 100%; try other guesses"));
}

#[test]
fn an_unknown_listed_twice() {
    for closing in ["x_s := Find(x, x) =", "[a; b] := Find(x, x)", "x_m := Minerr(y, x, y)"] {
        let d = solved_doc(&["x := 1", "y := 1", "Given", "x^2 == 2", closing]);
        let name = if closing.contains("Minerr") { "y" } else { "x" };
        assert_eq!(region(&d, 5).0.state, syntax(&format!("{name} is listed twice")), "{closing}");
        shows_nothing(&d, 3);
    }
}

#[test]
fn a_given_before_the_first_block_ends_closes_it_unended() {
    // A block whose Find is missing: the next Given starts a block of its own, the first Given
    // has the error, and what is between them works as usual.
    let d =
        solved_doc(&["x := 1", "Given", "x^2 == 2", "y := 3", "z := y + 1 =", "w := 1", "Given", "w^2 == 4", "w_s := Find(w) =", "v := w_s + z ="]);
    assert_eq!(region(&d, 2).0.state, syntax(UNCLOSED));
    shows_nothing(&d, 3);
    for id in [4, 6, 7, 8] {
        assert_eq!(region(&d, id).0.state, CalcState::Fresh, "region {id}");
    }
    assert_eq!(shown_number(&d, 5), 4.0);
    let w_s = shown_number(&d, 9);
    assert!(sig3(w_s, 2.0) && (w_s * w_s - 4.0).abs() <= 1e-9, "{w_s}");
    assert!(sig3(shown_number(&d, 10), 6.0));
}

#[test]
fn a_find_inside_an_expression() {
    const WHOLE: &str = "must be the whole right side of the line that ends a Given block";
    for (line, kind) in [("x_s := 2·Find(x)", "Find"), ("x_s := Minerr(x) + 1", "Minerr"), ("2·Find(x) =", "Find")] {
        let d = solved_doc(&["x := 1", "Given", "x^2 == 2", line, "z := x_s + 1 ="]);
        assert_eq!(region(&d, 4).0.state, syntax(&format!("{kind} {WHOLE}")), "{line}");
        assert_eq!(region(&d, 2).0.state, syntax(UNCLOSED), "{line}");
        // With a Find below to end the block, the line still says what is wrong with it.
        let d = solved_doc(&["x := 1", "Given", "x^2 == 2", line, "x_t := Find(x) ="]);
        assert_eq!(region(&d, 4).0.state, syntax(&format!("{kind} {WHOLE}")), "{line}");
        assert_eq!(region(&d, 5).0.state, err(CalcError::Upstream), "{line}");
    }
    // Outside a block too.
    let d = solved_doc(&["x := 1", "x_s := 2·Find(x) ="]);
    assert_eq!(region(&d, 2).0.state, syntax(&format!("Find {WHOLE}")));
}

#[test]
fn a_unit_error_names_its_equation() {
    let d = solved_doc(&["p := 1 psi", "z := 2 ft", "Given", "p == 2 psi", "p == z", "p_s := Find(p) =", "p_s ="]);
    match &region(&d, 6).0.state {
        CalcState::Error(m) => assert!(m.starts_with("equation 2: "), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(region(&d, 7).0.state, err(CalcError::Upstream));
    shows_nothing(&d, 5);
}
