use super::*;

#[test]
fn csv_parsing() {
    let r = parse_csv("Name,City\n\"Smith, Ann\",Paris\r\nBo,\"Oslo \"\"N\"\"\"\n\n");
    assert_eq!(r, vec![vec!["Name", "City"], vec!["Smith, Ann", "Paris"], vec!["Bo", "Oslo \"N\""]]);
    assert_eq!(parse_csv("a;b\n1;2")[1], vec!["1", "2"]);
}

/// #4001: finishing a merge opens a new document, with a new undo history. It is not an
/// edit of the open document, even when the merged document holds the same content.
#[test]
fn finishing_a_merge_opens_a_new_document_with_a_new_history() {
    let mut s = Session::new(Document::from_text("Hello"));
    s.run("text.insert", &json!({"text": "!"})).unwrap();
    assert_eq!(s.undo_depth(), 1);
    s.run("caret.docStart", &json!({})).unwrap();
    // One recipient and no merge fields: the merged document holds the same content.
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Ada"}]})).unwrap();
    let id = s.document_id();
    s.run("mailings.finish", &json!({})).unwrap();
    assert_ne!(s.document_id(), id, "the document was replaced");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "!Hello");
    assert_eq!(s.undo_depth(), 0, "the new document starts with an empty undo history");
    // Undo cannot bring the old document's text back.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "!Hello", "the old text does not come back");
}

/// #4004: finishing a merge to a file saves the merged copy there and leaves the open
/// document alone: it changes nothing, so it is not an edit.
#[test]
fn finishing_a_merge_to_a_file_is_not_an_edit() {
    let dir = std::env::temp_dir().join(format!("wordcraft-merge-to-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("merged.docx");
    let mut s = Session::new(Document::from_text("Hello"));
    s.run("text.insert", &json!({"text": "!"})).unwrap();
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Ada"}]})).unwrap();
    let (depth, dirty, text) = (s.undo_depth(), s.dirty, s.doc.plain_text(StoryRef::Body));
    s.run("mailings.finish", &json!({"path": out.to_string_lossy()})).unwrap();
    assert!(out.exists(), "the merged copy was written");
    assert_eq!(s.doc.plain_text(StoryRef::Body), text, "the open document is unchanged");
    assert_eq!(s.undo_depth(), depth, "no undo step");
    assert_eq!(s.dirty, dirty, "the unsaved-changes flag is unchanged");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn csv_delimiter_ignores_quoted_text() {
    let r = parse_csv("\"Name, first, last\";City\nAnn;Paris\n");
    assert_eq!(r, vec![vec!["Name, first, last", "City"], vec!["Ann", "Paris"]]);
}

#[test]
fn csv_delimiter_skips_leading_blank_lines() {
    let want = vec![vec!["Name", "Email"], vec!["Ann", "ann@x.org"]];
    assert_eq!(parse_csv("\nName,Email\nAnn,ann@x.org\n"), want);
    assert_eq!(parse_csv("\r\n\r\nName;Email\r\nAnn;ann@x.org"), want);
}

/// Field types ignore case (ECMA-376 Part 1, 17.16.1), and `MERGEFIELDX` is another field.
#[test]
fn merge_field_types_ignore_case() {
    assert_eq!(mergefield_name(" mergefield City ").as_deref(), Some("City"));
    assert_eq!(mergefield_name(r#"MergeField "First Name""#).as_deref(), Some("First Name"));
    assert_eq!(mergefield_name(r#"MERGEFIELD"City""#).as_deref(), Some("City"));
    assert_eq!(mergefield_name("MERGEFIELDX City"), None);
    assert_eq!(placeholder("mergefield City").as_deref(), Some("«City»"));
    assert_eq!(placeholder(r#"if { mergefield City } = "x" "y" "z""#).as_deref(), Some(""));
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Alpha"}, {"Name": "Beta"}]})).unwrap();
    s.run("mailings.start", &json!({"kind": "directory"})).unwrap();
    s.run("insert.field", &json!({"instr": "mergefield Name", "result": ""})).unwrap();
    s.run("insert.field", &json!({"instr": r#"if { mergefield Name } = "Beta" " (b)" """#, "result": ""})).unwrap();
    s.run("mailings.finish", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Alpha\nBeta (b)");
}

/// An IF is a merge rule only when the field it tests is a MERGEFIELD: the word in a quoted
/// value doesn't make `IF { PAGE } = "mergefield" …` one.
#[test]
fn an_if_on_another_field_is_not_a_merge_rule() {
    let other = r#"IF { PAGE } = "mergefield" "yes" "no""#;
    assert_eq!(placeholder(other), None);
    assert_eq!(placeholder(r#"IF { PAGE } = { MERGEFIELD City } "yes" "no""#), None);
    assert_eq!(placeholder(r#"IF { MERGEFIELD City } = "mergefield" "yes" "no""#).as_deref(), Some(""));
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Alpha"}]})).unwrap();
    s.run("mailings.start", &json!({"kind": "directory"})).unwrap();
    s.run("insert.field", &json!({"instr": "mergefield Name", "result": ""})).unwrap();
    s.run("insert.field", &json!({"instr": other, "result": " kept"})).unwrap();
    s.run("mailings.finish", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Alpha kept");
}

#[test]
fn merge_end_to_end() {
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({"csv": "First Name,Last Name,City\nAda,Lovelace,London\nAlan,Turing,Wilmslow"})).unwrap();
    s.run("mailings.greetingLine", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "See you in "})).unwrap();
    s.run("mailings.insertField", &json!({"field": "City"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("«City»"));
    s.run("mailings.preview", &json!({"value": true})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Dear Ada Lovelace,"));
    s.run("mailings.next", &json!({})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Wilmslow"));
    let chk = s.run("mailings.checkErrors", &json!({})).unwrap();
    assert_eq!(chk["ok"], true);
    let r = s.run("mailings.finish", &json!({})).unwrap();
    assert_eq!(r["records"], 2);
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.contains("See you in London") && t.contains("Dear Alan Turing,"), "{t}");
    assert_eq!(s.layout().pages.len(), 2);
}

/// A typed list (#240) keeps its column order, and rows may be arrays in that order.
#[test]
fn recipients_from_a_workbook_ask_for_the_sheet_when_there_are_several() {
    use std::io::Write;
    let cell = |t: &str| format!(r#"<table:table-cell office:value-type="string"><text:p>{t}</text:p></table:table-cell>"#);
    let row = |cells: &[&str]| format!("<table:table-row>{}</table:table-row>", cells.iter().map(|c| cell(c)).collect::<String>());
    let content = format!(
        r#"<office:document-content xmlns:office="o" xmlns:table="t" xmlns:text="x"><office:body><office:spreadsheet><table:table table:name="Notes">{}</table:table><table:table table:name="Guests">{}{}{}</table:table></office:spreadsheet></office:body></office:document-content>"#,
        row(&["n"]),
        row(&["Name", "", "Name"]),
        row(&["Ada", "London", "A"]),
        row(&["Alan", "Wilmslow", "B"]),
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in [("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", content.as_str())] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    let data = super::super::insert::base64_encode(&z.finish().unwrap().into_inner());
    let mut s = Session::new(Document::new());
    // Two sheets and none chosen: the names come back and nothing loads.
    let r = s.run("mailings.recipients", &json!({"data": data})).unwrap();
    assert_eq!(r, json!({"sheets": ["Notes", "Guests"], "chooseSheet": true}));
    assert!(s.merge.headers.is_empty());
    let r = s.run("mailings.recipients", &json!({"data": data, "sheet": "guests"})).unwrap();
    assert_eq!(r, json!({"fields": ["Name", "Column 2", "Name 2"], "records": 2, "sheet": "Guests"}));
    assert_eq!(s.merge.rows[1], ["Alan", "Wilmslow", "B"]);
    // Without a header row every row is a record.
    let r = s.run("mailings.recipients", &json!({"data": data, "sheet": 2, "headers": false})).unwrap();
    assert_eq!(r["fields"], json!(["Column 1", "Column 2", "Column 3"]));
    assert_eq!(r["records"], 3);
    assert!(s.run("mailings.recipients", &json!({"data": data, "sheet": 3})).is_err());
    assert!(s.run("mailings.recipients", &json!({"data": data, "sheet": 0})).is_err());
    assert!(s.run("mailings.recipients", &json!({"data": "%%%"})).is_err());
}

#[test]
fn recipients_from_fields_and_array_rows() {
    let mut s = Session::new(Document::new());
    let r = s
        .run("mailings.recipients", &json!({"fields": ["Last Name", " City ", "Age"], "rows": [["Lovelace", "London", 36], ["Turing"], 7]}))
        .unwrap();
    assert_eq!(r, json!({"fields": ["Last Name", "City", "Age"], "records": 2}));
    assert_eq!(s.merge.rows, vec![vec!["Lovelace".to_string(), "London".into(), "36".into()], vec!["Turing".into(), String::new(), String::new()]]);
    // Objects follow `fields` too; missing and null cells are blank.
    s.run("mailings.editRecipients", &json!({"fields": ["Name", "City"], "rows": [{"City": "Paris", "Name": null}]})).unwrap();
    assert_eq!(s.merge.headers, vec!["Name", "City"]);
    assert_eq!(s.merge.rows, vec![vec![String::new(), "Paris".to_string()]]);
    // Array rows need the field names.
    let err = s.run("mailings.recipients", &json!({"rows": [["Ada"]]})).unwrap_err();
    assert!(matches!(err, CmdError::Params(_)), "{err}");
    assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");
}

/// Start Mail Merge takes only the kinds it knows; Edit Recipient List needs a list.
#[test]
fn merge_kinds_and_edit_list_needs_recipients() {
    let mut s = Session::new(Document::new());
    for k in MERGE_KINDS {
        assert_eq!(s.run("mailings.start", &json!({"kind": k})).unwrap(), json!({"kind": k}));
    }
    assert_eq!(s.run("mailings.start", &json!({"kind": " Directory "})).unwrap(), json!({"kind": "directory"}));
    assert!(matches!(s.run("mailings.start", &json!({"kind": "fax"})), Err(CmdError::Params(_))));
    assert_eq!(s.merge.kind, "directory", "a rejected kind leaves the current one");
    assert!(matches!(s.run("mailings.editRecipients", &json!({})), Err(CmdError::Disabled(_))));
    s.run("mailings.recipients", &json!({"csv": "Name\nAda"})).unwrap();
    assert_eq!(
        s.run("mailings.editRecipients", &json!({})).unwrap(),
        json!({"headers": ["Name"], "rows": [["Ada"]], "records": 1, "rules": [], "sort": []})
    );
}

fn merged_text(rule: Option<Value>, rows: Value) -> String {
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({ "rows": rows })).unwrap();
    s.run("mailings.start", &json!({"kind": "directory"})).unwrap();
    if let Some(r) = rule {
        s.run("mailings.rules", &r).unwrap();
    }
    s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
    s.run("mailings.finish", &json!({})).unwrap();
    s.doc.plain_text(StoryRef::Body)
}

/// #427: SKIPIF leaves out the recipients that match its condition.
#[test]
fn skipif_drops_matching_records() {
    let rows = json!([{"Name": "Alpha", "Skip": "yes"}, {"Name": "Beta", "Skip": "no"}]);
    let skip = |v: &str| Some(json!({"rule": "SKIPIF", "field": "Skip", "value": v}));
    assert_eq!(merged_text(skip("yes"), rows.clone()), "Beta");
    assert_eq!(merged_text(skip("no"), rows.clone()), "Alpha");
    assert_eq!(merged_text(skip("absent"), rows.clone()), "Alpha\nBeta");
    assert_eq!(merged_text(None, rows), "Alpha\nBeta");
    let all = json!([{"Name": "Alpha", "Skip": "yes"}, {"Name": "Beta", "Skip": "yes"}]);
    assert_eq!(merged_text(skip("yes"), all), "");
}

/// #428: NEXT moves the following merge fields to the next recipient, and the next merged
/// copy starts after it.
#[test]
fn next_record_advances_following_fields() {
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Alpha"}, {"Name": "Beta"}, {"Name": "Gamma"}]})).unwrap();
    s.run("mailings.start", &json!({"kind": "directory"})).unwrap();
    s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
    s.run("text.insert", &json!({"text": " / "})).unwrap();
    s.run("mailings.rules", &json!({"rule": "NEXT"})).unwrap();
    s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
    s.run("mailings.preview", &json!({"value": true})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Alpha / Beta");
    s.run("mailings.finish", &json!({})).unwrap();
    // Gamma has no record after it: NEXT past the end leaves the field blank, never panics.
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Alpha / Beta\nGamma / ");
}

/// #3028: First, Last and Go to Record move the preview; Finish & Merge and Print Documents
/// take all records, the current one, or a range by record number.
#[test]
fn record_navigation_and_merge_ranges() {
    let names = ["Alpha", "Beta", "Gamma", "Delta"];
    let fresh = || {
        let mut s = Session::new(Document::new());
        s.run("mailings.recipients", &json!({"rows": names.map(|n| json!({"Name": n}))})).unwrap();
        s.run("mailings.start", &json!({"kind": "directory"})).unwrap();
        s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
        s
    };
    let mut s = fresh();
    let body = |s: &Session| s.doc.plain_text(StoryRef::Body);
    assert_eq!(s.run("mailings.last", &json!({})).unwrap(), json!({"record": 4, "of": 4}));
    assert_eq!(body(&s), "Delta");
    assert_eq!(s.run("mailings.first", &json!({})).unwrap()["record"], 1);
    assert_eq!(body(&s), "Alpha");
    assert_eq!(s.run("mailings.goToRecord", &json!({"record": 3})).unwrap()["record"], 3);
    assert_eq!(body(&s), "Gamma");
    assert_eq!(s.run("mailings.goToRecord", &json!({"record": " 2 "})).unwrap()["record"], 2);
    // Past either end: the last or first record, as the step buttons do.
    assert_eq!(s.run("mailings.goToRecord", &json!({"record": 1141})).unwrap()["record"], 4);
    assert_eq!(s.run("mailings.goToRecord", &json!({"record": -5})).unwrap()["record"], 1);
    for bad in [json!({}), json!({"record": "two"}), json!({"record": 2.5})] {
        assert!(matches!(s.run("mailings.goToRecord", &bad), Err(CmdError::Params(_))), "{bad}");
    }

    s.run("mailings.goToRecord", &json!({"record": 3})).unwrap();
    assert_eq!(s.run("mailings.finish", &json!({"records": "current"})).unwrap()["records"], 1);
    assert_eq!(body(&s), "Gamma");
    let mut s = fresh();
    assert_eq!(s.run("mailings.finish", &json!({"from": 2, "to": 3})).unwrap()["records"], 2);
    assert_eq!(body(&s), "Beta\nGamma");
    let mut s = fresh();
    assert_eq!(s.run("mailings.finish", &json!({"records": "all", "from": 3, "to": 99})).unwrap()["records"], 2);
    assert_eq!(body(&s), "Gamma\nDelta");
    // An empty range is refused and the document is kept, not replaced by an empty one.
    let mut s = fresh();
    let before = body(&s);
    for bad in [json!({"from": 3, "to": 2}), json!({"from": 5}), json!({"records": "some"})] {
        assert!(s.run("mailings.finish", &bad).is_err(), "{bad}");
        assert_eq!(body(&s), before, "{bad}");
    }
    let e = s.run("mailings.finish", &json!({"from": 5})).unwrap_err().to_string();
    assert_eq!(e, "no records from 5 to 4 (the list has 4)", "a plain message for the dialog");

    // Print Documents merges the same way, then asks the interface for Print.
    let mut s = fresh();
    s.ui_requests.clear();
    assert_eq!(s.run("mailings.printDocuments", &json!({"from": 4, "path": "ignored.docx"})).unwrap()["records"], 1);
    assert_eq!(body(&s), "Delta");
    assert!(s.ui_requests.contains(&json!({"open": "print"})));
    let mut s = fresh();
    s.ui_requests.clear();
    assert!(s.run("mailings.printDocuments", &json!({"from": 9})).is_err());
    assert!(s.ui_requests.is_empty(), "nothing to print, no Print dialog");
}

/// #429: an IF rule shows its true / false text in Preview Results, as Finish & Merge does.
#[test]
fn if_rule_previews_its_result() {
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Alpha", "Flag": "yes"}, {"Name": "Beta", "Flag": "no"}]})).unwrap();
    s.run("mailings.start", &json!({"kind": "directory"})).unwrap();
    s.run("mailings.rules", &json!({"rule": "IF", "field": "Flag", "value": "yes", "then": "Selected", "else": "Other"})).unwrap();
    s.run("mailings.preview", &json!({"value": true})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Selected");
    s.run("mailings.next", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Other");
    s.run("mailings.previous", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Selected");
    s.run("mailings.finish", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Selected\nOther");
}

/// #430: merge fields in headers and footers are previewed and merged; each letter gets its
/// own header.
#[test]
fn header_and_footer_fields_merge() {
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Alpha"}, {"Name": "Beta"}]})).unwrap();
    let hdr = s.run("insert.header", &json!({"text": "Recipient: "})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Body: "})).unwrap();
    s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
    let header = |s: &Session, id: u32| s.doc.plain_text(StoryRef::Part(id));
    s.run("mailings.preview", &json!({"value": true})).unwrap();
    assert_eq!(header(&s, hdr), "Recipient: Alpha");
    s.run("mailings.preview", &json!({"value": false})).unwrap();
    assert_eq!(header(&s, hdr), "Recipient: «Name»");
    s.run("mailings.finish", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Body: Alpha\nBody: Beta");
    let secs = s.doc.sections();
    assert_eq!(secs.len(), 2, "one section per letter");
    let texts: Vec<String> = secs.iter().map(|(_, sp)| header(&s, sp.headers.default.unwrap())).collect();
    assert_eq!(texts, ["Recipient: Alpha", "Recipient: Beta"]);
    assert!(!s.doc.parts.contains_key(&hdr), "the template header is replaced by the letters' copies");
}

#[test]
fn label_and_envelope_text_is_capped() {
    let mut s = Session::new(Document::new());
    let long = "x".repeat(2001);
    assert!(matches!(s.run("mailings.labels", &json!({"text": long})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("mailings.envelopes", &json!({"delivery": long})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("mailings.envelopes", &json!({"return": long})), Err(CmdError::Params(_))));
    s.run("mailings.labels", &json!({"text": "x".repeat(2000), "rows": 40, "cols": 10})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).chars().count() <= 400 * 2000 + 400 * 20);
}

#[test]
fn envelopes_and_labels() {
    let mut s = Session::new(Document::new());
    s.run("mailings.envelopes", &json!({"delivery": "Jo Doe\n1 Main St"})).unwrap();
    assert!(s.doc.last_section.page_w > s.doc.last_section.page_h);
    s.run("mailings.labels", &json!({"text": "Hello"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).matches("Hello").count() == 30);
}

/// #407: Envelopes adds an envelope section of the chosen size before the letter; Labels
/// lays out a sheet of the chosen product, or a single label at a row and column.
#[test]
fn envelope_sizes_add_to_document_and_label_products() {
    let mut s = Session::new(Document::from_text("Dear Jo,"));
    let r = s.run("mailings.envelopes", &json!({"delivery": "Jo Doe\n1 Main St", "return": "Me", "size": "C5", "add": true})).unwrap();
    assert_eq!((r["width"].as_f64().map(|w| w.round()), r["height"].as_f64().map(|h| h.round())), (Some(649.0), Some(459.0)));
    let secs = s.doc.sections();
    assert_eq!(secs.len(), 2, "an envelope section of its own");
    assert!(secs[0].1.landscape && (secs[0].1.page_w - 649.1).abs() < 0.1);
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.starts_with("Me") && text.contains("1 Main St") && text.trim_end().ends_with("Dear Jo,"), "{text}");
    assert!(s.run("mailings.envelopes", &json!({"size": "Envelope X"})).is_err());

    let r = s.run("mailings.labels", &json!({"text": "Ada", "product": "a4-2x7"})).unwrap();
    assert_eq!(r["labels"], 14);
    assert!((s.doc.last_section.page_w - 595.3).abs() < 0.1, "A4 sheet");
    let Some(Block::Table(t)) = s.doc.body.first().map(|b| &**b) else { panic!("no table") };
    assert_eq!((t.rows.len(), t.grid.len()), (7, 3), "two label columns and the gap between");
    assert_eq!(s.doc.plain_text(StoryRef::Body).matches("Ada").count(), 14);
    let r = s.run("mailings.labels", &json!({"text": "Bo", "product": "letter-3x10", "single": {"row": 2, "col": 3}})).unwrap();
    assert_eq!(r["filled"], 1);
    let Some(Block::Table(t)) = s.doc.body.first().map(|b| &**b) else { panic!("no table") };
    // Row 2, column 3 (grid column 5: label, gap, label, gap, label).
    let cell = t.rows.get(1).and_then(|r| r.cells.get(4)).and_then(|c| c.blocks.first());
    assert!(matches!(cell.map(|b| &**b), Some(Block::Para(p)) if p.text == "Bo"));
    assert!(s.run("mailings.labels", &json!({"product": "nope"})).is_err());
}

/// #4880: every selected recipient gets a label, including those beyond the first sheet.
#[test]
fn recipient_labels_continue_on_product_sheets() {
    for product in LABEL_PRODUCTS {
        let per_sheet = product.rows * product.cols;
        for count in [1, per_sheet, per_sheet + 1, per_sheet * 2, per_sheet * 2 + 5] {
            let mut s = Session::new(Document::from_text("Original letter"));
            let rows: Vec<Value> = (0..count).map(|i| json!({"FirstName": format!("Recipient{i:03}"), "Address": "Original Street"})).collect();
            s.run("mailings.recipients", &json!({"rows": rows})).unwrap();
            let r = s.run("mailings.labels", &json!({"product": product.id, "fromRecipients": true})).unwrap();
            let sheets = count.div_ceil(per_sheet);
            assert_eq!(r["filled"], count, "{}: {count} recipients", product.id);
            assert_eq!(r["labels"], sheets * per_sheet);
            assert_eq!(s.doc.sections().len(), sheets);
            let tables: Vec<_> = s
                .doc
                .body
                .iter()
                .filter_map(|b| match &**b {
                    Block::Table(t) => Some(t),
                    _ => None,
                })
                .collect();
            assert_eq!(tables.len(), sheets);
            let label_text: Vec<_> = tables
                .iter()
                .flat_map(|t| t.rows.iter())
                .flat_map(|r| &r.cells)
                .flat_map(|c| &c.blocks)
                .filter_map(|b| match &**b {
                    Block::Para(p) if !p.text.is_empty() => Some(p.text.clone()),
                    _ => None,
                })
                .collect();
            let expected: Vec<_> = (0..count).flat_map(|i| [format!("Recipient{i:03}"), "Original Street".to_string()]).collect();
            assert_eq!(label_text, expected);
            assert!(tables.windows(2).all(|t| t[0].grid == t[1].grid && t[0].props == t[1].props));
        }
    }
}

#[test]
fn recipient_label_sheets_keep_the_source_and_selected_order() {
    let mut s = Session::new(Document::from_text("Original letter"));
    s.run("text.insert", &json!({"text": "Draft "})).unwrap();
    let original = s.doc.clone();
    let id = s.document_id();
    let rows: Vec<Value> = (0..65).map(|i| json!({"FirstName": format!("Recipient{i:03}")})).collect();
    s.run("mailings.recipients", &json!({"rows": rows})).unwrap();
    s.run(
        "mailings.filterRecipients",
        &json!({"rules": [{"field": "FirstName", "op": "notEqual", "value": "Recipient003"}],
        "sort": [{"field": "FirstName", "descending": true}]}),
    )
    .unwrap();
    let selected = s.run("mailings.editRecipients", &json!({})).unwrap();
    let r = s.run("mailings.labels", &json!({"fromRecipients": true})).unwrap();
    assert_eq!(r["filled"], 64);
    assert_eq!(r["labels"], 90);
    assert_eq!(s.undo_depth(), 0, "labels are a new document");
    let text = s.doc.plain_text(StoryRef::Body);
    let names: Vec<_> = text.split_whitespace().filter(|l| l.starts_with("Recipient")).collect();
    assert_eq!(names, (0..65).rev().filter(|i| *i != 3).map(|i| format!("Recipient{i:03}")).collect::<Vec<_>>());
    s.run("view.switchWindows", &json!({"id": id})).unwrap();
    assert_eq!(s.doc, original, "the original letter stays open and unchanged");
    assert_eq!(s.run("mailings.editRecipients", &json!({})).unwrap(), selected);
    assert_eq!(s.undo_depth(), 1, "the source keeps its edit history");
}

fn b64(b: &[u8]) -> String {
    super::super::insert::base64_encode(b)
}

/// #778: a CSV saved by a Chinese-locale spreadsheet is GBK, not UTF-8; one saved as
/// "Unicode text" is UTF-16 with tabs. Both read as the characters they hold.
#[test]
fn recipient_lists_in_legacy_and_utf16_encodings() {
    let text = "物料编号,送检日期,检验日期,批号,数量\nPLND0119,2024/4/16,2024/4/16,4164001,12\nE655,2024/4/17,2024/4/17,4174001,1\n";
    let (gbk, _, bad) = encoding_rs::GBK.encode(text);
    assert!(!bad && std::str::from_utf8(&gbk).is_err());
    let mut s = Session::new(Document::new());
    let r = s.run("mailings.recipients", &json!({"path": "list.csv", "data": b64(&gbk)})).unwrap();
    assert_eq!(r["fields"], json!(["物料编号", "送检日期", "检验日期", "批号", "数量"]));
    assert_eq!(s.merge.rows[1], ["E655", "2024/4/17", "2024/4/17", "4174001", "1"]);
    // Big5 (Traditional Chinese) and Shift_JIS too.
    for (enc, t) in
        [(encoding_rs::BIG5, "姓名,城市\n陳大文,臺北市\n林小明,高雄市\n"), (encoding_rs::SHIFT_JIS, "氏名,住所\n山田太郎,東京都\n鈴木花子,大阪府\n")]
    {
        let (b, _, _) = enc.encode(t);
        s.run("mailings.recipients", &json!({"data": b64(&b)})).unwrap();
        assert_eq!(s.merge.headers.join(","), t.lines().next().unwrap(), "{}", enc.name());
        assert_eq!(s.merge.rows[0].join(","), t.lines().nth(1).unwrap(), "{}", enc.name());
    }
    // UTF-16 LE with BOM, tab-separated.
    let mut u16le = vec![0xFF, 0xFE];
    for u in "姓名\t城市\r\n张三\t北京\r\n".encode_utf16() {
        u16le.extend_from_slice(&u.to_le_bytes());
    }
    s.run("mailings.recipients", &json!({"path": "list.txt", "data": b64(&u16le)})).unwrap();
    assert_eq!(s.merge.headers, ["姓名", "城市"]);
    assert_eq!(s.merge.rows, [["张三", "北京"]]);
    // An explicit encoding wins over detection; an unknown one is a params error.
    let (sj, _, _) = encoding_rs::SHIFT_JIS.encode("名前\n田中\n");
    s.run("mailings.recipients", &json!({"data": b64(&sj), "encoding": "shift_jis"})).unwrap();
    assert_eq!(s.merge.rows, [["田中"]]);
    assert!(matches!(s.run("mailings.recipients", &json!({"data": b64(&sj), "encoding": "klingon"})), Err(CmdError::Params(_))));
    // Plain UTF-8 and Windows-1252 still read as before.
    s.run("mailings.recipients", &json!({"data": b64(b"Name\nJos\xE9\n")})).unwrap();
    assert_eq!(s.merge.rows, [["José"]]);
}

/// #778: legacy Excel 97–2003 workbooks are recipient lists; a CSV named .xls reads as text.
#[test]
fn recipients_from_an_xls_workbook() {
    let row = |r: &[&str]| r.iter().map(|c| c.to_string()).collect::<Vec<_>>();
    let book = wordcraft_formats::sheet::xls_fixture(&[(
        "收件人",
        vec![row(&["姓名", "城市", "数量"]), row(&["张三", "北京", "12"]), row(&["李四", "上海", "2"])],
    )])
    .unwrap();
    let mut s = Session::new(Document::new());
    let r = s.run("mailings.recipients", &json!({"path": "名单.xls", "data": b64(&book)})).unwrap();
    assert_eq!(r, json!({"fields": ["姓名", "城市", "数量"], "records": 2, "sheet": "收件人"}));
    assert_eq!(s.merge.rows[1], ["李四", "上海", "2"]);
    // Detected by content without a name.
    assert_eq!(s.run("mailings.recipients", &json!({"data": b64(&book)})).unwrap()["records"], 2);
    let r = s.run("mailings.recipients", &json!({"path": "export.xls", "data": b64(b"Name\tCity\nAda\tLondon\n")})).unwrap();
    assert_eq!(r["fields"], json!(["Name", "City"]));
    let e = s.run("mailings.recipients", &json!({"path": "export.xls", "data": b64(b"<html><table><tr><td>x</td></tr></table></html>")}));
    assert!(matches!(e, Err(CmdError::Failed(m)) if m.contains("web page")));
    // #1287: an Excel 2003 XML spreadsheet named .xls says what it is.
    let xml = b"<?xml version=\"1.0\"?>\n<Workbook xmlns=\"urn:schemas-microsoft-com:office:spreadsheet\"></Workbook>";
    let e = s.run("mailings.recipients", &json!({"path": "export.xls", "data": b64(xml)}));
    assert!(matches!(e, Err(CmdError::Failed(m)) if m.contains("XML spreadsheet")));
    assert!(
        s.run("mailings.recipients", &json!({"path": "junk.xls", "data": b64(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 1])})).is_err()
    );
    assert!(super::is_spreadsheet_name("a.XLS") && super::is_spreadsheet_name("b.ods") && !super::is_spreadsheet_name("c.csv"));
    assert!(super::RECIPIENT_EXTENSIONS.contains(&"xls"));
}

fn people() -> Session {
    let mut s = Session::new(Document::new());
    s.run(
        "mailings.recipients",
        &json!({"csv": "Name,City,Age,Email\nAda,London,36,ada@x.org\nAlan,Wilmslow,41,\nGrace,New York,85,grace@navy.mil\nKatherine,Hampton,101,\nLinus,Helsinki,9,linus@kernel.org"}),
    )
    .unwrap();
    s
}

fn names(s: &Session) -> Vec<String> {
    s.merge.rows.iter().map(|r| r[0].clone()).collect()
}

fn filter(s: &mut Session, rules: Value) -> Vec<String> {
    s.run("mailings.filterRecipients", &json!({"rules": rules})).unwrap();
    names(s)
}

/// #778: Edit Recipient List › Filter: every comparison, numbers as numbers, And / Or.
#[test]
fn filter_recipients_comparisons_and_joins() {
    let mut s = people();
    assert_eq!(filter(&mut s, json!([{"field": "city", "op": "equal", "value": "london"}])), ["Ada"]);
    assert_eq!(filter(&mut s, json!([{"field": "City", "op": "<>", "value": "London"}])), ["Alan", "Grace", "Katherine", "Linus"]);
    // 9 < 36 as numbers, though "9" > "36" as text.
    assert_eq!(filter(&mut s, json!([{"field": "Age", "op": "less", "value": "36"}])), ["Linus"]);
    assert_eq!(filter(&mut s, json!([{"field": "Age", "op": "greater", "value": "41"}])), ["Grace", "Katherine"]);
    assert_eq!(filter(&mut s, json!([{"field": "Age", "op": "lessOrEqual", "value": 36}])), ["Ada", "Linus"]);
    assert_eq!(filter(&mut s, json!([{"field": "Age", "op": ">=", "value": "85"}])), ["Grace", "Katherine"]);
    assert_eq!(filter(&mut s, json!([{"field": "Email", "op": "blank"}])), ["Alan", "Katherine"]);
    assert_eq!(filter(&mut s, json!([{"field": "Email", "op": "isNotBlank"}])), ["Ada", "Grace", "Linus"]);
    assert_eq!(filter(&mut s, json!([{"field": "Email", "op": "contains", "value": ".ORG"}])), ["Ada", "Linus"]);
    assert_eq!(filter(&mut s, json!([{"field": "Email", "op": "doesNotContain", "value": "org"}])), ["Alan", "Grace", "Katherine"]);
    // And binds tighter than Or: (Email blank And Age > 50) Or City = London.
    let r = json!([
        {"field": "Email", "op": "blank"},
        {"field": "Age", "op": "greater", "value": "50", "join": "and"},
        {"field": "City", "op": "equal", "value": "London", "join": "or"}
    ]);
    assert_eq!(filter(&mut s, r), ["Ada", "Katherine"]);
    let r = s
        .run(
            "mailings.filterRecipients",
            &json!({"rules": [{"field": "Age", "op": "less", "value": 50}, {"field": "City", "op": "contains", "value": "on", "join": "and"}]}),
        )
        .unwrap();
    assert_eq!(r["records"], 1);
    assert_eq!(r["of"], 5);
    assert_eq!(r["rules"][1], json!({"field": "City", "op": "contains", "value": "on", "join": "and"}));
    // {} clears.
    assert_eq!(s.run("mailings.filterRecipients", &json!({})).unwrap()["records"], 5);
    // Bad rules are params errors and leave the filter alone.
    filter(&mut s, json!([{"field": "City", "op": "equal", "value": "Hampton"}]));
    for bad in [
        json!({"rules": [{"field": "Nope", "op": "equal", "value": "x"}]}),
        json!({"rules": [{"field": "City", "op": "resembles", "value": "x"}]}),
        json!({"rules": [{"field": "City", "join": "xor"}]}),
        json!({"rules": [{"op": "equal"}]}),
        json!({"rules": "City = London"}),
        json!({"rules": [], "sort": [{"field": "Nope"}]}),
    ] {
        assert!(matches!(s.run("mailings.filterRecipients", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    assert_eq!(names(&s), ["Katherine"]);
    // Loading another list clears the filter.
    s.run("mailings.recipients", &json!({"csv": "Name\nA\nB"})).unwrap();
    assert_eq!(names(&s), ["A", "B"]);
    assert!(s.merge.filter.is_empty());
}

/// #778: Sort Recipients: by one or more columns, numbers as numbers, ties in list order.
#[test]
fn sort_recipients() {
    let mut s = people();
    s.run("mailings.sortRecipients", &json!({"field": "Age", "descending": true})).unwrap();
    assert_eq!(names(&s), ["Katherine", "Grace", "Alan", "Ada", "Linus"]);
    s.run("mailings.sortRecipients", &json!({"sort": [{"field": "Email"}, {"field": "Name", "descending": true}]})).unwrap();
    assert_eq!(names(&s), ["Katherine", "Alan", "Ada", "Grace", "Linus"]);
    // Filter and sort together; a filter call without `sort` keeps the sort.
    s.run("mailings.filterRecipients", &json!({"rules": [{"field": "Age", "op": "greater", "value": 20}], "sort": [{"field": "City"}]})).unwrap();
    assert_eq!(names(&s), ["Katherine", "Ada", "Grace", "Alan"]);
    s.run("mailings.filterRecipients", &json!({"rules": [{"field": "Age", "op": "less", "value": 90}]})).unwrap();
    assert_eq!(names(&s), ["Linus", "Ada", "Grace", "Alan"]);
    s.run("mailings.sortRecipients", &json!({})).unwrap();
    assert_eq!(names(&s), ["Ada", "Alan", "Grace", "Linus"]);
    // Edit Recipient List shows every record, and editing keeps the filter.
    let r = s.run("mailings.editRecipients", &json!({})).unwrap();
    assert_eq!(r["rows"].as_array().unwrap().len(), 5);
    assert_eq!(r["records"], 4);
    let mut rows: Vec<Value> = r["rows"].as_array().unwrap().clone();
    rows.push(json!(["Margaret", "Boston", "88", ""]));
    s.run("mailings.editRecipients", &json!({"fields": ["Name", "City", "Age", "Email"], "rows": rows})).unwrap();
    assert_eq!(names(&s), ["Ada", "Alan", "Grace", "Linus", "Margaret"]);
    assert_eq!(s.merge.filter.len(), 1);
    // Removing the filtered field drops its rule.
    s.run("mailings.editRecipients", &json!({"fields": ["Name"], "rows": [["X"], ["Y"]]})).unwrap();
    assert!(s.merge.filter.is_empty());
    assert_eq!(names(&s), ["X", "Y"]);
}

/// #1287: an Edit Recipient List that fails to load keeps the list's filter and sort.
#[test]
fn failed_edit_recipients_keeps_filter_and_sort() {
    let mut s = people();
    s.run("mailings.filterRecipients", &json!({"rules": [{"field": "Age", "op": "greater", "value": 20}], "sort": [{"field": "City"}]})).unwrap();
    let before = names(&s);
    // Array rows without `fields` fail to load.
    let e = s.run("mailings.editRecipients", &json!({"rows": [["X", "Y"]]}));
    assert!(matches!(e, Err(CmdError::Params(_))));
    assert_eq!(s.merge.filter.len(), 1);
    assert_eq!(s.merge.sort.len(), 1);
    assert_eq!(names(&s), before);
    assert_eq!(s.run("mailings.editRecipients", &json!({})).unwrap()["rules"].as_array().map(Vec::len), Some(1));
}

/// Review fix for #885: sorting a column that mixes numbers and text is a total order
/// (numbers first, numerically; then text; ties keep list order) and never panics.
#[test]
fn sort_mixed_number_and_text_column() {
    let headers = vec!["Id".to_string()];
    let rows = |vals: &[&str]| vals.iter().map(|v| vec![v.to_string()]).collect::<Vec<_>>();
    let sorted = |rows: &[Vec<String>], desc: bool| {
        let order = select_records(&headers, rows, &[], &[SortKey { field: "Id".into(), descending: desc }]);
        order.iter().map(|&i| rows[i][0].clone()).collect::<Vec<_>>()
    };
    let r = rows(&["1a", "9", "10", "b", "2"]);
    assert_eq!(sorted(&r, false), ["2", "9", "10", "1a", "b"]);
    assert_eq!(sorted(&r, true), ["b", "1a", "10", "9", "2"]);
    // Equal keys stay in list order.
    let r = rows(&["x", "1", "X", "1.0", ""]);
    assert_eq!(sorted(&r, false), ["1", "1.0", "", "x", "X"]);
    // A long mixed column (where an inconsistent comparator makes `sort_by` panic).
    let vals: Vec<String> = (0..400)
        .map(|i| match i % 4 {
            0 => format!("{}", (i * 37) % 101),
            1 => format!("{}a", (i * 13) % 17),
            2 => format!("{}", (i * 7) % 23),
            _ => format!("Z{}", i % 11),
        })
        .collect();
    let r: Vec<Vec<String>> = vals.iter().map(|v| vec![v.clone()]).collect();
    let out = sorted(&r, false);
    assert!(out.windows(2).all(|w| compare_cells(&w[0], &w[1]) != std::cmp::Ordering::Greater));
}

/// #778: preview, Finish & Merge and labels use only the filtered records, in sort order; a
/// filter matching nothing leaves nothing to merge.
#[test]
fn merge_uses_only_filtered_records_in_sort_order() {
    let mut s = people();
    s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
    s.run("mailings.filterRecipients", &json!({"rules": [{"field": "Email", "op": "notBlank"}], "sort": [{"field": "Age", "descending": true}]}))
        .unwrap();
    s.run("mailings.preview", &json!({"value": true})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Grace"));
    s.run("mailings.next", &json!({})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).contains("Ada"));
    assert_eq!(s.run("mailings.next", &json!({})).unwrap()["of"], 3);
    assert!(s.run("mailings.findRecipient", &json!({"text": "Alan"})).is_err());
    let r = s.run("mailings.finish", &json!({})).unwrap();
    assert_eq!(r["records"], 3);
    let t = s.doc.plain_text(StoryRef::Body);
    let (g, a, l) = (t.find("Grace").unwrap(), t.find("Ada").unwrap(), t.find("Linus").unwrap());
    assert!(g < a && a < l, "{t}");
    assert!(!t.contains("Alan") && !t.contains("Katherine"), "{t}");
    let mut s = people();
    s.run("mailings.filterRecipients", &json!({"rules": [{"field": "City", "op": "equal", "value": "Atlantis"}]})).unwrap();
    assert!(s.merge.rows.is_empty());
    assert!(s.run("mailings.finish", &json!({})).is_err());
    assert!(s.run("mailings.editRecipients", &json!({})).is_ok());
}
