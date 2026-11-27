//! #6101: the commands `wordcraft-cli` runs for a caller (`run --cmd`, `run --script`,
//! `zotero --cmd`) are automation, so they can't attach a template on another machine, which Word
//! would fetch when the saved file is opened. A template on this computer still goes in.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::json;

/// What the refusal says.
const REFUSED: &str = "automation path rejected: the attached template must be a file on this computer";

/// Templates on another machine, in `\\host`, `\\?\UNC\` and forward-slash spelling.
const REMOTE: [&str; 3] = [r"\\attacker\share\t.dotm", r"\\?\UNC\attacker\share\t.dotm", "//attacker/share/t.dotm"];

/// `wordcraft-cli` with `args`, run to the end.
fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(args).output().expect("wordcraft-cli runs")
}

/// A fresh scratch folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wc-cli-6101-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Assert `out` failed because the attached template was refused, and saved nothing to `saved`.
fn refused(out: &Output, saved: &Path, what: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{what}: {stderr}");
    assert!(stderr.contains(REFUSED), "{what}: {stderr}");
    assert!(!saved.exists(), "{what}: nothing is saved");
}

/// The template the document saved at `path` is attached to.
fn attached(path: &str) -> String {
    let listed = cli(&["run", "--file", path, "--cmd", "tools.templates", "--print"]);
    assert!(listed.status.success(), "{}", String::from_utf8_lossy(&listed.stderr));
    String::from_utf8_lossy(&listed.stdout).into_owned()
}

#[test]
fn run_commands_cannot_attach_a_template_on_another_machine() {
    let dir = folder("run");
    let saved = dir.join("out.docx");
    let out_s = saved.to_str().unwrap();
    for remote in REMOTE {
        for command in ["tools.attachTemplate", "file.newFromTemplate"] {
            let cmd = format!("{command}={}", json!({"path": remote, "data": ""}));
            refused(&cli(&["run", "--cmd", &cmd, "--save", out_s]), &saved, &format!("--cmd {cmd}"));
        }
        let script = dir.join("job.json");
        let job = json!({"commands": [{"command": "tools.attachTemplate", "params": {"path": remote}}], "save": out_s});
        std::fs::write(&script, job.to_string()).unwrap();
        refused(&cli(&["run", "--script", script.to_str().unwrap()]), &saved, &format!("--script {remote}"));
    }

    // A template on this computer still goes in, with either command.
    let local = r"C:\Templates\Report.dotx";
    for command in ["tools.attachTemplate", "file.newFromTemplate"] {
        let cmd = format!("{command}={}", json!({"path": local, "data": ""}));
        let made = cli(&["run", "--cmd", &cmd, "--save", out_s]);
        assert!(made.status.success(), "{command}: {}", String::from_utf8_lossy(&made.stderr));
        let listed = attached(out_s);
        assert!(listed.contains(&json!({"template": local, "linkStyles": false}).to_string()), "{command}: {listed}");
        std::fs::remove_file(&saved).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `zotero --cmd` runs its commands before it talks to Zotero: they are refused the same way.
#[test]
fn zotero_commands_cannot_attach_a_template_on_another_machine() {
    let dir = folder("zotero");
    let doc = dir.join("in.docx");
    let doc_s = doc.to_str().unwrap();
    let made = cli(&["run", "--save", doc_s]);
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let saved = dir.join("out.docx");
    for remote in REMOTE {
        let cmd = format!("tools.attachTemplate={}", json!({"path": remote}));
        // Port 9 (discard): if the command went through, the failure would be Zotero's, not ours.
        let out = cli(&["zotero", "refresh", doc_s, "--cmd", &cmd, "--port", "9", "--save", saved.to_str().unwrap()]);
        refused(&out, &saved, &format!("zotero --cmd {cmd}"));
    }
    assert!(!attached(doc_s).contains("attacker"), "the document is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
