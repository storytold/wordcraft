//! `wordcraft-cli`: WordCraft from the command line.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod automation_args;
mod script;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use wordcraft_engine::Session;

const USAGE: &str = "\
wordcraft-cli — WordCraft from the command line

USAGE:
  wordcraft-cli convert <in> <out> [--markup] [--update-fields] [--format wordml2003|flatopc]
                                              convert between formats (docx, doc [read], pdf, odt, rtf, html, md, tex, txt, json, png,
                                              xml); --update-fields refreshes unlocked fields before saving;
                                              an .xml file is a Word XML Document (Flat OPC), or with
                                              --format wordml2003 a Word 2003 XML Document;
                                              --markup (Print Markup): comments beside each page and
                                              tracked changes in pdf/png, a comment list in html/md/txt
  wordcraft-cli info <file>                   pages, words, paragraphs, properties (JSON)
  wordcraft-cli text <file>                   plain text
  wordcraft-cli inspect <file>                document structure (JSON)
  wordcraft-cli render <file> <out.png> [--page N] [--scale S]
  wordcraft-cli run [--file F | --template T] [--script FILE|-] --cmd 'id={json}' [--cmd …]
                    [--save OUT] [--print] [--author NAME]
                                              run commands headlessly, then save (--author names
                                              who tracked changes and comments are by).
                                              --script runs a JSON file (- is stdin) first:
                                              {\"file\"?, \"template\"?, \"commands\": [{\"command\": id,
                                              \"params\"?: {…}} | id, …], \"save\"?: [OUT, …]}; save
                                              formats follow the extension (docx and pdf in one run);
                                              relative paths resolve against the script's folder.
                                              file.new and file.open open another document and
                                              make it active; later commands and the saves work on
                                              the active one; view.switchWindows lists the open
                                              documents and with {\"id\": N} makes one active
  wordcraft-cli commands [--json]             list every command
  wordcraft-cli parity [--markdown]           feature-catalog parity
  wordcraft-cli mcp [--connect HOST:PORT | --author NAME] [--automation-read-root DIR] [--automation-write-root DIR]
                                              MCP server on stdio (headless, or bridged to the app).
                                              --automation-read-root / --automation-write-root confine
                                              the files the agent's commands read (open, insert text
                                              or picture, mail-merge data, templates…) and write (save,
                                              export, merge output…) to DIR, links followed; a root left
                                              out grants none of its access. They apply to the headless
                                              server and are an error with --connect.
  wordcraft-cli zotero <command> <file> [--cmd 'id={json}' …] [--save OUT] [--trace] [--port P]
                                              run a Zotero command on a document (Zotero must be
                                              running): addEditCitation, addEditBibliography,
                                              addNote, refresh, removeCodes, setDocPrefs
                                              (--cmd runs first, e.g. --cmd caret.docEnd)
  wordcraft-cli [<command>] -h | --help       this help
  wordcraft-cli --version

A password-protected Word file is opened with --password PASS, --password-file PATH or the
WORDCRAFT_PASSWORD environment variable; a password given on the command line is readable by
every other process, so the other two are better. A file opened with a password is written with
it again, as the app does. Exports that cannot carry a password (pdf, html, txt) stay plain.

Paper: documents created here, and HTML, Markdown and text files (which give no page size), use
the system region's paper (A4, or Letter in the US and other Letter regions), read from
LC_ALL / LC_PAPER / LANG or the platform. Set WORDCRAFT_REGION=en-US (or another tag) for the
same output on every machine.
";

/// The subcommands, in `USAGE` order; `--help` or `-h` after any of them prints `USAGE` (#968).
const SUBCOMMANDS: &[&str] = &["convert", "info", "text", "inspect", "render", "run", "commands", "parity", "mcp", "zotero"];

fn wants_help(cmd: &str, rest: &[String]) -> bool {
    SUBCOMMANDS.contains(&cmd) && rest.iter().any(|a| a == "--help" || a == "-h")
}

/// The error every subcommand gives when a required path is not there.
fn missing_argument() -> Failure {
    format!("missing argument\n\n{USAGE}").into()
}

/// Open a document for a subcommand. A password-protected Word file needs its password
/// ([`password_from`]). As in the app, a document opened with one is saved with it again, so
/// `convert` does not quietly write a copy the password no longer protects.
fn open(path: &Path, password: Option<&str>) -> Result<Session, String> {
    // A file that gives no page size (HTML, Markdown, plain text) opens on the system region's
    // paper, as in the app: A4 in Italy, Letter in the US (#1381).
    let region = system_region();
    let (doc, encrypted) = wordcraft_engine::io::open_path_in(path, password, &region)?;
    let mut s = session(doc);
    s.path = Some(path.into());
    if encrypted {
        s.password = password.map(wordcraft_engine::io::Password::new);
    }
    Ok(s)
}

/// The commands that open a file, and which of their arguments not starting with `--` it is (as
/// `pos` counts). `run` opens the value of `--file`.
const OPENS_FILE: &[(&str, usize)] = &[("convert", 0), ("info", 0), ("text", 0), ("inspect", 0), ("render", 0), ("zotero", 1)];

/// The arguments as text. Only the file the command opens may have a name that isn't valid Unicode,
/// such as a Latin-1 one (#1440): it comes back as the path to open, and lossy among the arguments.
/// Any other such argument is an error naming it.
fn text_args<S: AsRef<OsStr>>(raw: &[S]) -> Result<(Vec<String>, Option<PathBuf>), String> {
    let args: Vec<String> = raw.iter().map(|a| a.as_ref().to_string_lossy().into_owned()).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let file_at = match OPENS_FILE.iter().find(|(c, _)| *c == cmd) {
        Some((_, n)) => args.iter().enumerate().skip(1).filter(|(_, a)| !a.starts_with("--")).nth(*n).map(|(i, _)| i),
        None if cmd == "run" => args.iter().position(|a| a == "--file").map(|i| i + 1),
        None => None,
    };
    // `run --script FILE` may also name its script with such a file name.
    let script_at = if cmd == "run" { args.iter().position(|a| a == "--script").map(|i| i + 1) } else { None };
    let mut file = None;
    for (i, a) in raw.iter().map(AsRef::as_ref).enumerate() {
        if a.to_str().is_none() {
            if Some(i) != file_at && Some(i) != script_at {
                return Err(format!("argument {} is not valid Unicode: {}", i + 1, a.to_string_lossy()));
            }
            if Some(i) == file_at {
                file = Some(PathBuf::from(a));
            }
        }
    }
    Ok((args, file))
}

/// The password for a protected Word file: `--password`, `--password-file`, or
/// `WORDCRAFT_PASSWORD`, in that order. The file form exists because a password on a command line
/// is readable by every other process on the machine; a trailing newline is dropped so a file
/// written by `echo` works. Never logged, and never named in an error.
fn password_from(args: &[String]) -> Result<Option<String>, String> {
    if let Some(password) = arg_value(args, "--password") {
        return Ok(Some(password));
    }
    if let Some(path) = arg_value(args, "--password-file") {
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("--password-file {path}: {e}"))?;
        return Ok(Some(raw.trim_end_matches(['\r', '\n']).to_string()));
    }
    Ok(std::env::var("WORDCRAFT_PASSWORD").ok().filter(|p| !p.is_empty()))
}

/// The positional arguments, skipping every option's value so an option value is never read as a
/// path. Mirrors [`validate_options`], which has already checked the options are known.
fn positionals<'a>(args: &'a [String], specs: &[OptionSpec]) -> Vec<&'a String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        let takes_value =
            arg.starts_with("--") && specs.iter().any(|s| s.name == arg && s.takes_value) && args.get(i + 1).is_some_and(|v| !v.starts_with("--"));
        if takes_value {
            i += 2;
        } else {
            if !arg.starts_with("--") {
                out.push(arg);
            }
            i += 1;
        }
    }
    out
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
fn listed_text(doc: &wordcraft_doc::Document) -> Result<String, String> {
    let bytes = wordcraft_engine::io::save_bytes("out.txt", doc)?;
    Ok(String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
}

/// `--author NAME`: the author recorded on tracked changes and comments (`file.setAuthor`).
/// The command also checks whether the open document is protected: this name determines which
/// named editing exceptions the session can use, so the flag must not bypass that check.
fn set_author(s: &mut Session, name: &str) -> Result<(), String> {
    s.run("file.setAuthor", &json!({"name": name})).map(|_| ()).map_err(|e| format!("--author: {e}"))
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

enum OptionArg<'a> {
    Absent,
    Duplicate,
    MissingValue,
    Value(&'a str),
}

fn find_option_arg<'a>(args: &'a [String], name: &str) -> OptionArg<'a> {
    let mut matches = args.iter().enumerate().filter(|(_, a)| *a == name);
    let Some((i, _)) = matches.next() else {
        return OptionArg::Absent;
    };
    if matches.next().is_some() {
        return OptionArg::Duplicate;
    }
    match args.get(i + 1) {
        None => OptionArg::MissingValue,
        Some(val) if val.starts_with("--") => OptionArg::MissingValue,
        Some(val) => OptionArg::Value(val.as_str()),
    }
}

fn parse_page_arg(command: &str, args: &[String]) -> Result<Option<u64>, String> {
    match find_option_arg(args, "--page") {
        OptionArg::Absent => Ok(None),
        OptionArg::Duplicate => Err(format!("{command}: duplicate --page option")),
        OptionArg::MissingValue => Err(format!("{command}: --page requires a value")),
        OptionArg::Value(val) => {
            let n = val.parse::<u64>().map_err(|_| format!("{command}: invalid --page `{val}`: not a valid integer"))?;
            if n == 0 {
                return Err(format!("{command}: invalid --page `0`: page numbers start at 1"));
            }
            Ok(Some(n))
        }
    }
}

fn parse_scale_arg(command: &str, args: &[String]) -> Result<Option<f64>, String> {
    match find_option_arg(args, "--scale") {
        OptionArg::Absent => Ok(None),
        OptionArg::Duplicate => Err(format!("{command}: duplicate --scale option")),
        OptionArg::MissingValue => Err(format!("{command}: --scale requires a value")),
        OptionArg::Value(val) => {
            let scale = val.parse::<f64>().map_err(|_| format!("{command}: invalid --scale `{val}`: not a number"))?;
            if !scale.is_finite() || scale <= 0.0 {
                return Err(format!("{command}: invalid --scale `{val}`: scale must be a positive number"));
            }
            Ok(Some(scale))
        }
    }
}

#[derive(Clone, Copy)]
struct OptionSpec {
    name: &'static str,
    takes_value: bool,
}

fn validate_options(command: &str, args: &[String], specs: &[OptionSpec]) -> Result<(), String> {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if let Some(name) = arg.strip_prefix("--") {
            let Some(spec) = specs.iter().find(|spec| spec.name == arg) else {
                return Err(format!("{command}: unknown option --{name}"));
            };
            if spec.takes_value && args.get(i + 1).is_some_and(|value| !value.starts_with("--")) {
                i += 2;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Take `--automation-read-root` / `--automation-write-root` (each `--flag DIR` or `--flag=DIR`,
/// once) out of `args`, and open the roots, which must be existing folders. Returns the arguments
/// that are left. A malformed root flag is an error, never a root silently left out.
fn automation_roots(
    command: &str,
    args: &[String],
) -> Result<(Option<std::sync::Arc<wordcraft_engine::file_access::AutomationRoots>>, Vec<String>), String> {
    let mut found = automation_args::AutomationArgs::new(None, None);
    let mut left = Vec::new();
    let mut it = args.iter().cloned();
    while let Some(arg) = it.next() {
        if !found.consume(&arg, &mut it).map_err(|e| format!("{command}: {e}"))? {
            left.push(arg);
        }
    }
    let roots = wordcraft_engine::file_access::AutomationRoots::new(found.read_root.as_deref(), found.write_root.as_deref())
        .map_err(|e| format!("{command}: {e}"))?;
    Ok((roots, left))
}

/// The password a protected Word file is opened with: `--password` (which any other process on
/// the machine can read from the command line), `--password-file`, or `WORDCRAFT_PASSWORD`.
const PASSWORD_OPTIONS: &[OptionSpec] =
    &[OptionSpec { name: "--password", takes_value: true }, OptionSpec { name: "--password-file", takes_value: true }];
const CONVERT_OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--markup", takes_value: false },
    OptionSpec { name: "--format", takes_value: true },
    OptionSpec { name: "--update-fields", takes_value: false },
    OptionSpec { name: "--password", takes_value: true },
    OptionSpec { name: "--password-file", takes_value: true },
];
const RENDER_OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--page", takes_value: true },
    OptionSpec { name: "--scale", takes_value: true },
    OptionSpec { name: "--password", takes_value: true },
    OptionSpec { name: "--password-file", takes_value: true },
];
const RUN_OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--file", takes_value: true },
    OptionSpec { name: "--password", takes_value: true },
    OptionSpec { name: "--password-file", takes_value: true },
    OptionSpec { name: "--template", takes_value: true },
    OptionSpec { name: "--cmd", takes_value: true },
    OptionSpec { name: "--script", takes_value: true },
    OptionSpec { name: "--save", takes_value: true },
    OptionSpec { name: "--page", takes_value: true },
    OptionSpec { name: "--scale", takes_value: true },
    OptionSpec { name: "--print", takes_value: false },
    OptionSpec { name: "--author", takes_value: true },
];
const COMMANDS_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--json", takes_value: false }];
const PARITY_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--markdown", takes_value: false }];
const MCP_OPTIONS: &[OptionSpec] = &[OptionSpec { name: "--connect", takes_value: true }, OptionSpec { name: "--author", takes_value: true }];
const ZOTERO_OPTIONS: &[OptionSpec] = &[
    OptionSpec { name: "--cmd", takes_value: true },
    OptionSpec { name: "--save", takes_value: true },
    OptionSpec { name: "--trace", takes_value: false },
    OptionSpec { name: "--port", takes_value: true },
];

/// Why a CLI run stopped early.
#[derive(Debug, PartialEq)]
enum Failure {
    /// A real error, reported on stderr with a failing exit status.
    Error(String),
    /// Whoever read stdout closed it (`wordcraft-cli commands --json | head`): there is nobody left
    /// to write to, so stop quietly and exit 0 like other command-line tools.
    StdoutClosed,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Error(e) => f.write_str(e),
            Failure::StdoutClosed => f.write_str("stdout closed"),
        }
    }
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Error(e)
    }
}

impl From<&str> for Failure {
    fn from(e: &str) -> Self {
        Failure::Error(e.into())
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::BrokenPipe { Failure::StdoutClosed } else { Failure::Error(e.to_string()) }
    }
}

/// Write to stdout through one locked handle and flush. Unlike `println!`, a closed stdout comes
/// back as an error (`Failure::StdoutClosed`) instead of a panic.
fn write_stdout(write: impl FnOnce(&mut std::io::StdoutLock<'static>) -> std::io::Result<()>) -> Result<(), Failure> {
    let mut out = std::io::stdout().lock();
    write(&mut out)?;
    out.flush()?;
    Ok(())
}

/// The operand after `--script`, by its real name (it may not be valid Unicode).
fn script_operand<S: AsRef<OsStr>>(raw: &[S]) -> Option<PathBuf> {
    let at = raw.iter().position(|a| a.as_ref() == OsStr::new("--script"))?;
    raw.get(at + 1).map(|a| PathBuf::from(a.as_ref()))
}

/// Whether `addr` (`HOST:PORT`, as `--connect` gives it) is at least shaped like an address: a
/// host of some kind and a numeric port. Malformed addresses fail at startup — a lazy remote
/// would otherwise retry one on every call and never say why nothing connects.
fn valid_connect_addr(addr: &str) -> bool {
    let Some((host, port)) = addr.rsplit_once(':') else { return false };
    !host.is_empty() && port.parse::<u16>().is_ok()
}

fn load_run_script(rest: &[String], script_path: Option<&Path>) -> Result<Option<script::Script>, Failure> {
    match arg_value(rest, "--script") {
        Some(path) if !path.starts_with("--") => {
            let source = script_path.unwrap_or(Path::new(&path));
            Ok(Some(script::load(source)?))
        }
        _ if rest.iter().any(|arg| arg == "--script") => Err("run: --script requires a value".into()),
        _ => Ok(None),
    }
}

fn run_source_paths(rest: &[String], script: Option<&script::Script>) -> (Option<String>, Option<String>) {
    // A source named on the command line wins over both of the script's.
    let (mut from_file, mut from_template) = (arg_value(rest, "--file"), arg_value(rest, "--template"));
    if from_file.is_none() && from_template.is_none() {
        from_file = script.and_then(|loaded| loaded.file.clone());
        from_template = script.and_then(|loaded| loaded.template.clone());
    }
    (from_file, from_template)
}

fn report_missing_metric_fonts() {
    for (word, used) in wordcraft_fonts::word::missing_metric_fonts() {
        eprintln!(
            "note: {word} and its metric-compatible twins aren't installed; the primary family is {used}, so lines and pages may not match Word's; uncovered characters may use glyph fallback"
        );
    }
}

/// Run the subcommand named by `args` (the command line after the program name). Files named
/// there are opened by their real names, which may not be valid Unicode.
fn run<S: AsRef<OsStr>>(args: &[S]) -> Result<(), Failure> {
    let script_path = script_operand(args);
    let (args, file) = text_args(args)?;
    // Opens the file by its real name, which its text in `args` may have lost, and with the
    // password of a protected document (`password_from`).
    let open = |text: &str, password: Option<&str>| open(file.as_deref().unwrap_or(Path::new(text)), password);
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    // Before options are checked or files and folders opened.
    if wants_help(cmd, &rest) {
        return write_stdout(|o| write!(o, "{USAGE}"));
    }
    match cmd {
        "convert" => {
            validate_options("convert", &rest, CONVERT_OPTIONS)?;
            // `--format wordml2003` (or `flatopc`) picks the kind of `.xml` file; its value isn't a path.
            let format = arg_value(&rest, "--format");
            let paths = positionals(&rest, CONVERT_OPTIONS);
            let (i, o) = (paths.first().ok_or_else(missing_argument)?.to_string(), paths.get(1).ok_or_else(missing_argument)?.to_string());
            let markup = rest.iter().any(|a| a == "--markup");
            let password = password_from(&rest)?;
            let mut s = open(&i, password.as_deref())?;
            report_missing_metric_fonts();
            if rest.iter().any(|a| a == "--update-fields") {
                s.run("references.updateFields", &json!({})).map_err(|e| e.to_string())?;
            }
            let r = if o.to_ascii_lowercase().ends_with(".png") {
                s.run("file.exportPng", &json!({"path": o, "page": 1, "markup": markup}))
            } else {
                let mut params = json!({"path": o, "markup": markup});
                if let Some(f) = format {
                    params["format"] = json!(f);
                }
                s.run("file.save", &params)
            };
            r.map_err(|e| e.to_string())?;
            eprintln!("wrote {o}");
            Ok(())
        }
        "info" => {
            validate_options("info", &rest, PASSWORD_OPTIONS)?;
            let paths = positionals(&rest, PASSWORD_OPTIONS);
            let password = password_from(&rest)?;
            let mut s = open(paths.first().ok_or_else(missing_argument)?, password.as_deref())?;
            let info = serde_json::to_string_pretty(&s.run("file.info", &json!({})).map_err(|e| e.to_string())?).unwrap_or_default();
            write_stdout(|o| writeln!(o, "{info}"))
        }
        "text" => {
            validate_options("text", &rest, PASSWORD_OPTIONS)?;
            let paths = positionals(&rest, PASSWORD_OPTIONS);
            let password = password_from(&rest)?;
            let s = open(paths.first().ok_or_else(missing_argument)?, password.as_deref())?;
            let text = listed_text(&s.doc)?;
            write_stdout(|o| writeln!(o, "{text}"))
        }
        "inspect" => {
            validate_options("inspect", &rest, PASSWORD_OPTIONS)?;
            let paths = positionals(&rest, PASSWORD_OPTIONS);
            let password = password_from(&rest)?;
            let mut s = open(paths.first().ok_or_else(missing_argument)?, password.as_deref())?;
            let tree = serde_json::to_string_pretty(&s.run("document.inspect", &json!({})).map_err(|e| e.to_string())?).unwrap_or_default();
            write_stdout(|o| writeln!(o, "{tree}"))
        }
        "render" => {
            validate_options("render", &rest, RENDER_OPTIONS)?;
            let paths = positionals(&rest, RENDER_OPTIONS);
            let password = password_from(&rest)?;
            let (Some(i), Some(o)) = (paths.first(), paths.get(1)) else {
                return Err(missing_argument());
            };
            let (i, o) = (i.to_string(), o.to_string());
            let page = parse_page_arg("render", &rest)?.unwrap_or(1);
            let scale = parse_scale_arg("render", &rest)?.unwrap_or(2.0);
            let mut s = open(&i, password.as_deref())?;
            let r = s.run("file.exportPng", &json!({"path": o, "page": page, "scale": scale})).map_err(|e| e.to_string())?;
            write_stdout(|o| writeln!(o, "{r}"))
        }
        "run" => {
            validate_options("run", &rest, RUN_OPTIONS)?;
            let page = parse_page_arg("run", &rest)?;
            let scale = parse_scale_arg("run", &rest)?;
            let password = password_from(&rest)?;
            let script = load_run_script(&rest, script_path.as_deref())?;
            let (from_file, from_template) = run_source_paths(&rest, script.as_ref());
            let mut s = match (from_file, from_template) {
                (Some(f), _) => open(&f, password.as_deref())?,
                // Both go through `file.new`, which puts a blank document on the region's paper (#3152).
                // `file.new` opens beside the session's first document, which is then closed: the
                // run starts with one open document.
                (None, t) => {
                    let mut s = session(wordcraft_doc::Document::new());
                    let first = s.document_id();
                    let params = t.map_or_else(|| json!({}), |t| json!({"template": t}));
                    s.run("file.new", &params).map_err(|e| e.to_string())?;
                    s.close_document(first).map_err(|e| e.to_string())?;
                    s
                }
            };
            if let Some(name) = arg_value(&rest, "--author") {
                set_author(&mut s, &name)?;
            }
            let print = rest.iter().any(|a| a == "--print");
            for (n, (id, params)) in script.iter().flat_map(|c| c.commands.iter()).enumerate() {
                let r = s.run(id, params).map_err(|e| format!("command {} ({id}): {e}", n + 1))?;
                if print {
                    write_stdout(|o| writeln!(o, "{id}: {r}"))?;
                }
            }
            let mut i = 0;
            while i < rest.len() {
                if rest.get(i).map(String::as_str) == Some("--cmd") {
                    let spec = rest.get(i + 1).ok_or("--cmd needs a value")?;
                    let (id, params) = match spec.split_once('=') {
                        Some((id, p)) => (id.to_string(), serde_json::from_str::<Value>(p).map_err(|e| format!("{id}: bad JSON params: {e}"))?),
                        None => (spec.clone(), json!({})),
                    };
                    let r = s.run(&id, &params).map_err(|e| format!("{id}: {e}"))?;
                    if print {
                        write_stdout(|o| writeln!(o, "{id}: {r}"))?;
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let outputs: Vec<String> = script.iter().flat_map(|c| c.save.iter().cloned()).chain(arg_value(&rest, "--save")).collect();
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
                    wordcraft_engine::io::save_bytes(out, &s.doc)?;
                }
            }
            for out in outputs {
                let r = if out.to_ascii_lowercase().ends_with(".png") {
                    let mut v = json!({"path": out});
                    if let Some(n) = page {
                        v["page"] = json!(n);
                    }
                    if let Some(k) = scale {
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
            validate_options("commands", &rest, COMMANDS_OPTIONS)?;
            let s = session(wordcraft_doc::Document::new());
            if rest.iter().any(|a| a == "--json") {
                let list = serde_json::to_string_pretty(&s.registry.describe()).unwrap_or_default();
                write_stdout(|o| writeln!(o, "{list}"))
            } else {
                write_stdout(|o| {
                    for c in s.registry.all() {
                        writeln!(o, "{:<32} {:<36} {:<24} {}", c.id, c.label, c.shortcut, c.location)?;
                    }
                    Ok(())
                })
            }
        }
        "parity" => {
            validate_options("parity", &rest, PARITY_OPTIONS)?;
            let s = session(wordcraft_doc::Document::new());
            let p = wordcraft_engine::catalog::parity(&s.registry);
            if rest.iter().any(|a| a == "--markdown") {
                write_stdout(|o| write!(o, "{}", parity_markdown(&p)))
            } else {
                write_stdout(|o| writeln!(o, "{}", serde_json::to_string_pretty(&p).unwrap_or_default()))
            }
        }
        "mcp" => {
            let (roots, rest) = automation_roots("mcp", &rest)?;
            validate_options("mcp", &rest, MCP_OPTIONS)?;
            let author = arg_value(&rest, "--author");
            let connect = arg_value(&rest, "--connect");
            if roots.is_some() && connect.is_some() {
                return Err(
                    "mcp: --automation-read-root and --automation-write-root confine the headless server; a running app is not confined by them"
                        .into(),
                );
            }
            let backend: Box<dyn wordcraft_mcp::Backend> = match connect {
                // The app's user name is the person's own setting; don't rewrite it from a flag.
                Some(_) if author.is_some() => {
                    return Err("mcp: --author applies to the headless server; with --connect, run file.setAuthor".into());
                }
                Some(addr) => {
                    // A malformed address must fail at startup, not on every tool call the lazy
                    // remote would retry it with.
                    if !valid_connect_addr(&addr) {
                        return Err(format!("mcp: --connect needs HOST:PORT with a numeric port, got `{addr}`").into());
                    }
                    match wordcraft_mcp::Remote::connect(&addr) {
                        Ok(remote) => {
                            eprintln!("wordcraft-cli mcp: connected to WordCraft at {addr}");
                            if let Some(w) = remote.key_warning() {
                                eprintln!("wordcraft-cli: {w}");
                            }
                            Box::new(remote)
                        }
                        // The app may start after the client does: serve anyway and connect on the
                        // first call, or the client marks the server failed for its whole session
                        // (#4078).
                        Err(e) => {
                            eprintln!("wordcraft-cli mcp: WordCraft is not reachable at {addr} yet ({e}); will retry on each call");
                            Box::new(wordcraft_mcp::Remote::lazy(&addr))
                        }
                    }
                }
                None => {
                    let mut headless = wordcraft_mcp::Headless::default();
                    if let Some(roots) = roots {
                        // Every thread of this process, not only the server's: nothing reaches past the roots.
                        wordcraft_engine::file_access::confine_process(roots.clone());
                        let show = |root: Option<&std::path::Path>| root.map_or_else(|| "none".to_string(), |r| r.display().to_string());
                        eprintln!("wordcraft-cli: files confined: read root {}, write root {}", show(roots.read_root()), show(roots.write_root()));
                        headless = headless.with_automation_roots(Some(roots));
                    }
                    if let Some(name) = author {
                        set_author(&mut headless.session, &name)?;
                    }
                    Box::new(headless)
                }
            };
            let mut server = wordcraft_mcp::Server::new(backend);
            let stdin = std::io::stdin();
            // A client that closes our stdout has hung up: `serve` returns BrokenPipe and we end cleanly.
            Ok(server.serve(stdin.lock(), std::io::stdout())?)
        }
        "zotero" => {
            validate_options("zotero", &rest, ZOTERO_OPTIONS)?;
            let paths = positionals(&rest, ZOTERO_OPTIONS);
            let (Some(name), Some(file)) = (paths.first(), paths.get(1)) else {
                return Err(missing_argument());
            };
            let command = wordcraft_zotero::Command::from_name(name).ok_or_else(|| format!("unknown Zotero command `{name}`\n\n{USAGE}"))?;
            let mut s = open(file, None)?;
            for spec in rest.iter().zip(rest.iter().skip(1)).filter(|(a, _)| *a == "--cmd").map(|(_, v)| v) {
                let (id, params) = match spec.split_once('=') {
                    Some((id, p)) => (id.to_string(), serde_json::from_str::<Value>(p).map_err(|e| format!("{id}: bad JSON params: {e}"))?),
                    None => (spec.clone(), json!({})),
                };
                s.run(&id, &params).map_err(|e| format!("{id}: {e}"))?;
            }
            let mut opts = wordcraft_zotero::client::Options::default();
            if let Some(p) = arg_value(&rest, "--port") {
                opts.addr.set_port(p.parse().map_err(|_| format!("bad port `{p}`"))?);
            }
            let trace = rest.iter().any(|a| a == "--trace");
            let mut bridge = wordcraft_zotero::Bridge::new();
            let mut host = wordcraft_zotero::Headless::default();
            let out = wordcraft_zotero::client::run_command(&opts, command, &mut |call| {
                let r = bridge.handle(&mut s, &mut host, call);
                if trace {
                    eprintln!("← {} {}", escape_peer(&call.method), escape_peer(&clip(&Value::Array(call.args.clone()).to_string())));
                    eprintln!("→ {}", escape_peer(&clip(&String::from_utf8_lossy(&wordcraft_zotero::wire::encode_reply(&r)))));
                }
                r
            })
            .map_err(|e| e.to_string())?;
            for a in &host.alerts {
                eprintln!("Zotero: {}", escape_peer(a));
            }
            eprintln!("{} calls, {}", out.calls, if out.completed { "completed" } else { "ended without completing (cancelled?)" });
            if let Some(o) = arg_value(&rest, "--save") {
                s.run("file.save", &json!({"path": o})).map_err(|e| e.to_string())?;
                eprintln!("wrote {o}");
            }
            Ok(())
        }
        "--version" | "-V" => write_stdout(|o| writeln!(o, "wordcraft-cli {}", env!("CARGO_PKG_VERSION"))),
        "" | "help" | "--help" | "-h" => write_stdout(|o| write!(o, "{USAGE}")),
        other => Err(format!("unknown command `{other}`\n\n{USAGE}").into()),
    }
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
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}') => {
                out.extend(c.escape_unicode())
            }
            c => out.push(c),
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

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match run(&args) {
        Ok(()) | Err(Failure::StdoutClosed) => ExitCode::SUCCESS,
        Err(Failure::Error(e)) => {
            eprintln!("wordcraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
