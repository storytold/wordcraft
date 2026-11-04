//! `wordcraft-cli mcp --automation-read-root <dir> --automation-write-root <dir>` (#769): through
//! the MCP protocol, files open and load only from inside the read root and are saved and
//! exported only inside the write root; paths that leave a root (`..`, another folder, a sibling
//! with the same prefix, a link) are refused, and nothing is written outside.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wordcraft_engine::file_access::AutomationRoots;
use wordcraft_mcp::{Headless, Server};

/// A 1x1 PNG, so `insert.picture` has something real to read.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// A fresh folder holding `in` (the read root, with `a.txt`, `people.csv` and `pic.png`), `out` (the
/// write root), `outside` (with `secret.txt`) and `in2`, and a server confined to `in` and `out`
/// (either may be left out).
fn confined(tag: &str, read: bool, write: bool) -> (Server, PathBuf) {
    let base = std::env::temp_dir().join(format!("wc-mcp-roots-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "out", "outside", "in2"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    std::fs::write(base.join("in/a.txt"), "Inside text").unwrap();
    std::fs::write(base.join("in/people.csv"), "Name\nAda\n").unwrap();
    std::fs::write(base.join("in/pic.png"), PNG).unwrap();
    std::fs::write(base.join("outside/secret.txt"), "TOP SECRET").unwrap();
    std::fs::write(base.join("outside/pic.png"), PNG).unwrap();
    std::fs::write(base.join("outside/t.docx"), "x").unwrap();
    std::fs::write(base.join("in2/c.txt"), "Sibling").unwrap();
    let (in_dir, out_dir) = (base.join("in"), base.join("out"));
    let roots = AutomationRoots::new(read.then_some(in_dir.as_path()), write.then_some(out_dir.as_path())).unwrap();
    (Server::new(Box::new(Headless::default().with_automation_roots(roots))), base)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `tools/call` -> (isError, the first text block).
fn call(server: &mut Server, name: &str, args: Value) -> (bool, String) {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string();
    let reply: Value = serde_json::from_str(&server.handle_line(&line).unwrap()).unwrap();
    let result = &reply["result"];
    let text = result["content"].as_array().unwrap().iter().find_map(|c| c["text"].as_str()).unwrap_or_default().to_string();
    (result["isError"] == true, text)
}

fn ok(server: &mut Server, name: &str, args: Value) -> String {
    let (error, text) = call(server, name, args.clone());
    assert!(!error, "{name} {args}: {text}");
    text
}

fn refused(server: &mut Server, name: &str, args: Value, why: &str) {
    let (error, text) = call(server, name, args.clone());
    assert!(error && text.contains(why), "{name} {args} should be refused ({why}): {text}");
}

fn run(server: &mut Server, command: &str, params: Value) -> (bool, String) {
    call(server, "execute", json!({"command": command, "params": params}))
}

#[test]
fn files_open_only_from_inside_the_read_root() {
    let (mut server, base) = confined("open", true, true);
    let sep = std::path::MAIN_SEPARATOR;
    ok(&mut server, "open_document", json!({"path": s(&base.join("in/a.txt"))}));
    assert!(ok(&mut server, "get_text", json!({})).contains("Inside text"));

    refused(&mut server, "open_document", json!({"path": s(&base.join("outside/secret.txt"))}), "outside the read root");
    refused(
        &mut server,
        "open_document",
        json!({"path": format!("{}{sep}..{sep}outside{sep}secret.txt", s(&base.join("in")))}),
        "outside the read root",
    );
    refused(&mut server, "open_document", json!({"path": s(&base.join("in2/c.txt"))}), "outside the read root");
    // The refused open left the document it had.
    assert!(ok(&mut server, "get_text", json!({})).contains("Inside text"));
    // The write root isn't readable: the two kinds of access are separate.
    std::fs::write(base.join("out/d.txt"), "d").unwrap();
    refused(&mut server, "open_document", json!({"path": s(&base.join("out/d.txt"))}), "outside the read root");
    // A relative path is the working directory's (the crate's folder here): outside.
    refused(&mut server, "open_document", json!({"path": "Cargo.toml"}), "outside the read root");
}

#[test]
fn commands_that_read_a_path_are_held_to_the_read_root() {
    let (mut server, base) = confined("reads", true, true);
    let outside = s(&base.join("outside/secret.txt"));
    // The injected-document scenario from the issue: pull a file in from outside, then read it back.
    for (command, params) in [
        ("insert.textFromFile", json!({"path": outside})),
        ("insert.object", json!({"path": outside})),
        ("insert.picture", json!({"path": s(&base.join("outside/pic.png"))})),
        ("mailings.recipients", json!({"path": outside})),
        ("file.newFromTemplate", json!({"path": outside})),
        ("file.listTemplates", json!({"dir": s(&base.join("outside"))})),
        ("review.compare", json!({"path": outside})),
        ("review.compare", json!({"text": "x", "original": outside})),
        ("file.open", json!({"path": outside})),
        ("styles.organizer", json!({"path": s(&base.join("outside/t.docx"))})),
        ("tools.updateStyles", json!({"path": s(&base.join("outside/t.docx"))})),
        ("styles.copyFrom", json!({"path": s(&base.join("outside/t.docx")), "styles": ["Heading 1"]})),
    ] {
        let (error, text) = run(&mut server, command, params);
        assert!(error && text.contains("outside the read root"), "{command}: {text}");
    }
    assert!(!ok(&mut server, "get_text", json!({})).contains("TOP SECRET"));

    // The same commands work on files inside the root.
    let (error, text) = run(&mut server, "insert.textFromFile", json!({"path": s(&base.join("in/a.txt"))}));
    assert!(!error, "{text}");
    assert!(ok(&mut server, "get_text", json!({})).contains("Inside text"));
    let (error, text) = run(&mut server, "insert.picture", json!({"path": s(&base.join("in/pic.png"))}));
    assert!(!error, "{text}");
    let (error, text) = run(&mut server, "mailings.recipients", json!({"path": s(&base.join("in/people.csv"))}));
    assert!(!error, "{text}");
}

#[test]
fn saves_and_exports_go_only_into_the_write_root() {
    let (mut server, base) = confined("write", true, true);
    let sep = std::path::MAIN_SEPARATOR;
    ok(&mut server, "type_text", json!({"text": "Hello"}));

    ok(&mut server, "save_document", json!({"path": s(&base.join("out/doc.docx"))}));
    assert!(base.join("out/doc.docx").is_file());
    ok(&mut server, "save_document", json!({"path": s(&base.join("out/doc.pdf"))}));
    ok(&mut server, "save_document", json!({"path": s(&base.join("out/page.png"))}));
    assert!(base.join("out/page.png").is_file());
    let (error, text) = run(&mut server, "file.exportPdf", json!({"path": s(&base.join("out/two"))}));
    assert!(!error, "{text}");
    // Printing to a PDF (#641) writes a file too.
    let (error, text) = run(&mut server, "file.printPdf", json!({"path": s(&base.join("out/printed"))}));
    assert!(!error, "{text}");
    assert!(base.join("out/printed.pdf").is_file());

    refused(&mut server, "save_document", json!({"path": s(&base.join("in/doc.docx"))}), "outside the write root");
    refused(&mut server, "save_document", json!({"path": s(&base.join("outside/doc.html"))}), "outside the write root");
    refused(&mut server, "save_document", json!({"path": s(&base.join("outside/page.png"))}), "outside the write root");
    refused(
        &mut server,
        "save_document",
        json!({"path": format!("{}{sep}..{sep}outside{sep}up.docx", s(&base.join("out")))}),
        "outside the write root",
    );
    let (error, text) = run(&mut server, "mailings.recipients", json!({"csv": "Name\nAda\n"}));
    assert!(!error, "{text}");
    for (command, params) in [
        ("file.saveAs", json!({"path": s(&base.join("outside/x.docx"))})),
        ("file.exportPdf", json!({"path": s(&base.join("outside/x.pdf"))})),
        ("file.printPdf", json!({"path": s(&base.join("outside/printed.pdf"))})),
        ("file.printPdf", json!({"path": format!("{}{sep}..{sep}outside{sep}up", s(&base.join("out")))})),
        ("file.exportPng", json!({"path": s(&base.join("outside/x.png"))})),
        ("file.saveTemplate", json!({"path": s(&base.join("outside/x.dotx"))})),
        ("mailings.finish", json!({"path": s(&base.join("outside/merged.docx"))})),
    ] {
        let (error, text) = run(&mut server, command, params);
        assert!(error && text.contains("outside the write root"), "{command}: {text}");
    }
    // The document saved inside the write root saves again in place.
    ok(&mut server, "save_document", json!({}));
    let mut written: Vec<String> =
        std::fs::read_dir(base.join("outside")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    assert_eq!(written, ["pic.png", "secret.txt", "t.docx"], "nothing was written outside the roots");
    assert!(!base.join("in/doc.docx").exists());
}

#[test]
fn a_root_left_out_grants_none_of_its_access() {
    let (mut server, base) = confined("read-only", true, false);
    ok(&mut server, "open_document", json!({"path": s(&base.join("in/a.txt"))}));
    refused(&mut server, "save_document", json!({"path": s(&base.join("out/doc.docx"))}), "write authority is absent");
    refused(&mut server, "save_document", json!({"path": s(&base.join("in/doc.docx"))}), "write authority is absent");
    assert!(!base.join("out/doc.docx").exists());

    let (mut server, base) = confined("write-only", false, true);
    refused(&mut server, "open_document", json!({"path": s(&base.join("in/a.txt"))}), "read authority is absent");
    ok(&mut server, "save_document", json!({"path": s(&base.join("out/doc.docx"))}));
}

#[test]
fn links_out_of_a_root_are_refused() {
    let (mut server, base) = confined("links", true, true);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(base.join("outside"), base.join("out/away")).is_ok()
        && std::os::unix::fs::symlink(base.join("outside/secret.txt"), base.join("in/leak.txt")).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(base.join("outside"), base.join("out/away")).is_ok()
        && std::os::windows::fs::symlink_file(base.join("outside/secret.txt"), base.join("in/leak.txt")).is_ok();
    #[cfg(not(any(unix, windows)))]
    let linked = false;
    // Windows needs Developer Mode or elevation to make links.
    if !linked {
        eprintln!("skipped: symbolic links can't be made here");
        return;
    }
    refused(&mut server, "open_document", json!({"path": s(&base.join("in/leak.txt"))}), "outside the read root");
    refused(&mut server, "save_document", json!({"path": s(&base.join("out/away/doc.docx"))}), "outside the write root");
    assert!(!base.join("outside/doc.docx").exists());
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[cfg(unix)]
#[test]
fn a_link_outside_the_write_root_into_it_is_not_written() {
    // #3735: the link's target is inside the write root, but a save replaces the link itself,
    // and its temporary file would sit beside it, outside.
    let (mut server, base) = confined("link-in", true, true);
    std::fs::write(base.join("out/doc.docx"), "inside").unwrap();
    std::os::unix::fs::symlink(base.join("out/doc.docx"), base.join("outside/alias.docx")).unwrap();
    let before = names(&base.join("outside"));
    ok(&mut server, "type_text", json!({"text": "Hello"}));
    refused(&mut server, "save_document", json!({"path": s(&base.join("outside/alias.docx"))}), "outside the write root");
    let (error, text) = run(&mut server, "file.exportPdf", json!({"path": s(&base.join("outside/alias.docx"))}));
    assert!(error && text.contains("outside the write root"), "{text}");
    assert_eq!(names(&base.join("outside")), before, "no temporary file, nothing replaced");
    assert!(std::fs::symlink_metadata(base.join("outside/alias.docx")).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(base.join("out/doc.docx")).unwrap(), "inside");
    // Saving to the real file inside still works.
    ok(&mut server, "save_document", json!({"path": s(&base.join("out/doc.docx"))}));
    assert_ne!(std::fs::read(base.join("out/doc.docx")).unwrap(), b"inside");
}
