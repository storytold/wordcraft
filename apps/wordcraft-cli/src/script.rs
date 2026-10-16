//! `run --script FILE`: commands read from a JSON file (`-` is stdin), so a job can be kept in a
//! file instead of a long chain of `--cmd` flags (#1963).
//!
//! ```json
//! {"file": "in.docx", "template": "sample",
//!  "commands": [{"command": "text.insert", "params": {"text": "Hi"}}, "caret.docEnd"],
//!  "save": ["out.docx", "out.pdf"]}
//! ```
//!
//! `file` and `template` are optional (`--file` / `--template` on the command line win), and a
//! command is an object with `command` and optional `params`, or just the id as a string. Relative
//! `file` and `save` paths resolve against the script's folder (the working folder for stdin).
use std::io::Read;
use std::path::Path;

use serde_json::Value;

/// The largest script read. A job is a list of commands; anything bigger is a mistake.
pub const MAX_SCRIPT_BYTES: u64 = 4 * 1024 * 1024;
/// The most commands a script may hold.
const MAX_COMMANDS: usize = 100_000;
/// The most outputs a script may save to.
const MAX_SAVES: usize = 64;

#[derive(Debug)]
pub struct Script {
    pub file: Option<String>,
    pub template: Option<String>,
    pub commands: Vec<(String, Value)>,
    pub save: Vec<String>,
}

/// Read the script at `arg` (`-` for stdin) and parse it.
pub fn load(arg: &str) -> Result<Script, String> {
    if arg == "-" {
        return load_from(std::io::stdin().lock(), arg, None);
    }
    let f = std::fs::File::open(arg).map_err(|e| format!("--script {arg}: {e}"))?;
    load_from(f, arg, Path::new(arg).parent().filter(|p| !p.as_os_str().is_empty()))
}

/// Read and parse a script from `r`, called `name` in errors; relative paths resolve against `base`.
pub fn load_from(r: impl Read, name: &str, base: Option<&Path>) -> Result<Script, String> {
    let text = read_capped(r, name)?;
    parse(&text, base).map_err(|e| format!("--script {name}: {e}"))
}

fn read_capped(r: impl Read, name: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    r.take(MAX_SCRIPT_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("--script {name}: {e}"))?;
    if bytes.len() as u64 > MAX_SCRIPT_BYTES {
        return Err(format!("--script {name}: script too large (over {} MiB)", MAX_SCRIPT_BYTES / 1024 / 1024));
    }
    String::from_utf8(bytes).map_err(|_| format!("--script {name}: not valid UTF-8"))
}

fn resolve(path: &str, base: Option<&Path>) -> String {
    match base {
        Some(b) if Path::new(path).is_relative() => b.join(path).to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

fn opt_string(obj: &serde_json::Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

pub fn parse(text: &str, base: Option<&Path>) -> Result<Script, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad JSON: {e}"))?;
    let obj = v.as_object().ok_or("a script is a JSON object with a `commands` list")?;
    let list = match obj.get("commands") {
        Some(Value::Array(a)) => a,
        _ => return Err("`commands` must be a list".into()),
    };
    if list.is_empty() {
        return Err("`commands` is empty".into());
    }
    if list.len() > MAX_COMMANDS {
        return Err(format!("too many commands (over {MAX_COMMANDS})"));
    }
    let mut commands = Vec::with_capacity(list.len());
    for (i, c) in list.iter().enumerate() {
        let n = i + 1;
        let entry = match c {
            Value::String(id) => Some((id.clone(), Value::Object(Default::default()))),
            Value::Object(o) => match o.get("command") {
                Some(Value::String(id)) => Some((id.clone(), o.get("params").cloned().unwrap_or_else(|| Value::Object(Default::default())))),
                _ => None,
            },
            _ => None,
        };
        match entry {
            Some((id, _)) if id.is_empty() => return Err(format!("command {n}: empty command id")),
            Some(e) => commands.push(e),
            None => return Err(format!("command {n}: expected a command id or {{\"command\": id, \"params\": {{…}}}}")),
        }
    }
    let save = match obj.get("save") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![resolve(s, base)],
        Some(Value::Array(a)) if a.len() <= MAX_SAVES => {
            a.iter().map(|s| s.as_str().map(|s| resolve(s, base)).ok_or("`save` entries must be strings")).collect::<Result<_, _>>()?
        }
        Some(_) => return Err(format!("`save` must be a path or a list of at most {MAX_SAVES} paths")),
    };
    Ok(Script { file: opt_string(obj, "file")?.map(|f| resolve(&f, base)), template: opt_string(obj, "template")?, commands, save })
}
