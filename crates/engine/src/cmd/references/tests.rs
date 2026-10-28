use super::*;

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

/// #775: body STYLEREF fields show the nearest paragraph of the style before them.
#[test]
fn body_styleref_finds_the_heading_before_it() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Intro\nChapter One\ntext here\nChapter Two\nend"));
    for t in ["Chapter One", "Chapter Two"] {
        s.run("select.text", &json!({"text": t})).unwrap();
        s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
    }
    s.run("select.text", &json!({"text": "text here"})).unwrap();
    s.run("caret.end", &json!({})).unwrap();
    s.run("insert.field", &json!({"instr": r#"STYLEREF "Heading 1" \* MERGEFORMAT"#})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.field", &json!({"instr": "STYLEREF 1 \\p"})).unwrap();
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("insert.field", &json!({"instr": "STYLEREF \"Heading 1\" \\p"})).unwrap();
    s.run("insert.field", &json!({"instr": "STYLEREF \"No Such Style\""})).unwrap();
    let results: Vec<String> = fields(&s).into_iter().map(|f| f.1).collect();
    assert_eq!(results, ["below", wordcraft_layout::styleref::NO_TEXT, "Chapter One", "above"]);
}

/// #775: a template's running header `STYLEREF "heading 1"` (a complex field, opened from
/// DOCX) shows each page's chapter, and keeps its code through a save.
#[test]
fn docx_styleref_header_shows_each_page_s_chapter() {
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let heading = |t: &str, brk: bool| {
        let b = if brk { "<w:pageBreakBefore/>" } else { "" };
        format!(r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/>{b}</w:pPr><w:r><w:t>{t}</w:t></w:r></w:p><w:p><w:r><w:t>Body of {t}.</w:t></w:r></w:p>"#)
    };
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document {ns}><w:body>{}{}{}<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr></w:body></w:document>"#,
        heading("Rivers", false),
        heading("Mountains", true),
        heading("Deserts", true)
    );
    let header = format!(
        r#"<w:hdr {ns}><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> STYLEREF  "heading 1" \* MERGEFORMAT </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Cached</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:hdr>"#
    );
    let styles = format!(
        r#"<w:styles {ns}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style></w:styles>"#
    );
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in [
        ("_rels/.rels", root),
        ("word/document.xml", body.as_str()),
        ("word/_rels/document.xml.rels", rels),
        ("word/header1.xml", header.as_str()),
        ("word/styles.xml", styles.as_str()),
    ] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zw, text.as_bytes()).unwrap();
    }
    let bytes = zw.finish().unwrap().into_inner();
    let headers = |s: &mut Session| -> Vec<String> {
        s.layout()
            .pages
            .iter()
            .map(|p| {
                p.header
                    .iter()
                    .filter_map(|it| {
                        if let wordcraft_layout::Placed::Lines { para, .. } = it {
                            Some(para.shown.iter().map(|x| x.1.clone()).collect::<String>())
                        } else {
                            None
                        }
                    })
                    .collect::<String>()
            })
            .collect()
    };
    let mut s = Session::new(crate::io::open_bytes("chapters.docx", &bytes).unwrap());
    assert_eq!(headers(&mut s), ["Rivers", "Mountains", "Deserts"]);
    let saved = crate::io::save_bytes("chapters.docx", &s.doc).unwrap();
    let mut again = Session::new(crate::io::open_bytes("chapters.docx", &saved).unwrap());
    assert_eq!(headers(&mut again), ["Rivers", "Mountains", "Deserts"]);
    let hid = again.doc.last_section.headers.default.unwrap();
    let instr = again.doc.para_paths(StoryRef::Part(hid)).iter().find_map(|p| {
        again.doc.para(StoryRef::Part(hid), p).unwrap().objects.iter().find_map(|o| match o {
            InlineObject::Field { instr, .. } => Some(instr.clone()),
            _ => None,
        })
    });
    assert!(instr.is_some_and(|i| i.contains(r#"STYLEREF  "heading 1""#)));
}

/// #402: Custom Table of Contents settings become TOC switches, and updating honours them:
/// levels (`\o`), a style with a level (`\t`), no hyperlinks, page numbers after a space
/// (`\p`), or none at all (`\n`).
#[test]
fn toc_switches_shape_the_entries() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Intro\nPart\nDeep\nNote"));
    for (text, style) in [("Intro", "Heading 1"), ("Part", "Heading 2"), ("Deep", "Heading 3"), ("Note", "Quote")] {
        s.run("select.text", &json!({"text": text})).unwrap();
        s.run("styles.apply", &json!({"style": style})).unwrap();
    }
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("references.toc", &json!({"levels": 2, "hyperlinks": false, "rightAlign": false, "styleLevels": {"Quote": 2}})).unwrap();
    assert_eq!(fields(&s)[0].0, "TOC \\o \"1-2\" \\z \\u \\p \" \" \\t \"Quote,2\"");
    let entries: Vec<(String, String)> = s
        .doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
        .map(|p| (p.props.style.clone().unwrap(), p.plain_text()))
        .collect();
    assert_eq!(entries, [("TOC1", "Intro 1"), ("TOC2", "Part 1"), ("TOC2", "Note 1")].map(|(a, b)| (a.to_string(), b.to_string())));
    assert!(s.doc.bookmarks().is_empty(), "no hyperlinks, no _Toc bookmarks");
    // Settings again replace the table in place; hyperlinks bookmark the headings.
    s.run("references.toc", &json!({"levels": 3, "pageNumbers": false})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body).matches("Contents").count(), 1);
    let toc: Vec<String> =
        s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some("TOC3")).map(|p| p.plain_text()).collect();
    assert_eq!(toc, ["Deep"]);
    let first = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.props.style.as_deref() == Some("TOC1")).unwrap();
    let link = first.runs[0].props.link.clone().unwrap();
    assert!(s.doc.bookmarks().iter().any(|(n, _)| format!("#{n}") == link), "{link}");
}

/// #402: Caption numbering — a custom label above the paragraph, letters, and chapter
/// numbers that restart after each Heading 1 (`STYLEREF 1 \s`, `SEQ … \s 1`).
#[test]
fn caption_numbering_with_chapters() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("One\nPic\nPic two\nTwo\nPic three"));
    for t in ["One", "Two"] {
        s.run("select.text", &json!({"text": t})).unwrap();
        s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
    }
    let opts = json!({"label": "Photo Plate", "format": "upperLetter", "chapter": 1, "separator": "."});
    for t in ["Pic three", "Pic two", "Pic"] {
        s.run("select.text", &json!({"text": t})).unwrap();
        let mut v = opts.clone();
        v["position"] = json!("above");
        v["text"] = json!(t);
        s.run("references.caption", &v).unwrap();
    }
    let caps: Vec<String> =
        s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some("Caption")).map(|p| p.plain_text()).collect();
    assert_eq!(caps, ["Photo Plate 1.A: Pic", "Photo Plate 1.B: Pic two", "Photo Plate 2.A: Pic three"]);
    assert!(fields(&s).contains(&("SEQ Photo_Plate \\* ALPHABETIC \\s 1".to_string(), "A".to_string())), "{:?}", fields(&s));
    // The caption sits above its paragraph; excluding the label leaves the number.
    let i = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.plain_text() == "Pic")).unwrap();
    assert_eq!(s.doc.body[i - 1].as_para().unwrap().plain_text(), "Photo Plate 1.A: Pic");
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("references.caption", &json!({"label": "Table", "excludeLabel": true, "format": "ROMAN"})).unwrap();
    assert_eq!(s.doc.body.last().unwrap().as_para().unwrap().plain_text(), "I");
}

#[test]
fn update_fields_ref_and_pageref() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Bookmark target text here.\nSecond paragraph."));
    s.run("select.text", &json!({"text": "Bookmark target text"})).unwrap();
    s.run("insert.bookmark", &json!({"name": "TargetMark"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.field", &json!({"instr": "REF \"TargetMark\" \\h", "result": "old"})).unwrap();
    s.run("insert.field", &json!({"instr": "REF TargetMark\\h", "result": "old"})).unwrap();
    s.run("insert.field", &json!({"instr": "PAGEREF \"TargetMark\" \\* Roman", "result": "0"})).unwrap();
    s.run("insert.field", &json!({"instr": "PAGEREF TargetMark \\p", "result": "above"})).unwrap();
    s.run("insert.field", &json!({"instr": "REF TargetMark \\p", "result": "above"})).unwrap();

    update_fields(&mut s).unwrap();

    let res: Vec<(String, String)> = fields(&s);
    assert_eq!(res[0], ("REF \"TargetMark\" \\h".into(), "Bookmark target text".into()));
    assert_eq!(res[1], ("REF TargetMark\\h".into(), "Bookmark target text".into()));
    assert_eq!(res[2], ("PAGEREF \"TargetMark\" \\* Roman".into(), "I".into()));
    assert_eq!(res[3], ("PAGEREF TargetMark \\p".into(), "above".into()));
    assert_eq!(res[4], ("REF TargetMark \\p".into(), "above".into()));
}

/// A document with a body AUTHOR field and a header holding AUTHOR, NUMWORDS and a locked TITLE,
/// all stale; the author is "Ada".
fn doc_with_header_fields() -> (wordcraft_doc::Document, u32) {
    use wordcraft_doc::{CharProps, Paragraph, PartKind};
    let field = |instr: &str, locked: bool| InlineObject::Field {
        instr: instr.into(),
        result: "stale".into(),
        locked,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let mut doc = wordcraft_doc::Document::from_text("one two three");
    doc.core.creator = "Ada".into();
    let mut p = Paragraph::new();
    for (instr, locked) in [("AUTHOR", false), ("NUMWORDS", false), ("TITLE", true)] {
        let off = p.len();
        p.insert_object(off, field(instr, locked), &CharProps::default()).unwrap();
    }
    let body_path = doc.para_paths(StoryRef::Body).remove(0);
    doc.para_mut(StoryRef::Body, &body_path).unwrap().insert_object(0, field("AUTHOR", false), &CharProps::default()).unwrap();
    let hdr = doc.add_part(PartKind::Header, vec![std::sync::Arc::new(wordcraft_doc::Block::Para(p))]);
    (doc, hdr)
}

fn story_fields(s: &Session, story: StoryRef) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(story) {
        for o in &s.doc.para(story, &path).unwrap().objects {
            if let InlineObject::Field { instr, result, .. } = o {
                out.push((instr.clone(), result.clone()));
            }
        }
    }
    out
}

#[test]
fn update_fields_filename() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.path = Some(std::path::PathBuf::from("/docs/my_thesis.docx"));
    s.run("insert.field", &serde_json::json!({"instr": "FILENAME"})).unwrap();
    let fields = story_fields(&s, StoryRef::Body);
    assert_eq!(fields[0], ("FILENAME".into(), "my_thesis.docx".into()));

    s.run("insert.field", &serde_json::json!({"instr": r"FILENAME \p"})).unwrap();
    let fields = story_fields(&s, StoryRef::Body);
    assert_eq!(fields[1], (r"FILENAME \p".into(), "/docs/my_thesis.docx".into()));
}

#[test]
fn update_fields_filename_titled_unsaved() {
    let mut doc = wordcraft_doc::Document::new();
    doc.core.title = "Important Document".into();
    let mut s = Session::new(doc);
    s.run("insert.field", &serde_json::json!({"instr": "FILENAME"})).unwrap();
    let fields = story_fields(&s, StoryRef::Body);
    assert_eq!(fields[0], ("FILENAME".into(), "Document1".into()));

    let l = s.layout();
    let text: String = l.pages[0]
        .items
        .iter()
        .filter_map(|it| {
            if let wordcraft_layout::Placed::Lines { para, .. } = it {
                Some(para.shown.iter().map(|x| x.1.clone()).collect::<String>())
            } else {
                None
            }
        })
        .collect();
    assert!(text.contains("Document1"), "live layout renders Document1: {text}");
    assert!(!text.contains("Important Document"), "live layout must not render title: {text}");

    s.run("references.updateFields", &serde_json::json!({})).unwrap();
    let fields_after = story_fields(&s, StoryRef::Body);
    assert_eq!(fields_after[0], ("FILENAME".into(), "Document1".into()));
}

/// #1706 (Word check): Update Field with the caret in the body leaves a header's fields alone.
#[test]
fn update_fields_in_the_body_leaves_headers_alone() {
    let (doc, hdr) = doc_with_header_fields();
    let mut s = Session::new(doc);
    s.run("references.updateFields", &json!({})).unwrap();
    assert_eq!(story_fields(&s, StoryRef::Body)[0], ("AUTHOR".to_string(), "Ada".to_string()));
    assert!(story_fields(&s, StoryRef::Part(hdr)).iter().all(|(_, r)| r == "stale"), "{:?}", story_fields(&s, StoryRef::Part(hdr)));
}

/// #1706: with the caret in a header (or other part), Update Field refreshes that part's
/// fields, but not locked ones and not the body's.
#[test]
fn update_fields_in_a_header_updates_that_header() {
    let (doc, hdr) = doc_with_header_fields();
    let mut s = Session::new(doc);
    let path = s.doc.para_paths(StoryRef::Part(hdr)).remove(0);
    s.sel = crate::session::Selection::caret(Pos::new(StoryRef::Part(hdr), path, 0));
    s.run("references.updateFields", &json!({})).unwrap();
    let got = story_fields(&s, StoryRef::Part(hdr));
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0].1, "Ada");
    assert_eq!(got[1].1, s.doc.word_count().to_string());
    assert_eq!(got[2].1, "stale", "a locked field stays as it was");
    assert!(story_fields(&s, StoryRef::Body).iter().all(|(_, r)| r == "stale"), "the body is not touched");
}

/// A session whose body reads "one two three" and whose header holds stale fields with
/// these codes, the caret in the header.
fn header_fields_session(doc: wordcraft_doc::Document, instrs: &[&str]) -> (Session, u32) {
    use wordcraft_doc::{CharProps, Paragraph, PartKind};
    let mut p = Paragraph::new();
    for instr in instrs {
        let off = p.len();
        let field =
            InlineObject::Field { instr: (*instr).into(), result: "stale".into(), locked: false, dirty: false, code: false, form: String::new() };
        p.insert_object(off, field, &CharProps::default()).unwrap();
    }
    let mut doc = doc;
    let hdr = doc.add_part(PartKind::Header, vec![std::sync::Arc::new(wordcraft_doc::Block::Para(p))]);
    let mut s = Session::new(doc);
    let path = s.doc.para_paths(StoryRef::Part(hdr)).remove(0);
    s.sel = crate::session::Selection::caret(Pos::new(StoryRef::Part(hdr), path, 0));
    (s, hdr)
}

/// SUBJECT, KEYWORDS, COMMENTS, LASTSAVEDBY and REVNUM show the document properties, and
/// NUMCHARS counts the body's characters without spaces; the counts take a `\*` format.
#[test]
fn document_information_fields_show_the_properties() {
    let mut doc = wordcraft_doc::Document::from_text("one two three");
    doc.core.subject = "Budget".into();
    doc.core.keywords = "plan; costs".into();
    doc.core.description = "Draft for review".into();
    doc.core.last_modified_by = "Grace".into();
    doc.core.revision = 7;
    let codes = ["SUBJECT", "KEYWORDS", "COMMENTS", "LASTSAVEDBY", "REVNUM", r"REVNUM \* ROMAN", "NUMCHARS", r"NUMWORDS \* roman"];
    let (mut s, hdr) = header_fields_session(doc, &codes);
    s.run("references.updateFields", &json!({})).unwrap();
    let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["Budget", "plan; costs", "Draft for review", "Grace", "7", "VII", "11", "iii"]);
}

/// A quoted text after TITLE, SUBJECT, AUTHOR, KEYWORDS or COMMENTS replaces the property
/// (at most 255 characters); the field and later ones, DOCPROPERTY too, show the new value.
#[test]
fn summary_fields_with_a_new_value_replace_the_property() {
    let mut doc = wordcraft_doc::Document::from_text("one two three");
    doc.core.subject = "Old".into();
    let long = "x".repeat(300);
    let long_code = format!("COMMENTS \"{long}\"");
    let codes = [
        r#"SUBJECT "Quarterly report""#,
        "SUBJECT",
        r#"TITLE "Plan" \* MERGEFORMAT"#,
        r#"AUTHOR "Ada""#,
        r#"KEYWORDS "a, b""#,
        &long_code,
        "DOCPROPERTY Subject",
    ];
    let (mut s, hdr) = header_fields_session(doc, &codes);
    s.run("references.updateFields", &json!({})).unwrap();
    let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got[..5], ["Quarterly report", "Quarterly report", "Plan", "Ada", "a, b"]);
    assert_eq!(got[5].chars().count(), 255);
    assert_eq!(got[6], "Quarterly report", "DOCPROPERTY reads the subject the paragraph set");
    let c = &s.doc.core;
    assert_eq!((c.subject.as_str(), c.title.as_str(), c.creator.as_str(), c.keywords.as_str()), ("Quarterly report", "Plan", "Ada", "a, b"));
    assert_eq!(c.description, got[5]);
}

/// One word after the name sets the property without quotes; a switch alone sets nothing.
#[test]
fn summary_fields_take_one_word_without_quotes() {
    let mut doc = wordcraft_doc::Document::from_text("one two three");
    doc.core.subject = "Old".into();
    doc.core.creator = "Grace".into();
    doc.core.title = "Plan".into();
    let (mut s, hdr) = header_fields_session(doc, &["SUBJECT Draft", "AUTHOR Ada", r"TITLE \* Upper"]);
    s.run("references.updateFields", &json!({})).unwrap();
    let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
    assert_eq!(got, ["Draft", "Ada", "PLAN"]);
    let c = &s.doc.core;
    assert_eq!((c.subject.as_str(), c.creator.as_str(), c.title.as_str()), ("Draft", "Ada", "Plan"));
}

/// `\* Upper`, `\* Lower`, `\* Caps` and `\* FirstCap` apply to the text Update Field computes,
/// and `\* CardText` and `\* OrdText` are lowercase unless one of them follows.
#[test]
fn update_fields_apply_the_capitalization_switches() {
    let mut doc = wordcraft_doc::Document::from_text("one two three");
    doc.core.subject = "budget".into();
    doc.core.creator = "Ada LOVELACE".into();
    doc.core.title = "weekly report on sales".into();
    doc.core.description = "weekly report on sales".into();
    doc.core.last_modified_by = "grace".into();
    doc.core.revision = 7;
    let codes = [
        r"SUBJECT \* Upper",
        r"AUTHOR \* Lower",
        r"TITLE \* Caps \* MERGEFORMAT",
        r"COMMENTS \*FirstCap",
        r"LASTSAVEDBY \* upper",
        r"NUMWORDS \* CardText",
        r"NUMWORDS \* CardText \* Caps",
        r"REVNUM \* OrdText",
        r"REVNUM \* OrdText \* FirstCap",
        r"DOCPROPERTY Subject \* Upper",
        r#"SUBJECT "new \* Upper""#,
    ];
    let (mut s, hdr) = header_fields_session(doc, &codes);
    s.run("references.updateFields", &json!({})).unwrap();
    let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
    assert_eq!(
        got,
        [
            "BUDGET",
            "ada lovelace",
            "Weekly Report On Sales",
            "Weekly report on sales",
            "GRACE",
            "three",
            "Three",
            "seventh",
            "Seventh",
            "BUDGET",
            "new * Upper"
        ]
    );
}

/// The capitalization switches also apply to REF, PAGEREF, SEQ and FILENAME results in the body.
#[test]
fn body_fields_apply_the_capitalization_switches() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Bookmark target text here.\nSecond paragraph."));
    s.run("select.text", &json!({"text": "Bookmark target text"})).unwrap();
    s.run("insert.bookmark", &json!({"name": "TargetMark"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    for instr in [r"REF TargetMark \* Upper", r"PAGEREF TargetMark \* CardText", r"SEQ Figure \* OrdText \* Caps", r"FILENAME \* Lower"] {
        s.run("insert.field", &json!({"instr": instr, "result": "old"})).unwrap();
    }
    update_fields(&mut s).unwrap();
    let got: Vec<String> = fields(&s).into_iter().map(|(_, r)| r).collect();
    let name = s.resolved_filenames().0.to_lowercase();
    assert_eq!(got, ["BOOKMARK TARGET TEXT".to_string(), "one".into(), "First".into(), name]);
}

/// A count the field's number format cannot write shows Word's error text.
#[test]
fn counts_past_the_number_format_show_the_error() {
    let mut doc = wordcraft_doc::Document::from_text("one two three");
    doc.core.revision = 1_000_000;
    let codes = [r"REVNUM \* ROMAN", r"REVNUM \* CardText", r"REVNUM \* alphabetic", r"NUMWORDS \* ROMAN"];
    let (mut s, hdr) = header_fields_session(doc, &codes);
    s.run("references.updateFields", &json!({})).unwrap();
    let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
    let error = wordcraft_layout::fields::NUMBER_FORMAT_ERROR;
    assert_eq!(got, [error, error, error, "III"]);
}

/// #1706: `all: true` (printing) updates the whole document even with the caret in a header.
#[test]
fn update_fields_all_covers_every_story() {
    let (doc, hdr) = doc_with_header_fields();
    let mut s = Session::new(doc);
    let path = s.doc.para_paths(StoryRef::Part(hdr)).remove(0);
    s.sel = crate::session::Selection::caret(Pos::new(StoryRef::Part(hdr), path, 0));
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    let got = story_fields(&s, StoryRef::Part(hdr));
    assert_eq!((got[0].1.as_str(), got[2].1.as_str()), ("Ada", "stale"), "{got:?}");
    assert_eq!(story_fields(&s, StoryRef::Body)[0].1, "Ada");
    // Junk values fall back to the caret's story.
    s.run("references.updateFields", &json!({"all": "yes"})).unwrap();
}

#[test]
fn toc_tabs_ignore_nonentries() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Before\nContents\nOld\t9\nAfter"));
    let stop = |pos| TabStop { pos, align: TabAlign::Left, leader: TabLeader::None };
    for (i, pos) in [(0, 96.0), (1, 84.0), (2, 24.0)] {
        let p = s.doc.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        p.props.style = Some("TOC2".into());
        p.props.tabs = Some(vec![stop(pos)]);
    }
    s.doc
        .para_mut(StoryRef::Body, &Path::top(1))
        .unwrap()
        .insert_object(
            0,
            InlineObject::Field { instr: "TOC".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
    assert_eq!(cached_toc_tabs(&s), vec![("TOC2".into(), vec![stop(24.0)])]);
}

/// #1230: a selected picture or shape in a table cell gets its caption in the cell, under
/// (or over) the paragraph that holds it, not after the whole table; the number counts on.
#[test]
fn caption_for_an_object_in_a_cell_stays_in_the_cell() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
    s.run("insert.shape", &json!({"kind": "ellipse", "width": 60, "height": 40})).unwrap();
    let (at, _) = crate::cmd::objects::selected(&s).expect("the shape is selected");
    let (table, r, c) = at.path.cell().expect("the shape is in a cell");
    s.run("references.caption", &json!({"label": "Figure", "text": "Kiln"})).unwrap();
    let cell = |s: &Session| s.doc.table(StoryRef::Body, &table).unwrap().rows[r].cells[c].blocks.clone();
    let texts: Vec<String> = cell(&s).iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert_eq!(texts[1], "Figure 1: Kiln", "the caption is under the shape, in the cell");
    assert_eq!(s.doc.body.iter().filter(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("Caption"))).count(), 0);
    // Above: over the shape's paragraph, and it is Figure 1 (the one below becomes 2).
    let end = Pos { off: at.off + wordcraft_doc::para::OBJ.len_utf8(), ..at.clone() };
    s.sel = Selection { anchor: at.clone(), focus: end };
    assert!(crate::cmd::objects::object_selection(&s).is_some());
    s.run("references.caption", &json!({"label": "Figure", "text": "Lid", "position": "above"})).unwrap();
    let texts: Vec<String> = cell(&s).iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect();
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert_eq!((texts[0].as_str(), texts[2].as_str()), ("Figure 1: Lid", "Figure 2: Kiln"));
    // A picture outside a table: the caption goes right after its paragraph.
    let mut s = Session::new(wordcraft_doc::Document::from_text("Before\nAfter"));
    s.run("select.text", &json!({"text": "Before"})).unwrap();
    s.run("caret.end", &json!({})).unwrap();
    s.run("insert.shape", &json!({"kind": "ellipse", "width": 60, "height": 40})).unwrap();
    s.run("references.caption", &json!({})).unwrap();
    let texts: Vec<String> = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect();
    assert_eq!(texts.get(1).map(String::as_str), Some("Figure 1"), "{texts:?}");
}
