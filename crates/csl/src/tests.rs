//! Parser tests: a golden tree over a hand-written style exercising every element, hostile
//! files, and a warning for each construct outside the subset.

use crate::model::*;
use crate::parse;
use crate::terms::TermSet;

/// The smallest loadable style, as a wrapper for fragments under test.
fn minimal(inner: &str) -> String {
    format!(r#"<style version="1.0">{inner}<citation><layout><text value="x"/></layout></citation></style>"#)
}

/// A full style for fragments that bring their own `<citation>` (no extra one appended).
fn whole(inner: &str) -> String {
    format!(r#"<style version="1.0">{inner}</style>"#)
}

fn p(inner: &str) -> Style {
    let xml = minimal(inner);
    parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

fn warned_contains(s: &Style, needle: &str) {
    assert!(s.warned().iter().any(|w| w.contains(needle)), "expected a warning containing {needle:?}, got {:?}", s.warned());
}

/// A hand-written style exercising every element of the subset.
const GOLDEN: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<style xmlns="http://purl.org/net/csl" version="1.0" class="in-text" default-locale="en-US">
  <info>
    <title>WordCraft Golden</title>
    <id>http://example.com/styles/golden</id>
    <updated>2026-01-01T00:00:00Z</updated>
    <author><name>WordCraft</name></author>
  </info>
  <locale xml:lang="en-US">
    <terms>
      <term name="et-al">and others</term>
      <term name="page" form="short"><single>p.</single><multiple>pp.</multiple></term>
    </terms>
  </locale>
  <macro name="author">
    <names variable="author editor">
      <name name-as-sort-order="all" initialize-with=". " and="symbol" delimiter="; " et-al-min="7" et-al-use-first="6"/>
      <label form="short" plural="always" prefix=" (" suffix=")"/>
      <substitute>
        <names variable="translator"/>
        <text variable="title"/>
      </substitute>
    </names>
  </macro>
  <macro name="year">
    <date variable="issued" prefix=" " suffix=".">
      <date-part name="year" form="long"/>
    </date>
  </macro>
  <citation>
    <layout prefix="(" suffix=")" delimiter="; ">
      <group delimiter=", ">
        <text macro="author"/>
        <text macro="year"/>
        <choose>
          <if variable="DOI URL" match="any">
            <text term="and" form="symbol"/>
          </if>
          <else-if type="book chapter" match="all">
            <text value="is a book" prefix="[" suffix="]"/>
          </else-if>
          <else-if position="first subsequent">
            <label variable="page" form="short" plural="never"/>
          </else-if>
          <else>
            <text variable="title" form="short" strip-periods="true"/>
          </else>
        </choose>
      </group>
    </layout>
  </citation>
  <bibliography>
    <sort>
      <key macro="author"/>
      <key variable="issued" sort="descending"/>
    </sort>
    <layout prefix="" suffix="." delimiter=". ">
      <text variable="title"/>
      <date variable="issued" form="text">
        <date-part name="month" form="short"/>
        <date-part name="day" form="numeric-leading-zeros"/>
        <date-part name="year"/>
      </date>
      <number variable="page" form="numeric"/>
    </layout>
  </bibliography>
</style>"#;

fn text(source: TextSource) -> Text {
    Text { source, form: None, plural: None, prefix: String::new(), suffix: String::new(), strip_periods: false }
}

fn layout(elements: Vec<Element>) -> Layout {
    Layout { elements, ..Layout::default() }
}

#[test]
fn golden_style_parses_into_the_expected_tree() {
    let s = parse(GOLDEN.as_bytes()).unwrap();
    assert_eq!(s.title, "WordCraft Golden");
    assert_eq!(s.id, "http://example.com/styles/golden");
    assert!(s.warned().is_empty(), "pristine golden style: {:?}", s.warned());

    // The en-US defaults, with the style's own en-US overrides merged in.
    let mut terms = TermSet::en_us();
    terms.set("et-al", Form::Long, crate::terms::Term::new("and others", "and others"));
    assert_eq!(s.locale_terms, terms);

    // Macros.
    let author = Element::Names(Names {
        variables: vec!["author".into(), "editor".into()],
        options: NameOptions {
            name_as_sort_order: Some(NameAsSortOrder::All),
            initialize_with: Some(". ".into()),
            et_al_min: Some(7),
            et_al_use_first: Some(6),
            and: Some(AndStyle::Symbol),
            delimiter: Some("; ".into()),
        },
        with_label: Some(Label {
            variable: String::new(), // a names' label has no variable of its own
            form: Some(Form::Short),
            plural: Some(Plural::Always),
            prefix: " (".into(),
            suffix: ")".into(),
        }),
        label_precedes_names: false, // the label is written after the <name>
        substitute: Some(vec![
            Element::Names(Names {
                variables: vec!["translator".into()],
                options: NameOptions::default(),
                with_label: None,
                label_precedes_names: false,
                substitute: None,
            }),
            Element::Text(text(TextSource::Variable("title".into()))),
        ]),
    });
    let year = Element::Date(Date {
        variable: "issued".into(),
        form: None,
        date_parts: DateParts::default(),
        parts: vec![DatePart { name: DatePartName::Year, form: Some(Form::Long), prefix: String::new(), suffix: String::new() }],
        prefix: " ".into(),
        suffix: ".".into(),
    });
    assert_eq!(s.macros.len(), 2);
    assert_eq!(s.macros["author"], vec![author.clone()]);
    assert_eq!(s.macros["year"], vec![year.clone()]);

    // Citation layout.
    let choose = Element::Choose(Choose {
        branches: vec![
            Branch {
                mode: Match::Any,
                conditions: vec![Condition::Variable { name: "DOI".into(), set: true }, Condition::Variable { name: "URL".into(), set: true }],
                layout: layout(vec![Element::Text(Text {
                    source: TextSource::Term("and".into()),
                    form: Some(Form::Symbol),
                    ..text(TextSource::Term(String::new()))
                })]),
            },
            Branch {
                mode: Match::All,
                conditions: vec![Condition::Type { names: vec!["book".into(), "chapter".into()] }],
                layout: layout(vec![Element::Text(Text {
                    source: TextSource::Value("is a book".into()),
                    prefix: "[".into(),
                    suffix: "]".into(),
                    ..text(TextSource::Value(String::new()))
                })]),
            },
            Branch {
                mode: Match::All,
                conditions: vec![Condition::Position { value: Position::First }, Condition::Position { value: Position::Subsequent }],
                layout: layout(vec![Element::Label(Label {
                    variable: "page".into(),
                    form: Some(Form::Short),
                    plural: Some(Plural::Never),
                    prefix: String::new(),
                    suffix: String::new(),
                })]),
            },
        ],
        else_branch: Some(layout(vec![Element::Text(Text {
            source: TextSource::Variable("title".into()),
            form: Some(Form::Short),
            strip_periods: true,
            ..text(TextSource::Variable(String::new()))
        })])),
    });
    assert_eq!(
        s.citation,
        Layout {
            elements: vec![Element::Group(Group {
                elements: vec![
                    Element::Text(text(TextSource::Macro("author".into()))),
                    Element::Text(text(TextSource::Macro("year".into()))),
                    choose,
                ],
                delimiter: ", ".into(),
                ..Group::default()
            })],
            prefix: "(".into(),
            suffix: ")".into(),
            delimiter: "; ".into(),
            ..Layout::default()
        }
    );

    // Bibliography with sort keys.
    let bib = s.bibliography.as_ref().unwrap();
    assert_eq!(
        bib.sort,
        vec![
            SortKey { field: SortField::Macro("author".into()), direction: SortDirection::Ascending },
            SortKey { field: SortField::Variable("issued".into()), direction: SortDirection::Descending },
        ]
    );
    assert_eq!(
        bib.layout,
        Layout {
            elements: vec![
                Element::Text(text(TextSource::Variable("title".into()))),
                Element::Date(Date {
                    variable: "issued".into(),
                    form: Some(Form::Text),
                    date_parts: DateParts::default(),
                    parts: vec![
                        DatePart { name: DatePartName::Month, form: Some(Form::Short), prefix: String::new(), suffix: String::new() },
                        DatePart { name: DatePartName::Day, form: Some(Form::NumericLeadingZeros), prefix: String::new(), suffix: String::new() },
                        DatePart { name: DatePartName::Year, form: None, prefix: String::new(), suffix: String::new() },
                    ],
                    prefix: String::new(),
                    suffix: String::new(),
                }),
                Element::Number(Number { variable: "page".into(), form: Some(Form::Numeric), prefix: String::new(), suffix: String::new() }),
            ],
            prefix: String::new(),
            suffix: ".".into(),
            delimiter: ". ".into(),
            ..Layout::default()
        }
    );
}

#[test]
fn hostile_files_error_or_warn_never_panic() {
    // Junk, truncation, oversize, deep nesting: Err, never a panic.
    assert!(parse(b"").is_err());
    assert!(parse(b"not a style").is_err());
    assert!(parse(&(0..=255u8).collect::<Vec<u8>>()).is_err());
    assert!(parse(b"<style><citation><layout><text var").is_err());
    let err = parse(&vec![b' '; 3 << 20]).unwrap_err();
    assert!(err.contains("2 MiB"), "{err}");
    let deep = format!("{}{}", "<a>".repeat(100), "</a>".repeat(100));
    let err = parse(deep.as_bytes()).unwrap_err();
    assert!(err.contains("deeper"), "{err}");
    let wide = format!("<style>{}<citation><layout/></citation></style>", "<b/>".repeat(100_001));
    let err = parse(wide.as_bytes()).unwrap_err();
    assert!(err.contains("too many"), "{err}");

    // A self-referencing macro loads, warns, and its references are cut (no hang).
    let cyclic = r#"<style version="1.0">
      <macro name="a"><text macro="a"/></macro>
      <macro name="b"><text macro="a"/><text macro="b"/></macro>
      <citation><layout><text macro="b"/></layout></citation>
    </style>"#;
    let s = parse(cyclic.as_bytes()).unwrap();
    warned_contains(&s, "deeper than 32");
    // The tree's reference and the macro bodies' references are dropped, not looped.
    assert_eq!(s.citation.elements, vec![Element::Text(text(TextSource::Value(String::new())))]);
    assert_eq!(s.macros["a"], vec![Element::Text(text(TextSource::Value(String::new())))]);
    assert_eq!(s.macros["b"].len(), 2);
}

#[test]
fn macro_references_are_resolved_at_parse_time() {
    // Unknown macros warn and are dropped from the tree.
    let s = p(r#"<citation><layout><text macro="nope"/></layout></citation>"#);
    warned_contains(&s, "unknown macro 'nope'");
    assert_eq!(s.citation.elements, vec![Element::Text(text(TextSource::Value(String::new())))]);

    // A bounded chain loads silently.
    let chain = format!(
        "<macro name=\"m0\"><text value=\"v\"/></macro>{}<citation><layout><text macro=\"m{}\"/></layout></citation>",
        (1..10).map(|i| format!("<macro name=\"m{i}\"><text macro=\"m{}\"/></macro>", i - 1)).collect::<String>(),
        9
    );
    let s = parse(whole(&chain).as_bytes()).unwrap();
    assert!(s.warned().is_empty(), "{:?}", s.warned());
    assert_eq!(s.citation.elements, vec![Element::Text(text(TextSource::Macro("m9".into())))]);

    // A chain deeper than 32 levels is cut with a warning.
    let deep = format!(
        "<macro name=\"d0\"><text value=\"v\"/></macro>{}<citation><layout><text macro=\"d{}\"/></layout></citation>",
        (1..40).map(|i| format!("<macro name=\"d{i}\"><text macro=\"d{}\"/></macro>", i - 1)).collect::<String>(),
        39
    );
    let s = parse(whole(&deep).as_bytes()).unwrap();
    warned_contains(&s, "deeper than 32");
    assert_eq!(s.citation.elements, vec![Element::Text(text(TextSource::Value(String::new())))]);

    // Duplicate macro names warn.
    let s = p("<macro name=\"dup\"><text value=\"1\"/></macro><macro name=\"dup\"><text value=\"2\"/></macro>");
    warned_contains(&s, "duplicate macro 'dup'");
    assert_eq!(s.macros["dup"], vec![Element::Text(text(TextSource::Value("2".into())))]);
}

#[test]
fn unsupported_constructs_load_with_a_warning() {
    let cases: &[(&str, &str)] = &[
        (r#"<citation collapse="citation-number"><layout/></citation>"#, "collapse"),
        (r#"<citation disambiguate-add-year-suffix="true"><layout/></citation>"#, "disambiguation"),
        (r#"<citation et-al-use-last="true"><layout/></citation>"#, "et-al-use-last"),
        (r#"<citation near-note-distance="2"><layout/></citation>"#, "near-note"),
        (r#"<citation><sort><key variable="author"/></sort><layout/></citation>"#, "sorting"),
        (r#"<bibliography second-field-align="margin"><layout/></bibliography>"#, "second-field-align"),
        (r#"<bibliography hanging-indent="true"><layout/></bibliography>"#, "bibliography formatting"),
        (r#"<bibliography><sort><key variable="issued" names-min="3"/></sort><layout/></bibliography>"#, "sort key"),
        (r#"<citation><layout><text variable="title" font-style="italic"/></layout></citation>"#, "formatting attributes"),
        (r#"<citation><layout><text variable="title" font-weight="bold"/></layout></citation>"#, "formatting attributes"),
        (r#"<citation><layout><text variable="title" text-case="uppercase"/></layout></citation>"#, "formatting attributes"),
        (r#"<citation><layout><text variable="title" quotes="true"/></layout></citation>"#, "formatting attributes"),
        (r#"<citation><layout><text variable="title" vertical-align="sup"/></layout></citation>"#, "formatting attributes"),
        (r#"<citation><layout><number variable="page" strip-periods="true"/></layout></citation>"#, "strip-periods"),
        (r#"<citation><layout><names variable="author"><institution/></names></layout></citation>"#, "institution"),
        (
            r#"<citation><layout><names variable="author" delimiter-precedes-last="always"><name delimiter-precedes-et-al="always"/></names></layout></citation>"#,
            "delimiter-precedes",
        ),
        (r#"<citation><layout><names variable="author"><name form="short"/></names></layout></citation>"#, "name form"),
        (r#"<citation><layout><names variable="author"><name sort-separator=" - "/></names></layout></citation>"#, "sort-separator"),
        (r#"<citation><layout><names variable="author" et-al-use-last="true"/></layout></citation>"#, "et-al-use-last"),
        (r#"<citation><layout><date variable="issued"><date-part name="month" range-delimiter="/"/></date></layout></citation>"#, "range-delimiter"),
        (r#"<citation><layout><choose><if is-numeric="page"><text value="x"/></if></choose></layout></citation>"#, "is-numeric"),
        (r#"<citation><layout><choose><if position="near-note"><text value="x"/></if></choose></layout></citation>"#, "near-note"),
        (r#"<citation><layout><choose><if position="ibid-with-locator"><text value="x"/></if></choose></layout></citation>"#, "ibid-with-locator"),
        (r#"<locale xml:lang="fr-FR"><terms><term name="and">et</term></terms></locale>"#, "en-US"),
        (r#"<locale><date form="text"><date-part name="month"/></date></locale>"#, "locale date"),
        (r#"<locale><style-options punctuation-in-quote="true"/></locale>"#, "style-options"),
        (r#"<locale><terms><term name="ordinal" match="last-digit">e</term></terms></locale>"#, "grammar"),
        (r#"<citation><layout><number/></layout></citation>"#, "without a variable"),
        (r#"<citation><layout><text/></layout></citation>"#, "without variable/macro/term/value"),
        (r#"<citation><layout><names/></layout></citation>"#, "without variables"),
        (r#"<citation><layout><label/></layout></citation>"#, "without a variable"),
    ];
    for (frag, needle) in cases {
        let s = p(frag);
        warned_contains(&s, needle);
    }

    // Style-level attributes.
    let style_cases = &[
        (r#"<style version="1.0" default-locale="de-DE"><citation><layout/></citation></style>"#, "en-US"),
        (r#"<style version="1.0" class="note"><citation><layout/></citation></style>"#, "note"),
        (r#"<style version="1.0" page-range-format="chicago"><citation><layout/></citation></style>"#, "page-range-format"),
        (
            r#"<style version="1.0" demote-non-dropping-particle="display-and-sort"><citation><layout/></citation></style>"#,
            "demote-non-dropping-particle",
        ),
        (r#"<style version="2.0"><citation><layout/></citation></style>"#, "version"),
        (r#"<style version="1.0" names-delimiter="; "><citation><layout/></citation></style>"#, "names-delimiter"),
        (r#"<style version="1.0" initialize-with-hyphen="false"><citation><layout/></citation></style>"#, "initialize-with-hyphen"),
    ];
    for (xml, needle) in style_cases {
        let s = parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
        let msg = format!("expected a warning containing {needle:?}, got {:?}", s.warned());
        assert!(s.warned().iter().any(|w| w.contains(needle)), "{msg}");
    }
}

#[test]
fn unknown_elements_and_attributes_are_silent() {
    let xml = r#"<style version="1.0"><info><title>T</title><id>i</id><future-element/></info><unknown-top/><weird attr="1"/><citation><layout><text variable="title" future-attr="2"/><mystery/></layout></citation></style>"#;
    let s = parse(xml.as_bytes()).unwrap();
    assert_eq!(s.title, "T");
    assert_eq!(s.id, "i");
    assert_eq!(s.citation.elements, vec![Element::Text(text(TextSource::Variable("title".into())))]);
    assert!(s.warned().is_empty(), "{:?}", s.warned());
}

#[test]
fn structural_errors() {
    let err = parse(b"<styles/>").unwrap_err();
    assert!(err.contains("not a CSL style"), "{err}");
    let err = parse(br#"<style version="1.0"/>"#).unwrap_err();
    assert!(err.contains("citation"), "{err}");
    let err = parse(br#"<style version="1.0"><citation/></style>"#).unwrap_err();
    assert!(err.contains("citation"), "{err}");
    let err =
        parse(br#"<style version="1.0"><citation><layout/></citation><bibliography><sort><key variable="author"/></sort></bibliography></style>"#)
            .unwrap_err();
    assert!(err.contains("bibliography"), "{err}");
}

#[test]
fn lenient_files_load() {
    // Elements bound to a prefix, attributes too.
    let s = parse(br#"<c:style xmlns:c="urn:anything" version="1.0"><c:citation><c:layout><c:text c:value="hi"/></c:layout></c:citation></c:style>"#)
        .unwrap();
    assert_eq!(s.citation.elements, vec![Element::Text(text(TextSource::Value("hi".into())))]);
    // Unclosed elements close at EOF (a truncated *tag* is a syntax error, an Err).
    let s = parse(br#"<style version="1.0"><citation><layout><text value="open">"#).unwrap();
    assert_eq!(s.citation.elements, vec![Element::Text(text(TextSource::Value("open".into())))]);
    // A UTF-8 BOM is fine.
    let mut b = b"\xEF\xBB\xBF".to_vec();
    b.extend_from_slice(br#"<style version="1.0"><citation><layout/></citation></style>"#);
    assert!(parse(&b).is_ok());
    // A lang-less <locale> targets the default (en-US) and its overrides are honored.
    let s =
        parse(br#"<style version="1.0"><locale><terms><term name="and">und</term></terms></locale><citation><layout/></citation></style>"#).unwrap();
    assert_eq!(s.locale_terms.text("and", Form::Long, false), Some("und"));
    assert!(s.warned().is_empty(), "{:?}", s.warned());
}

#[test]
fn name_options_merge_names_and_name_child() {
    let xml = whole(r#"<citation><layout><names variable="author" delimiter=", "><name delimiter="; " et-al-min="3"/></names></layout></citation>"#);
    let s = parse(xml.as_bytes()).unwrap();
    let Element::Names(n) = &s.citation.elements[0] else { panic!("not names") };
    assert_eq!(n.options.delimiter, Some("; ".into())); // the name child's, not the <names> one
    assert_eq!(n.options.et_al_min, Some(3));
    // The <names> delimiter separates name variables (unsupported), never names: warned.
    assert_eq!(s.warned().len(), 1, "{:?}", s.warned());
    assert!(s.warned()[0].contains("name variables"), "{:?}", s.warned());
}

#[test]
fn match_modes_and_conditions_parse() {
    let s = p(r#"<citation><layout><choose>
        <if match="none" variable="author"><text value="anon"/></if>
        <else-if type="book"><text value="b"/></else-if>
        <else-if position="ibid"><text value="i"/></else-if>
      </choose></layout></citation>"#);
    let Element::Choose(c) = &s.citation.elements[0] else { panic!("not choose") };
    assert_eq!(c.branches.len(), 3);
    assert_eq!(c.branches[0].mode, Match::None);
    assert_eq!(c.branches[0].conditions, vec![Condition::Variable { name: "author".into(), set: true }]);
    assert_eq!(c.branches[1].mode, Match::All); // the default
    assert_eq!(c.branches[1].conditions, vec![Condition::Type { names: vec!["book".into()] }]);
    assert_eq!(c.branches[2].conditions, vec![Condition::Position { value: Position::Ibid }]);
    assert_eq!(c.else_branch, None);
}

#[test]
fn warnings_dedupe_and_cap() {
    // The same unsupported construct warns once, however often it appears.
    let s = p(r#"<citation><layout><text variable="a" font-style="italic"/><text variable="b" font-style="normal"/></layout></citation>"#);
    assert_eq!(s.warned().iter().filter(|w| w.contains("formatting")).count(), 1);

    // 300 distinct unknown macros still yield at most 256 warnings.
    let macros = (0..300).map(|i| format!(r#"<macro name="m{i}"><text value="v"/></macro>"#)).collect::<String>();
    let s = p(&format!(r#"{macros}<citation><layout/></citation>"#));
    // The macro-count cap (512) doesn't fire at 300; the unknown ones don't exist here.
    assert!(s.warned().len() <= 256, "{}", s.warned().len());
    let refs = (0..300).map(|i| format!(r#"<text macro="x{i}"/>"#)).collect::<String>();
    let s = p(&format!(r#"<citation><layout>{refs}</layout></citation>"#));
    assert!(s.warned().len() <= 256, "{}", s.warned().len());
    assert!(s.warned().iter().any(|w| w.contains("unknown macro 'x0'")));

    // More macros than the cap: the rest are ignored with a warning.
    let macros = (0..600).map(|i| format!(r#"<macro name="m{i}"><text value="v"/></macro>"#)).collect::<String>();
    let s = p(&format!(r#"{macros}<citation><layout/></citation>"#));
    assert_eq!(s.macros.len(), 512);
    warned_contains(&s, "512");
}

#[test]
fn long_strings_are_clamped() {
    let v = "a".repeat(5_000);
    let s = p(&format!(r#"<citation><layout><text value="{v}"/></layout></citation>"#));
    let Element::Text(t) = &s.citation.elements[0] else { panic!("not text") };
    assert_eq!(
        match &t.source {
            TextSource::Value(v) => v.chars().count(),
            _ => 0,
        },
        4_000
    );
}

#[test]
fn the_full_en_us_set_resolves_common_term_references() {
    let s = parse(
        whole(
            r#"<citation><layout>
        <text term="in" prefix=" " suffix=": "/>
        <text term="chapter"/>
        <text term="opus" form="short"/>
        <text term="circa" form="short"/>
        <text term="long-ordinal-03"/>
        <label variable="verse" form="short"/>
      </layout></citation>"#,
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(s.locale_terms.text("in", Form::Long, false), Some("in"));
    assert_eq!(s.locale_terms.text("opus", Form::Short, true), Some("opp."));
    assert_eq!(s.locale_terms.text("circa", Form::Short, false), Some("c."));
    assert_eq!(s.locale_terms.text("long-ordinal-03", Form::Long, false), Some("third"));
    assert_eq!(s.locale_terms.text("verse", Form::Short, true), Some("vv."));
    assert!(s.warned().is_empty(), "{:?}", s.warned());
}

#[test]
fn unresolvable_term_references_warn() {
    // One unknown term, however often it appears: exactly one warning.
    let s = parse(
        whole(r#"<citation><layout><text term="in"/><text term="gibberish-term"/><text term="gibberish-term"/></layout></citation>"#).as_bytes(),
    )
    .unwrap();
    assert_eq!(s.warned().len(), 1, "{:?}", s.warned());
    warned_contains(&s, "gibberish-term");
    // References inside macros and branches are checked too.
    let s = parse(
        whole(
            r#"<macro name="m"><text term="made-up-in-macro"/></macro><citation><layout><choose><if variable="page"><text macro="m"/></if><else><text term="also-made-up"/></else></choose></layout></citation>"#,
        )
        .as_bytes(),
    )
    .unwrap();
    warned_contains(&s, "made-up-in-macro");
    warned_contains(&s, "also-made-up");
    // An unknown label variable warns as well.
    let s = parse(whole(r#"<citation><layout><label variable="gibberish-label"/></layout></citation>"#).as_bytes()).unwrap();
    warned_contains(&s, "gibberish-label");
    // A style's en-US override can define a term the defaults lack, resolving the reference.
    let s = parse(
        whole(
            r#"<locale xml:lang="en-US"><terms><term name="custom-word">Custom</term></terms></locale><citation><layout><text term="custom-word"/></layout></citation>"#,
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(s.locale_terms.text("custom-word", Form::Long, false), Some("Custom"));
    assert!(s.warned().is_empty(), "{:?}", s.warned());
}

#[test]
fn wide_attribute_lists_load_with_truncation_warnings() {
    // APA-scale type lists (18 entries) load whole, silently.
    let types = (1..=18).map(|i| format!("t{i:02}")).collect::<Vec<_>>().join(" ");
    let s = parse(whole(&format!(r#"<citation><layout><choose><if type="{types}"><text value="x"/></if></choose></layout></citation>"#)).as_bytes())
        .unwrap();
    let Element::Choose(c) = &s.citation.elements[0] else { panic!("not choose") };
    let Some(Condition::Type { names }) = c.branches[0].conditions.first() else { panic!("not a type condition") };
    assert_eq!(names.len(), 18);
    assert!(s.warned().is_empty(), "{:?}", s.warned());

    // 130 types: 128 kept, with a warning; the same for variables and positions.
    let over = (1..=130).map(|i| format!("t{i}")).collect::<Vec<_>>().join(" ");
    let s = parse(whole(&format!(r#"<citation><layout><choose><if type="{over}"><text value="x"/></if></choose></layout></citation>"#)).as_bytes())
        .unwrap();
    warned_contains(&s, "more than 128");
    let Element::Choose(c) = &s.citation.elements[0] else { panic!("not choose") };
    let Some(Condition::Type { names }) = c.branches[0].conditions.first() else { panic!("not a type condition") };
    assert_eq!(names.len(), 128);

    let vars = (1..=130).map(|i| format!("v{i}")).collect::<Vec<_>>().join(" ");
    let s =
        parse(whole(&format!(r#"<citation><layout><choose><if variable="{vars}"><text value="x"/></if></choose></layout></citation>"#)).as_bytes())
            .unwrap();
    warned_contains(&s, "more than 128");
    let Element::Choose(c) = &s.citation.elements[0] else { panic!("not choose") };
    assert_eq!(c.branches[0].conditions.len(), 128);

    // 70 name variables: 64 kept, with a warning.
    let vars = (1..=70).map(|i| format!("n{i}")).collect::<Vec<_>>().join(" ");
    let s = parse(whole(&format!(r#"<citation><layout><names variable="{vars}"/></layout></citation>"#)).as_bytes()).unwrap();
    warned_contains(&s, "more than 64");
    let Element::Names(n) = &s.citation.elements[0] else { panic!("not names") };
    assert_eq!(n.variables.len(), 64);
}

#[test]
fn names_label_position_parses() {
    // The label written before the <name> child precedes the names; after it, it follows.
    let s = parse(whole(r#"<citation><layout><names variable="editor"><label form="verb"/><name/></names></layout></citation>"#).as_bytes()).unwrap();
    let Element::Names(n) = &s.citation.elements[0] else { panic!("not names") };
    assert!(n.label_precedes_names);
    let s = parse(whole(r#"<citation><layout><names variable="editor"><name/><label form="verb"/></names></layout></citation>"#).as_bytes()).unwrap();
    let Element::Names(n) = &s.citation.elements[0] else { panic!("not names") };
    assert!(!n.label_precedes_names);
    // Without a <name> child the label is the only child; it precedes.
    let s = parse(whole(r#"<citation><layout><names variable="editor"><label/></names></layout></citation>"#).as_bytes()).unwrap();
    let Element::Names(n) = &s.citation.elements[0] else { panic!("not names") };
    assert!(n.label_precedes_names);
}

#[test]
fn inheritable_name_options_parse_from_root_and_sections() {
    // Root options land as the style's defaults; `name-delimiter` is the spec's attribute.
    let s =
        parse(br#"<style version="1.0" initialize-with=". " name-as-sort-order="all" name-delimiter="; "><citation><layout/></citation></style>"#)
            .unwrap();
    assert_eq!(s.name_options.initialize_with, Some(". ".into()));
    assert_eq!(s.name_options.name_as_sort_order, Some(NameAsSortOrder::All));
    assert_eq!(s.name_options.delimiter, Some("; ".into()));
    assert!(s.warned().is_empty(), "{:?}", s.warned());

    // Section options merge over the root's; the bibliography, setting none, inherits.
    let s = parse(
        br#"<style version="1.0" initialize-with=". " et-al-min="9">
          <citation et-al-min="3" et-al-use-first="1"><layout/></citation>
          <bibliography and="symbol"><layout/></bibliography>
        </style>"#,
    )
    .unwrap();
    assert_eq!(s.name_options.et_al_min, Some(9));
    assert_eq!(s.citation.name_options.et_al_min, Some(3));
    assert_eq!(s.citation.name_options.et_al_use_first, Some(1));
    assert_eq!(s.citation.name_options.initialize_with, Some(". ".into())); // inherited
    assert_eq!(s.bibliography.as_ref().map(|b| b.layout.name_options.and), Some(Some(crate::model::AndStyle::Symbol)));
    assert_eq!(s.bibliography.as_ref().map(|b| b.layout.name_options.et_al_min), Some(Some(9))); // inherited
    assert!(s.warned().is_empty(), "{:?}", s.warned());

    // `names-delimiter` (between name variables) is still outside the subset.
    let s = parse(br#"<style version="1.0" names-delimiter="; "><citation><layout/></citation></style>"#).unwrap();
    warned_contains(&s, "names-delimiter");
    // The section options that cannot render still warn, at every level.
    let s = parse(br#"<style version="1.0" et-al-use-last="true"><citation><layout/></citation></style>"#).unwrap();
    warned_contains(&s, "et-al-use-last");
}

#[test]
fn pristine_minimal_style_has_no_warnings() {
    let s = parse(br#"<style version="1.0"><citation><layout><text variable="title"/></layout></citation></style>"#).unwrap();
    assert_eq!(s.title, "");
    assert_eq!(s.id, "");
    assert_eq!(s.bibliography, None);
    assert_eq!(s.citation.elements.len(), 1);
    assert!(s.warned().is_empty(), "{:?}", s.warned());
}
