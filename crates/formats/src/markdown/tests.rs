use super::*;

fn inl(s: &str) -> Vec<Inline> {
    parse_inlines(s, &Fmt::default(), 0, &Refs::new())
}

fn spans(s: &str) -> Vec<(String, bool, bool)> {
    inl(s).into_iter().filter_map(|i| if let Inline::Text(t, f) = i { Some((t, f.bold, f.italic)) } else { None }).collect()
}

#[test]
fn emphasis() {
    assert_eq!(spans("a *b* c"), vec![("a ".into(), false, false), ("b".into(), false, true), (" c".into(), false, false)]);
    assert_eq!(spans("**b**"), vec![("b".into(), true, false)]);
    assert_eq!(spans("***x***"), vec![("x".into(), true, true)]);
    assert_eq!(spans("*a **b** c*"), vec![("a ".into(), false, true), ("b".into(), true, true), (" c".into(), false, true)]);
    assert_eq!(spans("snake_case_name"), vec![("snake_case_name".into(), false, false)]);
    assert_eq!(spans("2 * 3 * 4"), vec![("2 * 3 * 4".into(), false, false)]);
    assert_eq!(spans("**unclosed"), vec![("**unclosed".into(), false, false)]);
    let s = inl("~~gone~~");
    assert!(matches!(&s[0], Inline::Text(t, f) if t == "gone" && f.strike));
}

#[test]
fn links_code_escapes() {
    let s = inl("see [the *site*](http://x.org \"T\") and `a*b*` \\*lit\\* &amp; <https://y.com>");
    let link: Vec<_> = s
        .iter()
        .filter_map(|i| if let Inline::Text(t, f) = i { f.link.as_ref().map(|l| (t.clone(), l.to_string(), f.italic)) } else { None })
        .collect();
    assert_eq!(link[0], ("the ".into(), "http://x.org".into(), false));
    assert_eq!(link[1], ("site".into(), "http://x.org".into(), true));
    assert!(link.iter().any(|l| l.1 == "https://y.com"));
    assert!(s.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "a*b*" && f.code)));
    let text: String = s.iter().filter_map(|i| if let Inline::Text(t, _) = i { Some(t.as_str()) } else { None }).collect();
    assert!(text.contains("*lit* & "), "{text}");
}

/// Reference links: `[text][label]`, `[text][]` and `[text]` take their destination from a
/// definition line, and the definition itself leaves the document. Before, none of the three
/// was a link and the definition stayed as a paragraph of its own (#3238).
#[test]
fn reference_links_resolve_and_their_definitions_leave_the_body() {
    let links = |md: &str| -> Vec<(String, String)> {
        match &*parse(md).blocks {
            [FBlock::Para(p), ..] => p
                .inlines
                .iter()
                .filter_map(|x| match x {
                    Inline::Text(t, f) => f.link.as_deref().map(|l| (t.clone(), l.to_string())),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    };
    let def = "\n\n[ref1]: https://example.com/ref\n";
    let to_ref = |text: &str| vec![(text.to_string(), "https://example.com/ref".to_string())];

    assert_eq!(links(&format!("[click here][ref1]{def}")), to_ref("click here"), "full");
    assert_eq!(links(&format!("[ref1][]{def}")), to_ref("ref1"), "collapsed");
    assert_eq!(links(&format!("[ref1]{def}")), to_ref("ref1"), "shortcut");
    // Labels match case-folded with runs of white space collapsed (CommonMark §4.7).
    assert_eq!(links(&format!("[click here][  REF1  ]{def}")), to_ref("click here"), "folded label");
    // An undefined label is not a link: an ordinary bracketed phrase stays text.
    assert_eq!(links(&format!("[nope]{def}")), vec![], "undefined");
    // Inline links are unaffected.
    assert_eq!(links("[x](https://inline.example)"), vec![("x".into(), "https://inline.example".into())], "inline");

    // The definition is not a paragraph, and its text is nowhere in the body.
    let doc = import(&format!("Reference link: [click here][ref1]{def}"));
    let text = doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert_eq!(text, "Reference link: click here", "{text:?}");

    // A definition inside a fence is code, not a definition.
    let fenced = "```\n[ref1]: https://example.com/ref\n```\n\n[ref1]\n";
    let body = import(fenced).plain_text(wordcraft_doc::StoryRef::Body);
    assert!(body.contains("[ref1]: https://example.com/ref"), "the fenced definition is still code: {body:?}");
    assert_eq!(links(fenced), vec![], "a fenced definition defines nothing");
}

/// Cached bracket pairs must not make undefined shortcut references normalize every nested
/// suffix again. One unrelated definition keeps this on the reference-lookup path.
#[test]
fn nested_undefined_reference_labels_have_bounded_work() {
    let literal = format!("prefix {}x{} suffix", "[".repeat(128_000), "]".repeat(128_000));
    let src = format!("{literal}\n\n[other]: https://example.com/other\n");
    let started = std::time::Instant::now();
    let doc = import(&src);
    let elapsed = started.elapsed();
    assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body), literal);
    assert!(elapsed < std::time::Duration::from_secs(10), "nested undefined references took {elapsed:?}");
}

#[test]
fn reference_lookup_keeps_long_unicode_labels_and_nested_text() {
    // The work guard is not a label-length cap. Collapsed/full/shortcut references keep their
    // case and whitespace matching, even when lowercasing changes the number of UTF-8 bytes.
    let label = format!("{}İ END", "Écho ".repeat(1500));
    let key = label.to_lowercase();
    let src = format!("[{label}] [{label}][] [full][{label}]\n\n[{key}]: https://example.com/long\n");
    let flow = parse(&src);
    let FBlock::Para(p) = &flow.blocks[0] else { panic!("paragraph") };
    let links: Vec<_> = p
        .inlines
        .iter()
        .filter_map(|x| match x {
            Inline::Text(t, f) if f.link.is_some() => Some((t.as_str(), f.link.as_deref())),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        vec![
            (label.as_str(), Some("https://example.com/long")),
            (label.as_str(), Some("https://example.com/long")),
            ("full", Some("https://example.com/long"))
        ]
    );

    let refs = Refs::from([("known".into(), Arc::from("https://example.com/known"))]);
    let out = parse_inlines("[outer [known]] tail [inline](https://example.com/inline)", &Fmt::default(), 0, &refs);
    let text: String = out.iter().filter_map(|x| if let Inline::Text(t, _) = x { Some(t.as_str()) } else { None }).collect();
    assert_eq!(text, "[outer known] tail inline");
    assert!(out.iter().any(|x| matches!(x, Inline::Text(t, f) if t == "known" && f.link.as_deref() == Some("https://example.com/known"))));
    assert!(out.iter().any(|x| matches!(x, Inline::Text(t, f) if t == "inline" && f.link.as_deref() == Some("https://example.com/inline"))));
}

/// A definition has to start a block (CommonMark §4.7), and a fence opens and closes by the
/// same rules the block parser uses. Reading a line without that context deleted text: a
/// definition-shaped line inside a paragraph, and every such line inside a code block whose
/// fence this pass thought had closed.
#[test]
fn a_definition_shaped_line_that_is_not_a_definition_stays_in_the_document() {
    let body = |md: &str| import(md).plain_text(wordcraft_doc::StoryRef::Body);

    // A definition cannot interrupt a paragraph: this is the author's text, lines and all.
    let para = "Status update\n[Warning]: unstable\nmore text\n";
    assert_eq!(body(para), "Status update [Warning]: unstable more text", "{:?}", body(para));

    // `` ```rust `` holds an info string, so it does not close a fence: the definition-shaped
    // line after it is still code.
    let code = "```\n```rust\n[foo]: /bar\n```\n\nSee [foo].\n";
    assert!(body(code).contains("[foo]: /bar"), "{:?}", body(code));
    assert!(body(code).ends_with("See [foo]."), "nothing defines foo: {:?}", body(code));

    // Four spaces of indentation is code, not a fence, so the definition that follows is
    // taken and the reference resolves.
    let indented = "    ```\n\n[x]: /y\n\nSee [x].\n";
    let doc = import(indented);
    let text = doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(text.contains("```") && text.ends_with("See x."), "{text:?}");

    // A definition still gives up its line after a blank line, at the start of the document,
    // after a heading, and after another definition.
    let starts = "[a]: /a\n[b]: /b\n\n# Head\n[c]: /c\n\nSee [a], [b], [c].\n";
    assert_eq!(body(starts), "Head\nSee a, b, c.", "{:?}", body(starts));

    // A footnote definition is not a link definition: it stays in the body.
    let footnote = "x[^1]\n\n[^1]: Note\n";
    assert!(body(footnote).contains("[^1]: Note"), "{:?}", body(footnote));
}

#[test]
fn blocks() {
    let f = parse(
        "# Title\n\nPara one\ncontinues.\n\n> quoted\n\n- a\n- b\n    1. nested\n\n```\ncode *x*\n```\n\n---\n\n| A | B |\n|---|:-:|\n| 1 | 2 |\n",
    );
    let kinds: Vec<String> = f
        .blocks
        .iter()
        .map(|b| match b {
            FBlock::Para(p) => format!("{:?}{:?}:{}", p.kind, p.list.map(|l| (l.ordered, l.level)), p.text()),
            FBlock::Table(t) => format!("table{}", t.rows.len()),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "Heading(1)None:Title",
            "NormalNone:Para one continues.",
            "QuoteNone:quoted",
            "NormalSome((false, 0)):a",
            "NormalSome((false, 0)):b",
            "NormalSome((true, 1)):nested",
            "CodeNone:code *x*",
            "RuleNone:",
            "table2",
        ]
    );
}

#[test]
fn export_escapes_round_trip() {
    let mut p = Para::default();
    p.push_text("1. not a list *really*", &Fmt::default());
    let md = escape_line_start(inlines_md(&p.inlines));
    let back = parse(&md);
    let FBlock::Para(q) = &back.blocks[0] else { panic!() };
    assert_eq!(q.text(), "1. not a list *really*");
    assert!(q.list.is_none());
}

#[test]
fn export_emphasis_next_to_punctuation_round_trips() {
    let marks: [(&str, fn(&mut Fmt)); 3] = [("bold", |f| f.bold = true), ("italic", |f| f.italic = true), ("strike", |f| f.strike = true)];
    let cases: [&[(&str, bool)]; 4] = [
        &[("注意：", true), ("请阅读", false)],
        &[("a.", true), ("b", false)],
        &[("a", false), ("!", true), ("b", false)],
        &[("a", false), ("(x)", true), ("b", false)],
    ];
    for (name, set) in marks {
        for runs in cases {
            let mut p = Para::default();
            for (t, on) in runs {
                let mut f = Fmt::default();
                if *on {
                    set(&mut f);
                }
                p.push_text(t, &f);
            }
            let md = inlines_md(&p.inlines);
            let back = parse(&md);
            let FBlock::Para(q) = &back.blocks[0] else { panic!() };
            let want: String = runs.iter().map(|r| r.0).collect();
            assert_eq!(q.text(), want, "{name} exported as {md:?}");
            let got: Vec<(String, bool)> = q
                .inlines
                .iter()
                .filter_map(|i| if let Inline::Text(t, f) = i { Some((t.clone(), f.bold || f.italic || f.strike)) } else { None })
                .collect();
            let expect: Vec<(String, bool)> = runs.iter().map(|(t, on)| (t.to_string(), *on)).collect();
            assert_eq!(got, expect, "{name} exported as {md:?}");
        }
    }
}

#[test]
fn export_adjacent_emphasis_next_to_punctuation_round_trips() {
    let b = Fmt { bold: true, ..Fmt::default() };
    let i = Fmt { italic: true, ..Fmt::default() };
    let s = Fmt { strike: true, ..Fmt::default() };
    let cases: [&[(&str, &Fmt)]; 6] = [
        &[("ab", &b), ("!c", &i)],
        &[("ab", &i), ("!c", &b)],
        &[("ab", &b), ("!c", &s)],
        &[("ab", &s), ("(c)", &b)],
        &[("a.", &b), ("b", &i)],
        &[("a.", &b), ("!b", &i)],
    ];
    for runs in cases {
        let mut p = Para::default();
        for (t, f) in runs {
            p.push_text(t, f);
        }
        let md = inlines_md(&p.inlines);
        let back = parse(&md);
        let FBlock::Para(q) = &back.blocks[0] else { panic!() };
        let got: Vec<(String, bool, bool, bool)> =
            q.inlines.iter().filter_map(|x| if let Inline::Text(t, f) = x { Some((t.clone(), f.bold, f.italic, f.strike)) } else { None }).collect();
        let expect: Vec<(String, bool, bool, bool)> = runs.iter().map(|(t, f)| (t.to_string(), f.bold, f.italic, f.strike)).collect();
        assert_eq!(got, expect, "exported as {md:?}");
    }
}

#[test]
fn escape_line_start_preserves_indentation() {
    assert_eq!(escape_line_start("   indented text".to_string()), "   indented text");
    assert_eq!(escape_line_start("   # not a heading".to_string()), "   \\# not a heading");
    assert_eq!(escape_line_start("  1. not a list".to_string()), "  1\\. not a list");
    assert_eq!(escape_line_start("  > quote".to_string()), "  \\> quote");
    // Four spaces or a tab would make an indented code block: the indent is capped at three.
    assert_eq!(escape_line_start("      deep".to_string()), "   deep");
    assert_eq!(escape_line_start("\tTabbed # text".to_string()), "   Tabbed # text");
    assert_eq!(escape_line_start("    # not a heading".to_string()), "   \\# not a heading");
    assert_eq!(escape_line_start(" \t1. not a list".to_string()), "   1\\. not a list");
}

#[test]
fn leading_unicode_whitespace_is_kept_and_does_not_start_blocks() {
    let kinds_text = |md: &str| -> Vec<(Kind, bool, String)> {
        parse(md).blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some((p.kind, p.list.is_some(), p.text())) } else { None }).collect()
    };
    assert_eq!(kinds_text("\u{3000}\u{3000}日本語の段落\n"), vec![(Kind::Normal, false, "\u{3000}\u{3000}日本語の段落".to_string())]);
    for src in ["\u{a0}# not a heading", "\u{3000}1. not a list", "\u{a0}- x", "\u{a0}> not a quote"] {
        let mut p = Para::default();
        p.push_text(src, &Fmt::default());
        let md = escape_line_start(inlines_md(&p.inlines));
        assert_eq!(kinds_text(&md), vec![(Kind::Normal, false, src.to_string())], "{src:?} via {md:?}");
    }
}

#[test]
fn single_backtick_code_span_escaped_pipe() {
    let md = "| Command | Status |\n| --- | --- |\n| `cmd \\| grep` | pass |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[1].len(), 2);
    let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
    assert_eq!(p0.inlines.len(), 1);
    assert!(matches!(&p0.inlines[0], Inline::Text(t, f) if t == "cmd | grep" && f.code));
    let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
    assert_eq!(p1.text(), "pass");

    let mut out = Vec::new();
    table_md(t, &mut out);
    let exported = out.join("\n");
    assert!(exported.contains("`cmd \\| grep`"), "Exported: {exported}");
}

#[test]
fn double_backtick_code_span_escaped_pipe() {
    let md = "| Syntax | Note |\n| --- | --- |\n| ``a \\| b`` | second |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[1].len(), 2);
    let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
    assert_eq!(p0.inlines.len(), 1);
    assert!(matches!(&p0.inlines[0], Inline::Text(t, f) if t == "a | b" && f.code));
    let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
    assert_eq!(p1.text(), "second");
}

#[test]
fn code_span_literal_backtick_and_escaped_pipe() {
    let md = "| Expression | Detail |\n| --- | --- |\n| ``a`b\\|c`` | distinct |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[1].len(), 2);
    let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
    assert_eq!(p0.inlines.len(), 1);
    assert!(matches!(&p0.inlines[0], Inline::Text(t, f) if t == "a`b|c" && f.code));
    let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
    assert_eq!(p1.text(), "distinct");
}

#[test]
fn three_column_table_with_distinctive_text() {
    let md = "| Col1 | Col2 | Col3 |\n| --- | --- | --- |\n| `p1\\|q1` | Distinctive Two | Distinctive Three |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[1].len(), 3);
    assert_eq!(t.rows[1][0].text(), "p1|q1");
    assert_eq!(t.rows[1][1].text(), "Distinctive Two");
    assert_eq!(t.rows[1][2].text(), "Distinctive Three");
    let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
    assert!(matches!(&p0.inlines[0], Inline::Text(_, f) if f.code));
    let FBlock::Para(p1) = &t.rows[1][1].blocks[0] else { panic!() };
    assert!(matches!(&p1.inlines[0], Inline::Text(_, f) if !f.code));
    let FBlock::Para(p2) = &t.rows[1][2].blocks[0] else { panic!() };
    assert!(matches!(&p2.inlines[0], Inline::Text(_, f) if !f.code));
}

#[test]
fn regular_table_cells_containing_escaped_pipes() {
    let md = "| Option | Description |\n| --- | --- |\n| Regular \\| Pipe | Normal text cell |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[1].len(), 2);
    assert_eq!(t.rows[1][0].text(), "Regular | Pipe");
    assert_eq!(t.rows[1][1].text(), "Normal text cell");
    let FBlock::Para(p0) = &t.rows[1][0].blocks[0] else { panic!() };
    assert!(matches!(&p0.inlines[0], Inline::Text(_, f) if !f.code));

    let mut out = Vec::new();
    table_md(t, &mut out);
    let exported = out.join("\n");
    assert!(exported.contains("Regular \\| Pipe"), "Exported: {exported}");
    assert!(!exported.contains("Regular \\\\| Pipe"), "Exported: {exported}");
}

#[test]
fn unescaped_pipes_behave_as_gfm_column_separators() {
    let md = "| Header 1 | Header 2 |\n| --- | --- |\n| `raw|pipe` | Extra Column |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[1].len(), 2);
    assert_eq!(t.rows[1][0].text(), "`raw");
    assert_eq!(t.rows[1][1].text(), "pipe`");
}

#[test]
fn ordinary_tables_and_standalone_code_spans_unchanged() {
    let md = "| Alpha | Beta |\n| --- | --- |\n| 1 | 2 |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!() };
    let mut out = Vec::new();
    table_md(t, &mut out);
    assert_eq!(out.join("\n"), "| Alpha | Beta |\n| --- | --- |\n| 1 | 2 |");

    let mut p = Para::default();
    let f = Fmt { code: true, ..Default::default() };
    p.push_text("cmd | grep", &f);
    let exported = inlines_md(&p.inlines);
    assert_eq!(exported, "`cmd | grep`");
}

#[test]
fn code_span_line_endings_stay_literal_on_export() {
    let cases = [
        ("a\n# b", "a # b"),
        ("a\n<div>b</div>", "a <div>b</div>"),
        ("a\n- b", "a - b"),
        ("a\n```", "a ```"),
        ("a\r# b", "a # b"),
        ("a\r\n# b", "a # b"),
        ("a\n\n> b", "a  > b"),
        ("\na\n", " a "),
        ("`a\n# b`", "`a # b`"),
        ("cmd | grep", "cmd | grep"),
    ];
    let shapes = [
        (Kind::Normal, None),
        (Kind::Normal, Some(ListInfo { ordered: false, level: 0, start: 1 })),
        (Kind::Code, Some(ListInfo { ordered: true, level: 0, start: 1 })),
    ];
    for (source, expected) in cases {
        for (kind, list) in shapes {
            let mut p = Para::new(kind);
            p.list = list;
            p.push_text(source, &Fmt { code: true, ..Default::default() });
            let mut doc = model::to_doc(&model::Flow { blocks: vec![FBlock::Para(p)], ..Default::default() });
            // The flow builder removes CR controls; keep the native document's raw input.
            let p = doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
            p.text = source.to_string();
            p.runs[0].len = source.len();
            let bytes = crate::export("md", &doc).unwrap().unwrap();
            let md = String::from_utf8(bytes).unwrap();
            assert_eq!(md.trim_end().lines().count(), 1, "{source:?} {kind:?} {list:?} -> {md:?}");
            assert!(!md.contains('\r'), "{md:?}");
            let back = crate::import("md", md.as_bytes()).unwrap().unwrap();
            let flow = model::from_doc(&back);
            let [FBlock::Para(q)] = flow.blocks.as_slice() else { panic!("{md:?}") };
            assert_eq!(q.text(), expected, "{source:?} {kind:?} {list:?} -> {md:?}");
            assert_eq!(q.list, list, "{md:?}");
            assert!(q.inlines.iter().all(|i| matches!(i, Inline::Text(_, f) if f.code)), "{md:?}");
        }
    }
    let fenced = "```\na\n# b\n```";
    assert_eq!(export(&import(fenced)).trim_end(), fenced);
}

#[test]
fn code_span_line_endings_in_table_cells_stay_literal() {
    for ending in ["\n", "\r", "\r\n"] {
        let source = format!("a{ending}# b|c");
        let mut p = Para::default();
        p.push_text(&source, &Fmt { code: true, ..Default::default() });
        let cell = Cell { blocks: vec![FBlock::Para(p)], ..Default::default() };
        let table = FTable { rows: vec![vec![cell, Cell::default()], vec![Cell::default(), Cell::default()]], ..Default::default() };
        let mut doc = model::to_doc(&model::Flow { blocks: vec![FBlock::Table(table)], ..Default::default() });
        let p = doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path(vec![0, 0, 0, 0])).unwrap();
        p.text = source.clone();
        p.runs[0].len = source.len();
        let md = export(&doc);
        let back = model::from_doc(&import(&md));
        let [FBlock::Table(table), ..] = back.blocks.as_slice() else { panic!("{md:?}") };
        assert!(back.blocks.iter().skip(1).all(|b| matches!(b, FBlock::Para(p) if p.text().is_empty())), "{md:?}");
        assert_eq!(table.rows.len(), 2, "{md:?}");
        assert_eq!(table.rows[0].len(), 2, "{md:?}");
        let [FBlock::Para(p)] = table.rows[0][0].blocks.as_slice() else { panic!("{md:?}") };
        assert_eq!(p.text(), "a # b|c", "{md:?}");
        assert!(p.inlines.iter().all(|i| matches!(i, Inline::Text(_, f) if f.code)), "{md:?}");
    }
}

#[test]
fn table_code_pipe_export_import_round_trip() {
    let mut table = FTable::default();
    let mut cell_a = Cell::default();
    let mut para_a = Para::default();
    let fmt_a = Fmt { code: true, ..Default::default() };
    para_a.push_text("a`b|c", &fmt_a);
    cell_a.blocks.push(FBlock::Para(para_a));

    let mut cell_b = Cell::default();
    let mut para_b = Para::default();
    para_b.push_text("CRITICAL_DISTINCTIVE_INFO", &Fmt::default());
    cell_b.blocks.push(FBlock::Para(para_b));

    let mut head_a = Cell::default();
    let mut head_para_a = Para::default();
    head_para_a.push_text("Col A", &Fmt::default());
    head_a.blocks.push(FBlock::Para(head_para_a));

    let mut head_b = Cell::default();
    let mut head_para_b = Para::default();
    head_para_b.push_text("Col B", &Fmt::default());
    head_b.blocks.push(FBlock::Para(head_para_b));

    table.rows.push(vec![head_a, head_b]);
    table.rows.push(vec![cell_a, cell_b]);

    let mut out = Vec::new();
    table_md(&table, &mut out);
    let exported_md = out.join("\n");

    assert!(exported_md.contains("``a`b\\|c``"), "Exported: {exported_md}");
    assert!(exported_md.contains("CRITICAL\\_DISTINCTIVE\\_INFO"), "Exported: {exported_md}");

    let back_flow = parse(&exported_md);
    let FBlock::Table(back_table) = &back_flow.blocks[0] else { panic!("expected table") };

    assert_eq!(back_table.rows.len(), 2, "Both rows must exist");
    let row1 = &back_table.rows[1];
    assert_eq!(row1.len(), 2, "Both cells must exist in row 1");

    let back_cell_a = &row1[0];
    let back_cell_b = &row1[1];
    assert_eq!(back_cell_a.text(), "a`b|c", "Cell A text must be preserved");
    assert_eq!(back_cell_b.text(), "CRITICAL_DISTINCTIVE_INFO", "Cell B text must be preserved");

    let FBlock::Para(back_p_a) = &back_cell_a.blocks[0] else { panic!() };
    assert_eq!(back_p_a.inlines.len(), 1);
    assert!(
        matches!(&back_p_a.inlines[0], Inline::Text(t, f) if t == "a`b|c" && f.code),
        "Code formatting must remain intact on Cell A: {:?}",
        back_p_a.inlines
    );

    let FBlock::Para(back_p_b) = &back_cell_b.blocks[0] else { panic!() };
    assert_eq!(back_p_b.inlines.len(), 1);
    assert!(
        matches!(&back_p_b.inlines[0], Inline::Text(t, f) if t == "CRITICAL_DISTINCTIVE_INFO" && !f.code),
        "Cell B formatting must remain plain text: {:?}",
        back_p_b.inlines
    );

    assert_eq!(back_cell_a.text(), "a`b|c");
    assert_eq!(back_cell_b.text(), "CRITICAL_DISTINCTIVE_INFO");

    assert!(!back_cell_a.text().contains("CRITICAL_DISTINCTIVE_INFO"));
    assert!(!back_cell_b.text().contains("a`b|c"));

    let doc = import(&exported_md);
    let re_exported = export(&doc);
    assert!(re_exported.contains("``a`b\\|c``"), "Re-exported: {re_exported}");
    assert!(re_exported.contains("CRITICAL\\_DISTINCTIVE\\_INFO"), "Re-exported: {re_exported}");
    let doc2 = import(&re_exported);
    let back_flow2 = model::from_doc(&doc2);
    let FBlock::Table(back_table2) = &back_flow2.blocks[0] else { panic!() };
    assert_eq!(back_table2.rows[1][0].text(), "a`b|c");
    assert_eq!(back_table2.rows[1][1].text(), "CRITICAL_DISTINCTIVE_INFO");
}

#[test]
fn ordered_list_keeps_start_number() {
    let txt = |md: &str| String::from_utf8_lossy(&crate::txt::export(&import(md))).into_owned();
    assert_eq!(txt("5. a\n6. b"), "5.\ta\r\n6.\tb");
    assert_eq!(txt("3. a\n- b\n4. c"), "3.\ta\r\n•\tb\r\n4.\tc");
    assert_eq!(export(&import("5. a\n6. b")).trim_end(), "5. a\n6. b");
}

#[test]
fn lines_after_line_break_stay_in_paragraph() {
    use wordcraft_doc::{CharProps, Document, Paragraph, StoryRef, para_block};
    let cases = ["Shopping\n- milk\n# not a heading", "a\n1. x", "a\n2) x", "a\n+ x", "a\n---", "a\n***", "a\n==", "a\n--", "a\n> q"];
    for s in cases {
        let mut d = Document::new();
        d.body = vec![para_block(Paragraph::with_text(s, CharProps::default()))];
        let md = export(&d);
        let back = import(&md);
        assert_eq!(back.body.len(), 1, "{s:?} -> {md:?}");
        assert_eq!(back.plain_text(StoryRef::Body), s, "{md:?}");
    }
}

#[test]
fn lines_after_line_break_stay_in_list_item_and_quote() {
    let cases = ["a\n- x", "a\n# h", "a\n1. x", "a\n--", "a\n---", "a\n==", "a\n> q"];
    let shapes: [(Kind, Option<ListInfo>); 5] = [
        (Kind::Normal, Some(ListInfo { ordered: false, level: 0, start: 1 })),
        (Kind::Normal, Some(ListInfo { ordered: true, level: 0, start: 1 })),
        (Kind::Normal, Some(ListInfo { ordered: false, level: 1, start: 1 })),
        (Kind::Normal, Some(ListInfo { ordered: true, level: 1, start: 1 })),
        (Kind::Quote, None),
    ];
    for s in cases {
        for (kind, list) in shapes {
            let mut blocks = Vec::new();
            if let Some(li) = list.filter(|l| l.level > 0) {
                let mut parent = Para::new(Kind::Normal);
                parent.list = Some(ListInfo { ordered: li.ordered, level: 0, start: 1 });
                parent.push_text("top", &Fmt::default());
                blocks.push(FBlock::Para(parent));
            }
            let mut p = Para::new(kind);
            p.list = list;
            p.push_text(s, &Fmt::default());
            blocks.push(FBlock::Para(p));
            let mut out = Vec::new();
            write_blocks(&blocks, &mut out);
            let md = out.join("\n");
            let back = parse(&md);
            assert_eq!(back.blocks.len(), blocks.len(), "{s:?} {kind:?} {list:?} -> {md:?}");
            let Some(FBlock::Para(q)) = back.blocks.last() else { panic!("{md:?}") };
            assert_eq!(q.kind, kind, "{md:?}");
            assert_eq!(q.list, list, "{md:?}");
            assert_eq!(q.text(), s, "{md:?}");
        }
    }
}

#[test]
fn table_column_alignments_import_and_export() {
    let md = "| Left | Center | Right | Default |\n| :--- | :---: | ---: | --- |\n| 1 | 2 | 3 | 4 |\n";
    let flow = parse(md);
    let FBlock::Table(t) = &flow.blocks[0] else { panic!("expected table") };
    assert_eq!(t.rows.len(), 2);
    let aligns: Vec<Option<Align>> =
        t.rows[0].iter().map(|c| c.blocks.iter().find_map(|b| if let FBlock::Para(p) = b { Some(p.align) } else { None }).flatten()).collect();
    assert_eq!(aligns, vec![Some(Align::Left), Some(Align::Center), Some(Align::Right), None]);

    let mut out = Vec::new();
    table_md(t, &mut out);
    let exported = out.join("\n");
    assert!(exported.contains("| :--- | :---: | ---: | --- |"), "Exported: {exported}");
}

#[test]
fn tab_separated_list_markers() {
    let f = parse("-\tbullet item\n*\tstar item\n+\tplus item\n1.\tone item\n2)\ttwo item\n");
    assert_eq!(f.blocks.len(), 5);
    for (i, b) in f.blocks.iter().enumerate() {
        let FBlock::Para(p) = b else { panic!("block {i} not a para") };
        assert!(p.list.is_some(), "block {i} should be a list item: {:?}", p);
    }
    let FBlock::Para(p0) = &f.blocks[0] else { panic!() };
    assert_eq!(p0.text(), "bullet item");
    assert!(!p0.list.unwrap().ordered);

    let FBlock::Para(p3) = &f.blocks[3] else { panic!() };
    assert_eq!(p3.text(), "one item");
    assert!(p3.list.unwrap().ordered);
}

#[test]
fn html_comments_are_ignored_in_blocks_and_inlines() {
    let md = "<!-- block comment -->\n\nHello <!-- inline comment -->world!\n\n<!--\nmultiline\ncomment\n-->\n\nAfter comment.";
    let flow = parse(md);
    assert_eq!(flow.blocks.len(), 2);
    let texts: Vec<String> = flow.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.text()) } else { None }).collect();
    assert_eq!(texts, ["Hello world!", "After comment."]);
}

#[test]
fn a_paragraph_of_delimiters_parses_in_linear_time() {
    // #3714: each delimiter's literal looked back for the text before it, to the start of
    // the paragraph when there was none (quadratic).
    let src = "*_".repeat(200_000);
    let t = std::time::Instant::now();
    let out = inl(&src);
    // Under a second on a loaded machine; the backwards search ran past 6 minutes.
    assert!(t.elapsed() < std::time::Duration::from_secs(10), "{:?}", t.elapsed());
    let text: String = out.iter().filter_map(|i| if let Inline::Text(t, _) = i { Some(t.as_str()) } else { None }).collect();
    assert!(text.chars().all(|c| matches!(c, '*' | '_')), "only delimiters come out");
    // Literal delimiters still take the link of the text before them.
    let s = inl("[a](http://x.org) [b*](http://y.org)");
    assert!(s.iter().any(|i| matches!(i, Inline::Text(t, f) if t == "b*" && f.link.as_deref() == Some("http://y.org"))), "{s:?}");
}

#[test]
fn exported_link_destinations_read_back_unchanged() {
    // Removing `<` and `>` from a destination after the safety check could make a relative
    // target a script or network-path one (`java<script:…` → `javascript:…`). They are
    // escaped instead, as are backslash escapes and entity references a reader would decode.
    let md = |l: &str| inlines_md(&[Inline::Text("x".into(), Fmt { link: Some(l.into()), ..Default::default() })]);
    assert_eq!(md("java<script:alert(1)"), "[x](<java\\<script:alert(1)>)");
    for l in [
        "java<script:alert(1)",
        "/</evil.example/(x)",
        "a>b c",
        "&#106;avascript:alert(1)",
        "&#x6A;avascript:x",
        "&amp;",
        "javascript\\:x",
        "dir\\",
        "dir\\(1)\\",
        "q?a=1&b=2",
    ] {
        let out = md(l);
        let back = inl(&out);
        let got: Vec<_> = back.iter().filter_map(|i| if let Inline::Text(_, f) = i { f.link.as_deref().map(str::to_string) } else { None }).collect();
        assert_eq!(got, [l.to_string()], "{l:?} → {out:?}");
    }
    // Ordinary destinations are written as before.
    assert_eq!(md("https://example.com/a?b=1&c=2"), "[x](https://example.com/a?b=1&c=2)");
    assert_eq!(md("https://example.com/a b"), "[x](<https://example.com/a b>)");
    assert_eq!(md("https://e.com/x\ny"), "[x](https://e.com/xy)");
}
