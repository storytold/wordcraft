//! With LawCraft on, `convert` and `render` run the applied court profile's checks on the export,
//! as the app does, and report their findings on standard error; the export still goes through.

use std::process::{Command, Output};

fn cli(args: &[&str], law: bool, home: &std::path::Path) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    c.args(args).env("WORDCRAFT_NO_PREFS", "1").env("HOME", home).env("XDG_CONFIG_HOME", home).env("APPDATA", home);
    if law {
        c.env("WORDCRAFT_LAW", "1");
    } else {
        c.env_remove("WORDCRAFT_LAW");
    }
    c.output().unwrap()
}

#[test]
fn convert_reports_the_applied_profile_s_findings() {
    let dir = std::env::temp_dir().join(format!("wc-cli-law-convert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let brief = dir.join("brief.docx");
    let out = cli(
        &["run", "--cmd", r#"law.profiles.apply={"id":"missouri-appellate-brief","doc":"brief"}"#, "--save", brief.to_str().unwrap()],
        true,
        &dir,
    );
    assert!(out.status.success(), "apply: {}", String::from_utf8_lossy(&out.stderr));

    // An empty brief misses the profile's required sections: the check says so.
    let pdf = dir.join("brief.pdf");
    let out = cli(&["convert", brief.to_str().unwrap(), pdf.to_str().unwrap()], true, &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(pdf.exists(), "the export still went through");
    assert!(stderr.lines().any(|l| l.starts_with("error: ") || l.starts_with("warning: ")), "findings reported: {stderr}");

    // With LawCraft off the file is just converted.
    let out = cli(&["convert", brief.to_str().unwrap(), pdf.to_str().unwrap()], false, &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(!stderr.lines().any(|l| l.starts_with("error: ") || l.starts_with("warning: ")), "no checks without LawCraft: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
