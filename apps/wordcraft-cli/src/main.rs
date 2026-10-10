//! `wordcraft-cli`: WordCraft from the command line.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::process::ExitCode;

use serde_json::{Value, json};
use wordcraft_engine::Session;

const USAGE: &str = "\
wordcraft-cli — WordCraft from the command line

USAGE:
  wordcraft-cli convert <in> <out>            convert between formats (docx, pdf, odt, rtf, html, md, txt, json, png)
  wordcraft-cli info <file>                   pages, words, paragraphs, properties (JSON)
  wordcraft-cli text <file>                   plain text
  wordcraft-cli inspect <file>                document structure (JSON)
  wordcraft-cli render <file> <out.png> [--page N] [--scale S]
  wordcraft-cli run [--file F | --template T] --cmd 'id={json}' [--cmd …] [--save OUT] [--print]
                                              run commands headlessly, then save
  wordcraft-cli commands [--json]             list every command
  wordcraft-cli parity [--markdown]           feature-catalog parity
  wordcraft-cli mcp [--connect HOST:PORT [--join CODE [--as @name]]]
                                              MCP server on stdio (headless, bridged to the app, or as an invited chat agent)
  wordcraft-cli chat <join|listen|send|read|view|do|commands|help> …  an invited agent's side of a window's chat (docs/chat.md)
  wordcraft-cli --version
";

fn open(path: &str) -> Result<Session, String> {
    let doc = wordcraft_engine::io::open_path(std::path::Path::new(path))?;
    let mut s = Session::new(doc);
    s.path = Some(path.into());
    Ok(s)
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn run(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let pos = |i: usize| rest.iter().filter(|a| !a.starts_with("--")).nth(i).cloned().ok_or_else(|| format!("missing argument\n\n{USAGE}"));
    match cmd {
        "convert" => {
            let (i, o) = (pos(0)?, pos(1)?);
            let mut s = open(&i)?;
            let r = if o.to_ascii_lowercase().ends_with(".png") {
                s.run("file.exportPng", &json!({"path": o, "page": 1}))
            } else {
                s.run("file.save", &json!({"path": o}))
            };
            r.map_err(|e| e.to_string())?;
            eprintln!("wrote {o}");
            Ok(())
        }
        "info" => {
            let mut s = open(&pos(0)?)?;
            println!("{}", serde_json::to_string_pretty(&s.run("file.info", &json!({})).map_err(|e| e.to_string())?).unwrap_or_default());
            Ok(())
        }
        "text" => {
            let s = open(&pos(0)?)?;
            println!("{}", s.doc.plain_text(wordcraft_doc::StoryRef::Body));
            Ok(())
        }
        "inspect" => {
            let mut s = open(&pos(0)?)?;
            println!("{}", serde_json::to_string_pretty(&s.run("document.inspect", &json!({})).map_err(|e| e.to_string())?).unwrap_or_default());
            Ok(())
        }
        "render" => {
            let (i, o) = (pos(0)?, pos(1)?);
            let mut s = open(&i)?;
            let page = arg_value(&rest, "--page").and_then(|p| p.parse::<u64>().ok()).unwrap_or(1);
            let scale = arg_value(&rest, "--scale").and_then(|p| p.parse::<f64>().ok()).unwrap_or(2.0);
            let r = s.run("file.exportPng", &json!({"path": o, "page": page, "scale": scale})).map_err(|e| e.to_string())?;
            println!("{r}");
            Ok(())
        }
        "run" => {
            let mut s = match (arg_value(&rest, "--file"), arg_value(&rest, "--template")) {
                (Some(f), _) => open(&f)?,
                (None, Some(t)) => {
                    let mut s = Session::new(wordcraft_doc::Document::new());
                    s.run("file.new", &json!({"template": t})).map_err(|e| e.to_string())?;
                    s
                }
                _ => Session::new(wordcraft_doc::Document::new()),
            };
            let mut i = 0;
            while i < rest.len() {
                if rest.get(i).map(String::as_str) == Some("--cmd") {
                    let spec = rest.get(i + 1).ok_or("--cmd needs a value")?;
                    let (id, params) = match spec.split_once('=') {
                        Some((id, p)) => (id.to_string(), serde_json::from_str::<Value>(p).map_err(|e| format!("{id}: bad JSON params: {e}"))?),
                        None => (spec.clone(), json!({})),
                    };
                    let r = s.run(&id, &params).map_err(|e| format!("{id}: {e}"))?;
                    if rest.iter().any(|a| a == "--print") {
                        println!("{id}: {r}");
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if let Some(out) = arg_value(&rest, "--save") {
                let r = if out.to_ascii_lowercase().ends_with(".png") {
                    let mut v = json!({"path": out});
                    if let Some(n) = arg_value(&rest, "--page").and_then(|p| p.parse::<u64>().ok()) {
                        v["page"] = json!(n);
                    }
                    if let Some(k) = arg_value(&rest, "--scale").and_then(|p| p.parse::<f64>().ok()) {
                        v["scale"] = json!(k);
                    }
                    s.run("file.exportPng", &v)
                } else {
                    s.run("file.save", &json!({"path": out}))
                };
                r.map_err(|e| e.to_string())?;
                eprintln!("wrote {out}");
            }
            Ok(())
        }
        "commands" => {
            let s = Session::new(wordcraft_doc::Document::new());
            if rest.iter().any(|a| a == "--json") {
                println!("{}", serde_json::to_string_pretty(&s.registry.describe()).unwrap_or_default());
            } else {
                for c in s.registry.all() {
                    println!("{:<32} {:<36} {:<24} {}", c.id, c.label, c.shortcut, c.location);
                }
            }
            Ok(())
        }
        "parity" => {
            let s = Session::new(wordcraft_doc::Document::new());
            let p = wordcraft_engine::catalog::parity(&s.registry);
            if rest.iter().any(|a| a == "--markdown") {
                print!("{}", parity_markdown(&p));
            } else {
                println!("{}", serde_json::to_string_pretty(&p).unwrap_or_default());
            }
            Ok(())
        }
        "mcp" => {
            use wordcraft_mcp::Backend as _;
            let backend: Box<dyn wordcraft_mcp::Backend> = match arg_value(&rest, "--connect") {
                Some(addr) => {
                    let mut remote = wordcraft_mcp::Remote::connect(&addr).map_err(|e| format!("can't reach the WordCraft app at {addr}: {e}"))?;
                    if let Some(w) = remote.key_warning() {
                        eprintln!("wordcraft-cli: {w}");
                    }
                    if let Some(code) = arg_value(&rest, "--join") {
                        let m = wordcraft_mcp::chat::join(&addr, &code, arg_value(&rest, "--as").as_deref()).map_err(|f| f.message)?;
                        let h = m.handle.clone();
                        remote.set_member(m.handle, m.key)?;
                        // stdout is the protocol stream.
                        eprintln!("{}", wordcraft_mcp::chat::tools::mcp_briefing(&h));
                    }
                    Box::new(remote)
                }
                None if arg_value(&rest, "--join").is_some() => return Err("--join needs --connect HOST:PORT".into()),
                None => Box::new(wordcraft_mcp::Headless::default()),
            };
            let mut server = wordcraft_mcp::Server::new(backend);
            let stdin = std::io::stdin();
            server.serve(stdin.lock(), std::io::stdout()).map_err(|e| e.to_string())
        }
        "--version" | "-V" => {
            println!("wordcraft-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "" | "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn parity_markdown(p: &Value) -> String {
    let mut s = String::from(
        "# WordCraft feature parity\n\nGenerated by `cargo xtask parity` (`wordcraft-cli parity --markdown`): the word-processor feature catalog (`crates/engine/src/catalog.rs`) compared with the live command registry.\n\n",
    );
    s += &format!(
        "**{} of {} catalog features have commands ({}%).**\n\n| Tab | Live | Total |\n|---|---|---|\n",
        p["live"], p["total"], p["percent"]
    );
    for t in p["tabs"].as_array().cloned().unwrap_or_default() {
        s += &format!("| {} | {} | {} |\n", t["tab"].as_str().unwrap_or(""), t["live"], t["total"]);
    }
    s += "\n## Missing\n\n";
    for t in p["tabs"].as_array().cloned().unwrap_or_default() {
        let m = t["missing"].as_array().cloned().unwrap_or_default();
        if m.is_empty() {
            continue;
        }
        s += &format!("### {}\n\n", t["tab"].as_str().unwrap_or(""));
        for x in m {
            s += &format!("- {}\n", x.as_str().unwrap_or(""));
        }
        s += "\n";
    }
    s
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("chat") {
        let rest = args.get(1..).unwrap_or_default();
        let exit = wordcraft_mcp::chat::cli::run(rest, &mut std::io::stdout(), &mut std::io::stderr());
        return ExitCode::from(exit.code());
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wordcraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}
