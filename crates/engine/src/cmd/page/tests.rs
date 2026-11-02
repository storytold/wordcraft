use super::*;

fn doc() -> Session {
    Session::new(wordcraft_doc::Document::from_text("one\ntwo\nthree"))
}

fn with_section(f: impl Fn(&mut Value)) -> Value {
    let mut v = serde_json::to_value(SectionProps::default()).unwrap();
    f(&mut v);
    json!({"section": v})
}

/// `layout.pageSetup {section}` checks the values like the dedicated commands do.
#[test]
fn page_setup_section_is_validated() {
    let mut s = doc();
    let before = sect(&s);
    // Refused with an error, nothing changes: non-finite (overflowing), margins that leave no text room.
    for (what, p) in [
        ("inf width", with_section(|v| v["pageW"] = json!(1e300))),
        ("inf margin", with_section(|v| v["marginTop"] = json!(1e300))),
        ("inf header", with_section(|v| v["header"] = json!(1e300))),
        ("inf column space", with_section(|v| v["columns"]["space"] = json!(1e300))),
        ("inf column width", with_section(|v| v["columns"]["widths"] = json!([[1e300, 0.0], [100.0, 0.0]]))),
        ("inf line number distance", with_section(|v| v["lineNumbers"] = json!({"distance": 1e300}))),
        ("inf grid pitch", with_section(|v| v["docGrid"] = json!({"type": "lines", "linePitch": 1e300, "charSpace": 0}))),
        ("inf border space", with_section(|v| v["pageBorders"] = json!({"top": {"style": "single", "width": 1.0, "space": 1e300}}))),
        ("inf border width", with_section(|v| v["pageBorders"] = json!({"top": {"style": "single", "width": 1e300, "space": 1.0}}))),
        ("margins > page", with_section(|v| v["marginLeft"] = json!(5000.0))),
        ("margins eat the height", with_section(|v| v["marginTop"] = json!(700.0))),
        ("gutter eats the width", with_section(|v| v["gutter"] = json!(600.0))),
        ("negative page", with_section(|v| v["pageW"] = json!(-5.0))),
        ("zero page height", with_section(|v| v["pageH"] = json!(0.0))),
    ] {
        assert!(s.run("layout.pageSetup", &p).is_err(), "{what}");
        assert_eq!(sect(&s), before, "{what}");
    }
    // Clamped.
    s.run(
        "layout.pageSetup",
        &with_section(|v| {
            v["pageW"] = json!(100.0);
            v["pageH"] = json!(100.0);
            v["marginTop"] = json!(-10.0);
            v["marginBottom"] = json!(0.0);
            v["marginLeft"] = json!(0.0);
            v["marginRight"] = json!(0.0);
            v["header"] = json!(-3.0);
            v["footer"] = json!(1e6);
            v["columns"]["count"] = json!(1000);
            v["columns"]["space"] = json!(-4.0);
            v["lineNumbers"] = json!({"countBy": 0, "start": 0, "distance": 1e6, "restart": "page"});
            v["docGrid"] = json!({"type": "lines", "linePitch": 1e9, "charSpace": 2_000_000_000});
            v["pageNumStart"] = json!(4_000_000_000u64);
            v["footnotePr"] = json!({"numStart": 4_000_000_000u64});
            v["endnotePr"] = json!({"numStart": 0});
            v["pageBorders"] =
                json!({"top": {"style": "single", "width": 1e6, "space": -5.0}, "left": {"style": "single", "width": 0.0, "space": 1e6}});
        }),
    )
    .unwrap();
    let x = sect(&s);
    assert_eq!((x.page_w, x.page_h), (100.0, 100.0));
    assert_eq!(x.margin_top, 0.0);
    assert_eq!(x.header, 0.0);
    assert_eq!(x.footer, x.page_h, "a footer distance stays on the page");
    assert_eq!(x.columns.count, 12);
    assert_eq!(x.columns.space, 0.0);
    let l = x.line_numbers.unwrap();
    assert_eq!((l.count_by, l.start, l.distance), (1, 1, 1584.0));
    let g = x.doc_grid.unwrap();
    assert_eq!((g.line_pitch, g.char_space), (wordcraft_doc::section::MAX_GRID_PITCH, wordcraft_doc::section::MAX_CHAR_SPACE));
    assert_eq!(x.page_num_start, Some(100_000));
    assert_eq!(x.footnote_pr.num_start, Some(wordcraft_doc::section::MAX_NOTE_START));
    assert_eq!(x.endnote_pr.num_start, Some(1));
    let b = x.page_borders.unwrap();
    let (t, lf) = (b.top.unwrap(), b.left.unwrap());
    assert_eq!((t.width, t.space), (6.0, 0.0));
    assert_eq!((lf.width, lf.space), (0.25, 31.0));
    // Huge pages clamp; columns and widths agree; orientation follows the sides.
    let mut s = doc();
    s.run(
        "layout.pageSetup",
        &with_section(|v| {
            v["pageW"] = json!(1e5);
            v["pageH"] = json!(792.0);
            v["landscape"] = json!(false);
            v["columns"]["count"] = json!(3);
            v["columns"]["widths"] = json!(vec![json!([1.0, 1e5]); 40]);
        }),
    )
    .unwrap();
    let x = sect(&s);
    assert_eq!(x.page_w, 1584.0);
    assert!(x.landscape, "wider than tall is landscape");
    assert_eq!(x.columns.widths.len(), 12);
    assert_eq!(x.columns.count, 12, "the count follows the widths");
    assert!(x.columns.widths.iter().all(|(w, sp)| *w >= 18.0 && (0.0..=300.0).contains(sp)));
    // One width is no custom layout: equal columns.
    s.run("layout.pageSetup", &with_section(|v| v["columns"]["widths"] = json!([[200.0, 0.0]]))).unwrap();
    assert!(sect(&s).columns.widths.is_empty());
}

/// A valid section still applies as given.
#[test]
fn page_setup_section_applies_a_valid_one() {
    let mut s = doc();
    s.run(
        "layout.pageSetup",
        &with_section(|v| {
            v["pageW"] = json!(595.0);
            v["pageH"] = json!(842.0);
            v["marginLeft"] = json!(50.0);
            v["columns"]["count"] = json!(2);
        }),
    )
    .unwrap();
    let x = sect(&s);
    assert_eq!((x.page_w, x.page_h, x.margin_left, x.columns.count), (595.0, 842.0, 50.0, 2));
}

/// A page smaller than the usual 36 pt of text room is fine with no margins; any margin or
/// gutter on it is refused.
#[test]
fn page_setup_section_allows_a_tiny_page_without_margins() {
    let tiny = |m: f32, g: f32| {
        with_section(|v| {
            v["pageW"] = json!(7.2);
            v["pageH"] = json!(20.0);
            for k in ["marginTop", "marginBottom", "marginLeft", "marginRight"] {
                v[k] = json!(m);
            }
            v["gutter"] = json!(g);
        })
    };
    let mut s = doc();
    let before = sect(&s);
    assert!(s.run("layout.pageSetup", &tiny(1.0, 0.0)).is_err(), "a margin on a tiny page");
    assert!(s.run("layout.pageSetup", &tiny(0.0, 1.0)).is_err(), "a gutter on a tiny page");
    assert_eq!(sect(&s), before);
    s.run("layout.pageSetup", &tiny(0.0, 0.0)).unwrap();
    assert_eq!((sect(&s).page_w, sect(&s).page_h), (7.2, 20.0));
}

/// Layout › Text Direction turns the section, or the table cell the caret is in; one undo step.
#[test]
fn text_direction_turns_the_section_or_the_cell() {
    use wordcraft_doc::props::TextDirection;
    let mut s = doc();
    s.run("layout.textDirection", &json!({"value": "down"})).unwrap();
    assert_eq!(sect(&s).text_direction, TextDirection::Down);
    assert_eq!(text_direction_at(&s), TextDirection::Down);
    // Without a value: the next one, like Word's button.
    s.run("layout.textDirection", &json!({})).unwrap();
    assert_eq!(sect(&s).text_direction, TextDirection::Up);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(sect(&s).text_direction, TextDirection::Down);
    assert!(s.run("layout.textDirection", &json!({"value": "diagonal"})).is_err());
    assert_eq!(sect(&s).text_direction, TextDirection::Down);
    // In a table cell it turns the cell, not the section.
    s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
    assert!(s.sel.focus.path.cell().is_some());
    s.run("layout.textDirection", &json!({"value": "btLr"})).unwrap();
    assert_eq!(text_direction_at(&s), TextDirection::Up);
    assert_eq!(sect(&s).text_direction, TextDirection::Down);
}

/// #1206: each Margins and Columns preset is recognised after it's applied (Mirrored and
/// Office 2003 share three margins; mirror margins tell them apart), and a value no preset
/// gives is none of them.
#[test]
fn page_setup_presets_are_recognised() {
    let mut s = doc();
    for (id, _) in MARGIN_PRESETS {
        s.run("layout.margins", &json!({"preset": id})).unwrap();
        assert_eq!(margins_preset_at(&s), Some(id));
    }
    s.run("layout.margins", &json!({"left": 80.0})).unwrap();
    assert_eq!(margins_preset_at(&s), None);
    for (params, id) in [
        (json!({"count": 2}), "two"),
        (json!({"preset": "left"}), "left"),
        (json!({"count": 3}), "three"),
        (json!({"preset": "right"}), "right"),
        (json!({"count": 1}), "one"),
    ] {
        s.run("layout.columns", &params).unwrap();
        assert_eq!(columns_preset_at(&s), Some(id), "{params}");
    }
    s.run("layout.columns", &json!({"count": 4})).unwrap();
    assert_eq!(columns_preset_at(&s), None);
}

/// #852: a continuous section break with text selected goes before the selection and keeps
/// it, as in Word; it used to delete the text, so the first section came out empty. Saved and
/// opened again, both sections' text lays out on page 1, the second below the first.
#[test]
fn section_break_keeps_the_selected_text() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("one\ntwo"));
    s.run("select.text", &json!({"text": "one"})).unwrap();
    s.run("layout.break", &json!({"kind": "continuous"})).unwrap();
    assert_eq!(s.selected_text(), "one");
    assert_eq!(s.doc.sections().len(), 2);
    let texts = |d: &wordcraft_doc::Document| -> Vec<String> {
        d.body.iter().filter_map(|b| if let Block::Para(p) = &**b { Some(p.plain_text().to_string()) } else { None }).collect()
    };
    assert_eq!(texts(&s.doc), ["", "one", "two"]);
    // A backward selection in the middle of a paragraph keeps its text and direction too.
    let mut s = Session::new(wordcraft_doc::Document::from_text("alpha beta\ngamma"));
    let p = |off| Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off };
    s.sel = Selection { anchor: p(10), focus: p(6) };
    s.run("layout.break", &json!({"kind": "continuous"})).unwrap();
    assert_eq!(s.selected_text(), "beta");
    assert!(s.sel.anchor > s.sel.focus);
    assert_eq!(texts(&s.doc), ["alpha ", "beta", "gamma"]);

    // The issue's document: "one", a continuous break, "two" — both on page 1 after a round trip.
    let mut s = Session::new(wordcraft_doc::Document::from_text("one\ntwo"));
    s.run("select.text", &json!({"text": "one"})).unwrap();
    s.run("caret.end", &json!({})).unwrap();
    s.run("layout.break", &json!({"kind": "continuous"})).unwrap();
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(back.sections().len(), 2);
    assert_eq!(back.last_section.start, SectionStart::Continuous);
    let mut s = Session::new(back);
    let l = s.layout();
    assert_eq!(l.pages.len(), 1);
    let mut lines: Vec<(String, f32)> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            crate::layout::Placed::Lines { path, y, .. } => {
                let Some(Block::Para(p)) = s.doc.body.get(path.last()).map(|b| &**b) else { return None };
                (!p.plain_text().is_empty()).then(|| (p.plain_text().to_string(), *y))
            }
            _ => None,
        })
        .collect();
    lines.sort_by(|a, b| a.1.total_cmp(&b.1));
    let order: Vec<&str> = lines.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(order, ["one", "two"]);
}

/// More Columns: unequal widths with a line between, as one undo step.
#[test]
fn unequal_columns_with_a_line_between() {
    let mut s = doc();
    let tw = sect(&s).text_width();
    s.run("layout.columns", &json!({"widths": [[150.0, 24.0], [tw - 174.0, 0.0]], "separator": true})).unwrap();
    let c = sect(&s).columns;
    assert_eq!((c.count, c.separator, c.widths.len()), (2, true, 2));
    assert_eq!(c.widths.first(), Some(&(150.0, 24.0)));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(sect(&s).columns, Columns::default());
    // Too wide, too narrow, too many, or nonsense: refused.
    for bad in [json!([[tw, 36.0], [100.0, 0.0]]), json!([[5.0, 0.0], [100.0, 0.0]]), json!(vec![[40.0, 0.0]; 13]), json!("x")] {
        assert!(s.run("layout.columns", &json!({"widths": bad})).is_err(), "{bad}");
    }
}

/// Apply to: the whole document, or from the caret on (a continuous section break first).
#[test]
fn columns_apply_to_the_document_or_from_the_caret_on() {
    let mut s = doc();
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("layout.columns", &json!({"count": 3, "apply": "document"})).unwrap();
    assert!(s.doc.sections().iter().all(|(_, x)| x.columns.count == 3));
    let mut s = doc();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("layout.columns", &json!({"count": 2, "apply": "forward"})).unwrap();
    let counts: Vec<u32> = s.doc.sections().iter().map(|(_, x)| x.columns.count).collect();
    assert_eq!(counts, vec![1, 2]);
    assert_eq!(s.doc.last_section.start, SectionStart::Continuous);
    assert!(s.run("layout.columns", &json!({"count": 2, "apply": "elsewhere"})).is_err());
}

/// Each section's paragraphs (a section break's own paragraph shows as "") and column count.
fn sections_of(s: &Session) -> Vec<(Vec<String>, u32)> {
    let mut from = 0;
    let mut out = Vec::new();
    for (end, x) in s.doc.sections() {
        let paras = (from..=end).map(|i| s.doc.para_at(&Pos::body(i, 0)).map(|p| p.text.clone()).unwrap_or_default()).collect();
        out.push((paras, x.columns.count));
        from = end + 1;
    }
    out
}

fn select(s: &mut Session, from: (usize, usize), to: (usize, usize)) {
    s.sel = Selection { anchor: Pos::body(from.0, from.1), focus: Pos::body(to.0, to.1) };
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Columns on selected text (#2024), as observed in Word: a continuous section break before
/// and after the selection, the columns on the section between, the rest left alone.
#[test]
fn columns_apply_to_the_selected_text() {
    let five = || Session::new(wordcraft_doc::Document::from_text("p1\np2\nalpha beta gamma\np4\np5"));
    // A whole paragraph (a triple-click takes its mark): each break gets a paragraph of its own.
    let mut s = five();
    select(&mut s, (2, 0), (3, 0));
    let whole = s.selected_text();
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    assert_eq!(sections_of(&s), vec![(strs(&["p1", "p2", ""]), 1), (strs(&["alpha beta gamma", ""]), 2), (strs(&["p4", "p5"]), 1)]);
    assert!(s.doc.sections().iter().skip(1).all(|(_, x)| x.start == SectionStart::Continuous));
    assert_eq!(s.selected_text(), whole, "the paragraph stays selected");
    // The same paragraph again: its section changes, no new breaks.
    select(&mut s, (3, 0), (4, 0));
    s.run("layout.columns", &json!({"count": 3})).unwrap();
    assert_eq!(sections_of(&s).iter().map(|(_, n)| *n).collect::<Vec<_>>(), vec![1, 3, 1]);
    // Taking the section break along changes nothing more.
    select(&mut s, (3, 0), (5, 0));
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    assert_eq!(sections_of(&s).iter().map(|(_, n)| *n).collect::<Vec<_>>(), vec![1, 2, 1]);
    // One undo step takes the breaks and the columns away.
    let mut s = five();
    select(&mut s, (2, 0), (3, 0));
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(sections_of(&s), vec![(strs(&["p1", "p2", "alpha beta gamma", "p4", "p5"]), 1)]);

    // Words inside a paragraph: the breaks split it there.
    let mut s = five();
    select(&mut s, (2, 10), (2, 6));
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    assert_eq!(sections_of(&s), vec![(strs(&["p1", "p2", "alpha "]), 1), (strs(&["beta"]), 2), (strs(&[" gamma", "p4", "p5"]), 1)]);
    assert_eq!(s.selected_text(), "beta");
    assert!(s.sel.anchor > s.sel.focus, "a backward selection stays backward");

    // At the document's start no break goes before it, at its end none after.
    let mut s = five();
    select(&mut s, (0, 0), (1, 0));
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    assert_eq!(sections_of(&s), vec![(strs(&["p1", ""]), 2), (strs(&["p2", "alpha beta gamma", "p4", "p5"]), 1)]);
    let mut s = five();
    select(&mut s, (4, 0), (4, 2));
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    assert_eq!(sections_of(&s), vec![(strs(&["p1", "p2", "alpha beta gamma", "p4", ""]), 1), (strs(&["p5"]), 2)]);
    // Select All: the whole document, no breaks.
    let mut s = five();
    s.run("select.all", &json!({})).unwrap();
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    assert_eq!(sections_of(&s), vec![(strs(&["p1", "p2", "alpha beta gamma", "p4", "p5"]), 2)]);

    // Without a selection the caret's section changes; "selection" then needs one.
    let mut s = five();
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    assert_eq!(sections_of(&s).len(), 1);
    assert!(s.run("layout.columns", &json!({"count": 2, "apply": "selection"})).is_err());
}

/// #407 Line Numbers options: start, count by, distance and restart; options alone keep the
/// restart mode, `none` turns numbering off.
#[test]
fn line_numbering_options() {
    use wordcraft_doc::section::LineNumberRestart;
    let mut s = Session::new(wordcraft_doc::Document::from_text("One\nTwo"));
    s.run("layout.lineNumbers", &json!({"value": "restartSection", "start": 5, "countBy": 2, "distance": 18})).unwrap();
    let l = s.doc.last_section.line_numbers.clone().unwrap();
    assert_eq!((l.start, l.count_by, l.distance, l.restart), (5, 2, 18.0, LineNumberRestart::Section));
    s.run("layout.lineNumbers", &json!({"countBy": 1e9, "start": -3})).unwrap();
    let l = s.doc.last_section.line_numbers.clone().unwrap();
    assert_eq!((l.start, l.count_by, l.restart), (5, 100, LineNumberRestart::Section), "clamped, mode kept");
    s.run("layout.lineNumbers", &json!({"value": "none"})).unwrap();
    assert!(s.doc.last_section.line_numbers.is_none());
    assert!(s.run("layout.lineNumbers", &json!({"value": "sometimes"})).is_err());
}

/// #407 Hyphenation Options: options don't toggle automatic hyphenation; Manual finds a
/// word at a line start that would fit hyphenated on the line above, and an optional hyphen
/// there breaks the word.
#[test]
fn hyphenation_options_and_manual_hyphenation() {
    let mut s =
        Session::new(wordcraft_doc::Document::from_text(&"a extraordinarily be uncharacteristic of responsibilities the considerations ".repeat(12)));
    let r = s.run("layout.hyphenation", &json!({"zone": 36, "caps": false, "limit": 3})).unwrap();
    assert_eq!(r, json!({"value": false, "zone": 36.0, "caps": false, "limit": 3}));
    s.run("layout.hyphenation", &json!({})).unwrap();
    assert!(s.doc.settings.auto_hyphenation, "nothing given toggles");
    s.run("layout.hyphenation", &json!({"value": false})).unwrap();
    // Accept every candidate (which words they are depends on the fonts).
    let mut found = 0;
    let mut from = json!(null);
    while found < 5 {
        let c = s.run("layout.manualHyphenation", &json!({"from": from})).unwrap();
        if c["done"] == true {
            break;
        }
        let word = c["word"].as_str().unwrap().to_string();
        let pts = c["points"].as_array().unwrap();
        assert!(!pts.is_empty() && pts.contains(&c["suggest"]), "{c}");
        let at = c["suggest"].as_u64().unwrap() as usize;
        let mut pos = c["pos"].clone();
        let off = pos["off"].as_u64().unwrap() as usize + word.char_indices().nth(at).unwrap().0;
        pos["off"] = json!(off);
        s.run("caret.set", &json!({"pos": pos})).unwrap();
        s.run("text.optionalHyphen", &json!({})).unwrap();
        pos["off"] = json!(off + word.len());
        from = pos;
        found += 1;
    }
    // Some line of thirty or so starts with a long word that fits hyphenated above, whatever the font.
    assert!(found > 0, "no candidates");
    assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).matches('\u{ad}').count(), found);
    assert!(s.run("layout.manualHyphenation", &json!({"from": {"block": 999, "off": 0}})).unwrap()["done"] == true);
}

/// `layout.pageNumberFormat` sets the section's number format and start; a call without
/// `start` keeps it (the ribbon's old plain call cleared it), and a null `start` numbers on
/// from the section before — the Format Page Numbers dialog's "Continue from previous
/// section" (#1113).
#[test]
fn page_number_format_keeps_or_clears_the_start() {
    use wordcraft_doc::section::NumFormat;
    let mut s = doc();
    s.run("layout.pageNumberFormat", &json!({"format": "upperRoman", "start": 3})).unwrap();
    let x = sect(&s);
    assert_eq!((x.page_num_format, x.page_num_start), (NumFormat::UpperRoman, Some(3)));
    s.run("layout.pageNumberFormat", &json!({"format": "lowerLetter"})).unwrap();
    let x = sect(&s);
    assert_eq!((x.page_num_format, x.page_num_start), (NumFormat::LowerLetter, Some(3)), "a format change keeps the start");
    s.run("layout.pageNumberFormat", &json!({"start": null})).unwrap();
    let x = sect(&s);
    assert_eq!((x.page_num_format, x.page_num_start), (NumFormat::LowerLetter, None), "continue numbers on from the section before");
    // The result carries the section back (scripts read it instead of the dialog).
    let back = s.run("layout.pageNumberFormat", &json!({"format": "decimal", "start": 1})).unwrap();
    assert_eq!(back["pageNumFormat"], "decimal");
    assert_eq!(back["pageNumStart"], 1);
}
