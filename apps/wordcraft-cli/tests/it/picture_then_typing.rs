//! `wordcraft-cli run`: a picture inserted by `--cmd insert.picture` stays in the saved file when
//! the next commands type (#965).

use std::process::Command;

/// A 1x1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn typing_after_insert_picture_keeps_it_in_the_saved_file() {
    let dir = std::env::temp_dir().join(format!("wordcraft-cli-965-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (pic, out) = (dir.join("pic.png"), dir.join("p.docx"));
    std::fs::write(&pic, PNG).unwrap();
    let picture = format!(r#"insert.picture={{"path":{},"width":250}}"#, serde_json::json!(pic.to_string_lossy()));
    let cmds = [r#"text.insert={"text":"x"}"#, "text.newParagraph", &picture, "text.newParagraph", r#"text.insert={"text":"cap"}"#];
    let mut cli = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    cli.args(["run", "--template", "blank"]);
    for c in cmds {
        cli.args(["--cmd", c]);
    }
    let run = cli.arg("--save").arg(&out).output().unwrap();
    let docx = std::fs::read(&out);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(run.status.success(), "stderr: {}", String::from_utf8_lossy(&run.stderr));
    assert!(docx.unwrap().windows(11).any(|w| w == b"word/media/"), "the saved file keeps the picture");
}
