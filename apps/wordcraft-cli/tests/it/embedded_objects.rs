//! #5393: `text`, `convert` and `run --save` say on stderr when the output doesn't hold an
//! embedded object's content, and write their output as before.

use std::process::Command;
use std::sync::Arc;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
}

/// A document (saved as WordCraft JSON) whose second paragraph holds an embedded Word document.
fn with_object(path: &std::path::Path) {
    use wordcraft_doc::graphic::Embedded;
    use wordcraft_doc::{Block, CharProps, InlineObject, Paragraph};
    let mut doc = wordcraft_doc::Document::new();
    let mut p = Paragraph::with_text("Attachment: ", CharProps::default());
    let object = InlineObject::Image {
        media: "image1.png".into(),
        w: 40.0,
        h: 30.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: Some(Arc::new(Embedded {
            xml: r#"<w:object><o:OLEObject Type="Embed" ProgID="Word.Document.12"/></w:object>"#.into(),
            refs: Vec::new(),
        })),
    };
    let end = p.text.len();
    p.insert_object(end, object, &CharProps::default()).unwrap();
    doc.body = vec![Arc::new(Block::Para(Paragraph::with_text("Body text.", CharProps::default()))), Arc::new(Block::Para(p))];
    std::fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
}

#[test]
fn outputs_without_an_objects_content_warn_on_stderr() {
    let dir = std::env::temp_dir().join(format!("wc-cli-5393-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.json");
    with_object(&input);
    let input = input.to_str().unwrap();
    let left_out = "warning: 1 embedded object (Word.Document.12) is left out; its content is not in";
    let pictured = "warning: 1 embedded object (Word.Document.12) is shown as its picture; its content is not in";

    // `text`: the text on stdout as before, the warning on stderr, success.
    let out = cli().args(["text", input]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Body text.") && stdout.contains("Attachment:"), "{stdout}");
    assert!(!stdout.contains("warning"), "the warning isn't in the text: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(&format!("{left_out} the text")), "{stderr}");

    // `convert` to plain text (no picture) and to HTML by an alias (the picture).
    for (name, warning) in [("out.txt", left_out), ("out.htm", pictured), ("out.markdown", left_out)] {
        let o = dir.join(name);
        let out = cli().args(["convert", input, o.to_str().unwrap()]).output().unwrap();
        assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(&format!("{warning} {}", o.display())), "{name}: {stderr}");
        let written = std::fs::read_to_string(&o).unwrap();
        assert!(written.contains("Body text."), "{name}: {written}");
    }

    // `run --save`: each output that loses the object says so; a .docx, which keeps it, doesn't.
    let pdf = dir.join("run.pdf");
    let docx = dir.join("run.docx");
    let out = cli().args(["run", "--file", input, "--save", pdf.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(&format!("{pictured} {}", pdf.display())), "{stderr}");
    let out = cli().args(["run", "--file", input, "--save", docx.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("warning"), "{stderr}");
    assert!(pdf.is_file() && docx.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
