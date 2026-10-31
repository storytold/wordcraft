use super::*;
use crate::Session;

#[test]
fn citations_and_bibliography() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Studios matter "})).unwrap();
    s.run("references.citation", &json!({"source": {"author": "Rivera, Alex; Chen, Mei", "title": "Shared Spaces", "year": "2021", "publisher": "Harbor Books", "city": "Portland"}})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("(Rivera & Chen, 2021)"), "{}", s.doc.plain_text(StoryRef::Body));
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.bibliography", &json!({})).unwrap();
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.contains("Rivera, A., & Chen, M. (2021). Shared Spaces. Harbor Books."), "{t}");
    s.run("references.citationStyle", &json!({"style": "MLA"})).unwrap();
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.contains("(Rivera and Chen)"), "{t}");
    s.run("references.citationStyle", &json!({"style": "IEEE"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("[1]"));
}

/// #4848: a corporate author is cited and listed as written, in every style: never turned into
/// a surname and initials ("Organization, W. H."), never left out for the title. Its tag and its
/// place in an author-sorted list come from the name too.
#[test]
fn a_corporate_author_is_cited_and_listed_as_written() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Hearing loss is common "})).unwrap();
    let who = "World Health Organization";
    s.run("references.citation", &json!({"source": {"kind": "report", "corporate": who, "title": "World report on hearing", "year": "2021", "publisher": "WHO Press", "city": "Geneva"}}))
        .unwrap();
    s.run("text.insert", &json!({"text": " and rising "})).unwrap();
    s.run("references.citation", &json!({"source": {"author": "Adams, Kim", "title": "Quiet Rooms", "year": "2019"}})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.bibliography", &json!({})).unwrap();
    assert_eq!(s.doc.sources[0].tag, "Wor21", "the tag starts with the corporate name");
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.contains(&format!("({who}, 2021)")), "{t}");
    assert!(t.contains(&format!("{who}. (2021). World report on hearing. WHO Press.")), "{t}");
    // Sorted by the corporate name: Adams before World.
    let adams = t.find("Adams, K. (2019)").unwrap_or(usize::MAX);
    let world = t.find(&format!("{who}. (2021)")).unwrap_or(0);
    assert!(adams < world, "{t}");
    for style in [
        "MLA",
        "Chicago",
        "GB/T 7714",
        "GOST (name sort)",
        "GOST (title sort)",
        "ISO 690 (author-date)",
        "ISO 690 (numerical)",
        "Harvard (Anglia)",
        "Turabian",
        "IEEE",
        "SIST02",
    ] {
        s.run("references.citationStyle", &json!({"style": style})).unwrap();
        let t = s.doc.plain_text(StoryRef::Body);
        let entry = t.lines().find(|l| l.contains("World report on hearing")).unwrap_or_default();
        assert!(entry.contains(who), "{style}: {entry}");
        assert!(!entry.contains("Organization, W") && !entry.contains("ORGANIZATION, W"), "{style}: {entry}");
    }
}

/// Updating fields in a document without generated lists leaves its styles alone: a new
/// style would make the next layout start over from nothing.
#[test]
fn update_fields_adds_the_entry_style_only_with_a_list() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("plain text"));
    s.run("select.all", &json!({})).unwrap();
    s.run("references.updateFields", &json!({})).unwrap();
    assert!(s.doc.styles.get(ENTRY_STYLE).is_none());
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.citation", &json!({"source": {"author": "Rivera, Alex", "title": "Shared Spaces", "year": "2021"}})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.bibliography", &json!({})).unwrap();
    assert!(s.doc.styles.get(ENTRY_STYLE).is_some());
}

#[test]
fn bibliography_round_trip_docx() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Studios matter "})).unwrap();
    s.run(
        "references.citation",
        &json!({
            "source": {
                "author": "Rivera, Alex; Chen, Mei",
                "title": "Shared Spaces",
                "year": "2021",
                "publisher": "Harbor Books",
                "city": "Portland",
                "editor": "Johnson, Robert",
                "edition": "2nd",
                "issue": "4",
                "month": "10",
                "day": "15",
                "accessed": "2024-05-15",
                "doi": "10.1000/182"
            }
        }),
    )
    .unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.bibliography", &json!({})).unwrap();

    // Serialize to DOCX and back — sources must survive.
    let bytes = wordcraft_docx::write(&s.doc).expect("docx write");
    let doc = wordcraft_docx::read(&bytes).expect("docx read");
    assert_eq!(doc.sources.len(), 1, "round-tripped sources: {:?}", doc.sources);
    assert_eq!(doc.sources[0].author, "Rivera, Alex; Chen, Mei");
    assert_eq!(doc.sources[0].title, "Shared Spaces");
    assert_eq!(doc.sources[0].editor, "Johnson, Robert");
    assert_eq!(doc.sources[0].edition, "2nd");
    assert_eq!(doc.sources[0].issue, "4");
    assert_eq!(doc.sources[0].month, "10");
    assert_eq!(doc.sources[0].day, "15");
    assert_eq!(doc.sources[0].accessed, "2024-05-15");
    assert_eq!(doc.sources[0].doi, "10.1000/182");

    // Round-tripped document in a new session works with citation styles.
    let mut s2 = Session::new(doc);
    s2.run("references.citationStyle", &json!({"style": "Harvard (Anglia)"})).unwrap();
    let t = s2.doc.plain_text(StoryRef::Body);
    assert!(t.contains("(Rivera and Chen, 2021)"), "{t}");
    assert!(t.contains("Rivera, A. and Chen, M., 2021. Shared Spaces. Portland: Harbor Books."), "{t}");
}

#[test]
fn citation_style_survives_docx_save_and_reopen() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("references.citation", &json!({"source": {"author": "Rivera, Alex", "title": "Shared Spaces", "year": "2021"}})).unwrap();
    s.run("references.citationStyle", &json!({"style": "Harvard (Anglia)"})).unwrap();
    let bytes = wordcraft_docx::write(&s.doc).expect("docx write");
    let doc = wordcraft_docx::read(&bytes).expect("docx read");
    assert_eq!(doc.settings.bib_style.as_deref(), Some("Harvard (Anglia)"));
    assert_eq!(Session::new(doc.clone()).bib_style, "Harvard (Anglia)");
    let mut other = Session::new(wordcraft_doc::Document::new());
    other.set_document(doc);
    assert_eq!(other.bib_style, "Harvard (Anglia)");
    // A document without a recorded style is APA.
    assert_eq!(Session::new(wordcraft_doc::Document::new()).bib_style, "APA");
}

#[test]
fn index_and_figures() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Kilns are hot.\nPresses are heavy.\nMore kilns."));
    s.run("select.text", &json!({"text": "Kilns"})).unwrap();
    s.run("references.markEntry", &json!({})).unwrap();
    s.run("select.text", &json!({"text": "Presses"})).unwrap();
    s.run("references.markEntry", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.index", &json!({})).unwrap();
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.contains("Kilns, 1") && t.contains("Presses, 1"), "{t}");
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("references.caption", &json!({"label": "Figure", "text": "The kiln"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("references.tableOfFigures", &json!({})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Figure 1: The kiln\t1"));
    let list = s.run("insert.crossReference", &json!({"to": "figure"})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    s.run("insert.crossReference", &json!({"to": "figure", "target": "Figure 1", "show": "number"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Figure 1"));
    // The number-only reference marks just "Figure 1" of the caption.
    let (name, _) = s.doc.bookmarks().into_iter().find(|(n, _)| n.starts_with("_Ref")).unwrap();
    assert_eq!(crate::cmd::references::bookmark_text(&s.doc, &name).as_deref(), Some("Figure 1"));
    assert!(fields(&s).contains(&(format!("REF {name} \\h"), "Figure 1".to_string())), "{:?}", fields(&s));
}

/// #2940: a caption with a label of the person's own (Insert Caption › New Label) is a
/// cross-reference type of its own, `label:<Name>`, next to Figure, Table and Equation.
#[test]
fn cross_reference_to_a_custom_caption_label() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Rock\nSee "));
    s.run("select.text", &json!({"text": "Rock"})).unwrap();
    s.run("references.caption", &json!({"label": "Plate", "text": "A rock"})).unwrap();
    assert_eq!(caption_labels(&s), vec!["Plate".to_string()]);
    let list = s.run("insert.crossReference", &json!({"to": "label:Plate"})).unwrap();
    assert_eq!(list, json!(["Plate 1: A rock"]));
    assert!(ref_targets(&s, "label:Figure").is_empty(), "only that label's captions");
    assert!(ref_targets(&s, "label:").is_empty() && ref_targets(&s, "label:Pla").is_empty(), "the whole label");
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.crossReference", &json!({"to": "label:Plate", "target": "Plate 1", "show": "number"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).ends_with("See Plate 1"), "{}", s.doc.plain_text(StoryRef::Body));
    assert!(fields(&s).iter().any(|(i, r)| i.starts_with("REF _Ref") && r == "Plate 1"), "{:?}", fields(&s));
}

/// #2940: the label comes from the caption's structure (the text before its `SEQ` number), so
/// letter and roman numbering, which have no digit, still name it; a multi-word label too.
#[test]
fn caption_labels_do_not_need_a_digit() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Rock\nGem\nSand\nSee "));
    for (text, label, format) in [("Rock", "Plate", "ALPHABETIC"), ("Gem", "Photo Plate", "ROMAN"), ("Sand", "Chart", "ARABIC")] {
        s.run("select.text", &json!({"text": text})).unwrap();
        s.run("references.caption", &json!({"label": label, "text": text, "format": format})).unwrap();
    }
    assert_eq!(caption_labels(&s), ["Plate", "Photo Plate", "Chart"]);
    assert_eq!(ref_targets(&s, "label:Plate").len(), 1);
    assert_eq!(ref_targets(&s, "label:Photo Plate").len(), 1);
    assert_eq!(ref_targets(&s, "label:Chart").len(), 1);
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.crossReference", &json!({"to": "label:Photo Plate", "target": "Photo Plate I", "show": "number"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).ends_with("See Photo Plate I"), "{}", s.doc.plain_text(StoryRef::Body));
}

/// #4411: captions restyled with a style based on Caption, directly (`Caption Table`) or through
/// another (`Caption Table Wide`), are still captions after a DOCX save and reopen: every `tableX`
/// caption is a cross-reference target and in the table of figures, and a Table caption is a
/// Table target. Captions in a style not based on Caption are not, nor are their labels.
#[test]
fn captions_in_a_style_based_on_caption() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("T1\nT2\nT3\nT4\nT5\nNote\nGrid\nStone\nSee "));
    let caption = |s: &mut Session, text: &str, label: &str, title: &str| {
        s.run("select.text", &json!({"text": text})).unwrap();
        s.run("references.caption", &json!({"label": label, "text": title})).unwrap();
    };
    for k in 1..=5 {
        caption(&mut s, &format!("T{k}"), "tableX", &format!("Row {k}"));
    }
    caption(&mut s, "Note", "tableX", "Aside");
    caption(&mut s, "Grid", "Table", "Grid");
    caption(&mut s, "Stone", "Plate", "Stone");
    s.run("styles.create", &json!({"name": "Caption Table", "basedOn": "Caption", "fromSelection": false})).unwrap();
    s.run("styles.create", &json!({"name": "Caption Table Wide", "basedOn": "Caption Table", "fromSelection": false})).unwrap();
    s.run("styles.create", &json!({"name": "Aside", "fromSelection": false})).unwrap();
    let style = |s: &Session, name: &str| s.doc.styles.find(name).map(|st| st.id.clone());
    let (table, wide, aside) = (style(&s, "Caption Table"), style(&s, "Caption Table Wide"), style(&s, "Aside"));
    for path in s.doc.para_paths(StoryRef::Body) {
        let p = s.doc.para_mut(StoryRef::Body, &path).unwrap();
        let text = p.plain_text();
        p.props.style = match text.get(..8).unwrap_or_default() {
            "tableX 1" | "tableX 2" => table.clone(),
            "tableX 3" | "Table 1:" => wide.clone(),
            "tableX 6" | "Plate 1:" => aside.clone(),
            _ => continue,
        };
    }
    let bytes = wordcraft_docx::write(&s.doc).expect("docx write");
    let mut s = Session::new(wordcraft_docx::read(&bytes).expect("docx read"));
    let want: Vec<String> = (1..=5).map(|k| format!("tableX {k}: Row {k}")).collect();
    assert_eq!(s.run("insert.crossReference", &json!({"to": "label:tableX"})).unwrap(), json!(want));
    assert_eq!(s.run("insert.crossReference", &json!({"to": "table"})).unwrap(), json!(["Table 1: Grid"]));
    assert_eq!(caption_labels(&s), ["tableX", "Table"]);
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("references.tableOfFigures", &json!({"label": "tableX", "pageNumbers": false})).unwrap();
    assert_eq!(list_entries(&s).into_iter().map(|(t, _)| t).collect::<Vec<_>>(), want);
}

/// The entries of every generated list, in order, with their last tab stop's leader.
fn list_entries(s: &Session) -> Vec<(String, Option<TabLeader>)> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref() == Some(ENTRY_STYLE))
        .map(|p| (p.plain_text(), p.props.tabs.as_ref().and_then(|t| t.last()).map(|t| t.leader)))
        .collect()
}

/// #3013: a table of figures for any caption label, so a document can have a list of figures
/// and a list of tables; each keeps its own label and leader when updated.
#[test]
fn table_of_figures_for_each_caption_label() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Kiln\nPress\nRock\nTablet\n"));
    for (text, label) in [("Kiln", "Figure"), ("Press", "Table"), ("Rock", "Photo Plate"), ("Tablet", "Figure")] {
        s.run("select.text", &json!({"text": text})).unwrap();
        s.run("references.caption", &json!({"label": label, "text": text})).unwrap();
    }
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("references.tableOfFigures", &json!({"label": "Table", "tabLeader": "hyphen"})).unwrap();
    assert_eq!(list_entries(&s), [("Table 1: Press\t1".to_string(), Some(TabLeader::Hyphen))]);
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.tableOfFigures", &json!({"label": "Photo Plate"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.tableOfFigures", &json!({})).unwrap();
    let instrs: Vec<String> = fields(&s).into_iter().map(|(i, _)| i).filter(|i| i.starts_with("TOC")).collect();
    assert_eq!(instrs, [r#"TOC \c "Table""#, r#"TOC \c "Photo_Plate""#, r#"TOC \c "Figure""#]);
    let want = [
        ("Table 1: Press\t1", TabLeader::Hyphen),
        ("Photo Plate 1: Rock\t1", TabLeader::Dot),
        ("Figure 1: Kiln\t1", TabLeader::Dot),
        ("Figure 2: Tablet\t1", TabLeader::Dot),
    ];
    let want: Vec<(String, Option<TabLeader>)> = want.iter().map(|(t, l)| (t.to_string(), Some(*l))).collect();
    assert_eq!(list_entries(&s), want);
    // Update Table rebuilds each list with its own label and keeps the leader.
    s.run("references.updateFigures", &json!({})).unwrap();
    assert_eq!(list_entries(&s), want);
    // A table of figures is not the table of contents.
    assert!(s.run("references.updateToc", &json!({})).is_ok());
    assert_eq!(list_entries(&s), want);
}

/// #3013: Insert Table of Figures' options as TOC switches: `\a` without the label and number,
/// `\n` without page numbers, `\p " "` with the page right after the text.
#[test]
fn table_of_figures_options() {
    assert_eq!(figures_instr(&json!({})), r#"TOC \c "Figure""#);
    assert_eq!(figures_instr(&json!({"label": "Table", "includeLabel": false})), r#"TOC \a "Table""#);
    assert_eq!(figures_instr(&json!({"pageNumbers": false})), r#"TOC \c "Figure" \n"#);
    assert_eq!(figures_instr(&json!({"rightAlign": false})), r#"TOC \c "Figure" \p " ""#);
    assert_eq!(figures_instr(&json!({"label": "\"\\ "})), r#"TOC \c "Figure""#, "an empty label is Figure");
    for (params, entry) in [
        (json!({"includeLabel": false}), "The kiln\t1"),
        (json!({"pageNumbers": false}), "Figure 1: The kiln"),
        (json!({"rightAlign": false, "includeLabel": false}), "The kiln 1"),
    ] {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Kiln\n"));
        s.run("select.text", &json!({"text": "Kiln"})).unwrap();
        s.run("references.caption", &json!({"label": "Figure", "text": "The kiln"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("references.tableOfFigures", &params).unwrap();
        let entries = list_entries(&s);
        assert_eq!(entries.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), [entry], "{params}");
        assert_eq!(entries.first().and_then(|(_, l)| *l).is_some(), entry.contains('\t'), "a tab stop only for a tab");
    }
}

/// #3013: a table of figures from Word (`TOC \h \z \c "Figure"`, switches in any order) and
/// hand-typed captions are listed (the label in any case); a caption whose label only starts
/// with another's is not.
#[test]
fn table_of_figures_from_word_and_typed_captions() {
    let mut d = wordcraft_doc::Document::from_text("Figure 1: Typed\nFigures 2: Not a figure\nFigure 3 \u{2014} Dashed\nfigure 4: Lower\nList");
    for path in d.para_paths(StoryRef::Body).into_iter().take(4) {
        d.para_mut(StoryRef::Body, &path).unwrap().props.style = Some("Caption".into());
    }
    let mut s = Session::new(d);
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    let at = s.sel.focus.clone();
    let field = InlineObject::Field {
        instr: r#"TOC \h \z \a "Figure""#.into(),
        result: String::new(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    s.doc.insert_object(&at, field, &CharProps::default()).unwrap();
    s.run("references.updateFigures", &json!({})).unwrap();
    let entries: Vec<String> = list_entries(&s).into_iter().map(|(t, _)| t).collect();
    assert_eq!(entries, ["Typed\t1", "Dashed\t1", "Lower\t1"]);
}

/// #3013 review: the leader asked for lands on the new list even when a list above it is out
/// of date and gains entries (moving the new one down) while every list is rebuilt.
#[test]
fn table_of_figures_leader_survives_a_stale_list_above() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Kiln\nPress\nRock\n"));
    for (text, label) in [("Kiln", "Figure"), ("Press", "Table")] {
        s.run("select.text", &json!({"text": text})).unwrap();
        s.run("references.caption", &json!({"label": label, "text": text})).unwrap();
    }
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("references.tableOfFigures", &json!({})).unwrap();
    s.run("select.text", &json!({"text": "Rock"})).unwrap();
    s.run("references.caption", &json!({"label": "Figure", "text": "Rock"})).unwrap();
    assert_eq!(list_entries(&s).len(), 1, "the list of figures is out of date");
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.tableOfFigures", &json!({"label": "Table", "tabLeader": "hyphen"})).unwrap();
    let want = [("Figure 1: Kiln\t1", TabLeader::Dot), ("Figure 2: Rock\t1", TabLeader::Dot), ("Table 1: Press\t1", TabLeader::Hyphen)];
    let want: Vec<(String, Option<TabLeader>)> = want.iter().map(|(t, l)| (t.to_string(), Some(*l))).collect();
    assert_eq!(list_entries(&s), want);
}

/// #3013 review: a table of figures from Word whose saved list sits in the field's result (one
/// paragraph) is replaced by the rebuilt entries, not listed twice, and keeps its leader.
#[test]
fn table_of_figures_from_word_replaces_its_saved_list() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Kiln\nList"));
    s.run("select.text", &json!({"text": "Kiln"})).unwrap();
    s.run("references.caption", &json!({"label": "Figure", "text": "Kiln"})).unwrap();
    let last = s.doc.body.len() - 1;
    let p = s.doc.para_mut(StoryRef::Body, &Path::top(last)).unwrap();
    p.props.style = Some("TableofFigures".into());
    p.props.tabs = Some(vec![TabStop { pos: 400.0, align: TabAlign::Right, leader: TabLeader::Hyphen }]);
    let end = p.len();
    p.delete(0, end).unwrap();
    let field = InlineObject::Field {
        instr: r#"TOC \h \z \c "Figure""#.into(),
        result: "Figure 1: Kiln\t1".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    p.insert_object(0, field, &CharProps::default()).unwrap();
    for _ in 0..2 {
        s.run("references.updateFigures", &json!({})).unwrap();
        let text = s.doc.plain_text(StoryRef::Body);
        assert_eq!(text.matches("Figure 1: Kiln\t1").count(), 1, "{text:?}");
        assert_eq!(list_entries(&s), [("Figure 1: Kiln\t1".to_string(), Some(TabLeader::Hyphen))]);
    }
}

/// #3013 review: `\c` picks captions by their SEQ identifier, as Word does, not by the text
/// before the number (`STYLEREF`, " Specimen ", `SEQ Figure` is a figure).
#[test]
fn table_of_figures_goes_by_the_seq_identifier() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Rock\n"));
    let field = |instr: &str| InlineObject::Field {
        instr: instr.into(),
        result: String::new(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let plain = CharProps::default();
    let mut imported = Paragraph::new().styled("Caption");
    imported.insert_object(0, field("STYLEREF 1 \\s"), &plain).unwrap();
    imported.insert_text(imported.len(), " Specimen ", &plain).unwrap();
    imported.insert_object(imported.len(), field("SEQ Figure \\* ARABIC"), &plain).unwrap();
    imported.insert_text(imported.len(), ": A rock", &plain).unwrap();
    s.doc.insert_block(StoryRef::Body, &Path::top(1), Block::Para(imported)).unwrap();
    for (label, listed) in [("Figure", true), ("Specimen", false)] {
        let mut s = Session::new(s.doc.clone());
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("references.tableOfFigures", &json!({"label": label, "includeLabel": false})).unwrap();
        let entries: Vec<String> = list_entries(&s).into_iter().map(|(t, _)| t).collect();
        assert_eq!(entries.len(), usize::from(listed), "{label}: {entries:?}");
        if listed {
            assert_eq!(entries.first().map(|t| t.split('\t').next()), Some(Some("A rock")));
        }
    }
}

/// #2940: a caption may start with its chapter number (`STYLEREF`) and carry its label text
/// between that and the `SEQ` number; the text is the label, not the `SEQ` identifier.
#[test]
fn caption_label_after_a_chapter_number_field() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Rock\nSee "));
    let field = |instr: &str| InlineObject::Field {
        instr: instr.into(),
        result: String::new(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let plain = CharProps::default();
    // STYLEREF, " Specimen ", SEQ Figure, ": A rock"
    let mut imported = Paragraph::new().styled("Caption");
    imported.insert_object(0, field("STYLEREF 1 \\s"), &plain).unwrap();
    imported.insert_text(imported.len(), " Specimen ", &plain).unwrap();
    imported.insert_object(imported.len(), field("SEQ Figure \\* ARABIC"), &plain).unwrap();
    imported.insert_text(imported.len(), ": A rock", &plain).unwrap();
    s.doc.insert_block(StoryRef::Body, &Path::top(1), Block::Para(imported)).unwrap();
    // A generated caption with a chapter number: label, STYLEREF, separator, SEQ.
    s.run("select.text", &json!({"text": "Rock"})).unwrap();
    s.run("references.caption", &json!({"label": "Plate", "chapter": 1, "separator": "-"})).unwrap();
    // No label text at all: the SEQ identifier names it, after the chapter number's separator.
    let mut bare = Paragraph::new().styled("Caption");
    bare.insert_object(0, field("STYLEREF 1 \\s"), &plain).unwrap();
    bare.insert_text(bare.len(), "-", &plain).unwrap();
    bare.insert_object(bare.len(), field("SEQ Photo_Plate"), &plain).unwrap();
    let end = s.doc.body.len();
    s.doc.insert_block(StoryRef::Body, &Path::top(end), Block::Para(bare)).unwrap();
    let labels = caption_labels(&s);
    assert!(labels.contains(&"Specimen".to_string()), "{labels:?}");
    assert!(labels.contains(&"Plate".to_string()), "{labels:?}");
    assert!(labels.contains(&"Photo Plate".to_string()), "{labels:?}");
    assert!(!labels.iter().any(|l| l == "-" || l.is_empty() || l == "Figure"), "{labels:?}");
}

/// #661: References › Table of Authorities › Update Table picks up citations marked after the table.
#[test]
fn table_of_authorities_updates() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Smith v. Jones held it.\nDoe v. Roe agreed."));
    s.run("select.text", &json!({"text": "Smith v. Jones"})).unwrap();
    s.run("references.markCitation", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.tableOfAuthorities", &json!({})).unwrap();
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.contains("Smith v. Jones") && !t.contains("Doe v. Roe, "), "{t}");
    s.run("select.text", &json!({"text": "Doe v. Roe"})).unwrap();
    s.run("references.markCitation", &json!({})).unwrap();
    s.run("references.updateAuthorities", &json!({})).unwrap();
    let after = s.doc.plain_text(StoryRef::Body);
    assert!(after.matches("Doe v. Roe").count() > t.matches("Doe v. Roe").count(), "{after}");
}

/// Field instructions and results in the body, in order.
fn fields(s: &Session) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        for o in &s.doc.para(StoryRef::Body, &path).unwrap().objects {
            if let InlineObject::Field { instr, result, .. } = o {
                out.push((instr.clone(), result.clone()));
            }
        }
    }
    out
}

#[test]
fn cross_references_point_at_real_bookmarks() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Kilns\nSee here."));
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.crossReference", &json!({"to": "heading", "target": "Kilns"})).unwrap();
    // The heading gets a hidden bookmark, and the REF field names it (not a literal `_Ref`).
    let marks = s.doc.bookmarks();
    assert_eq!(marks.len(), 1, "{marks:?}");
    let name = marks[0].0.clone();
    assert!(name.starts_with("_Ref") && name.len() > 4, "{name}");
    assert_eq!(marks[0].1.path, s.doc.para_paths(StoryRef::Body)[0]);
    assert_eq!(fields(&s), vec![(format!("REF {name} \\h"), "Kilns".to_string())]);
    // A second reference to the same heading reuses the bookmark.
    s.run("insert.crossReference", &json!({"to": "heading", "target": "Kilns", "show": "page"})).unwrap();
    assert_eq!(s.doc.bookmarks().len(), 1);
    assert_eq!(fields(&s)[1], (format!("PAGEREF {name} \\h"), "1".to_string()));
    // Update Field follows the heading's new text.
    s.run("select.text", &json!({"text": "ilns"})).unwrap();
    s.run("text.insert", &json!({"text": "ILNS"})).unwrap();
    s.run("references.updateFields", &json!({})).unwrap();
    assert_eq!(fields(&s)[0].1, "KILNS");
    assert!(s.doc.plain_text(StoryRef::Body).ends_with("See here.KILNS1"), "{}", s.doc.plain_text(StoryRef::Body));
}

#[test]
fn cross_reference_to_a_bookmark_shows_its_text() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Kilns are hot.\nSee "));
    s.run("select.text", &json!({"text": "Kilns"})).unwrap();
    s.run("insert.bookmark", &json!({"name": "Spot"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.crossReference", &json!({"to": "bookmark", "target": "Spot"})).unwrap();
    assert_eq!(s.doc.bookmarks().len(), 1, "no extra bookmark for a named one");
    assert_eq!(fields(&s), vec![("REF Spot \\h".to_string(), "Kilns".to_string())]);
}

#[test]
fn cross_reference_inside_its_own_target_paragraph() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Kilns"));
    s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.crossReference", &json!({"to": "heading", "target": "Kilns"})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "KilnsKilns");
    // The field sits after the bookmark, so it doesn't mark itself.
    let name = s.doc.bookmarks()[0].0.clone();
    assert_eq!(crate::cmd::references::bookmark_text(&s.doc, &name).as_deref(), Some("Kilns"));
}

fn src_of(author: &str, year: &str) -> Value {
    json!({"author": author, "year": year, "title": "T"})
}

#[test]
fn editing_author_and_year_keeps_the_tag_and_updates_the_citation_text() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("references.sources", &json!({"add": src_of("Smith, Ann", "2020")})).unwrap();
    assert_eq!(s.doc.sources[0].tag, "Smi20");
    s.run("references.citation", &json!({"tag": "Smi20"})).unwrap();
    // What the Edit Source dialog sends: the unchanged tag with the new author and year.
    let mut edited = src_of("Jones, Bo", "2021");
    edited["tag"] = json!("Smi20");
    s.run("references.sources", &json!({"add": edited})).unwrap();
    assert_eq!(s.doc.sources.len(), 1);
    assert_eq!(s.doc.sources[0].tag, "Smi20");
    let f = fields(&s);
    assert_eq!(f[0].0, "CITATION Smi20");
    assert!(f[0].1.contains("Jones") && f[0].1.contains("2021"), "{:?}", f[0].1);
}

#[test]
fn a_new_sources_tag_is_three_letters_and_two_year_digits() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    for (a, y) in [
        ("Kim", "2000"),
        ("Kim", "2000"),
        ("Kim", "2000"),
        ("Lee", "2019"),
        ("Li", "1999"),
        ("", ""),
        ("", ""),
        ("Ünal, Ece", "7"),
        ("O'Neil", "20x5"),
    ] {
        s.run("references.sources", &json!({"add": src_of(a, y)})).unwrap();
    }
    let tags: Vec<_> = s.doc.sources.iter().map(|x| x.tag.as_str()).collect();
    assert_eq!(tags, ["Kim00", "Kim001", "Kim002", "Lee19", "Li99", "Src", "Src1", "Üna7", "ONe05"]);
    let mut s = Session::new(wordcraft_doc::Document::new());
    let mut a = src_of("Lee", "2019");
    a["tag"] = json!("MyTag");
    s.run("references.sources", &json!({"add": a})).unwrap();
    assert_eq!(s.doc.sources[0].tag, "MyTag");
}

fn two_sources(s: &mut Session, a_title: &str) {
    let a =
        json!({"tag": "A", "author": "Rivera, Alex; Chen, Mei", "title": a_title, "year": "2021", "publisher": "Harbor Books", "city": "Portland"});
    let b = json!({"tag": "B", "author": "Okafor, Ngozi", "title": "Kilns and Clay", "year": "2019", "journal": "Studio Quarterly", "volume": "4", "pages": "10-20"});
    s.run("references.sources", &json!({"add": a})).unwrap();
    s.run("references.sources", &json!({"add": b})).unwrap();
}

#[test]
fn author_date_styles_cite_and_list() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    two_sources(&mut s, "Shared Spaces");
    s.run("references.citation", &json!({"tag": "A", "pages": "12"})).unwrap();
    s.run("references.citation", &json!({"tag": "B"})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.bibliography", &json!({})).unwrap();
    let cases = [
        (
            "Harvard (Anglia)",
            "(Rivera and Chen, 2021, p. 12)(Okafor, 2019)",
            "Okafor, N., 2019. Kilns and Clay. Studio Quarterly, 4, pp. 10-20.\nRivera, A. and Chen, M., 2021. Shared Spaces. Portland: Harbor Books.",
        ),
        (
            "ISO 690 - First Element and Date",
            "(Rivera and Chen 2021, p. 12)(Okafor 2019)",
            "OKAFOR, Ngozi, 2019. Kilns and Clay. Studio Quarterly, 4, 10-20.\nRIVERA, Alex and CHEN, Mei, 2021. Shared Spaces. Portland: Harbor Books.",
        ),
        (
            "turabian",
            "(Rivera and Chen 2021, 12)(Okafor 2019)",
            "Okafor, Ngozi. “Kilns and Clay.” Studio Quarterly 4 (2019): 10-20.\nRivera, Alex, and Mei Chen. Shared Spaces. Portland: Harbor Books, 2021.",
        ),
    ];
    for (style, cites, list) in cases {
        s.run("references.citationStyle", &json!({"style": style})).unwrap();
        let t = s.doc.plain_text(StoryRef::Body);
        assert!(t.starts_with(cites) && t.contains(list), "{style}: {t}");
    }
    assert!(s.run("references.citationStyle", &json!({"style": "Vancouver"})).unwrap_err().to_string().contains("Turabian"));
    assert_eq!(s.bib_style, "Turabian", "an unknown style keeps the current one");
}

/// #2640: Manage Sources › Import reads a Word bibliography file (`b:Sources`: Word's own
/// `Sources.xml`) into the document. An untagged source gets a tag like a new one; a tag
/// already here is left alone; the whole import is one undo step; and a file that isn't
/// `b:Sources` is an error, not a silent success.
#[test]
fn import_sources_reads_a_word_bibliography_file() {
    let dir = std::env::temp_dir().join(format!("wc-import-sources-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Sources.xml");
    let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"
           xmlns="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"
           SelectedStyle="\APA.XSL" StyleName="APA">
  <b:Source><b:Tag>Riv21</b:Tag><b:SourceType>Book</b:SourceType>
    <b:Author><b:Author><b:NameList><b:Person><b:Last>Rivera</b:Last><b:First>Alex</b:First></b:Person></b:NameList></b:Author></b:Author>
    <b:Title>Shared Spaces</b:Title><b:Year>2021</b:Year><b:City>Portland</b:City><b:Publisher>Harbor Books</b:Publisher>
  </b:Source>
  <b:Source><b:SourceType>Book</b:SourceType></b:Source>
  <b:Source><b:SourceType>JournalArticle</b:SourceType>
    <b:Author><b:Author><b:NameList><b:Person><b:Last>Okafor</b:Last><b:First>Ngozi</b:First></b:Person></b:NameList></b:Author></b:Author>
    <b:Title>Kilns and Clay</b:Title><b:JournalName>Studio Quarterly</b:JournalName><b:Year>2019</b:Year>
  </b:Source>
</b:Sources>"#;
    std::fs::write(&path, xml).unwrap();
    let path = path.to_string_lossy().to_string();

    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("references.sources", &json!({"import": path})).unwrap();
    assert_eq!(s.doc.sources.len(), 2, "the source with no content is dropped");
    assert_eq!(s.doc.sources[0].tag, "Riv21");
    assert_eq!((s.doc.sources[0].author.as_str(), s.doc.sources[0].title.as_str()), ("Rivera, Alex", "Shared Spaces"));
    assert_eq!((s.doc.sources[1].tag.as_str(), s.doc.sources[1].kind.as_str()), ("Oka19", "article"), "an untagged source gets a tag like a new one");
    assert!(s.dirty, "the document is marked edited");

    // The import is one undo step, and importing again adds nothing.
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.doc.sources.is_empty(), "one undo takes the whole import back");
    s.run("references.sources", &json!({"import": path})).unwrap();
    s.doc.sources[0].title = "Shared Spaces, revised".into();
    s.run("references.sources", &json!({"import": path})).unwrap();
    assert_eq!(s.doc.sources.len(), 2, "a tag already here is not added again");
    assert_eq!(s.doc.sources[0].title, "Shared Spaces, revised", "an import never overwrites an edited source");
    assert_eq!(s.doc.sources[1].tag, "Oka19", "the untagged source keeps the tag it was given");

    // Anything that isn't b:Sources is refused.
    let bad = dir.join("not-sources.xml");
    std::fs::write(&bad, "<html><body>no</body></html>").unwrap();
    let e = s.run("references.sources", &json!({"import": bad.to_string_lossy()})).unwrap_err();
    assert!(e.to_string().contains("not a Word document"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #2640: untagged sources that share a tag stem get the tag, then `1`, `2`… in import order,
/// and a second import adds none of them (the untagged one matches by title).
#[test]
fn an_import_tags_each_untagged_source_once() {
    let dir = std::env::temp_dir().join(format!("wc-import-tags-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Sources.xml");
    std::fs::write(
        &path,
        r#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
  <b:Source><b:SourceType>Book</b:SourceType><b:Title>First Work</b:Title><b:Year>2021</b:Year>
    <b:Author><b:Author><b:NameList><b:Person><b:Last>Rivera</b:Last><b:First>Alex</b:First></b:Person></b:NameList></b:Author></b:Author></b:Source>
  <b:Source><b:SourceType>Book</b:SourceType><b:Title>Second Work</b:Title><b:Year>2021</b:Year>
    <b:Author><b:Author><b:NameList><b:Person><b:Last>Rivera</b:Last><b:First>Alex</b:First></b:Person></b:NameList></b:Author></b:Author></b:Source>
  <b:Source><b:SourceType>Book</b:SourceType><b:Title>Third Work</b:Title></b:Source>
</b:Sources>"#,
    )
    .unwrap();
    let path = path.to_string_lossy().to_string();
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("references.sources", &json!({"import": path})).unwrap();
    let tags: Vec<_> = s.doc.sources.iter().map(|x| x.tag.as_str()).collect();
    assert_eq!(tags, ["Riv21", "Riv211", "Src"], "{tags:?}");
    s.run("references.sources", &json!({"import": path})).unwrap();
    assert_eq!(s.doc.sources.len(), 3, "the tags are here and the untagged one matches by title");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn numbered_styles_count_in_order_of_first_citation() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    two_sources(&mut s, "Shared Spaces");
    s.run("references.citationStyle", &json!({"style": "ISO 690 (numerical)"})).unwrap();
    s.run("text.insert", &json!({"text": "Later "})).unwrap();
    s.run("references.citation", &json!({"tag": "A", "pages": "12"})).unwrap();
    assert_eq!(fields(&s)[0].1, "(1, p. 12)");
    // Citing B earlier in the text makes it number 1 and renumbers A.
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("references.citation", &json!({"tag": "B"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.bibliography", &json!({})).unwrap();
    let cases = [
        (
            "ISO 690 (numerical)",
            "(1)Later (2, p. 12)",
            "1. OKAFOR, Ngozi. Kilns and Clay. Studio Quarterly. 2019, 4, 10-20.\n2. RIVERA, Alex and CHEN, Mei. Shared Spaces. Portland: Harbor Books, 2021.",
        ),
        (
            "GB/T 7714",
            "[1]Later [2, 12]",
            "[1] OKAFOR N. Kilns and Clay[J]. Studio Quarterly, 2019, 4: 10-20.\n[2] RIVERA A, CHEN M. Shared Spaces[M]. Portland: Harbor Books, 2021.",
        ),
        (
            "SIST02",
            "(1)Later (2, p. 12)",
            "1) Okafor, Ngozi. Kilns and Clay. Studio Quarterly. 2019, vol. 4, p. 10-20.\n2) Rivera, Alex; Chen, Mei. Shared Spaces. Portland, Harbor Books, 2021.",
        ),
        (
            "IEEE",
            "[1]Later [2]",
            "[1] N. Okafor, “Kilns and Clay,” Studio Quarterly, 4, 10-20, 2019.\n[2] A. Rivera, M. Chen, “Shared Spaces,” Portland: Harbor Books, 2021.",
        ),
    ];
    for (style, cites, list) in cases {
        s.run("references.citationStyle", &json!({"style": style})).unwrap();
        let t = s.doc.plain_text(StoryRef::Body);
        assert!(t.starts_with(cites) && t.contains(list), "{style}: {t}");
    }
}

#[test]
fn gost_numbers_follow_its_name_or_title_sort() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    two_sources(&mut s, "Atlas of Studios");
    s.run("references.citation", &json!({"tag": "A"})).unwrap();
    s.run("references.citation", &json!({"tag": "B", "pages": "15"})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.bibliography", &json!({})).unwrap();
    let okafor = "Okafor N. Kilns and Clay // Studio Quarterly. – 2019. – Vol. 4. – P. 10-20.";
    let rivera = "Rivera A., Chen M. Atlas of Studios. – Portland : Harbor Books, 2021.";
    // Word's style and style-sheet names pick the same styles.
    s.run("references.citationStyle", &json!({"style": "\\GostName.XSL"})).unwrap();
    assert_eq!(s.bib_style, "GOST (name sort)");
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.starts_with("[2][1, p. 15]") && t.contains(&format!("1. {okafor}\n2. {rivera}")), "{t}");
    s.run("references.citationStyle", &json!({"style": "GOST - Title Sort"})).unwrap();
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.starts_with("[1][2, p. 15]") && t.contains(&format!("1. {rivera}\n2. {okafor}")), "{t}");
    assert_eq!(find_style("ISO 690 - Numerical Reference").map(|x| x.name), Some("ISO 690 (numerical)"));
    assert_eq!(find_style("Harvard - Anglia 2008").map(|x| x.name), Some("Harvard (Anglia)"));
    assert!(find_style("").is_none() && find_style("Vancouver").is_none());
}
