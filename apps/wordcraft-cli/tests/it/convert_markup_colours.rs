//! #5233: `convert --markup` takes the tracked-change colours on the command line.
use std::process::Command;
use wordcraft_doc::{CharProps, Document, Paragraph, Revision, RevisionKind, para_block};

#[test]
fn convert_colours_tracked_changes_from_flags() {
    let dir = std::env::temp_dir().join(format!("wc-convert-colours-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc = Document::new();
    // Saving stamps an empty creation date with the time, which the PDF carries: a fixed one
    // keeps runs a second apart byte-identical.
    doc.core.created = "2024-05-06T07:08:09Z".into();
    doc.revisions.push(Revision { kind: RevisionKind::Delete, author: "Ada".into(), date: String::new(), move_name: None });
    doc.revisions.push(Revision { kind: RevisionKind::Insert, author: "Ada".into(), date: String::new(), move_name: None });
    let mut p = Paragraph::with_text("Governed by ", CharProps::default());
    let n = p.len();
    p.insert_text(n, "Kansas ", &CharProps { del: Some(0), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_text(n, "Missouri law.", &CharProps { ins: Some(1), ..Default::default() }).unwrap();
    doc.body = vec![para_block(p)];
    let input = dir.join("in.json");
    std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let convert = |name: &str, flags: &[&str]| {
        let out = dir.join(name);
        let run =
            Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).arg("convert").arg(&input).arg(&out).arg("--markup").args(flags).output().unwrap();
        (run.status.success(), String::from_utf8_lossy(&run.stderr).into_owned(), std::fs::read(&out).ok())
    };
    let (ok, err, by_author) = convert("by-author.pdf", &[]);
    assert!(ok, "{err}");
    let (ok, err, again) = convert("again.pdf", &["--insert-color", "byAuthor", "--delete-color", "byAuthor"]);
    assert!(ok, "{err}");
    assert_eq!(again, by_author, "byAuthor is the default");
    let (ok, err, coloured) = convert("coloured.pdf", &["--insert-color", "0000FF", "--delete-color", "FF0000"]);
    assert!(ok, "{err}");
    assert!(coloured.is_some() && coloured != by_author, "the colours didn't reach the PDF");
    // A colour that isn't one fails instead of exporting by author.
    let (ok, err, wrote) = convert("bad.pdf", &["--insert-color", "blueish"]);
    assert!(!ok && err.contains("insertColor") && wrote.is_none(), "{err}");
    // A flag without its value, or given twice, fails too instead of being ignored.
    for flags in [&["--insert-color"][..], &["--delete-color", "--update-fields"], &["--insert-color", "0000FF", "--insert-color", "blueish"]] {
        let (ok, err, wrote) = convert("bad.pdf", flags);
        assert!(!ok && wrote.is_none(), "{flags:?} was accepted: {err}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
