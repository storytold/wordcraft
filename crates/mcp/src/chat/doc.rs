//! Reading and editing the document as a member: numbered paragraphs with tracked changes, the
//! owner's selection, page images, steps. Everything printed from the window goes through
//! [`one_line`].

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use wordcraft_chat::rules::one_line;

use super::{Caller, During, Exit, Failure, GONE, LinkError, REMOVED};

const OBJ: char = '\u{FFFC}';
/// Longest result printed per step (characters).
const STEP_RESULT: usize = 300;
/// Most paragraphs shown on each side of a `--find` hit: a larger `--context` counts as this.
pub const MAX_CONTEXT: u64 = 50;
/// Largest page image accepted from the window (bytes).
pub const MAX_PNG: usize = 32 << 20;
/// Longest steps text (bytes); the window reads at most 1 MiB per request.
pub const MAX_STEPS_TEXT: usize = 1 << 20;

/// The paragraphs `read` prints: `from..=to`, or the paragraphs that contain `find`
/// (case-insensitive) with `context` paragraphs on each side; all of them by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadOpts {
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub find: Option<String>,
    /// Paragraphs around each `find` hit (at most [`MAX_CONTEXT`]).
    pub context: u64,
}

/// A link error as a failure whose message can be printed (one line).
fn failure(e: LinkError) -> Failure {
    let f = Failure::from_link(e, During::Call);
    Failure { exit: f.exit, message: one_line(&f.message) }
}

fn call(c: &mut dyn Caller, method: &str, params: Value) -> Result<Value, Failure> {
    c.call(method, params).map_err(failure)
}

/// Errors after which `read` stops: the window is gone, the member was removed, or the link
/// broke or timed out (every further call would wait again).
fn stops_read(e: &LinkError) -> bool {
    matches!(e, LinkError::Refused | LinkError::Unauthorized | LinkError::Timeout | LinkError::Closed)
}

/// What an inline object shows as text (fields: result, equations: linear form).
fn obj_text(o: Option<&Value>) -> String {
    let Some(o) = o else { return String::new() };
    let key = match o.get("type").and_then(Value::as_str) {
        Some("field") => "result",
        Some("equation") => "linear",
        Some("opaque") => "text",
        _ => return String::new(),
    };
    o.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A paragraph (`document.paragraph`) with its tracked changes, the same for every reader:
/// `[+inserted+](@author)` and `[-deleted-](@author)`. `authors`: byte offset → author.
pub fn tracked_text(para: &Value, authors: &BTreeMap<u64, String>) -> String {
    let raw = para.get("text").and_then(Value::as_str).unwrap_or("").as_bytes();
    let mut objs = para.get("objects").and_then(Value::as_array).into_iter().flatten();
    let mut expand = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).chars().map(|c| if c == OBJ { obj_text(objs.next()) } else { c.to_string() }).collect::<String>()
    };
    let mut pieces: Vec<(&str, Option<u64>, String, Option<String>)> = Vec::new();
    let mut off: usize = 0;
    for run in para.get("runs").and_then(Value::as_array).into_iter().flatten() {
        let n = run.get("len").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
        let end = off.saturating_add(n).min(raw.len());
        let seg = expand(raw.get(off..end).unwrap_or_default());
        let props = run.get("props");
        let rev = |k: &str| props.and_then(|p| p.get(k)).and_then(Value::as_u64);
        let (kind, r) = match (rev("del"), rev("ins")) {
            (Some(d), _) => ("del", Some(d)),
            (None, Some(i)) => ("ins", Some(i)),
            _ => ("", None),
        };
        match pieces.last_mut() {
            Some(last) if last.0 == kind && last.1 == r => last.2.push_str(&seg),
            _ => pieces.push((kind, r, seg, authors.get(&u64::try_from(off).unwrap_or(u64::MAX)).cloned())),
        }
        off = end;
    }
    if off < raw.len() {
        pieces.push(("", None, expand(raw.get(off..).unwrap_or_default()), None));
    }
    pieces
        .into_iter()
        .map(|(kind, _, text, author)| {
            let by = author.map(|a| format!("({a})")).unwrap_or_default();
            match kind {
                "del" => format!("[-{text}-]{by}"),
                "ins" => format!("[+{text}+]{by}"),
                _ => text,
            }
        })
        .collect()
}

/// Paragraph numbers to print (as printed): a range, or the hits of `find` with `context`
/// (at most [`MAX_CONTEXT`]) around them. Each paragraph is checked against the nearest hits:
/// no work proportional to `context`.
pub fn pick(blocks: &[Value], o: &ReadOpts) -> Vec<u64> {
    let index = |b: &Value| b.get("index").and_then(Value::as_u64);
    let idx = blocks.iter().filter_map(index);
    if o.from.is_some() || o.to.is_some() {
        let (lo, hi) = (o.from.unwrap_or(0), o.to.unwrap_or(u64::MAX));
        return idx.filter(|i| lo <= *i && *i <= hi).collect();
    }
    if let Some(f) = &o.find {
        let f = f.to_lowercase();
        let mut hits: Vec<u64> = blocks
            .iter()
            .filter(|b| b.get("text").and_then(Value::as_str).is_some_and(|t| t.to_lowercase().contains(&f)))
            .filter_map(index)
            .collect();
        hits.sort_unstable();
        let context = o.context.min(MAX_CONTEXT);
        let near = |i: u64| {
            let k = hits.partition_point(|h| *h < i);
            [k.checked_sub(1), Some(k)].into_iter().flatten().filter_map(|k| hits.get(k)).any(|h| i.abs_diff(*h) <= context)
        };
        return idx.filter(|i| near(*i)).collect();
    }
    idx.collect()
}

/// Paragraph → byte offset → author, from `review.changes` (body paragraphs only).
fn change_authors(changes: &Value) -> BTreeMap<u64, BTreeMap<u64, String>> {
    let mut out: BTreeMap<u64, BTreeMap<u64, String>> = BTreeMap::new();
    for c in changes.as_array().into_iter().flatten() {
        let start = c.get("start");
        if !start.and_then(|s| s.get("story")).is_none_or(|st| st == "body") {
            continue;
        }
        let Some([p]) = start.and_then(|s| s.get("path")).and_then(Value::as_array).map(Vec::as_slice) else { continue };
        let (Some(p), Some(off)) = (p.as_u64(), start.and_then(|s| s.get("off")).and_then(Value::as_u64)) else { continue };
        out.entry(p).or_default().insert(off, c.get("author").and_then(Value::as_str).unwrap_or("?").to_string());
    }
    out
}

/// `read`: one line per paragraph (`[n] text`), tracked changes marked in body paragraphs.
/// The window may refuse `review.changes` or one paragraph's detail: that text is shown
/// without marks. A window that is gone, does not answer or drops the link stops the read.
pub fn read(c: &mut dyn Caller, o: &ReadOpts) -> Result<Vec<String>, Failure> {
    let inspect = call(c, "document.inspect", json!({"text": true}))?;
    let changes = match c.call("review.changes", json!({})) {
        Ok(v) => v,
        Err(e) if stops_read(&e) => return Err(failure(e)),
        Err(_) => Value::Null,
    };
    let blocks = inspect.get("blocks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let show: BTreeSet<u64> = pick(blocks, o).into_iter().collect();
    if let Some(f) = &o.find
        && show.is_empty()
    {
        return Ok(vec![one_line(&format!("nothing found: {f}"))]);
    }
    let authors = change_authors(&changes);
    let mut detail: BTreeMap<u64, Value> = BTreeMap::new();
    for i in show.iter().filter(|i| authors.contains_key(i)) {
        // The body, whatever story the member's own selection is in.
        match c.call("document.paragraph", json!({"path": [i], "story": "body"})) {
            Ok(v) => {
                if let Some(p) = v.get("paragraph") {
                    detail.insert(*i, p.clone());
                }
            }
            Err(e) if stops_read(&e) => return Err(failure(e)),
            Err(_) => {}
        }
    }
    Ok(blocks
        .iter()
        .filter_map(|b| {
            let i = b.get("index").and_then(Value::as_u64)?;
            if !show.contains(&i) {
                return None;
            }
            let text = match (detail.get(&i), authors.get(&i)) {
                (Some(p), Some(a)) => tracked_text(p, a),
                _ => b.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
            };
            Some(one_line(&format!("[{i}] {text}")))
        })
        .collect())
}

/// `read --sel`: the owner's selection (`select.owner` also makes it the member's).
pub fn owner_selection(c: &mut dyn Caller) -> Result<String, Failure> {
    let v = call(c, "select.owner", json!({}))?;
    let p: Vec<u64> = v.get("paragraphs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
    let place = match (p.first(), p.last()) {
        (Some(a), Some(b)) if a == b => format!("[{a}]"),
        (Some(a), Some(b)) => format!("[{a}-{b}]"),
        _ => String::new(),
    };
    let note = match (v.get("caretOnly").and_then(Value::as_bool), v.get("note").and_then(Value::as_str)) {
        (Some(true), Some(n)) => format!(" ({n})"),
        _ => String::new(),
    };
    Ok(one_line(&format!("OWNER'S SELECTION {place}{note}: {}", v.get("text").and_then(Value::as_str).unwrap_or(""))))
}

/// Page `page` (1-based) as PNG bytes, at most [`MAX_PNG`].
pub fn view_page(c: &mut dyn Caller, page: u64) -> Result<Vec<u8>, Failure> {
    let v = call(c, "view.page", json!({"page": page, "scale": 1.0}))?;
    let no_png = || Failure::error("the window sent no PNG");
    // Decoded before the size check: the reply bounds it (a `Link` reads at most 64 MiB).
    let png = v.get("png_base64").and_then(Value::as_str).and_then(wordcraft_engine::cmd::insert::base64_decode).ok_or_else(no_png)?;
    if png.len() > MAX_PNG {
        return Err(Failure::error(format!("the page image is larger than {} MiB", MAX_PNG >> 20)));
    }
    if !png.starts_with(b"\x89PNG") {
        return Err(no_png());
    }
    Ok(png)
}

/// One step of a steps file: a command id and its parameters (`{}` when not given).
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub cmd: String,
    pub params: Value,
}

/// `[{"cmd": id, "params": {…}}, …]`, not empty, at most [`MAX_STEPS_TEXT`] bytes.
pub fn parse_steps(text: &str) -> Result<Vec<Step>, Failure> {
    if text.len() > MAX_STEPS_TEXT {
        return Err(Failure::usage(format!("the steps are longer than {} MiB", MAX_STEPS_TEXT >> 20)));
    }
    let v: Value = serde_json::from_str(text).map_err(|e| Failure::usage(format!("steps must be a JSON list: {e}")))?;
    let list = v.as_array().filter(|a| !a.is_empty()).ok_or_else(|| Failure::usage("steps must be a non-empty JSON list"))?;
    list.iter()
        .enumerate()
        .map(|(n, s)| match s.get("cmd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            Some(cmd) => Ok(Step { cmd: cmd.to_string(), params: s.get("params").filter(|p| !p.is_null()).cloned().unwrap_or(json!({})) }),
            None => Err(Failure::usage(format!("step {} has no \"cmd\"", n.saturating_add(1)))),
        })
        .collect()
}

/// One printed step line: `n cmd result` (the result shortened).
fn step_line(n: usize, cmd: &str, result: &Value) -> String {
    let short: String = result.to_string().chars().take(STEP_RESULT).collect();
    one_line(&format!("{n} {cmd} {short}"))
}

/// Run the steps in order; stop at the first failure and say which steps did not run.
pub fn run_steps(c: &mut dyn Caller, steps: &[Step]) -> (Vec<String>, Exit) {
    let mut lines = Vec::new();
    for (k, s) in steps.iter().enumerate() {
        let n = k.saturating_add(1);
        match c.call("engine.execute", json!({"command": s.cmd, "params": s.params})) {
            Ok(v) => lines.push(step_line(n, &s.cmd, &v)),
            Err(LinkError::Unauthorized) => {
                lines.push(REMOVED.to_string());
                return (lines, Exit::Removed);
            }
            Err(LinkError::Refused) => {
                lines.push(GONE.to_string());
                return (lines, Exit::Gone);
            }
            Err(e) => {
                lines.push(step_line(n, &s.cmd, &json!({"ERROR": failure(e).message})));
                if n < steps.len() {
                    lines.push(format!("STOPPED after step {n} failed: steps {}..{} were NOT run", n.saturating_add(1), steps.len()));
                }
                return (lines, Exit::Error);
            }
        }
    }
    (lines, Exit::Ok)
}

/// The commands this member may run, `id params`, filtered (case-insensitive).
pub fn commands(c: &mut dyn Caller, filter: &str) -> Result<Vec<String>, Failure> {
    let v = call(c, "engine.commands", json!({}))?;
    let f = filter.to_lowercase();
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            one_line(&format!("{:28} {}", c.get("id").and_then(Value::as_str).unwrap_or(""), c.get("params").and_then(Value::as_str).unwrap_or("")))
        })
        .filter(|l| l.to_lowercase().contains(&f))
        .collect())
}
