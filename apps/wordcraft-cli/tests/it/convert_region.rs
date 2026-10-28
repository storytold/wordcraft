//! `wordcraft-cli convert page.html out.docx` puts an HTML file, which gives no page size, on the
//! system region's paper, as the app does: A4 in Italy, Letter in the US (#1381).

use std::process::Command;

const HTML: &str = "<html><body><h2>Premessa</h2><table border=1><thead><tr><th>Indicatore</th><th>Valore</th></tr></thead>\
<tbody><tr><td>Audit</td><td style=\"text-align:right;\">4</td></tr></tbody></table></body></html>";

/// Run the CLI in `region` with `args` (`{src}` is the sample HTML, `{out}` a JSON file it
/// writes) and read back the page size of the JSON it saved.
fn page_size_of(region: &str, args: &[&str]) -> (f64, f64) {
    let dir = std::env::temp_dir().join(format!("wordcraft-cli-region-{region}-{}-{}", args.len(), std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, out) = (dir.join("page.html"), dir.join("out.json"));
    std::fs::write(&src, HTML).unwrap();
    let args: Vec<String> = args.iter().map(|a| a.replace("{src}", &src.to_string_lossy()).replace("{out}", &out.to_string_lossy())).collect();
    let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(&args).env("WORDCRAFT_REGION", region).env_remove("LC_ALL").output().unwrap();
    assert!(run.status.success(), "{args:?}: {}", String::from_utf8_lossy(&run.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let sp = &doc["lastSection"];
    (sp["pageW"].as_f64().unwrap(), sp["pageH"].as_f64().unwrap())
}

/// Convert the sample to JSON in `region` and read back the page size.
fn page_size(region: &str) -> (f64, f64) {
    page_size_of(region, &["convert", "{src}", "{out}"])
}

fn is_a4((w, h): (f64, f64)) -> bool {
    (w - 595.28).abs() < 0.01 && (h - 841.89).abs() < 0.01
}

#[test]
fn html_converts_onto_the_region_paper() {
    let (w, h) = page_size("it-IT");
    assert!(is_a4((w, h)), "A4 in Italy: {w} x {h}");
    assert_eq!(page_size("en-US"), (612.0, 792.0), "Letter in the US");
}

#[test]
fn every_run_session_follows_the_region() {
    // `run --file`, `run --template` and a bare `run` (with or without `file.new`) all start a session in the system region,
    // so `file.new` gives the same paper whichever way the session began.
    for args in [
        &["run", "--cmd", "file.new", "--save", "{out}"][..],
        &["run", "--cmd", "text.insert={\"text\":\"x\"}", "--save", "{out}"][..],
        &["run", "--template", "blank", "--save", "{out}"][..],
        &["run", "--file", "{src}", "--cmd", "file.new", "--save", "{out}"][..],
    ] {
        assert!(is_a4(page_size_of("it-IT", args)), "A4 in Italy: {args:?} gave {:?}", page_size_of("it-IT", args));
        assert_eq!(page_size_of("en-US", args), (612.0, 792.0), "Letter in the US: {args:?}");
    }
}

#[test]
fn headless_mcp_uses_the_region_for_initial_new_and_unsized_documents() {
    use std::io::Write;
    use std::process::Stdio;

    for (region, expected) in [("it-IT", (595.28, 841.89)), ("en-US", (612.0, 792.0))] {
        let dir = std::env::temp_dir().join(format!("wordcraft-mcp-region-{region}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("page.html");
        std::fs::write(&src, HTML).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
            .args(["mcp", "--author", "Region Author", "--automation-read-root"])
            .arg(&dir)
            .arg("--automation-write-root")
            .arg(&dir)
            .env("WORDCRAFT_REGION", region)
            .env_remove("LC_ALL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let requests = [
            serde_json::json!({"name":"save_document", "arguments":{"path":dir.join("initial.json")}}),
            serde_json::json!({"name":"execute", "arguments":{"command":"file.new", "params":{}}}),
            serde_json::json!({"name":"save_document", "arguments":{"path":dir.join("new.json")}}),
            serde_json::json!({"name":"open_document", "arguments":{"path":src}}),
            serde_json::json!({"name":"save_document", "arguments":{"path":dir.join("opened.json")}}),
            serde_json::json!({"name":"execute", "arguments":{"command":"layout.size", "params":{"name":"Legal"}}}),
            serde_json::json!({"name":"save_document", "arguments":{"path":dir.join("explicit.json")}}),
            serde_json::json!({"name":"open_document", "arguments":{"path":dir.join("explicit.json")}}),
            serde_json::json!({"name":"save_document", "arguments":{"path":dir.join("reopened.json")}}),
        ];
        for (i, params) in requests.iter().enumerate() {
            writeln!(input, "{}", serde_json::json!({"jsonrpc":"2.0", "id":i, "method":"tools/call", "params":params})).unwrap();
        }
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let replies: Vec<serde_json::Value> = String::from_utf8(output.stdout).unwrap().lines().map(|s| serde_json::from_str(s).unwrap()).collect();
        assert_eq!(replies.len(), requests.len());
        for reply in replies {
            assert!(reply.get("error").is_none() && reply["result"]["isError"] != true, "{reply}");
        }
        for name in ["initial", "new", "opened", "reopened"] {
            let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.json"))).unwrap()).unwrap();
            let (w, h) = (doc["lastSection"]["pageW"].as_f64().unwrap(), doc["lastSection"]["pageH"].as_f64().unwrap());
            let expected = if name == "reopened" { (612.0, 1008.0) } else { expected };
            assert!((w - expected.0).abs() < 0.01 && (h - expected.1).abs() < 0.01, "{region} {name}: {w} x {h}, expected {expected:?}");
            assert_eq!(doc["core"]["lastModifiedBy"], "Region Author", "{region} {name}");
        }
    }
}
