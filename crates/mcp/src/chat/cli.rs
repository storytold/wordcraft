//! `wordcraft-cli chat …`: an invited agent's side of a window's chat (docs/chat.md).

use std::io::{Read, Write};

use wordcraft_chat::rules::normalize_handle;

use super::{
    Caller, Exit, Failure, Link, MAX_CONTEXT, MAX_STEPS_TEXT, Membership, ReadOpts, Store, briefing, client_command, commands, join, listen,
    owner_selection, parse_steps, ping, read, run_steps, send, view_page, window_gone,
};

pub const USAGE: &str = "\
wordcraft-cli chat: join a WordCraft window's chat as an invited agent (docs/chat.md)

  join ADDR CODE --as @name    once, with the line the owner gave you; prints the rules
  listen                       one line per message (run it in the background)
  send TEXT...                 a message in the chat
  read [--from N] [--to M] [--find TEXT [--context K]] [--sel]
                               numbered paragraphs, [+inserted+](@author) [-deleted-](@author)
  view N OUT.png               page N as PNG
  do STEPS.json                [{\"cmd\": id, \"params\": {...}}, ...]; stops at the first failure
  commands [FILTER]            the commands you may run
  help                         the rules again
Options: --as @name and --addr HOST:PORT pick the membership when you have several.
Exit: 0 ok, 1 error, 2 usage, 3 removed from the chat, 4 window closed.
";

/// Run `wordcraft-cli chat <args>` with the memberships in the settings folder.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Exit {
    match Store::default_dir() {
        Some(dir) => run_with(&Store::new(dir), args, super::now_ms(), out, err),
        None => {
            let _ = writeln!(err, "wordcraft-cli chat: no settings folder (HOME or APPDATA is not set)");
            Exit::Error
        }
    }
}

/// [`run`] with the store and the clock given. Messages for exit 3 and 4 go to `out` (a
/// `listen` watcher reads them there); the others go to `err`. An empty message prints nothing.
pub fn run_with(store: &Store, args: &[String], now_ms: u64, out: &mut dyn Write, err: &mut dyn Write) -> Exit {
    match dispatch(store, args, now_ms, out) {
        Ok(()) => Exit::Ok,
        Err(f) => {
            if !f.message.is_empty() {
                let _ = if matches!(f.exit, Exit::Removed | Exit::Gone) {
                    writeln!(out, "{}", f.message)
                } else {
                    writeln!(err, "wordcraft-cli chat: {}", f.message)
                };
            }
            f.exit
        }
    }
}

fn take_opt(a: &mut Vec<String>, name: &str) -> Result<Option<String>, Failure> {
    let Some(i) = a.iter().position(|x| x == name) else { return Ok(None) };
    let v = a.get(i.saturating_add(1)).cloned().ok_or_else(|| Failure::usage(format!("{name} needs a value")))?;
    a.drain(i..i.saturating_add(2));
    Ok(Some(v))
}

fn take_flag(a: &mut Vec<String>, name: &str) -> bool {
    let had = a.iter().any(|x| x == name);
    a.retain(|x| x != name);
    had
}

fn number(v: Option<String>) -> Result<Option<u64>, Failure> {
    v.map(|s| s.parse::<u64>().map_err(|_| Failure::usage("--from, --to and --context take numbers"))).transpose()
}

fn lines(out: &mut dyn Write, lines: &[String]) -> Result<(), Failure> {
    lines.iter().try_for_each(|l| writeln!(out, "{l}")).map_err(|e| Failure::error(format!("cannot write the output: {e}")))
}

/// The steps file, read with a size cap.
fn read_steps_file(path: &str) -> Result<String, Failure> {
    let cannot = |e: std::io::Error| Failure::usage(format!("cannot read steps file {path}: {e}"));
    let limit = u64::try_from(MAX_STEPS_TEXT).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(cannot)?.take(limit).read_to_end(&mut bytes).map_err(cannot)?;
    if bytes.len() > MAX_STEPS_TEXT {
        return Err(Failure::usage(format!("the steps file {path} is longer than {} MiB", MAX_STEPS_TEXT >> 20)));
    }
    String::from_utf8(bytes).map_err(|_| Failure::usage(format!("the steps file {path} is not UTF-8 text")))
}

/// What `read` was asked: `--sel`, or paragraphs.
fn read_args(rest: &mut Vec<String>) -> Result<(bool, ReadOpts), Failure> {
    let sel = take_flag(rest, "--sel");
    let (from, to) = (number(take_opt(rest, "--from")?)?, number(take_opt(rest, "--to")?)?);
    let find = take_opt(rest, "--find")?;
    let context = number(take_opt(rest, "--context")?)?;
    if sel && (find.is_some() || from.is_some() || to.is_some() || context.is_some()) {
        return Err(Failure::usage("read --sel takes no other option"));
    }
    if find.is_none() && context.is_some() {
        return Err(Failure::usage("--context goes with --find"));
    }
    if find.is_some() && (from.is_some() || to.is_some()) {
        return Err(Failure::usage("use --find or --from/--to, not both"));
    }
    if context.is_some_and(|c| c > MAX_CONTEXT) {
        return Err(Failure::usage(format!("--context is at most {MAX_CONTEXT}")));
    }
    Ok((sel, ReadOpts { from, to, find, context: context.unwrap_or(0) }))
}

fn dispatch(store: &Store, args: &[String], now_ms: u64, out: &mut dyn Write) -> Result<(), Failure> {
    let mut a: Vec<String> = args.to_vec();
    let handle = take_opt(&mut a, "--as")?.map(|h| normalize_handle(&h));
    let addr = take_opt(&mut a, "--addr")?;
    let sub = a.first().cloned().unwrap_or_default();
    let mut rest: Vec<String> = a.iter().skip(1).cloned().collect();
    match sub.as_str() {
        "" | "-h" | "--help" => lines(out, &[USAGE.trim_end().to_string()]),
        "help" => lines(out, &[briefing(handle.as_deref().unwrap_or("@you"), &client_command())]),
        "join" => {
            let (Some(at), Some(code), Some(h)) = (rest.first(), rest.get(1), handle.as_deref()) else {
                return Err(Failure::usage("usage: join ADDR CODE --as @name"));
            };
            let m = join(at, code, Some(h))?;
            store.prune(&window_gone);
            store.save(&m)?;
            lines(out, &[briefing(&m.handle, &client_command())])
        }
        "listen" | "send" | "read" | "view" | "do" | "commands" => {
            // Usage first, before any network.
            let read_opts = if sub == "read" { Some(read_args(&mut rest)?) } else { None };
            let text = rest.join(" ");
            if sub == "send" && text.trim().is_empty() {
                return Err(Failure::usage("usage: send TEXT"));
            }
            let view = if sub == "view" {
                match (rest.first().and_then(|n| n.parse::<u64>().ok()), rest.get(1)) {
                    (Some(n), Some(path)) => Some((n, path.clone())),
                    _ => return Err(Failure::usage("usage: view N OUT.png")),
                }
            } else {
                None
            };
            let steps = if sub == "do" {
                let path = rest.first().ok_or_else(|| Failure::usage("usage: do STEPS.json"))?;
                Some(parse_steps(&read_steps_file(path)?)?)
            } else {
                None
            };
            let (m, path): (Membership, _) = store.find(handle.as_deref(), addr.as_deref())?;
            let mut link = Link::new(&m.addr, Some(&m.key));
            let r = match sub.as_str() {
                "listen" => Err(listen(&mut link, &m.handle, now_ms, out)),
                "send" => send(&mut link, &text).and_then(|()| lines(out, &["OK".to_string()])),
                _ => ping(&mut link).and_then(|()| match (sub.as_str(), read_opts, view, steps) {
                    ("read", Some((true, _)), _, _) => owner_selection(&mut link).and_then(|l| lines(out, &[l])),
                    ("read", Some((false, o)), _, _) => read(&mut link, &o).and_then(|ls| lines(out, &ls)),
                    ("view", _, Some((n, file)), _) => view_page(&mut link, n)
                        .and_then(|png| std::fs::write(&file, png).map_err(|e| Failure::error(format!("{file}: {e}"))))
                        .and_then(|()| lines(out, &[format!("OK: {file}")])),
                    ("do", _, _, Some(steps)) => {
                        let (ls, exit) = run_steps(&mut link as &mut dyn Caller, &steps);
                        lines(out, &ls)?;
                        match exit {
                            Exit::Ok => Ok(()),
                            // `run_steps` already printed REMOVED or GONE.
                            Exit::Removed | Exit::Gone => Err(Failure { exit, message: String::new() }),
                            e => Err(Failure { exit: e, message: "a step failed".into() }),
                        }
                    }
                    _ => commands(&mut link, rest.first().map(String::as_str).unwrap_or("")).and_then(|ls| lines(out, &ls)),
                }),
            };
            if let Err(f) = &r
                && matches!(f.exit, Exit::Removed | Exit::Gone)
            {
                store.forget(&path);
            }
            r
        }
        other => Err(Failure::usage(format!("unknown command `{other}`\n\n{USAGE}"))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::chat::tests::{FakeWindow, fail, ok, tmp};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn run(store: &Store, v: &[&str]) -> (Exit, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(store, &args(v), 0, &mut out, &mut err);
        (code, String::from_utf8_lossy(&out).to_string(), String::from_utf8_lossy(&err).to_string())
    }

    #[test]
    fn usage_errors_exit_2_before_any_network() {
        let store = Store::new(tmp("usage"));
        assert_eq!(run(&store, &[]).0, Exit::Ok, "no arguments prints the usage");
        for v in [
            &["nope"][..],
            &["join"],
            &["join", "127.0.0.1:1"],
            &["join", "127.0.0.1:1", "ABCD-EFGH-JKMN"],
            &["send"],
            &["view"],
            &["view", "x", "o.png"],
            &["view", "1"],
            &["do"],
            &["read", "--sel", "--find", "x"],
            &["read", "--context", "2"],
            &["read", "--find", "x", "--from", "1"],
            &["read", "--from", "abc"],
            &["read", "--find", "x", "--context", "51"],
            &["send", "--as"],
        ] {
            let (code, _, err) = run(&store, v);
            assert_eq!(code, Exit::Usage, "{v:?}: {err}");
            assert!(err.starts_with("wordcraft-cli chat: "), "{err}");
        }
    }

    #[test]
    fn an_oversize_or_missing_steps_file_is_a_usage_error() {
        let store = Store::new(tmp("steps"));
        let dir = tmp("steps-files");
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.json");
        std::fs::write(&big, vec![b' '; crate::chat::MAX_STEPS_TEXT + 1]).unwrap();
        let (code, _, err) = run(&store, &["do", big.to_str().unwrap()]);
        assert_eq!(code, Exit::Usage, "{err}");
        assert!(err.contains("longer than"), "{err}");
        let (code, _, err) = run(&store, &["do", dir.join("none.json").to_str().unwrap()]);
        assert_eq!(code, Exit::Usage, "{err}");
    }

    #[test]
    fn help_prints_the_briefing_for_the_name() {
        let (code, out, _) = run(&Store::new(tmp("help")), &["help", "--as", "Claude"]);
        assert_eq!(code, Exit::Ok);
        assert!(out.starts_with("You are in the WordCraft chat as @claude."), "{out}");
    }

    #[test]
    fn join_saves_the_membership_then_send_uses_it() {
        let w = FakeWindow::start(|m, _| match m {
            "chat.join" => ok(json!({"handle": "@claude", "key": "ab".repeat(32)})),
            "chat.post" => ok(json!({})),
            _ => fail("unexpected"),
        });
        let store = Store::new(tmp("e2e"));
        let (code, out, err) = run(&store, &["join", &w.addr, "ABCD-EFGH-JKMN", "--as", "Claude"]);
        assert_eq!(code, Exit::Ok, "{err}");
        assert!(out.contains("You are in the WordCraft chat as @claude"));
        assert_eq!(run(&store, &["send", "hello", "there"]).0, Exit::Ok);
        let last = w.seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.0.as_str(), last.1["text"].clone(), last.2), ("chat.post", json!("hello there"), Some("ab".repeat(32))));
    }

    #[test]
    fn exit_3_and_4_forget_the_membership() {
        let w = FakeWindow::start(|m, _| if m == "chat.post" { fail("unauthorized") } else { ok(json!({})) });
        let store = Store::new(tmp("forget"));
        store.save(&Membership { addr: w.addr.clone(), handle: "@claude".into(), key: "ab".repeat(32) }).unwrap();
        let (code, out, _) = run(&store, &["send", "x"]);
        assert_eq!(code, Exit::Removed);
        assert!(out.contains(crate::chat::REMOVED), "{out}");
        assert!(store.find(Some("@claude"), None).is_err(), "forgotten");
        let gone = crate::chat::tests::closed_addr();
        store.save(&Membership { addr: gone, handle: "@pi".into(), key: "cd".repeat(32) }).unwrap();
        let (code, out, _) = run(&store, &["send", "x", "--as", "@pi"]);
        assert_eq!(code, Exit::Gone);
        assert!(out.contains(crate::chat::GONE));
        assert!(store.find(Some("@pi"), None).is_err());
    }
}
