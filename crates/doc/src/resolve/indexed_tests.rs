use super::*;
use crate::styles::Style;

fn style(id: &str, based_on: Option<&str>, kind: StyleKind) -> Style {
    Style { id: id.into(), based_on: based_on.map(str::to_owned), kind, ..Default::default() }
}

#[test]
fn indexed_chains_keep_first_id_missing_parents_cycles_and_depth() {
    let mut sheet = StyleSheet::builtin();
    let mut first = style("Duplicate", Some("Normal"), StyleKind::Paragraph);
    first.chr.bold = Some(true);
    let mut second = first.clone();
    second.chr.bold = Some(false);
    sheet.styles.extend([
        first,
        second,
        style("MissingParent", Some("Absent"), StyleKind::Paragraph),
        style("CycleA", Some("CycleB"), StyleKind::Paragraph),
        style("CycleB", Some("CycleA"), StyleKind::Paragraph),
    ]);
    for i in 0..40 {
        sheet.styles.push(style(&format!("Deep{i}"), (i > 0).then(|| format!("Deep{}", i - 1)).as_deref(), StyleKind::Paragraph));
    }
    let original = serde_json::to_vec(&sheet).unwrap();
    let resolver = StyleResolver::new(&sheet);
    for id in ["Duplicate", "MissingParent", "Absent", "CycleA", "CycleB", "Deep39", "Normal", ""] {
        assert_eq!(resolver.chain(id), sheet.chain(id), "chain {id}");
        assert_eq!(
            resolver.resolve_para(&ParaProps { style: Some(id.into()), ..Default::default() }),
            sheet.resolve_para(&ParaProps { style: Some(id.into()), ..Default::default() })
        );
        assert_eq!(resolver.resolve_char(Some(id), &CharProps::default()), sheet.resolve_char(Some(id), &CharProps::default()));
    }
    assert!(resolver.resolve_char(Some("Duplicate"), &CharProps::default()).bold);
    assert!(resolver.chain("Absent").is_empty());
    assert_eq!(resolver.chain("CycleA").iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["CycleB", "CycleA"]);
    assert_eq!(resolver.chain("Deep39").len(), 32);
    assert_eq!(resolver.chain("Deep39")[0].id, "Deep8");
    assert_eq!(serde_json::to_vec(&sheet).unwrap(), original, "indexing must not reorder or change the sheet");
    sheet.styles.swap(0, 1);
    sheet.styles.insert(0, style("Duplicate", None, StyleKind::Paragraph));
    assert!(
        !StyleResolver::new(&sheet).resolve_char(Some("Duplicate"), &CharProps::default()).bold,
        "a new pass observes edits and earlier duplicates"
    );
}

#[test]
fn indexed_resolution_keeps_tables_tabs_script_sizes_and_direct_formatting() {
    let mut sheet = StyleSheet::builtin();
    sheet.default_chr = CharProps { size: Some(12.0), size_cs: Some(15.0), italic: Some(true), ..Default::default() };
    sheet.default_para.space_before = Some(3.0);
    let mut base = style("Base", None, StyleKind::Paragraph);
    base.para = ParaProps {
        indent_left: Some(14.0),
        numbering: Some(NumRef { num: 2, level: 0 }),
        tabs: Some(vec![
            TabStop { pos: 20.0, align: TabAlign::Left, leader: Default::default() },
            TabStop { pos: 40.0, align: TabAlign::Left, leader: Default::default() },
        ]),
        ..Default::default()
    };
    base.chr = CharProps { size: Some(18.0), bold: Some(true), ..Default::default() };
    let mut derived = style("Derived", Some("Base"), StyleKind::Paragraph);
    derived.para.tabs = Some(vec![TabStop { pos: 20.0, align: TabAlign::Clear, leader: Default::default() }]);
    derived.chr.size_cs = Some(23.0);
    let mut character = style("Character", Some("Derived"), StyleKind::Character);
    character.chr.italic = Some(false);
    sheet.styles.extend([base, derived, character]);
    let resolver = StyleResolver::new(&sheet);
    let table_p = ParaProps { indent_left: Some(99.0), space_before: Some(8.0), ..Default::default() };
    let table_c = CharProps { size: Some(30.0), size_cs: Some(31.0), underline: Some(Underline::Single), ..Default::default() };
    let p = ParaProps { style: Some("Derived".into()), space_after: Some(7.0), ..Default::default() };
    let c = CharProps { style: Some("Character".into()), bold: Some(false), size: Some(19.0), ..Default::default() };
    for table in [None, Some(&table_p)] {
        assert_eq!(resolver.resolve_para_in(&p, table), sheet.resolve_para_in(&p, table));
    }
    for table in [None, Some(&table_c)] {
        assert_eq!(resolver.resolve_char_in(Some("Derived"), table, &c), sheet.resolve_char_in(Some("Derived"), table, &c));
    }
    assert_eq!(resolver.indents_over_list("Derived"), sheet.indents_over_list("Derived"));
    assert_eq!(resolver.indents_over_list("Derived"), (true, false));
    let rp = resolver.resolve_para_in(&p, Some(&table_p));
    assert_eq!((rp.indent_left, rp.space_before, rp.space_after), (14.0, 8.0, 7.0));
    assert_eq!(rp.tabs.iter().map(|t| t.pos).collect::<Vec<_>>(), [40.0]);
    let rc = resolver.resolve_char_in(Some("Derived"), Some(&table_c), &c);
    assert_eq!((rc.size, rc.size_cs, rc.bold, rc.italic), (19.0, 23.0, false, false));
    assert_eq!(rc.underline, Underline::Single);
}
