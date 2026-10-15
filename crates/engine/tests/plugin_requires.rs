//! Requirements between add-ins (manifest 1.3) end to end: blocked add-ins, start order,
//! `initialize`'s `host.protocol` and `addins`, the commands, and installing missing add-ins from
//! (fake) GitHub releases. The plugins are this test binary started again with `--serve-plugin`
//! (no libtest harness, so standard output is the protocol's alone).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::Document;
use wordcraft_plugin::install::{Fetched, fetched_from_zip};
use wordcraft_plugin::version::Range;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--serve-plugin") {
        serve(args.get(i + 1).map(PathBuf::from), args.get(i + 2).cloned().unwrap_or_default());
        return;
    }
    let tests: [(&str, fn()); 4] = [
        ("required_add_ins_start_first_and_initialize_lists_add_ins", required_add_ins_start_first_and_initialize_lists_add_ins),
        ("commands_report_and_change_requirements", commands_report_and_change_requirements),
        ("missing_add_ins_install_from_releases_in_range", missing_add_ins_install_from_releases_in_range),
        ("installing_out_of_a_dependents_range_needs_force", installing_out_of_a_dependents_range_needs_force),
    ];
    for (name, t) in tests {
        t();
        println!("test {name} ... ok");
    }
}

/// The plugin side: logs `initialize` (one JSON line per start) and answers commands with nothing
/// to do.
fn serve(log: Option<PathBuf>, id: String) {
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
        let Some(rid) = msg.get("id").cloned() else { break };
        let result = match msg["method"].as_str() {
            Some("initialize") => {
                if let Some(log) = &log
                    && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log)
                {
                    let _ = writeln!(f, "{}", json!({"id": id, "params": msg["params"]}));
                }
                json!({"protocolVersion": 1})
            }
            Some("commands/run") => json!({"commands": [], "message": format!("{id} ran")}),
            _ => json!({}),
        };
        if writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": rid, "result": result})).is_err() {
            break;
        }
    }
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-requires-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A manifest for this binary as plugin `id`, logging to `log`.
fn manifest(id: &str, version: &str, log: &Path, extra: Json) -> Json {
    let exe = std::env::current_exe().unwrap();
    let mut m = json!({
        "id": id, "name": id.rsplit('.').next().unwrap(), "version": version,
        "run": {"command": exe, "args": ["--serve-plugin", log, id]},
        "commands": [{"id": "go", "label": "Go"}],
    });
    m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    m
}

/// Writes `manifest` into `root/<name>/plugin.json`; returns that folder.
fn write(root: &Path, name: &str, manifest: &Json) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    dir
}

fn session() -> Session {
    Session::new(Document::new())
}

fn run(s: &mut Session, id: &str, params: Json) -> Result<Json, wordcraft_engine::CmdError> {
    s.run(id, &params)
}

fn required_add_ins_start_first_and_initialize_lists_add_ins() {
    let root = temp("order");
    let log = root.join("init.log");
    let mut s = session();
    let top = manifest(
        "t.order.top",
        "1.0.0",
        &log,
        json!({"requires": {"protocol": ">=2.0", "addins": [
            {"id": "t.order.base", "version": "^1.2"},
            {"id": "t.order.extra", "version": "*", "optional": true}
        ]}}),
    );
    let info = run(&mut s, "plugins.load", json!({"path": write(&root, "top", &top)})).unwrap();
    assert_eq!(info["state"], "blocked");
    let e = run(&mut s, "plugins.run", json!({"plugin": "t.order.top", "command": "go"})).unwrap_err().to_string();
    assert!(e.contains("blocked") && e.contains("isn't installed"), "a blocked add-in's commands are refused: {e}");
    assert!(!log.exists(), "nothing started");
    run(&mut s, "plugins.load", json!({"path": write(&root, "base", &manifest("t.order.base", "1.3.0", &log, json!({}))) })).unwrap();
    let r = run(&mut s, "plugins.run", json!({"plugin": "t.order.top", "command": "go"})).unwrap();
    assert_eq!(r["message"], "t.order.top ran", "unblocked without a restart");
    let starts: Vec<Json> = std::fs::read_to_string(&log).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let ids: Vec<&str> = starts.iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["t.order.base", "t.order.top"], "the required add-in starts first");
    let init = &starts[1]["params"];
    assert_eq!(init["host"]["protocol"], "2.0");
    assert_eq!(init["protocolVersion"], 1);
    let addins = init["addins"].as_array().unwrap();
    let base = addins.iter().find(|a| a["id"] == "t.order.base").unwrap();
    assert_eq!((base["version"].as_str(), base["enabled"].as_bool()), (Some("1.3.0"), Some(true)));
    assert!(addins.iter().any(|a| a["id"] == "t.order.top"));
    assert!(!addins.iter().any(|a| a["id"] == "t.order.extra"), "only installed add-ins");
    run(&mut s, "plugins.unload", json!({"id": "t.order.top"})).unwrap();
    run(&mut s, "plugins.unload", json!({"id": "t.order.base"})).unwrap();
    let _ = std::fs::remove_dir_all(root);
}

fn commands_report_and_change_requirements() {
    let root = temp("commands");
    let log = root.join("init.log");
    let mut s = session();
    let app = manifest(
        "t.cmd.app",
        "1.0.0",
        &log,
        json!({"requires": {"addins": [{"id": "t.cmd.lib", "version": ">=0.1, <0.3", "github": "acme/lib", "why": "Does the maths"}]}}),
    );
    run(&mut s, "plugins.load", json!({"path": write(&root, "app", &app)})).unwrap();
    run(&mut s, "plugins.load", json!({"path": write(&root, "lib", &manifest("t.cmd.lib", "0.2.1", &log, json!({})))})).unwrap();
    let list = run(&mut s, "plugins.list", json!({})).unwrap();
    let find = |list: &Json, id: &str| list.as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();
    assert_eq!(find(&list, "t.cmd.app")["state"], "loaded");
    assert_eq!(find(&list, "t.cmd.app")["requires"]["addins"][0]["why"], "Does the maths");
    assert_eq!(find(&list, "t.cmd.lib")["requiredBy"], json!(["t.cmd.app"]));
    let r = run(&mut s, "plugins.requirements", json!({"id": "t.cmd.app"})).unwrap();
    assert_eq!(r["satisfied"], true);
    assert_eq!(r["tree"][0]["need"], ">=0.1, <0.3");
    assert_eq!(r["tree"][0]["have"], "0.2.1");
    assert!(run(&mut s, "plugins.requirements", json!({"id": "t.cmd.nope"})).is_err());

    // Turning the library off (programmatically: no question) blocks the app, and says so.
    let off = run(&mut s, "plugins.setEnabled", json!({"id": "t.cmd.lib", "enabled": false})).unwrap();
    assert_eq!(off["dependentsBlocked"], json!(["t.cmd.app"]));
    let app_info = find(&run(&mut s, "plugins.list", json!({})).unwrap(), "t.cmd.app");
    assert_eq!(app_info["state"], "blocked");
    assert_eq!(app_info["blockedBy"][0]["message"], "Needs lib, which is turned off");
    assert!(run(&mut s, "plugins.run", json!({"plugin": "t.cmd.app", "command": "go"})).unwrap_err().to_string().contains("blocked"));
    let on = run(&mut s, "plugins.setEnabled", json!({"id": "t.cmd.lib", "enabled": true})).unwrap();
    assert_eq!(on["dependentsBlocked"], json!([]));
    assert_eq!(find(&run(&mut s, "plugins.list", json!({})).unwrap(), "t.cmd.app")["state"], "loaded");
    run(&mut s, "plugins.run", json!({"plugin": "t.cmd.app", "command": "go"})).unwrap();

    // Unloading it too.
    let gone = run(&mut s, "plugins.unload", json!({"id": "t.cmd.lib"})).unwrap();
    assert_eq!(gone["dependentsBlocked"], json!(["t.cmd.app"]));
    let r = run(&mut s, "plugins.requirements", json!({"id": "t.cmd.app"})).unwrap();
    assert_eq!((r["satisfied"].as_bool(), r["tree"][0]["state"].as_str()), (Some(false), Some("missing")));
    assert_eq!(r["blockedBy"][0]["message"], "Needs lib, which isn't installed");
    run(&mut s, "plugins.unload", json!({"id": "t.cmd.app"})).unwrap();
    let _ = std::fs::remove_dir_all(root);
}

/// A zip holding `manifest` in a folder, as a GitHub source zip does.
fn zip_of(manifest: &Json) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
    zip.start_file("repo-abc123/plugin.json", opts).unwrap();
    zip.write_all(manifest.to_string().as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

/// A fake GitHub: per repository, releases `(tag, manifest)`; picks as the real one does.
fn fake_github(repos: Vec<(&'static str, Vec<(&'static str, Json)>)>) -> impl FnMut(&str, &Range) -> Result<Fetched, String> {
    move |github: &str, range: &Range| {
        let (_, releases) = repos.iter().find(|(r, _)| *r == github).ok_or_else(|| format!("not found on GitHub ({github})"))?;
        let list: Vec<Json> = releases.iter().map(|(tag, _)| json!({"tag_name": tag})).collect();
        let (i, _) = wordcraft_plugin::requires::pick_release(&list, range).ok_or_else(|| format!("{github} has no release in {range}"))?;
        let (tag, m) = &releases[i];
        fetched_from_zip(zip_of(m), json!({"repository": format!("https://github.com/{github}"), "release": tag}))
    }
}

fn missing_add_ins_install_from_releases_in_range() {
    let root = temp("missing");
    let log = root.join("init.log");
    let plugins = root.join("installed");
    let mut s = session();
    let app = manifest(
        "t.miss.app",
        "1.0.0",
        &log,
        json!({"requires": {"addins": [
            {"id": "t.miss.vba", "version": "^0.1.4", "github": "acme/craft-vba", "why": "Runs the macros"},
            {"id": "t.miss.chart", "version": "*", "github": "acme/chart", "optional": true},
            {"id": "t.miss.nogit", "version": "*", "optional": true}
        ]}}),
    );
    let info = run(&mut s, "plugins.load", json!({"path": write(&root, "app", &app)})).unwrap();
    assert_eq!(info["state"], "blocked");
    // The VBA add-in needs a helper in turn.
    let vba = |v: &str| manifest("t.miss.vba", v, &log, json!({"requires": {"addins": [{"id": "t.miss.helper", "github": "acme/helper"}]}}));
    let mut github = fake_github(vec![
        ("acme/craft-vba", vec![("v0.2.0", vba("0.2.0")), ("v0.1.6", vba("0.1.6")), ("v0.1.3", vba("0.1.3"))]),
        ("acme/helper", vec![("1.0.0", manifest("t.miss.helper", "1.0.0", &log, json!({})))]),
    ]);
    let r = wordcraft_engine::cmd::plugins::install_missing_with(&mut s, "t.miss.app", &plugins, false, &mut github);
    let installed: Vec<(&str, &str)> =
        r["installed"].as_array().unwrap().iter().map(|x| (x["id"].as_str().unwrap(), x["release"].as_str().unwrap())).collect();
    assert_eq!(installed, [("t.miss.vba", "v0.1.6"), ("t.miss.helper", "1.0.0")], "the newest release in range, then what it needs");
    assert_eq!(r["failed"], json!([]));
    assert_eq!(r["satisfied"], true);
    let still: Vec<&str> = r["missing"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(still, ["t.miss.chart", "t.miss.nogit"], "optional ones aren't installed");
    assert!(plugins.join("t.miss.vba").join("plugin.json").is_file());
    assert_eq!(wordcraft_plugin::get("t.miss.vba").unwrap().manifest.version, "0.1.6");

    // A repository holding another add-in, or none in range, fails without stopping the rest.
    run(&mut s, "plugins.unload", json!({"id": "t.miss.vba"})).unwrap();
    let mut wrong = fake_github(vec![("acme/craft-vba", vec![("v0.1.9", manifest("t.miss.other", "0.1.9", &log, json!({})))])]);
    let r = wordcraft_engine::cmd::plugins::install_missing_with(&mut s, "t.miss.app", &plugins, false, &mut wrong);
    assert!(r["failed"][0]["error"].as_str().unwrap().contains("holds t.miss.other, not t.miss.vba"), "{r}");
    let mut none = fake_github(vec![("acme/craft-vba", vec![("v0.3.0", vba("0.3.0"))])]);
    let r = wordcraft_engine::cmd::plugins::install_missing_with(&mut s, "t.miss.app", &plugins, false, &mut none);
    assert!(r["failed"][0]["error"].as_str().unwrap().contains("no release"), "{r}");
    assert_eq!(r["satisfied"], false);
    // A tag that lies about the version inside.
    let mut liar = fake_github(vec![("acme/craft-vba", vec![("v0.1.8", vba("0.1.2"))])]);
    let r = wordcraft_engine::cmd::plugins::install_missing_with(&mut s, "t.miss.app", &plugins, false, &mut liar);
    assert!(r["failed"][0]["error"].as_str().unwrap().contains("isn't in ^0.1.4"), "{r}");
    assert!(run(&mut s, "plugins.installMissing", json!({"id": "t.miss.none"})).is_err());

    for id in ["t.miss.app", "t.miss.helper"] {
        run(&mut s, "plugins.unload", json!({"id": id})).unwrap();
    }
    let _ = std::fs::remove_dir_all(root);
}

fn installing_out_of_a_dependents_range_needs_force() {
    let root = temp("force");
    let log = root.join("init.log");
    let plugins = root.join("installed");
    let mut s = session();
    run(&mut s, "plugins.load", json!({"path": write(&root, "lib1", &manifest("t.force.lib", "1.2.0", &log, json!({})))})).unwrap();
    let app = manifest("t.force.app", "1.0.0", &log, json!({"requires": {"addins": [{"id": "t.force.lib", "version": "~1.2"}]}}));
    run(&mut s, "plugins.load", json!({"path": write(&root, "app", &app)})).unwrap();
    // Loading (updating to) 1.3.0 from a folder: refused, then forced.
    let newer = write(&root, "lib2", &manifest("t.force.lib", "1.3.0", &log, json!({})));
    let e = run(&mut s, "plugins.load", json!({"path": newer})).unwrap_err().to_string();
    assert!(e.contains("app needs lib ~1.2; 1.3.0 doesn't match"), "{e}");
    assert_eq!(wordcraft_plugin::get("t.force.lib").unwrap().manifest.version, "1.2.0", "nothing changed");
    run(&mut s, "plugins.load", json!({"path": newer, "force": true})).unwrap();
    assert!(wordcraft_plugin::get("t.force.app").unwrap().is_blocked());
    // Installing from a (fake) download: the same, then with requirements.
    let fetched = fetched_from_zip(zip_of(&manifest("t.force.lib", "2.0.0", &log, json!({}))), json!({"release": "v2.0.0"})).unwrap();
    let mut no_network = |_: &str, _: &Range| -> Result<Fetched, String> { Err("no network in tests".into()) };
    run(&mut s, "plugins.load", json!({"path": write(&root, "lib3", &manifest("t.force.lib", "1.2.5", &log, json!({})))})).unwrap();
    assert!(!wordcraft_plugin::get("t.force.app").unwrap().is_blocked(), "back in range: unblocked");
    let e =
        wordcraft_engine::cmd::plugins::install_fetched(&mut s, fetched.clone(), &plugins, false, false, &mut no_network).unwrap_err().to_string();
    assert!(e.contains("2.0.0 doesn't match"), "{e}");
    let info = wordcraft_engine::cmd::plugins::install_fetched(&mut s, fetched, &plugins, true, true, &mut no_network).unwrap();
    assert_eq!((info["version"].as_str(), info["release"].as_str()), (Some("2.0.0"), Some("v2.0.0")));
    assert_eq!(info["missing"], json!([]));
    assert!(wordcraft_plugin::get("t.force.app").unwrap().is_blocked());
    // The app's own install reports what it's missing.
    let app_zip = fetched_from_zip(zip_of(&app), json!({})).unwrap();
    let info = wordcraft_engine::cmd::plugins::install_fetched(&mut s, app_zip, &plugins, false, false, &mut no_network).unwrap();
    assert_eq!(info["missing"][0]["id"], "t.force.lib");
    assert_eq!(info["missing"][0]["installed"], "2.0.0");
    for id in ["t.force.app", "t.force.lib"] {
        run(&mut s, "plugins.unload", json!({"id": id})).unwrap();
    }
    let _ = std::fs::remove_dir_all(root);
}
