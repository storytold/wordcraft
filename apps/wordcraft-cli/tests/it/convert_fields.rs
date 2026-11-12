//! Explicit field refresh for headless conversions; ordinary conversion keeps cached results.
use std::process::Command;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, Path, Pos, StoryRef};

#[test]
fn convert_refresh_is_opt_in() {
    let dir = std::env::temp_dir().join(format!("wc-convert-fields-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc = Document::from_text("Contents\nTarget");
    doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
    doc.insert_object(&Pos::body(1, 0), InlineObject::BookmarkStart { name: "target".into() }, &Default::default()).unwrap();
    for locked in [true, false] {
        doc.insert_object(
            &Pos::body(0, 0),
            InlineObject::Field { instr: "PAGEREF target".into(), result: "104".into(), locked, dirty: true, code: false, form: String::new() },
            &Default::default(),
        )
        .unwrap();
    }
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
        assert_eq!(results, if refresh { vec!["2", "104"] } else { vec!["104", "104"] });
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
