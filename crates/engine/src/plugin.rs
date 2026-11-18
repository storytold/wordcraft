//! Plug-ins: WebAssembly modules that edit the document through the same commands as the ribbon,
//! the shortcuts, the CLI, the control channel and MCP, inside the sandbox of craft-libs'
//! `craft-plugin` (RFC-0046). This module is WordCraft's side of it: which commands a plug-in may
//! call ([`ALLOWED`], [`policy`], [`refusal`]), one undo step per run (rolled back when the run
//! fails), read-only access checked after every call, each plug-in's settings (kept beside the
//! user's `Plug-ins` folder) and the installed plug-ins (that folder, or this session only). The
//! `plugin.*` commands are in `cmd::plugins`. See `docs/plugins.md`.
//!
//! A run works on the active document and its selection; it never gets a file, a folder, another
//! document, a mail-merge data source, a template on disk, the undo history, the clipboard,
//! macros or the person's preferences (see [`ALLOWED`]).

mod host;
mod runtime;

use std::sync::OnceLock;

pub use craft_plugin;
use craft_plugin::fetch::{self, Fetch};
use craft_plugin::{API_VERSION, AppInfo, Instance, KIND_COMMAND, Limits, Manifest, Policy, RunOutput, Source, UiChange, bundle, catalog, github};
use serde_json::{Map, Value, json};

use crate::{CmdError, CmdResult, Session};
use host::EngineHost;
pub use runtime::{InstalledPlugin, PluginPrefs, Runtime};

/// WordCraft as manifests name it: command plug-ins of every shared ABI version, and only
/// manifests that name `wordcraft` in `apps`.
pub const APP: AppInfo<'static> = AppInfo {
    id: "wordcraft",
    version: env!("CARGO_PKG_VERSION"),
    api: 1..=API_VERSION,
    kinds: &[KIND_COMMAND],
    accept_legacy: false,
    kind_api: &[],
};

/// What one plug-in may keep in its settings (keys and JSON values, in bytes), and its longest key.
pub const MAX_SETTINGS_BYTES: usize = 64 * 1024;
pub const MAX_SETTING_KEY: usize = 128;
/// Catalogs a person may add.
pub const MAX_CATALOGS: usize = 32;
/// Undo steps one run's calls may make. The undo history keeps its last 500 steps; a run stays
/// well inside that, so the step before the run is still there to fold the run's steps into.
pub const MAX_RUN_STEPS: u64 = 100;
/// Messages a run may show (`notify`), and their length in characters.
const MAX_NOTES: usize = 20;
const MAX_NOTE_CHARS: usize = 1000;
/// Least instruction budget `plugin.run {fuel}` may ask for.
const MIN_FUEL: u64 = 1000;
/// The undo step `plugin.run` opens (its command label); renamed after the plug-in when it ends.
pub(crate) const RUN_LABEL: &str = "Run Plug-in";

/// The only commands a plug-in may call: everything else is refused (`*` in [`policy`]), so a
/// command added to WordCraft later is refused until someone decides it is safe here.
///
/// What is on it reads or edits the active document through undoable commands, at the selection
/// or where its parameters say: typing and deleting, the caret and the selection, character and
/// paragraph formatting, styles, lists, tables, headers and footers, page layout, fields, notes,
/// citations from the document's own sources, tables of contents, comments and tracked changes,
/// shapes, text boxes, diagrams, charts and equations, content controls, Find and Replace, and the
/// queries agents use (`document.*`, `*.state`, `*.get`). What is not, and why:
/// - files and paths: open, save, export, print, send (`file.*`), pictures and text from files,
///   objects and embedded or linked files, templates on disk (`tools.*Template`, `styles.copyFrom`,
///   `styles.organizer`, user templates), fonts (`font.*`), language packs (`proof.*`);
/// - mail merge (`mailings.*`): its data sources are files, and finishing a merge replaces the
///   document or writes others;
/// - other documents and windows: compare, combine, versions, new windows (`view.*`);
/// - the undo history (`edit.undo*`, `edit.redo`, `edit.repeat`) and the clipboard, the Spike
///   and the Clipboard pane (`edit.copy`, `edit.cut`, `edit.paste*`, Format Painter);
/// - macros and AutoText (`tools.macros`, `tools.recordMacro`, Quick Parts), which run or keep
///   commands and text outside the document;
/// - the person's preferences: File › Options, AutoCorrect, the custom dictionary (`Add to
///   Dictionary`, `Ignore All`), keyboard and ribbon customization, the default language, the
///   view and interface (`view.*`, `ui.*`), Read Aloud;
/// - Restrict Editing (`review.restrict`), which could lock the person out of their document;
/// - interactive tools and drags (ink, table drawing, Format Painter, the mouse-driven table
///   commands), which wait for the pointer.
pub const ALLOWED: &[&str] = &[
    // reading
    "document.text",
    "document.inspect",
    "document.layout",
    "document.paragraph",
    "document.selection",
    "format.state",
    "styles.list",
    "styles.inspect",
    "list.get",
    "equation.get",
    "fields.selected",
    "developer.control",
    "outline.state",
    "review.issues",
    "review.suggestions",
    "review.thesaurus",
    "review.wordCount",
    "link.targets",
    // the caret and the selection
    "caret.docEnd",
    "caret.docStart",
    "caret.down",
    "caret.end",
    "caret.home",
    "caret.left",
    "caret.pageDown",
    "caret.pageUp",
    "caret.paraDown",
    "caret.paraUp",
    "caret.right",
    "caret.set",
    "caret.up",
    "caret.wordLeft",
    "caret.wordRight",
    "select.all",
    "select.collapse",
    "select.extend",
    "select.line",
    "select.paragraph",
    "select.range",
    "select.sentence",
    "select.similar",
    "select.text",
    "select.word",
    "edit.findNext",
    "edit.findPrevious",
    "edit.replaceAll",
    // typing and deleting
    "document.setText",
    "text.insert",
    "text.backspace",
    "text.delete",
    "text.deleteWordBack",
    "text.deleteWordForward",
    "text.deleteSentenceBack",
    "text.newParagraph",
    "text.lineBreak",
    "text.pageBreak",
    "text.columnBreak",
    "text.insertTab",
    "text.nbsp",
    "text.nbHyphen",
    "text.optionalHyphen",
    "text.emDash",
    "text.ellipsis",
    "text.copyright",
    "text.registered",
    "text.trademark",
    // character formatting
    "format.allCaps",
    "format.bold",
    "format.border",
    "format.changeCase",
    "format.charStyle",
    "format.clear",
    "format.color",
    "format.doubleStrikethrough",
    "format.doubleUnderline",
    "format.emboss",
    "format.emphasisMark",
    "format.engrave",
    "format.font",
    "format.growFont",
    "format.growFont1",
    "format.hidden",
    "format.highlight",
    "format.italic",
    "format.outline",
    "format.position",
    "format.resetChar",
    "format.scale",
    "format.set",
    "format.shading",
    "format.shadow",
    "format.shrinkFont",
    "format.shrinkFont1",
    "format.size",
    "format.smallCaps",
    "format.spacing",
    "format.strikethrough",
    "format.subscript",
    "format.superscript",
    "format.textEffects",
    "format.underline",
    "format.underlineColor",
    "format.wordUnderline",
    // paragraphs, lists and styles
    "para.addSpaceBefore",
    "para.align",
    "para.alignCenter",
    "para.alignLeft",
    "para.alignRight",
    "para.blankBetween",
    "para.borders",
    "para.bullets",
    "para.convertBreaks",
    "para.demote",
    "para.distribute",
    "para.double",
    "para.hangingIndent",
    "para.heading1",
    "para.heading2",
    "para.heading3",
    "para.indent",
    "para.indents",
    "para.justify",
    "para.keepLines",
    "para.keepNext",
    "para.lineSpacing",
    "para.listLevel",
    "para.ltr",
    "para.moveDown",
    "para.moveUp",
    "para.multilevel",
    "para.normal",
    "para.numbering",
    "para.oneAndHalf",
    "para.outdent",
    "para.outlineLevel",
    "para.pageBreakBefore",
    "para.promote",
    "para.removeBlank",
    "para.removeExtraSpaces",
    "para.removeHanging",
    "para.removeLineBreaks",
    "para.removeSpaceAfter",
    "para.reset",
    "para.restartNumbering",
    "para.rtl",
    "para.separateParagraphs",
    "para.set",
    "para.setNumberingValue",
    "para.shading",
    "para.single",
    "para.sort",
    "para.spacing",
    "para.style",
    "para.tabs",
    "para.trimLeadingSpaces",
    "para.widowControl",
    "list.define",
    "list.multilevelPreset",
    "styles.create",
    "styles.modify",
    "styles.delete",
    "styles.rename",
    "styles.updateToMatch",
    // inserting
    "insert.blankPage",
    "insert.bookmark",
    "insert.deleteBookmark",
    "insert.chart",
    "insert.crossReference",
    "insert.dateField",
    "insert.dateTime",
    "insert.dropCap",
    "insert.equation",
    "insert.field",
    "insert.horizontalLine",
    "insert.link",
    "insert.removeLink",
    "insert.pageBreak",
    "insert.pageField",
    "insert.pageNumber",
    "insert.shape",
    "insert.smartArt",
    "insert.symbol",
    "insert.table",
    "insert.textBox",
    "insert.timeField",
    "insert.wordArt",
    // tables
    "table.autofit",
    "table.borders",
    "table.cellAlign",
    "table.cellMargins",
    "table.columnWidth",
    "table.deleteCells",
    "table.deleteColumn",
    "table.deleteRow",
    "table.deleteTable",
    "table.distributeColumns",
    "table.distributeRows",
    "table.formula",
    "table.fromText",
    "table.insertColumnLeft",
    "table.insertColumnRight",
    "table.insertRowAbove",
    "table.insertRowBelow",
    "table.look",
    "table.merge",
    "table.properties",
    "table.quick",
    "table.repeatHeader",
    "table.rowHeight",
    "table.rtl",
    "table.selectCell",
    "table.selectColumn",
    "table.selectRow",
    "table.selectTable",
    "table.shading",
    "table.sort",
    "table.split",
    "table.splitTable",
    "table.style",
    "table.textDirection",
    "table.toText",
    // design and page layout
    "design.pageBorders",
    "design.pageColor",
    "design.paragraphSpacing",
    "design.styleSet",
    "design.theme",
    "design.themeColors",
    "design.themeFonts",
    "design.watermark",
    "layout.break",
    "layout.columns",
    "layout.differentFirstPage",
    "layout.differentOddEven",
    "layout.hyphenation",
    "layout.lineNumbers",
    "layout.linkToPrevious",
    "layout.margins",
    "layout.orientation",
    "layout.pageNumberFormat",
    "layout.pageSetup",
    "layout.section",
    "layout.size",
    "layout.textDirection",
    "layout.verticalAlign",
    // fields, notes and references
    "fields.lock",
    "fields.unlock",
    "fields.unlink",
    "fields.next",
    "fields.previous",
    "references.footnote",
    "references.endnote",
    "references.toc",
    "references.updateToc",
    "references.removeToc",
    "references.addText",
    "references.caption",
    "references.citation",
    "references.bibliography",
    "references.updateFields",
    "references.markEntry",
    "references.index",
    "references.updateIndex",
    "references.tableOfFigures",
    "references.updateFigures",
    // comments and tracked changes
    "review.newComment",
    "review.reply",
    "review.editComment",
    "review.deleteComment",
    "review.resolveComment",
    "review.nextComment",
    "review.previousComment",
    "review.trackChanges",
    "review.accept",
    "review.reject",
    "review.acceptAll",
    "review.rejectAll",
    "review.nextChange",
    "review.previousChange",
    "review.language",
    "review.applySuggestion",
    // objects
    "arrange.align",
    "arrange.bounds",
    "arrange.bringForward",
    "arrange.sendBackward",
    "arrange.group",
    "arrange.ungroup",
    "arrange.position",
    "arrange.rotate",
    "arrange.rotation",
    "arrange.wrap",
    "object.altText",
    "shape.alignText",
    "shape.change",
    "shape.effects",
    "shape.fill",
    "shape.outline",
    "shape.textDirection",
    "smartArt.addShape",
    "smartArt.colors",
    "smartArt.demote",
    "smartArt.items",
    "smartArt.layout",
    "smartArt.promote",
    "chart.dataLabels",
    "chart.editData",
    "chart.legend",
    "chart.title",
    "chart.type",
    "wordArt.style",
    "wordArt.textFill",
    "wordArt.textOutline",
    "wordArt.transform",
    // equations
    "equation.convert",
    "equation.display",
    "equation.insertStructure",
    "equation.insertSymbol",
    "equation.set",
    // content controls
    "developer.richText",
    "developer.plainText",
    "developer.checkBox",
    "developer.comboBox",
    "developer.dropDown",
    "developer.datePicker",
    "developer.removeControl",
    "developer.toggleCheckBox",
    "developer.setDate",
    "developer.chooseItem",
    // outline
    "outline.promote",
    "outline.demote",
    "outline.demoteToBody",
    "outline.promoteToHeading1",
    "outline.moveUp",
    "outline.moveDown",
];

/// Parameter names that name a file or folder on disk when their value is text (or a list of
/// text), at any depth. No allowed command needs one: a plug-in passes content itself. (A `path`
/// that is a list of numbers is a place in the document, as `document.paragraph` takes it.)
const FILE_KEYS: &[&str] = &["path", "paths", "file", "files", "dir", "folder"];

/// The rules every plug-in run follows: [`craft_plugin::ALWAYS_DENIED`], and every command but
/// [`ALLOWED`].
pub fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| Policy::new(&["*"], ALLOWED))
}

/// Why plug-in `m` may not call `command` with `params`, before it runs: a refused command, one
/// outside its manifest's `permissions.commands`, or parameters that name a file.
pub fn refusal(m: &Manifest, command: &str, params: &Value) -> Option<String> {
    if let Err(why) = craft_plugin::check_call(m, policy(), command) {
        return Some(why);
    }
    if file_key_inside(params, 0) {
        return Some(format!(
            "plug-ins can't name files or folders (`{command}` was given one of {}): work on the open document",
            FILE_KEYS.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")
        ));
    }
    None
}

/// Whether `v` holds one of [`FILE_KEYS`] naming a file (text, or a list holding text) at any
/// depth.
fn file_key_inside(v: &Value, depth: usize) -> bool {
    let names_a_file = |v: &Value| match v {
        Value::String(_) => true,
        Value::Array(a) => a.iter().any(Value::is_string),
        _ => false,
    };
    // serde_json refuses nesting past 128, so nothing deeper reaches here
    depth < 128
        && match v {
            Value::Object(m) => m.iter().any(|(k, v)| (FILE_KEYS.contains(&k.as_str()) && names_a_file(v)) || file_key_inside(v, depth + 1)),
            Value::Array(a) => a.iter().any(|v| file_key_inside(v, depth + 1)),
            _ => false,
        }
}

/// A plug-in's bad request, as the `plugin.*` commands report it.
pub(crate) fn bad(cmd: &str, e: impl std::fmt::Display) -> CmdError {
    CmdError::Params(format!("{cmd}: {e}"))
}

/// A plug-in that failed, as the engine reports it.
fn failed(name: &str, e: impl std::fmt::Display) -> CmdError {
    CmdError::Failed(format!("{name}: {e}"))
}

/// Store (`Some`) or forget (`None`) plug-in `id`'s setting `key` in `all`, within
/// [`MAX_SETTINGS_BYTES`] and [`MAX_SETTING_KEY`].
fn set_setting(
    all: &mut std::collections::BTreeMap<String, Map<String, Value>>,
    id: &str,
    key: &str,
    value: Option<&str>,
) -> std::result::Result<(), String> {
    let Some(text) = value else {
        if let Some(mine) = all.get_mut(id) {
            mine.remove(key);
            if mine.is_empty() {
                all.remove(id);
            }
        }
        return Ok(());
    };
    if key.is_empty() || key.chars().count() > MAX_SETTING_KEY || key.chars().any(char::is_control) {
        return Err(format!("a setting's key must be 1–{MAX_SETTING_KEY} characters without control characters"));
    }
    let too_big = || format!("a plug-in's settings are limited to {} KB", MAX_SETTINGS_BYTES / 1024);
    if key.len().saturating_add(text.len()) > MAX_SETTINGS_BYTES {
        return Err(too_big());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("a setting must be JSON: {e}"))?;
    let others: usize = all.get(id).map_or(0, |mine| {
        mine.iter().filter(|(k, _)| k.as_str() != key).map(|(k, v)| k.len().saturating_add(v.to_string().len())).fold(0, usize::saturating_add)
    });
    if others.saturating_add(key.len()).saturating_add(v.to_string().len()) > MAX_SETTINGS_BYTES {
        return Err(too_big());
    }
    all.entry(id.to_string()).or_default().insert(key.to_string(), v);
    Ok(())
}

// ---------- running ----------

/// The selection a plug-in sees: the anchor and focus positions (`{story, path, off}`).
fn selection_json(s: &Session) -> Value {
    json!({"anchor": s.sel.anchor, "focus": s.sel.focus, "collapsed": s.sel.is_collapsed()})
}

/// Run plug-in `id`'s `command` (default: its first) with `params` on the active document, as one
/// undo step named after the plug-in. `plugin.run` is a mutating command, so [`Session::run`]
/// opened that step before this runs and puts the document, the selection and the undo history
/// back when this fails (a trap, an exhausted budget, a refusal the plug-in passes on, a non-zero
/// status); the calls' own steps are folded into it when it succeeds.
pub fn run(s: &mut Session, id: &str, command: Option<&str>, params: &Value, fuel: Option<u64>) -> CmdResult {
    let cmd = "plugin.run";
    let p = s.plugins.get(id).cloned().ok_or_else(|| bad(cmd, format!("no plug-in `{id}` is installed (see plugin.list)")))?;
    let m = &p.manifest;
    let api = m.target(&APP).map_err(|e| bad(cmd, format!("{} can't run here: {e}", m.name)))?.api();
    let c = m.command(command).ok_or_else(|| bad(cmd, format!("{} has no command `{}`", m.name, command.unwrap_or_default())))?;
    let params = c.resolve_params(params).map_err(|e| bad(cmd, e))?;
    let mut limits = Limits::default();
    if let Some(f) = fuel {
        limits.fuel = f.clamp(MIN_FUEL, limits.fuel);
    }
    let mut instance = Instance::new(&p.wasm, &limits).map_err(|e| failed(&m.name, e))?;
    let input = json!({"api": api, "command": c.id, "params": params, "selection": selection_json(s)}).to_string();
    let label = if m.commands.len() > 1 { format!("{}: {}", m.name, c.label.trim_end_matches(['…', '.'])) } else { m.name.clone() };

    // What the calls change outside the document's undo: Repeat and a macro being recorded
    // (they keep `plugin.run`, not the commands it runs), the UI's requests and the settings.
    let last_command = s.last_command.clone();
    let recording = s.recording.take();
    let ui_requests = s.ui_requests.clone();
    let settings = s.plugins.prefs.settings.clone();
    let mark = s.undo_mark();

    let mut host = EngineHost::new(s, m, mark);
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| instance.call(&input, &mut host)));
    let notes = host.finish();
    s.last_command = last_command;
    s.recording = recording;
    let ran: std::result::Result<RunOutput, String> = match ran {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("the plug-in run panicked".into()),
    };
    let out = match ran {
        Ok(out) => out,
        Err(e) => {
            // Session::run puts the document, selection and history back; the rest is ours.
            s.ui_requests = ui_requests;
            s.plugins.prefs.settings = settings;
            return Err(failed(&m.name, e));
        }
    };
    s.fold_undo_since(mark, RUN_LABEL, &label);
    s.plugins.last = Some((m.id.clone(), c.id.clone(), params));
    if s.plugins.prefs.settings != settings {
        s.plugins.save_prefs();
    }

    let output = serde_json::from_str::<Value>(&out.output).unwrap_or(Value::String(out.output));
    // Panels (`ui_show`) aren't shown in WordCraft yet: say so rather than drop it silently
    let panel = matches!(out.ui, Some(UiChange::Show(_))).then_some("WordCraft doesn't show plug-in panels yet");
    if let Some(note) = notes.last() {
        s.status = note.clone();
    }
    Ok(
        json!({"plugin": m.id, "command": c.id, "label": label, "output": output, "notes": notes, "logs": out.logs, "calls": out.calls, "fuel": out.fuel, "panel": panel}),
    )
}

/// The requests `after` holds that `before` didn't: the queue only grows at its end, dropping its
/// oldest past its limit.
fn new_ui_requests<'a>(before: &[Value], after: &'a [Value]) -> &'a [Value] {
    for dropped in 0..=before.len() {
        let kept = before.get(dropped..).unwrap_or_default();
        if after.get(..kept.len()) == Some(kept) {
            return after.get(kept.len()..).unwrap_or_default();
        }
    }
    after
}

// ---------- installing ----------

/// Where a bundle to install comes from (`plugin.install`'s parameters).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// A `.craftplugin` file on this computer.
    Path(String),
    /// The bundle itself (base64 in the parameters), with a name to remember it by.
    Data { bytes: Vec<u8>, name: String },
    /// A GitHub repository's release: `owner/repo` or `owner/repo@tag`.
    Repo(String),
    /// A catalog's package (its newest release for WordCraft, or `version`).
    Catalog { url: String, id: String, version: Option<String> },
}

impl Request {
    /// The source `p` names: `path`, `data` (base64), `repo` or `catalog` + `id`.
    pub fn parse(cmd: &str, p: &Value) -> Result<Request, CmdError> {
        let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
        if let Some(path) = text("path") {
            return Ok(Request::Path(path.into()));
        }
        if let Some(data) = text("data") {
            if data.len() > bundle::MAX_BUNDLE_BYTES / 3 * 4 + 4 {
                return Err(bad(cmd, format!("the bundle is larger than {} MB", bundle::MAX_BUNDLE_BYTES >> 20)));
            }
            let bytes = crate::cmd::insert::base64_decode(data).ok_or_else(|| bad(cmd, "`data` isn't base64"))?;
            return Ok(Request::Data { bytes, name: text("name").unwrap_or("data").chars().take(200).collect() });
        }
        if let Some(spec) = text("repo") {
            return Ok(Request::Repo(spec.into()));
        }
        if let Some(url) = text("catalog") {
            let id = text("id").ok_or_else(|| bad(cmd, "`id` required with `catalog` (a package of the catalog)"))?;
            return Ok(Request::Catalog { url: url.into(), id: id.into(), version: text("version").map(str::to_string) });
        }
        Err(bad(cmd, "pass `path`, `data` (base64), `repo` (owner/repo or owner/repo@tag) or `catalog` with `id`"))
    }

    /// Whether getting the bundle goes over the network (a repository, or a catalog that isn't a
    /// local `file://` one).
    pub fn needs_network(&self) -> bool {
        match self {
            Request::Repo(_) => true,
            Request::Catalog { url, .. } => remote(url),
            Request::Path(_) | Request::Data { .. } => false,
        }
    }
}

/// Whether `url` is read over the network (not a `file://` catalog).
pub fn remote(url: &str) -> bool {
    !url.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("file://"))
}

/// A bundle obtained for installing: its bytes, where it came from and, for a repository, whether
/// a catalog in use lists it.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub bytes: Vec<u8>,
    pub source: Source,
    /// `Some(true)` from a catalog; for a repository, whether a catalog in use lists it.
    pub in_catalog: Option<bool>,
}

/// A bundle checked and shown for review (`plugin.install {review: true}`), kept so the install
/// that follows doesn't download it again.
pub(crate) struct Pending {
    pub sha256: String,
    /// The `plugin.install` parameters that named it.
    pub request: Value,
    pub candidate: Candidate,
}

/// The app's [`Fetch`] with local files held to the automation read root (MCP's
/// `--automation-read-root`): a `file://` catalog and the files it names are read only where a
/// path in a command could be.
struct Confined<'a>(&'a mut dyn Fetch);

impl Fetch for Confined<'_> {
    fn get(&mut self, url: &str, max: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        self.0.get(url, max)
    }

    fn read_local(&mut self, path: &std::path::Path, max: usize) -> std::result::Result<Vec<u8>, String> {
        crate::file_access::check_read(&path.to_string_lossy())?;
        fetch::read_local_file(path, max)
    }
}

/// Read the bundle `req` names, downloading over `f` (catalogs in use: `catalogs`, to tell
/// whether a repository is listed in one). Needs no session, so a frontend can run it on a worker
/// thread and hand the result to [`hold`].
pub fn obtain_with(req: &Request, f: &mut dyn Fetch, catalogs: &[String]) -> std::result::Result<Candidate, String> {
    let f = &mut Confined(f);
    match req {
        Request::Path(path) => {
            let bytes = read_bundle_file(path)?;
            Ok(Candidate { bytes, source: Source::File { path: path.clone() }, in_catalog: None })
        }
        Request::Data { bytes, name } => Ok(Candidate { bytes: bytes.clone(), source: Source::File { path: name.clone() }, in_catalog: None }),
        Request::Repo(spec) => {
            let repo = github::parse_repo_spec(spec)?;
            let plan = fetch::resolve_repo(f, &repo)?;
            let bytes = fetch::download_repo(f, &plan)?;
            let source = Source::Github { repo: plan.repo.slug(), tag: plan.repo.tag.clone().unwrap_or_default(), asset: plan.asset.clone() };
            let in_catalog = catalogs.iter().any(|url| {
                fetch::fetch_catalog(f, url)
                    .is_ok_and(|(cat, _)| cat.packages.iter().any(|pkg| pkg.repo_ref().is_some_and(|r| r.same_repo(&repo.owner, &repo.name))))
            });
            Ok(Candidate { bytes, source, in_catalog: Some(in_catalog) })
        }
        Request::Catalog { url, id, version } => {
            let (cat, _) = fetch::fetch_catalog(f, url)?;
            let package = cat.package(id).ok_or_else(|| format!("{url} has no package `{id}`"))?;
            let release = match version {
                Some(v) => package.release(v).ok_or_else(|| format!("{id} has no version {v} in {url}"))?,
                None => package.latest_for(&APP).ok_or_else(|| format!("{id} has no release for WordCraft {}", APP.version))?,
            };
            release.target(&package.kind, &APP)?;
            let bytes = fetch::download_release(f, url, package, release)?;
            let (_, manifest) = bundle::open(&bytes)?;
            package.check_manifest(release, &manifest)?;
            Ok(Candidate { bytes, source: Source::Catalog { catalog: url.clone(), package: id.clone() }, in_catalog: Some(true) })
        }
    }
}

/// Run `f` with the session's HTTPS client (the app sets one, [`Runtime::set_fetch`]), or one that
/// refuses network addresses (local files and `file://` catalogs still read).
pub(crate) fn with_fetch<T>(s: &Session, f: impl FnOnce(&mut dyn Fetch) -> T) -> T {
    let mut client = s.plugins.fetch.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match client.as_deref_mut() {
        Some(http) => f(http),
        None => f(&mut NoNetwork),
    }
}

/// [`obtain_with`] from a command, over the session's client.
pub(crate) fn obtain(s: &Session, cmd: &str, req: &Request) -> Result<Candidate, CmdError> {
    let catalogs = s.plugins.prefs.catalogs.clone();
    with_fetch(s, |f| obtain_with(req, f, &catalogs)).map_err(|e| bad(cmd, e))
}

/// Longest part of a URL an error repeats.
const SHOWN_URL_CHARS: usize = 120;

/// The fetcher of sessions without network access: local files and `file://` catalogs still read.
pub struct NoNetwork;

impl Fetch for NoNetwork {
    fn get(&mut self, url: &str, _max: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        let shown: String = url.chars().take(SHOWN_URL_CHARS).collect();
        Err(format!("there is no network access here to read {shown}: install the plug-in from a file, or in the WordCraft app"))
    }
}

/// The bundle file at `path` (inside the automation read root, when there is one), refused past
/// [`bundle::MAX_BUNDLE_BYTES`] before it is read whole.
#[cfg(not(target_arch = "wasm32"))]
fn read_bundle_file(path: &str) -> std::result::Result<Vec<u8>, String> {
    crate::io::read_file_capped(path, bundle::MAX_BUNDLE_BYTES as u64, "plug-in")
}

#[cfg(target_arch = "wasm32")]
fn read_bundle_file(path: &str) -> std::result::Result<Vec<u8>, String> {
    Err(format!("{path}: the browser has no file paths: pass the bundle as `data` (base64)"))
}

/// What an install confirmation shows about `c`: the manifest's permissions, kind and commands,
/// the source, the SHA-256, whether a catalog lists it and what it replaces. An error when the
/// bundle isn't a plug-in WordCraft can run.
pub(crate) fn review(s: &Session, cmd: &str, c: &Candidate) -> Result<(Manifest, Value), CmdError> {
    let (_, manifest) = bundle::open(&c.bytes).map_err(|e| bad(cmd, e))?;
    manifest.target(&APP).map_err(|e| bad(cmd, format!("{} can't be installed: {e}", manifest.name)))?;
    c.source.check().map_err(|e| bad(cmd, e))?;
    let old = s.plugins.get(&manifest.id);
    let mut v = serde_json::to_value(&manifest).unwrap_or_else(|_| json!({"id": manifest.id}));
    v["kind"] = json!(manifest.kind);
    v["sha256"] = json!(bundle::sha256_hex(&c.bytes));
    v["source"] = serde_json::to_value(&c.source).unwrap_or(Value::Null);
    v["inCatalog"] = json!(c.in_catalog);
    v["replaces"] = json!(old.map(|o| &o.manifest.version));
    v["permissionsChanged"] = json!(old.is_some_and(|o| craft_plugin::permissions::changed(&o.manifest, &manifest)));
    v["stored"] = json!(s.plugins.has_store());
    Ok((manifest, v))
}

/// Install `c` (already reviewed): into the user's folder when the session has one, else for the
/// session. Replaces an installed plug-in with the same id; its settings stay.
pub(crate) fn install(s: &mut Session, cmd: &str, c: Candidate) -> Result<InstalledPlugin, CmdError> {
    let (manifest, _) = review(s, cmd, &c)?;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(store) = &s.plugins.store {
        let (i, _) = store.install_for(&APP, &c.bytes, c.source).map_err(|e| bad(cmd, e))?;
        let plugin = InstalledPlugin {
            manifest: i.manifest,
            sha256: i.record.sha256,
            source: i.record.source,
            stored: true,
            wasm: std::sync::Arc::new(i.bundle.wasm),
        };
        return Ok(s.plugins.keep(plugin));
    }
    let sha256 = bundle::sha256_hex(&c.bytes);
    let (b, _) = bundle::open(&c.bytes).map_err(|e| bad(cmd, e))?;
    Ok(s.plugins.keep(InstalledPlugin { manifest, sha256, source: c.source, stored: false, wasm: std::sync::Arc::new(b.wasm) }))
}

/// Keep `c`, named by the `plugin.install` parameters `request` (without `review` and `sha256`),
/// for the install that follows its review (one at a time). Returns its SHA-256, which that
/// install passes. A frontend that downloaded `c` itself ([`obtain_with`]) hands it over here.
pub fn hold(s: &mut Session, request: Value, c: Candidate) -> String {
    let sha256 = bundle::sha256_hex(&c.bytes);
    s.plugins.pending = Some(Pending { sha256: sha256.clone(), request: install_request(&request), candidate: c });
    sha256
}

/// `plugin.install`'s parameters as [`hold`] and the install after a review compare them.
pub fn install_request(p: &Value) -> Value {
    let mut request: Map<String, Value> = p.as_object().cloned().unwrap_or_default();
    request.remove("review");
    request.remove("sha256");
    Value::Object(request)
}

/// The bundle reviewed last, when it is `sha256` and `request` named it.
pub(crate) fn take_held(s: &mut Session, sha256: &str, request: &Value) -> Option<Candidate> {
    let held = s.plugins.pending.take_if(|h| h.sha256.eq_ignore_ascii_case(sha256) && h.request == *request)?;
    Some(held.candidate)
}

/// Remove plug-in `id` and its settings; whether it was installed.
pub(crate) fn remove(s: &mut Session, cmd: &str, id: &str) -> Result<bool, CmdError> {
    let Some(p) = s.plugins.get(id).cloned() else { return Ok(false) };
    #[cfg(not(target_arch = "wasm32"))]
    if p.stored
        && let Some(store) = &s.plugins.store
    {
        store.remove(&p.manifest.id).map_err(|e| bad(cmd, e))?;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = cmd;
    s.plugins.forget(&p.manifest.id);
    s.plugins.save_prefs();
    Ok(true)
}

/// The catalog `url`'s packages WordCraft can run, with their newest release for it, read over
/// `f` (`installed` is left out: [`crate::cmd`] adds it).
pub fn catalog_packages_with(f: &mut dyn Fetch, url: &str) -> std::result::Result<(Vec<Value>, Vec<String>), String> {
    let (cat, warnings) = fetch::fetch_catalog(&mut Confined(f), url)?;
    let packages = cat
        .packages
        .iter()
        .filter_map(|pkg| {
            let release = pkg.latest_for(&APP)?;
            Some(json!({
                "catalog": url, "id": pkg.id, "name": pkg.name, "version": release.version, "author": pkg.author,
                "description": pkg.description, "license": pkg.license, "repo": pkg.repo, "kind": pkg.kind,
                "sha256": release.sha256,
            }))
        })
        .collect();
    Ok((packages, warnings))
}

/// Whether `url` is a catalog address WordCraft reads (`https://` on GitHub's hosts, or
/// `file://`), or why not.
pub(crate) fn check_catalog_url(url: &str) -> std::result::Result<(), String> {
    catalog::check_url(url, None).map(|_| ())
}

/// Where plug-in `p` has a newer release: the install parameters that get it and its version (a
/// tag, for a repository). `None` when it is up to date or came from a file.
pub fn update_for(f: &mut dyn Fetch, p: &InstalledPlugin) -> std::result::Result<Option<(Value, String)>, String> {
    let f = &mut Confined(f);
    match &p.source {
        Source::Catalog { catalog: url, package: id } => {
            let (cat, _) = fetch::fetch_catalog(f, url)?;
            let release = cat.package(id).and_then(|pkg| pkg.latest_for(&APP));
            Ok(release
                .filter(|r| bundle::newer(&r.version, &p.manifest.version))
                .map(|r| (json!({"catalog": url, "id": id, "version": r.version}), r.version.clone())))
        }
        Source::Github { repo, tag: installed, .. } => {
            let mut repo = github::parse_repo_spec(repo)?;
            repo.tag = None;
            let plan = fetch::resolve_repo(f, &repo)?;
            let tag = plan.repo.tag.clone().unwrap_or_default();
            Ok((tag != *installed).then(|| (json!({"repo": format!("{}@{tag}", plan.repo.slug())}), tag)))
        }
        Source::File { .. } | Source::Other { .. } => Ok(None),
    }
}

#[cfg(test)]
mod tests;
