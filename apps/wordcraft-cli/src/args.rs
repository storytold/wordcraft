//! The command line as clap derive types (RFC-0018). Values arrive typed, so the commands never validate a string; what
//! clap cannot say alone is a [`UsageError`], printed and exited like clap's own.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{ArgMatches, Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

/// The end of every subcommand's `--help`.
const EXIT_STATUS: &str = "Exit status:\n  0  success\n  1  the command failed (for `convert --strict` and `render --strict`, a refusal; for `validate`, a verdict other than PASS)\n  2  usage error\n\nA process ended by a signal exits 128 + the signal number (130 for Ctrl-C), as in the shell.";

/// The top-level `--help` ends with the environment and the exit status.
const TOP_NOTES: &str = "A password-protected Word file is opened with --password PASS, --password-file PATH or the WORDCRAFT_PASSWORD environment variable; a password given on the command line is readable by every other process, so the other two are better. A file opened with a password is written with it again, as the app does. Exports that cannot carry a password (pdf, html, txt) stay plain.\n\nPaper: documents created here, and HTML, Markdown and text files (which give no page size), use the system region's paper (A4, or Letter in the US and other Letter regions), read from LC_ALL / LC_PAPER / LANG or the platform. Set WORDCRAFT_REGION=en-US (or another tag) for the same output on every machine.\n\nEnvironment:\n  WORDCRAFT_PASSWORD  the password of a protected Word file, when neither --password nor --password-file is given\n  WORDCRAFT_REGION    the paper and region to use instead of the system's (a tag such as en-US)\n\nExit status:\n  0  success\n  1  the command failed (for `convert --strict` and `render --strict`, a refusal; for `validate`, a verdict other than PASS)\n  2  usage error\n\nA process ended by a signal exits 128 + the signal number (130 for Ctrl-C), as in the shell.";

/// WordCraft from the command line: convert, inspect, render, run commands, validate, serve MCP
#[derive(Parser, Debug)]
#[command(name = "wordcraft-cli", version, propagate_version = true, arg_required_else_help = true, after_help = TOP_NOTES)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Convert between formats (docx, doc [read], pdf, odt, rtf, html, md, tex, txt, json, png, xml)
    ///
    /// `--update-fields` refreshes unlocked fields before saving (if protection blocks updates, they are skipped with a
    /// warning). An .xml file is a Word XML Document (Flat OPC), or with `--format wordml2003` a Word 2003 XML Document.
    /// `--markup` (Print Markup) puts comments beside each page and tracked changes in pdf/png, and a comment list in
    /// html/md/txt. `--strict` (pdf or png output only) writes nothing and exits 1 if a character can't be drawn or a
    /// picture is missing or undecodable; the report is on stderr, with one JSON line.
    #[command(after_help = EXIT_STATUS)]
    Convert(ConvertArgs),
    /// Pages, words, paragraphs and properties (JSON)
    #[command(after_help = EXIT_STATUS)]
    Info(FileArgs),
    /// The plain text
    #[command(after_help = EXIT_STATUS)]
    Text(FileArgs),
    /// The document structure (JSON)
    #[command(after_help = EXIT_STATUS)]
    Inspect(FileArgs),
    /// Render one page to a PNG
    ///
    /// `--strict` writes nothing and exits 1 if a character on the page can't be drawn or a picture is missing or
    /// undecodable (the report is on stderr, as for `convert --strict`).
    #[command(after_help = EXIT_STATUS)]
    Render(RenderArgs),
    /// Run commands headlessly, then save
    ///
    /// `--script` runs a JSON file (`-` is stdin) first: {"file"?, "template"?, "commands": [{"command": id, "params"?:
    /// {…}} | id, …], "save"?: [OUT, …]}; save formats follow the extension (docx and pdf in one run); relative paths
    /// resolve against the script's folder. `file.new` and `file.open` open another document and make it active; later
    /// commands and the saves work on the active one; `view.switchWindows` lists the open documents and with {"id": N}
    /// makes one active. PNG saves use `--page` (1-based, default 1) and `--scale` (default 2).
    #[command(after_help = EXIT_STATUS)]
    Run(RunArgs),
    /// List every command
    #[command(after_help = EXIT_STATUS)]
    Commands(JsonArgs),
    /// Feature-catalog parity
    #[command(after_help = EXIT_STATUS)]
    Parity(ParityArgs),
    /// Structural integrity check
    ///
    /// Exit 0 for PASS; 1 for FAIL, INCOMPLETE or a file that can't be read (the verdict is the first line of the report,
    /// and `verdict` in `--json`).
    #[command(after_help = EXIT_STATUS)]
    Validate(ValidateArgs),
    /// MCP server on stdio, with a headless session
    ///
    /// Both roots are required: the agent's commands read files (open, insert text or picture, mail-merge data,
    /// templates…) only inside the read root and write them (save, export, merge output…) only inside the write root,
    /// links followed; the same folder may be both. `--connect` (bridge to the running app) is refused until the app
    /// confines its control channel (#3695, #5819).
    #[command(after_help = EXIT_STATUS)]
    Mcp(McpArgs),
    /// Run a Zotero command on a document (Zotero must be running)
    ///
    /// The commands are addEditCitation, addEditBibliography, addNote, refresh, removeCodes and setDocPrefs; `--cmd`
    /// runs first (for example `--cmd caret.docEnd`).
    #[command(after_help = EXIT_STATUS)]
    Zotero(ZoteroArgs),
}

/// How a password-protected Word file is opened.
#[derive(Args, Debug, Default)]
pub(crate) struct Password {
    /// The password (readable by every other process on the machine; prefer --password-file)
    #[arg(long, value_name = "PASS", env = "WORDCRAFT_PASSWORD", hide_env_values = true)]
    pub(crate) password: Option<String>,
    /// Read the password from this file (a trailing newline is dropped)
    #[arg(long = "password-file", value_name = "PATH")]
    pub(crate) password_file: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct FileArgs {
    /// The document
    pub(crate) file: PathBuf,
    #[command(flatten)]
    pub(crate) open: Password,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum XmlFormat {
    /// A Word 2003 XML Document
    Wordml2003,
    /// A Word XML Document (Flat OPC), the default
    Flatopc,
}

impl XmlFormat {
    pub(crate) fn name(self) -> &'static str {
        match self {
            XmlFormat::Wordml2003 => "wordml2003",
            XmlFormat::Flatopc => "flatopc",
        }
    }
}

#[derive(Args, Debug)]
pub(crate) struct ConvertArgs {
    /// The document to read
    pub(crate) input: PathBuf,
    /// The file to write; its extension names the format
    pub(crate) output: PathBuf,
    /// Print Markup: comments beside each page and tracked changes
    #[arg(long)]
    pub(crate) markup: bool,
    /// Refresh unlocked fields before saving
    #[arg(long = "update-fields")]
    pub(crate) update_fields: bool,
    /// For pdf or png output: write nothing and fail if a character can't be drawn or a picture is missing
    #[arg(long)]
    pub(crate) strict: bool,
    /// The kind of .xml file to write
    #[arg(long, value_enum, value_name = "KIND")]
    pub(crate) format: Option<XmlFormat>,
    #[command(flatten)]
    pub(crate) open: Password,
}

#[derive(Args, Debug)]
pub(crate) struct RenderArgs {
    /// The document
    pub(crate) input: PathBuf,
    /// The PNG to write
    pub(crate) output: PathBuf,
    /// The page, from 1
    #[arg(long, value_name = "N", value_parser = page, default_value = "1")]
    pub(crate) page: u64,
    /// Device pixels per point
    #[arg(long, value_name = "S", allow_negative_numbers = true, value_parser = scale, default_value = "2")]
    pub(crate) scale: f64,
    /// Write nothing and fail if a character can't be drawn or a picture is missing
    #[arg(long)]
    pub(crate) strict: bool,
    #[command(flatten)]
    pub(crate) open: Password,
}

/// One `--cmd ID[=JSON]`.
#[derive(Clone, Debug)]
pub(crate) struct Cmd {
    pub(crate) id: String,
    pub(crate) params: Value,
}

fn cmd(text: &str) -> Result<Cmd, UsageError> {
    match text.split_once('=') {
        Some((id, p)) => {
            let params = serde_json::from_str(p).map_err(|e| UsageError::CmdJson { id: id.to_owned(), why: e.to_string() })?;
            Ok(Cmd { id: id.to_owned(), params })
        }
        None => Ok(Cmd { id: text.to_owned(), params: json!({}) }),
    }
}

fn page(text: &str) -> Result<u64, UsageError> {
    match text.parse::<u64>() {
        Ok(0) => Err(UsageError::Page("invalid --page `0`: page numbers start at 1".to_owned())),
        Ok(n) => Ok(n),
        Err(_) => Err(UsageError::Page(format!("invalid --page `{text}`: not a valid integer"))),
    }
}

fn scale(text: &str) -> Result<f64, UsageError> {
    let n = text.parse::<f64>().map_err(|_| UsageError::Scale(format!("invalid --scale `{text}`: not a number")))?;
    if n.is_finite() && n > 0.0 { Ok(n) } else { Err(UsageError::Scale(format!("invalid --scale `{text}`: scale must be a finite positive number"))) }
}

#[derive(Args, Debug)]
pub(crate) struct RunArgs {
    /// Open this document first
    #[arg(long, value_name = "F", conflicts_with = "template")]
    pub(crate) file: Option<PathBuf>,
    /// Start from this template
    #[arg(long, value_name = "T")]
    pub(crate) template: Option<String>,
    /// A script to run first (`-` is stdin)
    #[arg(long, value_name = "FILE")]
    pub(crate) script: Option<PathBuf>,
    /// Run one command, `ID` or `ID={json}`; repeatable
    #[arg(long = "cmd", value_name = "ID={JSON}", value_parser = cmd)]
    pub(crate) cmds: Vec<Cmd>,
    /// Save the result here (the extension names the format)
    #[arg(long, value_name = "OUT")]
    pub(crate) save: Option<PathBuf>,
    /// The page of a PNG save, from 1 (default 1)
    #[arg(long, value_name = "N", value_parser = page)]
    pub(crate) page: Option<u64>,
    /// The scale of a PNG save (default 2)
    #[arg(long, value_name = "S", allow_negative_numbers = true, value_parser = scale)]
    pub(crate) scale: Option<f64>,
    /// Print each command's result
    #[arg(long)]
    pub(crate) print: bool,
    /// Who tracked changes and comments are by
    #[arg(long, value_name = "NAME")]
    pub(crate) author: Option<String>,
    #[command(flatten)]
    pub(crate) open: Password,
}

#[derive(Args, Debug)]
pub(crate) struct JsonArgs {
    /// JSON alone on stdout
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ParityArgs {
    /// Markdown instead of JSON
    #[arg(long)]
    pub(crate) markdown: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ValidateArgs {
    /// The document
    pub(crate) file: PathBuf,
    /// JSON alone on stdout
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct McpArgs {
    /// The folder the agent may read in (links followed); the same folder may be the write root
    #[arg(long = "automation-read-root", value_name = "DIR", required_unless_present = "connect", value_parser = folder)]
    pub(crate) read_root: Option<PathBuf>,
    /// The folder the agent may write in (links followed)
    #[arg(long = "automation-write-root", value_name = "DIR", required_unless_present = "connect", value_parser = folder)]
    pub(crate) write_root: Option<PathBuf>,
    /// Who tracked changes and comments are by
    #[arg(long, value_name = "NAME")]
    pub(crate) author: Option<String>,
    /// Refused for now: the desktop app does not confine its control channel to automation roots yet
    #[arg(long, value_name = "HOST:PORT")]
    pub(crate) connect: Option<String>,
}

fn folder(text: &str) -> Result<PathBuf, UsageError> {
    if text.is_empty() { Err(UsageError::EmptyFolder) } else { Ok(PathBuf::from(text)) }
}

#[derive(Args, Debug)]
pub(crate) struct ZoteroArgs {
    /// The Zotero command (addEditCitation, addEditBibliography, addNote, refresh, removeCodes or setDocPrefs)
    #[arg(value_parser = zotero_command)]
    pub(crate) command: String,
    /// The document
    pub(crate) file: PathBuf,
    /// Run one command first, `ID` or `ID={json}`; repeatable
    #[arg(long = "cmd", value_name = "ID={JSON}", value_parser = cmd)]
    pub(crate) cmds: Vec<Cmd>,
    /// Save the document here afterwards
    #[arg(long, value_name = "OUT")]
    pub(crate) save: Option<PathBuf>,
    /// Show the calls to and from Zotero on stderr
    #[arg(long)]
    pub(crate) trace: bool,
    /// Zotero's port
    #[arg(long, value_name = "P")]
    pub(crate) port: Option<u16>,
}

fn zotero_command(text: &str) -> Result<String, UsageError> {
    if wordcraft_zotero::Command::from_name(text).is_some() { Ok(text.to_owned()) } else { Err(UsageError::Zotero(text.to_owned())) }
}

/// A mistake in the command line that clap can't see by itself.
#[derive(Debug, PartialEq, thiserror::Error)]
pub(crate) enum UsageError {
    #[error("{0}")]
    Page(String),
    #[error("{0}")]
    Scale(String),
    #[error("--cmd {id}: bad JSON params: {why}")]
    CmdJson { id: String, why: String },
    #[error("a folder is needed, not an empty name")]
    EmptyFolder,
    #[error("unknown Zotero command `{0}` (addEditCitation, addEditBibliography, addNote, refresh, removeCodes or setDocPrefs)")]
    Zotero(String),
}

/// The clap definition of the whole command line (completions and man pages are generated from it).
pub fn command() -> clap::Command {
    Cli::command()
}

/// Parse the arguments (without the program name). Help and the version come back as the `Err` clap uses for them.
pub(crate) fn parse(args: &[OsString]) -> Result<Cli, clap::Error> {
    let matches = Cli::command().try_get_matches_from(std::iter::once(OsString::from("wordcraft-cli")).chain(args.iter().cloned()))?;
    let mut cli = Cli::from_arg_matches(&matches)?;
    if let Some((name, sub)) = matches.subcommand() {
        password_precedence(&mut cli.command, sub, name);
    }
    Ok(cli)
}

/// `--password`, `--password-file`, then `WORDCRAFT_PASSWORD`: a password file named on the command line beats the
/// variable, which clap would otherwise read into `--password`.
fn password_precedence(command: &mut Command, sub: &ArgMatches, _name: &str) {
    let open = match command {
        Command::Convert(a) => &mut a.open,
        Command::Info(a) | Command::Text(a) | Command::Inspect(a) => &mut a.open,
        Command::Render(a) => &mut a.open,
        Command::Run(a) => &mut a.open,
        _ => return,
    };
    let from_env = sub.value_source("password") == Some(clap::parser::ValueSource::EnvVariable);
    if from_env && open.password_file.is_some() {
        open.password = None;
    }
    // A blank variable counts as unset.
    if from_env && open.password.as_deref() == Some("") {
        open.password = None;
    }
}
