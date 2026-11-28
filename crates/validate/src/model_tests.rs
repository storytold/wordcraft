//! Spot checks on the generated content-model tables in `model.rs`.
//!
//! These pin facts the grammar checks rely on, so a regeneration that drifts (a new ECMA
//! edition, an extractor regression, a lost collision split) fails here loudly.

use super::model::{
    Compositor, ELEMENTS, MaxOccurs, NUM_FMT_VALUES, TRANSITIONAL_TYPES, TYPES, TypeModel, element_model, transitional_type_model, type_model,
};

/// The model selected by an element's canonical type.
fn canonical(name: &str) -> &'static TypeModel {
    let el = element_model(name).unwrap_or_else(|| panic!("no element row for {name}"));
    type_model(el.canonical).unwrap_or_else(|| panic!("no type row for {}", el.canonical))
}

/// How many child slots of `model` are named `child`.
fn slots(model: &TypeModel, child: &str) -> usize {
    model.children.iter().filter(|c| c.name == child).count()
}

#[test]
fn tables_are_sorted_and_unique() {
    // Binary search in the lookups needs strictly ascending keys.
    assert!(TYPES.windows(2).all(|w| w[0].key < w[1].key));
    assert!(TRANSITIONAL_TYPES.windows(2).all(|w| w[0].key < w[1].key));
    assert!(ELEMENTS.windows(2).all(|w| w[0].name < w[1].name));
}

#[test]
fn table_sizes_match_the_extraction_record() {
    assert_eq!(TYPES.len(), 380 + 367);
    assert_eq!(ELEMENTS.len(), 794 + 541);
    assert_eq!(TRANSITIONAL_TYPES.len(), 19 + 1);
}

#[test]
fn positions_index_the_flattened_child_list() {
    for t in TYPES.iter().chain(TRANSITIONAL_TYPES) {
        for (i, c) in t.children.iter().enumerate() {
            assert_eq!(usize::from(c.position), i, "{} child {}", t.key, c.name);
        }
    }
}

#[test]
fn every_canonical_type_is_a_variant() {
    for e in ELEMENTS {
        assert!(e.variants.iter().any(|v| v.type_key == e.canonical), "{}", e.name);
    }
}

#[test]
fn wml_run_has_one_rpr_and_no_trailing_t() {
    assert_eq!(element_model("w:r").map(|e| e.canonical), Some("w:CT_R"));
    let r = canonical("w:r");
    assert_eq!(r.children.len(), 33);
    assert_eq!(slots(r, "w:rPr"), 1);
    assert_eq!(slots(r, "w:t"), 1, "only the run-content w:t, no trailing direct t");
    assert_eq!(r.children.first().map(|c| c.name), Some("w:rPr"));
}

#[test]
fn math_run_keeps_its_trailing_t() {
    assert_eq!(element_model("m:r").map(|e| e.canonical), Some("m:CT_R"));
    let r = canonical("m:r");
    assert_eq!(r.children.len(), 35);
    assert_eq!(r.children.last().map(|c| c.name), Some("m:t"));
    assert_eq!(slots(r, "m:rPr"), 1);
    assert_eq!(slots(r, "w:rPr"), 1);
}

#[test]
fn style_in_styles_is_the_full_wml_model() {
    assert_eq!(element_model("w:style").map(|e| e.canonical), Some("w:CT_Style"));
    let style = canonical("w:style");
    assert_eq!(style.children.len(), 22);
    assert_eq!(style.children.first().map(|c| c.name), Some("w:name"));
    assert_eq!(style.children.last().map(|c| (c.name, c.max)), Some(("w:tblStylePr", MaxOccurs::Unbounded)));
}

#[test]
fn chart_plot_area_requires_one_of_its_chart_type_choice() {
    let plot = canonical("c:plotArea");
    let bar = plot.children.iter().find(|c| c.name == "c:barChart").expect("c:barChart slot");
    assert_eq!(bar.compositor, Compositor::Choice);
    assert_eq!(bar.min, 1);
    let group = bar.choice_group.expect("chart types share a choice group");
    let members = plot.children.iter().filter(|c| c.choice_group == Some(group)).count();
    assert_eq!(members, 16, "the 16 chart-type elements");
    let layout = plot.children.iter().find(|c| c.name == "c:layout").expect("c:layout slot");
    assert_eq!((layout.min, layout.max), (0, MaxOccurs::Limited(1)));
}

#[test]
fn theme_font_collection_requires_latin_ea_cs() {
    let font = canonical("a:majorFont");
    let required: Vec<&str> = font.children.iter().filter(|c| c.min >= 1).map(|c| c.name).collect();
    assert_eq!(required, ["a:latin", "a:ea", "a:cs"]);
}

#[test]
fn collision_families_are_separate_rows() {
    for (a, b) in [
        ("w:CT_R", "m:CT_R"),
        ("w:CT_Style", "m:CT_Style"),
        ("w:CT_OnOff", "m:CT_OnOff"),
        ("w:CT_String", "m:CT_String"),
        ("w:CT_Text", "m:CT_Text"),
        ("w:CT_TwipsMeasure", "m:CT_TwipsMeasure"),
        ("a:CT_Boolean", "c:CT_Boolean"),
    ] {
        assert!(type_model(a).is_some() && type_model(b).is_some(), "{a} / {b}");
    }
    assert!(type_model("m:CT_Style").is_some_and(|t| t.children.is_empty()));
    assert!(type_model("a:CT_Boolean").is_some_and(|t| t.children.is_empty()));
    assert!(type_model("c:CT_Boolean").is_some_and(|t| t.children.is_empty()));
}

#[test]
fn sect_pr_variants_carry_their_parent_contexts() {
    let sect = element_model("w:sectPr").expect("w:sectPr row");
    let contexts = |key: &str| sect.variants.iter().find(|v| v.type_key == key).map(|v| v.contexts).unwrap_or_default();
    assert!(contexts("w:CT_SectPr").contains(&"w:CT_Body"));
    assert!(contexts("w:CT_SectPr").contains(&"w:CT_PPr"));
    assert!(contexts("w:CT_SectPrBase").contains(&"w:CT_SectPrChange"));
    assert_eq!(slots(type_model("w:CT_SectPrBase").expect("base"), "w:headerReference"), 0);
}

#[test]
fn ins_group_context_expands_to_parent_types() {
    let ins = element_model("w:ins").expect("w:ins row");
    let contexts = |key: &str| ins.variants.iter().find(|v| v.type_key == key).map(|v| v.contexts).unwrap_or_default();
    // The run-level variant is declared in a group; the group must expand to the paragraph type.
    assert!(contexts("w:CT_RunTrackChange").contains(&"w:CT_P"));
    assert!(contexts("w:CT_TrackChange").contains(&"w:CT_NumPr"));
}

#[test]
fn unflattened_group_context_adds_no_parent() {
    // `a:EG_Effect` is only the bare top-level content of `a:CT_EffectContainer`, which the
    // extractor records as empty, so no type holds a slot from it. The group context must add no
    // parent (and must not stop generation); the explicit `a:CT_Blip` context remains.
    let el = element_model("a:alphaBiLevel").expect("a:alphaBiLevel row");
    let contexts: Vec<&str> = el.variants.iter().flat_map(|v| v.contexts.iter().copied()).collect();
    assert_eq!(contexts, ["a:CT_Blip"]);
    // The two types whose content is such a group carry no child slots (README scope limits).
    for key in ["a:CT_EffectContainer", "w:CT_RubyContent"] {
        assert!(type_model(key).expect(key).children.is_empty(), "{key}");
    }
}

#[test]
fn print_settings_has_its_transitional_override() {
    let strict = type_model("c:CT_PrintSettings").expect("strict row");
    let trans = transitional_type_model("c:CT_PrintSettings").expect("transitional row");
    assert_eq!(slots(strict, "c:legacyDrawingHF"), 0);
    assert_eq!(slots(trans, "c:legacyDrawingHF"), 1);
    assert!(transitional_type_model("w:CT_Tbl").is_none(), "only differing types are listed");
}

#[test]
fn num_fmt_values_are_the_whole_enumeration() {
    assert_eq!(NUM_FMT_VALUES.len(), 63);
    assert_eq!(NUM_FMT_VALUES.first(), Some(&"decimal"));
    assert_eq!(NUM_FMT_VALUES.last(), Some(&"custom"));
    for v in ["hebrew1", "arabicAlpha", "hindiNumbers", "thaiCounting", "bahtText", "custom"] {
        assert!(NUM_FMT_VALUES.contains(&v), "{v}");
    }
}

#[test]
fn unknown_names_are_not_found() {
    assert!(element_model("w:noSuchElement").is_none());
    assert!(type_model("w:CT_NoSuchType").is_none());
    assert!(element_model("").is_none());
}
