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
