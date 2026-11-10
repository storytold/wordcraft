//! Lists (ODF 1.3 §5.3, §16.30–16.33): each `text:list` is a list of its own unless it continues
//! one; its list style gives each level's number format, prefix and suffix, the levels shown,
//! the bullet character and the start value.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::section::NumFormat;

const NS_X: &str = r#"xmlns:xml="http://www.w3.org/XML/1998/namespace""#;

fn doc(styles: &str, body: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_X}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

fn number(name: &str, format: &str, prefix: &str, suffix: &str, extra: &str) -> String {
    format!(
        r#"<text:list-style style:name="{name}"><text:list-level-style-number text:level="1" style:num-format="{format}" style:num-prefix="{prefix}" style:num-suffix="{suffix}" {extra}/><text:list-level-style-number text:level="2" style:num-format="a" style:num-suffix=")"/></text:list-style>"#
    )
}

fn bullet(name: &str, ch: &str) -> String {
    format!(r#"<text:list-style style:name="{name}"><text:list-level-style-bullet text:level="1" text:bullet-char="{ch}"/></text:list-style>"#)
}

fn list(style: &str, items: &[&str], attrs: &str) -> String {
    let items: String = items.iter().map(|t| format!("<text:list-item><text:p>{t}</text:p></text:list-item>")).collect();
    format!(r#"<text:list text:style-name="{style}" {attrs}>{items}</text:list>"#)
}

/// Each list paragraph: (its text, its list instance, level 0's format, label text and start).
fn items(d: &Document) -> Vec<(String, u32, NumFormat, String, u32)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .filter_map(|p| {
            let n = p.props.numbering?;
            let lv = d.numbering.level(n.num, 0)?;
            Some((p.text.clone(), n.num, lv.format, lv.text.clone(), lv.start))
        })
        .collect()
}

fn styles() -> String {
    [number("N1", "1", "", ".", ""), number("N2", "i", "", ".", ""), number("N3", "A", "(", ")", ""), bullet("B1", "–")].concat()
}

fn lists_body() -> String {
    [list("N1", &["One", "Two"], ""), list("N2", &["i one", "i two"], ""), list("N3", &["A one"], ""), list("B1", &["dash"], "")].concat()
}

#[test]
fn separate_lists_stay_separate_with_their_formats() {
    let d = doc(&styles(), &lists_body());
    let got = items(&d);
    let nums: Vec<u32> = got.iter().map(|i| i.1).collect();
    assert_eq!(nums[0], nums[1]);
    assert!(nums[1] != nums[2] && nums[3] != nums[4] && nums[4] != nums[5], "each text:list is its own list: {nums:?}");
    let shown: Vec<(NumFormat, &str)> = got.iter().map(|i| (i.2, i.3.as_str())).collect();
    assert_eq!(
        shown,
        [
            (NumFormat::Decimal, "%1."),
            (NumFormat::Decimal, "%1."),
            (NumFormat::LowerRoman, "%1."),
            (NumFormat::LowerRoman, "%1."),
            (NumFormat::UpperLetter, "(%1)"),
            (NumFormat::Bullet, "–"),
        ]
    );
}

#[test]
fn a_list_can_continue_another() {
    let body = [
        list("N1", &["One", "Two"], r#"xml:id="first""#),
        "<text:p>Between</text:p>".to_string(),
        list("N1", &["Three"], r#"text:continue-numbering="true""#),
        list("N2", &["i"], ""),
        list("N1", &["Four"], r#"text:continue-list="first""#),
    ]
    .concat();
    let d = doc(&styles(), &body);
    let got = items(&d);
    assert_eq!(got.len(), 5);
    assert_eq!(got[0].1, got[2].1, "continue-numbering goes on with the list before");
    assert_eq!(got[0].1, got[4].1, "continue-list goes on with the list it names");
    assert_ne!(got[0].1, got[3].1);
}

#[test]
fn start_values_and_levels_shown() {
    let styles = [number("S", "1", "", ".", r#"text:start-value="5" text:display-levels="1""#), {
        r#"<text:list-style style:name="D"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/><text:list-level-style-number text:level="2" style:num-format="1" style:num-suffix="." text:display-levels="2"/></text:list-style>"#.to_string()
    }]
    .concat();
    let body = [list("S", &["five"], ""), r#"<text:list text:style-name="D"><text:list-item><text:p>1</text:p><text:list><text:list-item><text:p>1.1</text:p></text:list-item></text:list></text:list-item></text:list>"#.to_string()].concat();
    let d = doc(&styles, &body);
    let got = items(&d);
    assert_eq!(got[0].4, 5);
    let n = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "1.1").and_then(|p| p.props.numbering).unwrap();
    assert_eq!(n.level, 1);
    assert_eq!(d.numbering.level(n.num, 1).unwrap().text, "%1.%2.");
}

#[test]
fn lists_round_trip() {
    let d =
        doc(&styles(), &[lists_body(), "<text:p>Between</text:p>".to_string(), list("N1", &["Three"], r#"text:continue-numbering="true""#)].concat());
    let back = import(&export(&d).unwrap()).unwrap();
    let strip = |v: Vec<(String, u32, NumFormat, String, u32)>| {
        // Instance ids may be renumbered; which items share one may not.
        let firsts: Vec<u32> = v.iter().map(|i| i.1).collect();
        let same: Vec<usize> = firsts.iter().map(|n| firsts.iter().position(|m| m == n).unwrap_or(0)).collect();
        v.into_iter().zip(same).map(|((t, _, f, l, s), k)| (t, k, f, l, s)).collect::<Vec<_>>()
    };
    assert_eq!(strip(items(&back)), strip(items(&d)));
}

#[test]
fn hostile_list_styles_are_bounded() {
    let styles = [
        number("H1", "?", "((((((((((((((((((((", "))))))))))))))))))", r#"text:start-value="99999999999" text:display-levels="99""#),
        r#"<text:list-style style:name="H2"><text:list-level-style-number text:level="999" style:num-format="1"/><text:list-level-style-bullet text:level="0" text:bullet-char=""/></text:list-style>"#.to_string(),
    ]
    .concat();
    let d = doc(&styles, &[list("H1", &["x"], ""), list("H2", &["y"], r#"text:continue-list="nope""#)].concat());
    let got = items(&d);
    assert_eq!(got.len(), 2);
    assert!(got[0].3.chars().count() <= 64, "{}", got[0].3);
    let _ = export(&d).unwrap();
}

#[test]
fn a_word_before_the_number_is_kept() {
    let d = doc(&number("C", "1", "Chapter ", ":", ""), &list("C", &["x"], ""));
    assert_eq!(items(&d)[0].3, "Chapter %1:");
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(items(&back)[0].3, "Chapter %1:");
}

#[test]
fn list_level_positions_are_read_and_written() {
    // Each level's indent, hanging and number alignment (ODF 1.3 §17.19–17.20,
    // `style:list-level-properties` in label-alignment mode).
    let styles = r#"<text:list-style style:name="P"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."><style:list-level-properties text:list-level-position-and-space-mode="label-alignment" fo:text-align="end"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="0.5in" fo:text-indent="-0.25in" fo:margin-left="0.5in"/></style:list-level-properties></text:list-level-style-number></text:list-style><text:list-style style:name="Q"><text:list-level-style-bullet text:level="1" text:bullet-char="•"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.1in" fo:margin-left="0.2in"/></style:list-level-properties></text:list-level-style-bullet></text:list-style>"#;
    let d = doc(styles, &[list("P", &["one"], ""), list("Q", &["dot"], "")].concat());
    let level = |i: usize| {
        let p = d.body.iter().filter_map(|b| b.as_para()).nth(i).unwrap();
        let n = p.props.numbering.unwrap();
        d.numbering.level(n.num, n.level).unwrap().clone()
    };
    let (a, b) = (level(0), level(1));
    assert_eq!((a.indent, a.hanging, a.align), (36.0, 18.0, wordcraft_doc::Align::Right));
    assert_eq!((b.indent, b.hanging), (14.4, 7.2));
    let back = import(&export(&d).unwrap()).unwrap();
    let again = |i: usize| {
        let p = back.body.iter().filter_map(|b| b.as_para()).nth(i).unwrap();
        let n = p.props.numbering.unwrap();
        back.numbering.level(n.num, n.level).unwrap().clone()
    };
    let (c, e) = (again(0), again(1));
    assert_eq!((c.indent, c.hanging, c.align), (a.indent, a.hanging, a.align));
    assert!((e.indent - b.indent).abs() < 0.01 && (e.hanging - b.hanging).abs() < 0.01);
}

#[test]
fn alphabetic_lists_count_sequentially_past_z_and_keep_start_values() {
    let styles = number("Letters", "A", "", ")", r#"text:start-value="28""#);
    let body = list("Letters", &["First", "Second"], "");
    let document = doc(&styles, &body);
    let mut counters = wordcraft_doc::numbering::Counters::default();
    let labels: Vec<_> = document
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter_map(|p| {
            let n = p.props.numbering?;
            counters.next_label(&document.numbering, n.num, n.level).map(|v| v.0)
        })
        .collect();
    assert_eq!(labels, ["AB)", "AC)"]);
}

fn labels(d: &Document) -> Vec<String> {
    let mut counters = wordcraft_doc::numbering::Counters::default();
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .filter_map(|p| {
            let n = p.props.numbering?;
            counters.next_label(&d.numbering, n.num, n.level).map(|v| v.0)
        })
        .collect()
}

#[test]
fn picture_custom_and_unresolved_bullets_display_as_bullets() {
    let styles = format!(
        r#"<text:list-style style:name="Picture"><text:list-level-style-image text:level="1"/></text:list-style>{}"#,
        bullet("Custom", "  ☑extra")
    );
    let d = doc(&styles, &[list("Picture", &["p"], ""), list("Custom", &["c"], ""), list("Missing", &["m"], "")].concat());
    for d in [&d, &import(&export(&d).unwrap()).unwrap()] {
        assert_eq!(labels(d), ["•", "☑", "•"]);
        let n = d.body[1].as_para().unwrap().props.numbering.unwrap();
        assert_eq!(d.numbering.level(n.num, 0).unwrap().chr.font.as_deref(), Some("Segoe UI Symbol"));
    }
}

#[test]
fn list_geometry_start_values_and_affixes_are_bounded() {
    for length in ["3e38in", "NaNpt", "99999pt", "-99999pt"] {
        let style = format!(
            r#"<text:list-style style:name="H"><text:list-level-style-number text:level="1" style:num-format="1" style:num-prefix="{}" style:num-suffix="{}" text:start-value="4294967295"><style:list-level-properties text:space-before="{length}" text:min-label-width="{length}"><style:list-level-label-alignment fo:margin-left="{length}" fo:text-indent="{length}" text:list-tab-stop-position="{length}"/></style:list-level-properties></text:list-level-style-number></text:list-style>"#,
            "x".repeat(10000),
            "y".repeat(10000)
        );
        let d = doc(
            &style,
            r#"<text:list text:style-name="H"><text:list-item text:start-value="4294967295"><text:p>x</text:p></text:list-item></text:list>"#,
        );
        let n = d.body[0].as_para().unwrap().props.numbering.unwrap();
        let level = d.numbering.level(n.num, 0).unwrap();
        assert_eq!(level.start, 32767);
        for value in [level.indent, level.hanging, level.tab.unwrap_or(0.0)] {
            assert!(value.is_finite() && value.abs() <= 1584.0, "{length}: {value}");
        }
        assert!(level.text.chars().count() <= 64);
        assert!(labels(&d)[0].contains("32767"));
    }
}

#[test]
fn label_alignment_defaults_to_a_tab_but_honours_explicit_suffixes() {
    use wordcraft_doc::numbering::LevelSuffix;
    for (attr, suffix) in [
        ("", LevelSuffix::Tab),
        (r#"text:label-followed-by="nothing""#, LevelSuffix::Nothing),
        (r#"text:label-followed-by="space""#, LevelSuffix::Space),
    ] {
        let style = format!(
            r#"<text:list-style style:name="N"><text:list-level-style-number text:level="1" style:num-format="1"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment {attr}/></style:list-level-properties></text:list-level-style-number></text:list-style>"#
        );
        let d = doc(&style, &list("N", &["x"], ""));
        let n = d.body[0].as_para().unwrap().props.numbering.unwrap();
        assert_eq!(d.numbering.level(n.num, 0).unwrap().suffix, suffix);
    }
}

#[test]
fn continuation_chains_and_unnumbered_headers_survive_saves() {
    let body = [
        r#"<text:list text:style-name="N1" xml:id="a"><text:list-header><text:p>Header</text:p></text:list-header><text:list-item><text:p>one</text:p></text:list-item></text:list>"#.into(),
        list("N1", &["two"], r#"xml:id="b" text:continue-list="a""#),
        list("N1", &["three"], r#"xml:id="c" text:continue-numbering="true""#),
        list("N1", &["four"], r#"text:continue-list="a""#),
    ].concat();
    let mut d = doc(&styles(), &body);
    for _ in 0..3 {
        assert_eq!(labels(&d), ["1.", "2.", "3.", "4."]);
        assert!(d.body[0].as_para().unwrap().props.numbering.is_none());
        d = import(&export(&d).unwrap()).unwrap();
    }
}

#[test]
fn rewritten_predecessor_does_not_leave_a_dangling_continue_list() {
    use std::sync::Arc;
    let body = [list("N1", &["one"], r#"xml:id="original""#), list("N1", &["two"], r#"text:continue-list="original""#)].concat();
    let mut d = doc(&styles(), &body);
    // Changing the first paragraph forces the first list to be written from the model.
    let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut d.body[0]) else { panic!("paragraph") };
    p.props.align = Some(wordcraft_doc::Align::Right);
    let bytes = export(&d).unwrap();
    let xml = crate::odt::tests::entry(&bytes, "content.xml");
    assert!(!xml.contains(r#"text:continue-list="original""#), "{xml}");
    assert_eq!(labels(&import(&bytes).unwrap()), ["1.", "2."]);
}

#[test]
fn paragraphs_share_their_lists_level_definitions() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{}</office:automatic-styles><office:body><office:text>{}</office:text></office:body></office:document-content>"#,
        styles(),
        list("N1", &["one", "two", "three"], "")
    );
    let flow = crate::odt::parse(&package(&content, None)).unwrap();
    let definitions: Vec<_> = flow
        .blocks
        .iter()
        .filter_map(|b| match b {
            crate::model::FBlock::Para(p) => p.explicit_list.as_ref().map(|l| &l.definitions),
            _ => None,
        })
        .collect();
    assert_eq!(definitions.len(), 3);
    assert!(std::sync::Arc::ptr_eq(definitions[0], definitions[1]));
    assert!(std::sync::Arc::ptr_eq(definitions[1], definitions[2]));
}

#[test]
fn legacy_and_missing_list_positions_keep_tabs_and_indents() {
    use wordcraft_doc::numbering::LevelSuffix;
    let styles = format!(
        r#"<text:list-style style:name="Old"><text:list-level-style-number text:level="1" style:num-format=" 1 "><style:list-level-properties text:space-before="0.25in" text:min-label-width="0.25in"/></text:list-level-style-number></text:list-style>{}"#,
        number("Default", "1", "", ".", "")
    );
    let d = doc(&styles, &[list("Old", &["old"], ""), list("Default", &["default"], ""), list("Missing", &["missing"], "")].concat());
    for p in d.body.iter().filter_map(|b| b.as_para()) {
        let n = p.props.numbering.unwrap();
        let lv = d.numbering.level(n.num, n.level).unwrap();
        assert_eq!((lv.suffix, lv.indent, lv.hanging), (LevelSuffix::Tab, 36.0, 18.0), "{}", p.text);
    }
    for (attrs, suffix, indent, hanging) in [
        (r#"text:min-label-width="0.25in""#, LevelSuffix::Tab, 18.0, 18.0),
        (r#"text:space-before="0.25in" text:min-label-width="0in""#, LevelSuffix::Nothing, 18.0, 0.0),
    ] {
        let styles = format!(
            r#"<text:list-style style:name="N"><text:list-level-style-number text:level="1" style:num-format="1"><style:list-level-properties {attrs}/></text:list-level-style-number></text:list-style>"#
        );
        let d = doc(&styles, &list("N", &["item"], ""));
        let n = d.body[0].as_para().unwrap().props.numbering.unwrap();
        let lv = d.numbering.level(n.num, n.level).unwrap();
        assert_eq!((lv.suffix, lv.indent, lv.hanging), (suffix, indent, hanging));
    }
}

#[test]
fn list_continuation_after_table_keeps_its_own_margin() {
    let styles = format!(
        r#"{}<style:style style:name="Inset" style:family="paragraph"><style:paragraph-properties fo:margin-left="1in"/></style:style>"#,
        number("N", "1", "", ".", "")
    );
    let d = doc(
        &styles,
        r#"<text:list text:style-name="N"><text:list-item><text:p>First</text:p><table:table><table:table-row><table:table-cell><text:p>Cell</text:p></table:table-cell></table:table-row></table:table><text:p text:style-name="Inset">Continuation</text:p></text:list-item><text:list-item><text:p>Second</text:p></text:list-item></text:list>"#,
    );
    let p = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "Continuation").unwrap();
    assert!(p.props.numbering.is_none());
    assert_eq!(p.props.indent_left, Some(72.0));
    assert_eq!(labels(&d), ["1.", "2."]);
}

#[test]
fn rewritten_letter_lists_preserve_letter_sync() {
    for (sync, expected) in [("true", ["BB.", "CC."]), ("false", ["AB.", "AC."])] {
        let mut d = doc(
            &number("N", "A", "", ".", &format!(r#"style:num-letter-sync="{sync}" text:start-value="28""#)),
            &list("N", &["first", "second"], ""),
        );
        // Editing the paragraph forces the list through the model writer.
        let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!("paragraph") };
        p.props.align = Some(wordcraft_doc::Align::Right);
        let saved = export(&d).unwrap();
        let xml = crate::odt::tests::content_of(&saved);
        assert!(xml.contains(&format!(r#"style:num-letter-sync="{sync}""#)), "{xml}");
        assert_eq!(labels(&import(&saved).unwrap()), expected);
    }
}
