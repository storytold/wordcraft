//! `wordcraft-cli run` browses the Library folders the app keeps in its preferences (#2868), and
//! inserts from them; `WORDCRAFT_NO_PREFS` leaves them out.

use std::process::Command;

/// A 1x1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn cli(profile: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    c.env("WORDCRAFT_PROFILE_DIR", profile).env_remove("WORDCRAFT_NO_PREFS");
    c
}

#[test]
fn run_browses_and_inserts_from_the_apps_library_folders() {
    let base = std::env::temp_dir().join(format!("wordcraft-cli-library-{}", std::process::id()));
    let (profile, pictures) = (base.join("profile"), base.join("pictures"));
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(&pictures).unwrap();
    std::fs::write(pictures.join("dot.png"), PNG).unwrap();
    let ui = serde_json::json!({"editing": {"libraryFolders": [pictures.to_string_lossy()]}, "language": "auto"});
    std::fs::write(profile.join("ui.json"), ui.to_string()).unwrap();
    let file = serde_json::json!(pictures.join("dot.png").to_string_lossy());
    let out = base.join("out.docx");
    let run = cli(&profile)
        .args(["run", "--template", "blank", "--print", "--cmd", "library.list"])
        .args(["--cmd", &format!(r#"library.insert={{"file":{file}}}"#)])
        .arg("--save")
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let docx = std::fs::read(&out);
    let quiet =
        cli(&profile).env("WORDCRAFT_NO_PREFS", "1").args(["run", "--template", "blank", "--print", "--cmd", "library.folders"]).output().unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert!(run.status.success(), "stderr: {}", String::from_utf8_lossy(&run.stderr));
    assert!(stdout.contains("dot.png") && stdout.contains("\"pictureCount\":1"), "{stdout}");
    assert!(docx.unwrap().windows(11).any(|w| w == b"word/media/"), "the saved file has the picture");
    assert!(quiet.status.success(), "stderr: {}", String::from_utf8_lossy(&quiet.stderr));
    assert!(String::from_utf8_lossy(&quiet.stdout).contains("library.folders: []"), "{}", String::from_utf8_lossy(&quiet.stdout));
}
