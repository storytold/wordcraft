//! The subcommands: each takes its parsed arguments and the output streams, and returns the exit status.

use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};
use wordcraft_engine::{CmdError, Session};

use crate::args::{self, Command, ConvertArgs, FileArgs, JsonArgs, McpArgs, ParityArgs, Password, RenderArgs, RunArgs, ValidateArgs, ZoteroArgs};
use crate::error::CliError;
use crate::script;
use crate::{Io, errln};

type Result<T> = std::result::Result<T, CliError>;

fn engine_error(e: impl std::fmt::Display) -> CliError {
    CliError::Failed(escape_text(&e.to_string(), false))
}

/// Open a document for a subcommand. A password-protected Word file needs its password
/// ([`password_from`]). As in the app, a document opened with one is saved with it again, so
/// `convert` does not quietly write a copy the password no longer protects.
fn open(path: &Path, password: Option<&str>) -> Result<Session> {
    // A file that gives no page size (HTML, Markdown, plain text) opens on the system region's
    // paper, as in the app: A4 in Italy, Letter in the US (#1381).
    let region = system_region();
    let (doc, encrypted) = wordcraft_engine::io::open_path_in(path, password, &region).map_err(engine_error)?;
    let mut s = session(doc);
    s.path = Some(path.into());
    if encrypted {
        s.password = password.map(wordcraft_engine::io::Password::new);
    }
    Ok(s)
}

/// The password for a protected Word file: `--password`, `--password-file` or `WORDCRAFT_PASSWORD` (clap reads the
/// variable into `--password`; a blank one counts as unset). The file form exists because a password on a command line
/// is readable by every other process on the machine; a trailing newline is dropped so a file written by `echo` works.
/// Never logged, and never named in an error.
pub(crate) fn password_from(open: &Password) -> Result<Option<String>> {
    if let Some(password) = &open.password {
        return Ok(Some(password.clone()));
    }
    if let Some(path) = &open.password_file {
        let raw =
            std::fs::read_to_string(path).map_err(|e| CliError::Failed(escape_text(&format!("--password-file {}: {e}", path.display()), false)))?;
        return Ok(Some(raw.trim_end_matches(['\r', '\n']).to_string()));
    }
    Ok(None)
}

/// A path the engine takes as text: output paths must be valid Unicode (only the opened file may not be, #1440).
fn unicode(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| CliError::Failed(format!("{}: the file name is not valid Unicode", path.display().to_string().escape_debug())))
}

/// A headless session in the system region: every way the CLI starts one gives `file.new` the
/// same paper (#1381).
fn session(doc: wordcraft_doc::Document) -> Session {
    let mut s = Session::new(doc);
    s.system_region = system_region();
    s
}

/// The system region, read as the app reads it (locale variables, else the platform's locale).
fn system_region() -> wordcraft_engine::Region {
    wordcraft_engine::Region::from_locale_vars(|name| std::env::var(name).ok(), sys_locale::get_locale)
}

/// The body text for `text`: the plain-text export, so list items keep their bullet or number label.
fn listed_text(doc: &wordcraft_doc::Document) -> std::result::Result<String, String> {
    let bytes = wordcraft_engine::io::save_bytes("out.txt", doc)?;
    Ok(String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
}

/// `--author NAME`: the author recorded on tracked changes and comments (`file.setAuthor`).
/// The command also checks whether the open document is protected: this name determines which
/// named editing exceptions the session can use, so the flag must not bypass that check.
fn set_author(s: &mut Session, name: &str) -> std::result::Result<(), String> {
    s.run("file.setAuthor", &json!({"name": name})).map(|_| ()).map_err(|e| escape_text(&format!("--author: {e}"), false))
}

/// `mcp`'s headless backend: a session confined to `roots`, its author `author` when given. The caller confines the
/// process to the same roots first (`confine_process`), which a test must not do.
fn headless_mcp(
    roots: std::sync::Arc<wordcraft_engine::file_access::AutomationRoots>,
    author: Option<&str>,
) -> std::result::Result<wordcraft_mcp::Headless, String> {
    let mut headless = wordcraft_mcp::Headless::default().with_automation_roots(Some(roots));
    if let Some(name) = author {
        set_author(&mut headless.session, name)?;
    }
    Ok(headless)
}

/// A failed `--strict` export: a strict refusal becomes [`CliError::Strict`] carrying what stderr
/// shows (each line of the report as `error: …`, then its JSON on one line, from
/// `file.strictReport` for `page` of a page image, else the PDF); any other error is an ordinary
/// failure.
fn refusal(s: &mut Session, e: wordcraft_engine::CmdError, markup: bool, page: Option<u64>) -> CliError {
    if !matches!(&e, wordcraft_engine::CmdError::Failed(m) if m.starts_with("strict: ")) {
        return engine_error(e);
    }
    let mut params = json!({"markup": markup});
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    CliError::Strict(match s.run("file.strictReport", &params).map_err(|e| e.to_string()) {
        Ok(json) => {
            let mut text = String::new();
            for line in report_lines(&json) {
                text += &format!("error: {}\n", escape_peer(&line));
            }
            text + &json_line_safe(&json.to_string())
        }
        Err(e) => format!("error: strict: refused, and the report could not be made: {}", escape_text(&e, false)),
    })
}

/// The report's human lines, from its JSON (the engine's `StrictReport::lines`).
fn report_lines(json: &Value) -> Vec<String> {
    wordcraft_engine::strict::lines_from_json(json)
}

/// Explicitly update unlocked fields in a session before saving the document.
///
/// If field updates are disabled by document protection (`CmdError::Disabled`), logs a warning to
/// stderr and continues conversion rather than aborting. Other command failures are converted to
/// [`CliError`] via [`engine_error`].
///
/// # Arguments
///
/// * `s` - The active document [`Session`] whose unlocked fields should be refreshed.
///
/// # Errors
///
/// Returns `Err(CliError)` if the command fails with an error other than `CmdError::Disabled`.
fn update_fields(s: &mut Session, io: &mut Io) -> Result<()> {
    if let Err(e) = s.run("references.updateFields", &json!({})) {
        if matches!(e, CmdError::Disabled(_)) {
            errln!(io, "warning: {e}");
        } else {
            return Err(engine_error(e));
        }
    }
    Ok(())
}

/// At most 400 characters of `s`, for traces.
fn clip(s: &str) -> String {
    match s.char_indices().nth(400) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or(s)),
        None => s.to_string(),
    }
}

/// Text from another program (Zotero, or whatever answered on its port) for one diagnostic
/// line: backslashes, control characters (newline, escape, C1…), line separators and bidi
/// overrides are shown as escapes (`\\`, `\n`, `\u{1b}`), so it can't end its line, drive the
/// terminal or pass for other output.
fn escape_peer(s: &str) -> String {
    escape_text(s, true)
}

/// Whether `c` must not reach a terminal as itself: a control character, a line or paragraph
/// separator, a bidi control, or one that shows nothing (soft hyphen, zero-width characters, word
/// joiners, the byte order mark, tag characters), which can hide or reorder what a line says.
fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
                | '\u{e0000}'..='\u{e007f}'
        )
}

/// [`escape_peer`], with backslashes escaped only when `backslash` (error messages quote Windows
/// paths, and a backslash can't forge or hide anything).
fn escape_text(s: &str, backslash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' if backslash => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if is_unsafe_char(c) => out.extend(c.escape_unicode()),
            c => out.push(c),
        }
    }
    out
}

/// A JSON line safe to print: serde escapes control characters below U+0020 but not the other
/// characters [`is_unsafe_char`] names, which a crafted file could put in a report. They become
/// `\uXXXX` escapes (a surrogate pair above U+FFFF), so the line parses to the same value and
/// can't drive or spoof a terminal.
fn json_line_safe(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if is_unsafe_char(c) {
            for unit in c.encode_utf16(&mut [0u16; 2]) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    out
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

/// Write a `validate` report for people: the verdict, then one line per finding
/// (`SEVERITY RULE [part] message`), then the coverage notes.
fn write_validate_report(o: &mut dyn Write, report: &wordcraft_validate::Report) -> std::io::Result<()> {
    o.write_all(render_validate_report(report).as_bytes())
}

/// Render a `validate` report for people. Document-controlled text — a ZIP entry name flowing into
/// a finding part or coverage note — is escaped so a name cannot end its line or drive the
/// terminal; printable text, including Unicode, is left as written.
///
/// The `--json` form is not escaped by hand; it is the validator's own `serde_json` serialization.
/// `serde_json` escapes `U+0000`–`U+001F`, `"` and `\`; C1 controls such as `U+009B` and the bidi
/// embeddings/overrides (`U+202A`–`U+202E`, `U+2066`–`U+2069`) are emitted raw. So a control or bidi
/// run carried by a ZIP entry name survives into the JSON text; JSON is only safe because a
/// *parser* takes the string value as data. If that text is printed directly to a terminal, the raw
/// C1/bidi run reaches the terminal exactly as it would from any other text, so the JSON form is
/// not a terminal-safe rendering. Only the human report below escapes those characters, and it does
/// so to defend the terminal; keeping the JSON byte-for-byte the validator's own serialization is
/// why its escaping is left to `serde_json`.
fn render_validate_report(report: &wordcraft_validate::Report) -> String {
    let mut out = format!("{}\n", verdict_label(report.verdict));
    for f in &report.findings {
        let severity = severity_label(f.severity);
        match &f.part {
            Some(part) => out.push_str(&format!("{severity} {} [{}] {}\n", f.rule, escape_report_text(part), escape_report_text(&f.message))),
            None => out.push_str(&format!("{severity} {} {}\n", f.rule, escape_report_text(&f.message))),
        }
    }
    for note in &report.coverage_notes {
        out.push_str(&format!("note: {}\n", escape_report_text(note)));
    }
    out
}

/// Escape the control and invisible-formatting characters that could spoof a report line, in the
/// same escape *form* `escape_debug` uses (`\u{1b}`, `\n`); printable characters (including Unicode)
/// are left untouched. The *selection* is a hand-picked set ([`is_invisible_format`] plus
/// `char::is_control`), not `escape_debug`'s full non-printable set: it omits, for example, the
/// U+E0000 tag block, U+FFF9–U+FFFB and the Hangul fillers. None of those can end a line or drive a
/// terminal, which is the threat this guards against. Backslash is not escaped, so a name that
/// literally contains the text `\u{1b}` prints exactly like an escaped ESC and the output is not
/// injective.
fn escape_report_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() || is_invisible_format(c) {
            out.extend(c.escape_debug());
        } else {
            out.push(c);
        }
    }
    out
}

/// Invisible Unicode formatting characters and line/paragraph separators: no glyph, but they can
/// alter how a terminal renders the rest of the line (`char::is_control` covers C0/C1 controls).
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'
    )
}

/// Map a validation verdict enum to its human-readable uppercase display label.
fn verdict_label(v: wordcraft_validate::Verdict) -> &'static str {
    match v {
        wordcraft_validate::Verdict::Pass => "PASS",
        wordcraft_validate::Verdict::Fail => "FAIL",
        wordcraft_validate::Verdict::Incomplete => "INCOMPLETE",
    }
}

/// Map a validation severity enum to its human-readable uppercase display label.
fn severity_label(s: wordcraft_validate::Severity) -> &'static str {
    match s {
        wordcraft_validate::Severity::Corrupt => "CORRUPT",
        wordcraft_validate::Severity::Error => "ERROR",
        wordcraft_validate::Severity::Warning => "WARNING",
    }
}

/// Why `mcp --connect` is refused: the running app would not confine the agent's file access.
// lean: refused outright; allow it once the app confines its control channel and reports both roots.
const MCP_CONNECT_REFUSED: &str = "mcp: --connect is refused for now: the desktop app does not confine its control \
     channel to automation roots yet (storytold/wordcraft#3695, #5819). Use the headless server: wordcraft-cli mcp \
     --automation-read-root DIR --automation-write-root DIR";

/// Run one parsed subcommand; the result is the exit status.
pub(crate) fn dispatch(command: Command, io: &mut Io) -> Result<u8> {
    match command {
        Command::Convert(a) => convert(a, io).map(|()| 0),
        Command::Info(a) => info(a, io).map(|()| 0),
        Command::Text(a) => text(a, io).map(|()| 0),
        Command::Inspect(a) => inspect(a, io).map(|()| 0),
        Command::Render(a) => render(a, io).map(|()| 0),
        Command::Run(a) => run_script(a, io).map(|()| 0),
        Command::Commands(a) => commands(a, io).map(|()| 0),
        Command::Parity(a) => parity(a, io).map(|()| 0),
        Command::Validate(a) => validate(a, io),
        Command::Mcp(a) => mcp(a, io).map(|()| 0),
        Command::Zotero(a) => zotero(a, io).map(|()| 0),
    }
}

/// Write to stdout and flush. A closed stdout comes back as [`CliError::StdoutClosed`] instead of a panic.
fn write_stdout(io: &mut Io, write: impl FnOnce(&mut dyn Write) -> std::io::Result<()>) -> Result<()> {
    write(&mut *io.out)?;
    io.out.flush()?;
    Ok(())
}

fn convert(a: ConvertArgs, io: &mut Io) -> Result<()> {
    let o = unicode(&a.output)?.to_owned();
    // Before the input is opened: a wrong output type is a usage error, not a file error.
    let to_png = o.to_ascii_lowercase().ends_with(".png");
    if a.strict && !(to_png || o.to_ascii_lowercase().ends_with(".pdf")) {
        return Err(CliError::Usage("convert: --strict needs a .pdf or .png output".into()));
    }
    let password = password_from(&a.open)?;
    let mut s = open(&a.input, password.as_deref())?;
    if a.update_fields {
        update_fields(&mut s, io)?;
    }
    let r = if to_png {
        s.run("file.exportPng", &json!({"path": o, "page": 1, "markup": a.markup, "strict": a.strict}))
    } else {
        let mut params = json!({"path": o, "markup": a.markup, "strict": a.strict});
        if let Some(f) = a.format {
            params["format"] = json!(f.name());
        }
        s.run("file.save", &params)
    };
    if let Err(e) = r {
        return Err(if a.strict { refusal(&mut s, e, a.markup, to_png.then_some(1)) } else { engine_error(e) });
    }
    errln!(io, "wrote {o}");
    Ok(())
}

fn open_file(a: &FileArgs) -> Result<Session> {
    let password = password_from(&a.open)?;
    open(&a.file, password.as_deref())
}

fn info(a: FileArgs, io: &mut Io) -> Result<()> {
    let mut s = open_file(&a)?;
    let info = serde_json::to_string_pretty(&s.run("file.info", &json!({})).map_err(engine_error)?).unwrap_or_default();
    write_stdout(io, |o| writeln!(o, "{info}"))
}

fn text(a: FileArgs, io: &mut Io) -> Result<()> {
    let s = open_file(&a)?;
    let text = listed_text(&s.doc)?;
    write_stdout(io, |o| writeln!(o, "{text}"))
}

fn inspect(a: FileArgs, io: &mut Io) -> Result<()> {
    let mut s = open_file(&a)?;
    let tree = serde_json::to_string_pretty(&s.run("document.inspect", &json!({})).map_err(engine_error)?).unwrap_or_default();
    write_stdout(io, |o| writeln!(o, "{tree}"))
}

fn render(a: RenderArgs, io: &mut Io) -> Result<()> {
    let o = unicode(&a.output)?.to_owned();
    let password = password_from(&a.open)?;
    let mut s = open(&a.input, password.as_deref())?;
    let mut params = json!({"path": o, "page": a.page, "scale": a.scale, "strict": a.strict});
    if a.strict {
        // The check and its report must see the page as it is written.
        params["markup"] = json!(false);
    }
    let r = match s.run("file.exportPng", &params) {
        Ok(r) => r,
        Err(e) if a.strict => return Err(refusal(&mut s, e, false, Some(a.page))),
        Err(e) => return Err(engine_error(e)),
    };
    write_stdout(io, |o| writeln!(o, "{r}"))
}

fn load_run_script(path: Option<&Path>) -> Result<Option<script::Script>> {
    path.map(|p| script::load(p).map_err(engine_error)).transpose()
}

fn run_script(a: RunArgs, io: &mut Io) -> Result<()> {
    let password = password_from(&a.open)?;
    let script = load_run_script(a.script.as_deref())?;
    // A source named on the command line wins over both of the script's.
    let (mut from_file, mut from_template) = (a.file.clone(), a.template.clone());
    if from_file.is_none() && from_template.is_none() {
        from_file = script.as_ref().and_then(|loaded| loaded.file.as_deref().map(std::path::PathBuf::from));
        from_template = script.as_ref().and_then(|loaded| loaded.template.clone());
    }
    let mut s = match (from_file, from_template) {
        (Some(f), _) => open(&f, password.as_deref())?,
        // Both go through `file.new`, which puts a blank document on the region's paper (#3152).
        // `file.new` opens beside the session's first document, which is then closed: the
        // run starts with one open document.
        (None, t) => {
            let mut s = session(wordcraft_doc::Document::new());
            let first = s.document_id();
            let params = t.map_or_else(|| json!({}), |t| json!({"template": t}));
            s.run("file.new", &params).map_err(engine_error)?;
            s.close_document(first).map_err(engine_error)?;
            s
        }
    };
    if let Some(name) = &a.author {
        set_author(&mut s, name).map_err(CliError::Failed)?;
    }
    for (n, (id, params)) in script.iter().flat_map(|c| c.commands.iter()).enumerate() {
        let r = s.run(id, params).map_err(|e| engine_error(format!("command {} ({id}): {e}", n + 1)))?;
        if a.print {
            write_stdout(io, |o| writeln!(o, "{id}: {r}"))?;
        }
    }
    for args::Cmd { id, params } in &a.cmds {
        let r = s.run(id, params).map_err(|e| engine_error(format!("{id}: {e}")))?;
        if a.print {
            write_stdout(io, |o| writeln!(o, "{id}: {r}"))?;
        }
    }
    let mut outputs: Vec<String> = script.iter().flat_map(|c| c.save.iter().cloned()).collect();
    if let Some(save) = &a.save {
        outputs.push(unicode(save)?.to_owned());
    }
    // Check every output can be made before writing any, so a bad one leaves no half-done job.
    for out in &outputs {
        let parent = Path::new(out).parent().filter(|p| !p.as_os_str().is_empty());
        if let Some(p) = parent.filter(|p| !p.is_dir()) {
            return Err(format!("{out}: folder {} does not exist", p.display()).into());
        }
        if Path::new(out).is_dir() {
            return Err(format!("{out}: is a folder, not a file").into());
        }
        if !out.to_ascii_lowercase().ends_with(".png") {
            wordcraft_engine::io::save_bytes(out, &s.doc).map_err(engine_error)?;
        }
    }
    for out in outputs {
        let r = if out.to_ascii_lowercase().ends_with(".png") {
            let mut v = json!({"path": out});
            if let Some(n) = a.page {
                v["page"] = json!(n);
            }
            if let Some(k) = a.scale {
                v["scale"] = json!(k);
            }
            s.run("file.exportPng", &v)
        } else {
            s.run("file.save", &json!({"path": out}))
        };
        r.map_err(engine_error)?;
        errln!(io, "wrote {out}");
    }
    Ok(())
}

fn commands(a: JsonArgs, io: &mut Io) -> Result<()> {
    let s = session(wordcraft_doc::Document::new());
    if a.json {
        let list = serde_json::to_string_pretty(&s.registry.describe()).unwrap_or_default();
        write_stdout(io, |o| writeln!(o, "{list}"))
    } else {
        write_stdout(io, |o| {
            for c in s.registry.all() {
                writeln!(o, "{:<32} {:<36} {:<24} {}", c.id, c.label, c.shortcut, c.location)?;
            }
            writeln!(o, "{:<32} Structural integrity check", "validate")
        })
    }
}

fn parity(a: ParityArgs, io: &mut Io) -> Result<()> {
    let s = session(wordcraft_doc::Document::new());
    let p = wordcraft_engine::catalog::parity(&s.registry);
    if a.markdown {
        write_stdout(io, |o| write!(o, "{}", parity_markdown(&p)))
    } else {
        write_stdout(io, |o| writeln!(o, "{}", serde_json::to_string_pretty(&p).unwrap_or_default()))
    }
}

fn mcp(a: McpArgs, io: &mut Io) -> Result<()> {
    if a.connect.is_some() {
        return Err(MCP_CONNECT_REFUSED.into());
    }
    // Shared rule 18 (#7124): an agent entry point never runs unconfined. clap requires both roots.
    let roots = wordcraft_engine::file_access::AutomationRoots::new(a.read_root.as_deref(), a.write_root.as_deref())
        .map_err(|e| CliError::Failed(format!("mcp: {e}")))?;
    let Some(roots) = roots.filter(|r| r.read_root().is_some() && r.write_root().is_some()) else {
        return Err(CliError::Usage("mcp: give both --automation-read-root DIR and --automation-write-root DIR".into()));
    };
    // Every thread of this process, not only the server's: nothing reaches past the roots.
    wordcraft_engine::file_access::confine_process(roots.clone());
    let show = |root: Option<&Path>| root.map_or_else(|| "none".to_string(), |r| r.display().to_string());
    errln!(io, "files confined: read root {}, write root {}", show(roots.read_root()), show(roots.write_root()));
    let backend: Box<dyn wordcraft_mcp::Backend> = Box::new(headless_mcp(roots, a.author.as_deref()).map_err(CliError::Failed)?);
    let mut server = wordcraft_mcp::Server::new(backend);
    let stdin = std::io::stdin();
    // A client that closes our stdout has hung up: `serve` returns BrokenPipe and we end cleanly.
    Ok(server.serve(stdin.lock(), &mut *io.out)?)
}

fn zotero(a: ZoteroArgs, io: &mut Io) -> Result<()> {
    let command =
        wordcraft_zotero::Command::from_name(&a.command).ok_or_else(|| CliError::Usage(format!("unknown Zotero command `{}`", a.command)))?;
    let mut s = open(&a.file, None)?;
    for args::Cmd { id, params } in &a.cmds {
        s.run(id, params).map_err(|e| engine_error(format!("{id}: {e}")))?;
    }
    let mut opts = wordcraft_zotero::client::Options::default();
    if let Some(p) = a.port {
        opts.addr.set_port(p);
    }
    let mut bridge = wordcraft_zotero::Bridge::new();
    let mut host = wordcraft_zotero::Headless::default();
    let mut trace = Vec::new();
    let out = wordcraft_zotero::client::run_command(&opts, command, &mut |call| {
        let r = bridge.handle(&mut s, &mut host, call);
        if a.trace {
            trace.push(format!("← {} {}", escape_peer(&call.method), escape_peer(&clip(&Value::Array(call.args.clone()).to_string()))));
            trace.push(format!("→ {}", escape_peer(&clip(&String::from_utf8_lossy(&wordcraft_zotero::wire::encode_reply(&r))))));
        }
        r
    })
    .map_err(engine_error)?;
    for line in &trace {
        errln!(io, "{line}");
    }
    for alert in &host.alerts {
        errln!(io, "Zotero: {}", escape_peer(alert));
    }
    errln!(io, "{} calls, {}", out.calls, if out.completed { "completed" } else { "ended without completing (cancelled?)" });
    if let Some(o) = &a.save {
        let o = unicode(o)?;
        s.run("file.save", &json!({"path": o})).map_err(engine_error)?;
        errln!(io, "wrote {o}");
    }
    Ok(())
}

/// `validate`: 0 for PASS, 1 for any other verdict or a file that can't be read (RFC-0018 rule 1.2 has no other code;
/// the verdict is in the report).
fn validate(a: ValidateArgs, io: &mut Io) -> Result<u8> {
    let report =
        wordcraft_validate::validate_path(&a.file).map_err(|e| CliError::Failed(escape_text(&format!("{}: {e}", a.file.display()), false)))?;
    let written = if a.json {
        let json = serde_json::to_string_pretty(&report).unwrap_or_default();
        write_stdout(io, |o| writeln!(o, "{json}"))
    } else {
        write_stdout(io, |o| write_validate_report(o, &report))
    };
    match written {
        // A reader closing early must not turn a FAIL or INCOMPLETE verdict into PASS.
        Ok(()) | Err(CliError::StdoutClosed) => {}
        Err(error) => return Err(error),
    }
    Ok(match report.verdict {
        wordcraft_validate::Verdict::Pass => 0,
        wordcraft_validate::Verdict::Fail | wordcraft_validate::Verdict::Incomplete => 1,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
