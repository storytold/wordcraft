//! Explicit field refresh for headless conversions; ordinary conversion keeps cached results,
//! except in a document with fields marked for updating (`w:dirty`).
use std::process::Command;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, Path, Pos, StoryRef};

/// A two-page document whose first paragraph holds a locked and an unlocked `PAGEREF` to the
/// second page's bookmark, both showing a stale "104", marked for updating or not.
fn pageref_document(dirty: bool) -> Document {
    let mut doc = Document::from_text("Contents\nTarget");
    doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
    doc.insert_object(&Pos::body(1, 0), InlineObject::BookmarkStart { name: "target".into() }, &Default::default()).unwrap();
    for locked in [true, false] {
        doc.insert_object(
            &Pos::body(0, 0),
            InlineObject::Field { instr: "PAGEREF target".into(), result: "104".into(), locked, dirty, code: false, form: String::new() },
            &Default::default(),
        )
        .unwrap();
    }
    doc
}

/// The results of the fields in `doc`'s first paragraph.
fn first_results(doc: &Document) -> Vec<String> {
    doc.para(StoryRef::Body, &Path::top(0))
        .unwrap()
        .objects
        .iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

/// #6771: fields marked for updating (`w:dirty`, as pandoc's `--toc` writes its empty table of
/// contents) are updated by a plain conversion, as Word asks to when it opens the file: every
/// unlocked field, the table of contents too, and the mark is cleared. A locked field keeps its
/// result.
#[test]
fn convert_updates_a_document_with_fields_marked_dirty() {
    let dir = std::env::temp_dir().join(format!("wc-convert-dirty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc = pageref_document(true);
    // A table of contents with nothing in it yet, above a heading.
    doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.style = Some("Heading1".into());
    let toc = r#"TOC \o "1-3" \h \z \u"#;
    let field = InlineObject::Field { instr: toc.into(), result: String::new(), locked: false, dirty: true, code: false, form: String::new() };
    doc.insert_object(&Pos::body(0, 0), field, &Default::default()).unwrap();
    let input = dir.join("in.json");
    std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let output = dir.join("out.json");
    let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).arg("convert").arg(&input).arg(&output).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let out: Document = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    let text = out.plain_text(StoryRef::Body);
    assert_eq!(text.matches("Target").count(), 2, "the heading and its entry in the table of contents: {text:?}");
    let pagerefs: Vec<(String, bool)> = out
        .para_paths(StoryRef::Body)
        .iter()
        .flat_map(|path| out.para(StoryRef::Body, path).unwrap().objects.clone())
        .filter_map(|o| match o {
            InlineObject::Field { instr, result, dirty, .. } if instr == "PAGEREF target" => Some((result, dirty)),
            _ => None,
        })
        .collect();
    assert_eq!(pagerefs, [("2".to_string(), false), ("104".to_string(), true)], "the unlocked one updated, the locked one kept");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn convert_refresh_is_opt_in() {
    let dir = std::env::temp_dir().join(format!("wc-convert-fields-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Not marked for updating: a plain conversion keeps the stored results.
    let doc = pageref_document(false);
    let input = dir.join("in.json");
    std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let docx = dir.join("in.docx");
    let make = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).arg("convert").arg(&input).arg(&docx).output().unwrap();
    assert!(make.status.success(), "{}", String::from_utf8_lossy(&make.stderr));
    for refresh in [false, true] {
        let output = dir.join(if refresh { "refreshed.json" } else { "cached.json" });
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
        cmd.arg("convert").arg(&docx).arg(&output);
        if refresh {
            cmd.arg("--update-fields");
        }
        let run = cmd.output().unwrap();
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        let out: Document = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        assert_eq!(first_results(&out), if refresh { vec!["2", "104"] } else { vec!["104", "104"] });
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Converting a protected document with `--update-fields` skips updating fields with a warning,
/// retains the cached field result, and preserves the protection mode on output.
#[test]
fn convert_protected_document_with_update_fields_skips_with_warning() {
    let dir = std::env::temp_dir().join(format!("wc-convert-fields-prot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (mode, expected_warning) in [("readOnly", "protected"), ("forms", "protected"), ("comments", "comments")] {
        let mut doc = Document::from_text("Contents\nTarget");
        doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
        doc.insert_object(&Pos::body(1, 0), InlineObject::BookmarkStart { name: "target".into() }, &Default::default()).unwrap();
        doc.insert_object(
            &Pos::body(0, 0),
            InlineObject::Field {
                instr: "PAGEREF target".into(),
                result: "104".into(),
                locked: false,
                dirty: true,
                code: false,
                form: String::new(),
            },
            &Default::default(),
        )
        .unwrap();
        doc.settings.protection = Some(mode.into());
        let input = dir.join(format!("in_{mode}.json"));
        std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let docx = dir.join(format!("in_{mode}.docx"));
        let make = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).arg("convert").arg(&input).arg(&docx).output().unwrap();
        assert!(make.status.success(), "{}", String::from_utf8_lossy(&make.stderr));

        let output = dir.join(format!("out_{mode}.json"));
        let _ = std::fs::remove_file(&output);

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
        cmd.arg("convert").arg(&docx).arg(&output).arg("--update-fields");
        let run = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "exit status was failure for mode {mode}: {stderr}");
        assert!(stderr.contains("warning:"), "stderr missing warning for mode {mode}: {stderr}");
        assert!(stderr.contains(expected_warning), "stderr missing '{expected_warning}' for mode {mode}: {stderr}");
        assert!(output.exists(), "output file was not created for mode {mode}");

        let out: Document = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        let results: Vec<_> = out
            .para(StoryRef::Body, &Path::top(0))
            .unwrap()
            .objects
            .iter()
            .filter_map(|o| match o {
                InlineObject::Field { result, .. } => Some(result.as_str()),
                _ => None,
            })
            .collect();
        // Result should remain "104" (cached) because the field could not be updated under protection
        assert_eq!(results, vec!["104"], "field result was unexpectedly modified for mode {mode}");
        assert_eq!(out.settings.protection.as_deref(), Some(mode), "protection mode was not preserved for mode {mode}");
    }

    std::fs::remove_dir_all(dir).unwrap();
}
