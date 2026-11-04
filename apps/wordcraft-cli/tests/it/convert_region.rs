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
