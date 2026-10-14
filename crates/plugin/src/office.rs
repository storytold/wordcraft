//! Office add-ins (Office.js) as plugins.
//!
//! An Office add-in is a web app described by an XML manifest (`OfficeApp`, the add-in manifest
//! schema published on learn.microsoft.com). Its task pane is a web page that calls the Office
//! JavaScript API (`Office.*`, `Word.run`). WordCraft reads the manifest into a plugin
//! [`Manifest`](crate::Manifest), so the add-in's ribbon buttons and right-click items are
//! ordinary plugin ones, and its commands open its task pane instead of starting a process.
//! The task pane runs WordCraft's own implementation of the API (never Microsoft's `office.js`),
//! which sends batches of [`Action`]s to the engine (`office.sync`).
//!
//! Only the public manifest schema and API reference are used (clean room).

use std::collections::BTreeMap;

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::{ButtonDecl, ButtonSize, CommandDecl, Manifest, MenuItemDecl, MenuKind, RibbonGroupDecl, RunSpec, UiDecl};

/// The manifest's file name in an add-in folder.
pub const MANIFEST_FILE: &str = "manifest.xml";
const MAX_NODES: usize = 50_000;
const MAX_DEPTH: usize = 64;
/// Longest URL kept from a manifest.
const MAX_URL: usize = 2048;

/// What an Office add-in may do with the document (`<Permissions>`), weakest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Permission {
    /// Settings and the selection's type only.
    #[default]
    Restricted,
    ReadDocument,
    ReadAllDocument,
    WriteDocument,
    ReadWriteDocument,
}

impl Permission {
    fn parse(s: &str) -> Permission {
        match s.trim() {
            "ReadDocument" => Permission::ReadDocument,
            "ReadAllDocument" => Permission::ReadAllDocument,
            "WriteDocument" => Permission::WriteDocument,
            "ReadWriteDocument" => Permission::ReadWriteDocument,
            _ => Permission::Restricted,
        }
    }
    /// May read cell values, formulas and formats.
    pub fn reads(self) -> bool {
        matches!(self, Permission::ReadDocument | Permission::ReadAllDocument | Permission::ReadWriteDocument)
    }
    /// May change the document.
    pub fn writes(self) -> bool {
        matches!(self, Permission::WriteDocument | Permission::ReadWriteDocument)
    }
}

/// What one of the add-in's commands does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum OfficeCommand {
    /// Opens a task pane on this page.
    ShowTaskpane {
        url: String,
        /// Buttons naming the same task pane share one pane.
        #[serde(default)]
        taskpane_id: Option<String>,
    },
    /// Calls a function of the add-in's function file (a page without a pane).
    ExecuteFunction { function: String, url: String },
}

/// What WordCraft keeps of an add-in manifest beyond the plugin manifest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeInfo {
    /// The manifest's `<Id>` (a GUID).
    pub office_id: String,
    pub permissions: Permission,
    /// The default task pane (`<DefaultSettings><SourceLocation>`).
    pub source_location: String,
    /// Origins the add-in may navigate to besides its own (`<AppDomains>`).
    pub app_domains: Vec<String>,
    /// Requirement sets it asks for: (name, minimum version).
    pub requirements: Vec<(String, String)>,
    pub icon_url: String,
    /// The plugin command id → what it does.
    pub commands: BTreeMap<String, OfficeCommand>,
}

// ---------------------------------------------------------------- a small XML tree

#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// `<X DefaultValue="…"/>` or `<X>…</X>`.
    fn value(&self) -> String {
        self.attr("DefaultValue").map(str::to_string).unwrap_or_else(|| self.text.trim().to_string())
    }
    fn child_value(&self, name: &str) -> String {
        self.child(name).map(Node::value).unwrap_or_default()
    }
    /// `xsi:type`, without a prefix.
    fn xsi_type(&self) -> &str {
        let t = self.attr("type").unwrap_or("");
        t.rsplit(':').next().unwrap_or(t)
    }
}

fn local(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    s.rsplit(':').next().unwrap_or(&s).to_string()
}

fn parse_xml(text: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut count = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| format!("{MANIFEST_FILE}: {e}"))?;
        if matches!(event, Event::Start(_) | Event::Empty(_)) {
            count += 1;
            if count > MAX_NODES {
                return Err(format!("{MANIFEST_FILE}: too many elements"));
            }
        }
        match event {
            Event::Start(e) => {
                if stack.len() > MAX_DEPTH {
                    return Err(format!("{MANIFEST_FILE}: nested too deeply"));
                }
                stack.push(element(&e));
            }
            Event::Empty(e) => {
                let node = element(&e);
                if let Some(top) = stack.last_mut() {
                    top.children.push(node);
                }
            }
            Event::End(_) => {
                let Some(node) = stack.pop() else { break };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Err(format!("{MANIFEST_FILE}: unbalanced elements")),
                }
            }
            Event::Text(t) => {
                if let (Some(top), Ok(s)) = (stack.last_mut(), t.decode()) {
                    top.text.push_str(&s);
                }
            }
            Event::GeneralRef(r) => {
                // `&amp;` and friends inside text.
                if let Some(top) = stack.last_mut() {
                    let name = String::from_utf8_lossy(&r);
                    top.text.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let root = stack.pop().ok_or_else(|| format!("{MANIFEST_FILE}: empty"))?;
    if !stack.is_empty() {
        return Err(format!("{MANIFEST_FILE}: unclosed elements"));
    }
    root.children
        .into_iter()
        .find(|n| n.name == "OfficeApp")
        .ok_or_else(|| format!("{MANIFEST_FILE}: not an Office add-in manifest (no <OfficeApp>)"))
}

fn element(e: &quick_xml::events::BytesStart) -> Node {
    let attrs = e
        .attributes()
        .filter_map(|a| a.ok())
        .map(|a| (local(a.key.as_ref()), a.normalized_value(quick_xml::XmlVersion::Explicit1_0).map(|v| v.to_string()).unwrap_or_default()))
        .collect();
    Node { name: local(e.name().as_ref()), attrs, ..Default::default() }
}

// ---------------------------------------------------------------- the manifest

/// The app (plugin host id) an Office host name is: `Presentation` is PowerPoint's, `Workbook`
/// Excel's, `Document` Word's.
fn app_of_host(host: &str) -> Option<&'static str> {
    match host {
        "Presentation" => Some("deckcraft"),
        "Workbook" => Some("gridcraft"),
        "Document" => Some("wordcraft"),
        _ => None,
    }
}

/// WordCraft's tab for an Office tab id (`<OfficeTab id="TabReferences">`).
fn tab_of(office_tab: &str) -> String {
    match office_tab {
        "TabHome" => "Home",
        "TabInsert" => "Insert",
        "TabDraw" => "Draw",
        "TabDesign" => "Design",
        "TabPageLayoutWord" | "TabLayout" => "Layout",
        "TabReferences" => "References",
        "TabMailings" => "Mailings",
        "TabReview" => "Review",
        "TabView" => "View",
        "TabDeveloper" => "Developer",
        "TabHelp" => "Help",
        _ => "Add-ins",
    }
    .to_string()
}

fn https_url(s: &str) -> Option<String> {
    let s = s.trim();
    (s.starts_with("https://") && s.len() <= MAX_URL && !s.contains(char::is_whitespace)).then(|| s.to_string())
}

/// A command id made from a control id: letters, digits, `.`, `-`, `_`.
fn command_id(control: &str, taken: &BTreeMap<String, OfficeCommand>) -> String {
    let mut id: String = control.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(100).collect();
    if !id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
        id.insert(0, 'c');
    }
    let base = id.clone();
    let mut n = 2;
    while taken.contains_key(&id) || id == "taskpane" {
        id = format!("{base}_{n}");
        n += 1;
    }
    id
}

/// Short labels are at most 64 characters (a plugin label's limit).
fn label(s: &str) -> String {
    let s = s.trim();
    let s: String = s.chars().take(64).collect();
    if s.is_empty() { "Add-in".into() } else { s }
}

/// The `resid` lookups of `<Resources>`: images, URLs, short and long strings.
struct Resources(BTreeMap<String, String>);

impl Resources {
    fn read(overrides: &Node) -> Resources {
        let mut map = BTreeMap::new();
        if let Some(res) = overrides.child("Resources") {
            for group in &res.children {
                for item in &group.children {
                    if let Some(id) = item.attr("id") {
                        map.insert(id.to_string(), item.value());
                    }
                }
            }
        }
        Resources(map)
    }
    fn get(&self, node: Option<&Node>) -> String {
        node.and_then(|n| n.attr("resid")).and_then(|id| self.0.get(id)).cloned().unwrap_or_default()
    }
}

/// Reads an Office add-in manifest (XML) into a plugin manifest for `host` (`gridcraft`).
pub fn parse_manifest(text: &str, host: &str) -> Result<Manifest, String> {
    let app = parse_xml(text)?;
    let office_id = app.child_value("Id");
    let valid_guid = !office_id.is_empty() && office_id.len() <= 64 && office_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if !valid_guid {
        return Err(format!("{MANIFEST_FILE}: <Id> must be a GUID"));
    }
    let name = label(&app.child_value("DisplayName"));
    let hosts: Vec<String> =
        app.child("Hosts").map(|h| h.children("Host").filter_map(|n| n.attr("Name")).map(str::to_string).collect()).unwrap_or_default();
    let apps: Vec<String> = hosts.iter().filter_map(|h| app_of_host(h)).map(str::to_string).collect();
    if !hosts.is_empty() && !apps.iter().any(|a| a == host) {
        return Err(format!("{MANIFEST_FILE}: {name} is an add-in for {}, not this app", hosts.join(", ")));
    }
    let source_location = app.child("DefaultSettings").map(|d| d.child_value("SourceLocation")).and_then(|u| https_url(&u)).unwrap_or_default();
    let mut office = OfficeInfo {
        office_id: office_id.to_ascii_lowercase(),
        permissions: Permission::parse(&app.child_value("Permissions")),
        source_location: source_location.clone(),
        app_domains: app
            .child("AppDomains")
            .map(|d| d.children("AppDomain").map(Node::value).filter(|v| !v.is_empty()).take(100).collect())
            .unwrap_or_default(),
        requirements: app
            .child("Requirements")
            .and_then(|r| r.child("Sets"))
            .map(|s| {
                let default = s.attr("DefaultMinVersion").unwrap_or("1.1").to_string();
                s.children("Set").filter_map(|n| Some((n.attr("Name")?.to_string(), n.attr("MinVersion").unwrap_or(&default).to_string()))).collect()
            })
            .unwrap_or_default(),
        icon_url: https_url(&app.child_value("IconUrl")).unwrap_or_default(),
        commands: BTreeMap::new(),
    };
    let mut commands = Vec::new();
    if !source_location.is_empty() {
        office.commands.insert("taskpane".into(), OfficeCommand::ShowTaskpane { url: source_location.clone(), taskpane_id: None });
        commands.push(CommandDecl {
            id: "taskpane".into(),
            label: format!("Show {name}"),
            description: app.child_value("Description"),
            params: String::new(),
        });
    }
    let mut ui = UiDecl::default();
    // Add-in commands: <VersionOverrides><Hosts><Host xsi:type="Document"><DesktopFormFactor>.
    if let Some(vo) = app.child("VersionOverrides") {
        let res = Resources::read(vo);
        let form = vo
            .child("Hosts")
            .into_iter()
            .flat_map(|h| h.children("Host"))
            .find(|h| app_of_host(h.xsi_type()) == Some(host))
            .and_then(|h| h.child("DesktopFormFactor"));
        if let Some(form) = form {
            let function_file = https_url(&res.get(form.child("FunctionFile"))).unwrap_or_default();
            for point in form.children("ExtensionPoint") {
                let kind = point.xsi_type();
                for container in point.children.iter().filter(|c| matches!(c.name.as_str(), "OfficeTab" | "CustomTab" | "OfficeMenu")) {
                    let tab = match container.name.as_str() {
                        "OfficeTab" => tab_of(container.attr("id").unwrap_or("")),
                        _ => label(&res.get(container.child("Label"))),
                    };
                    for group in container.children("Group") {
                        let group_label = label(&res.get(group.child("Label")));
                        let buttons = controls(group, &res, &function_file, &mut office.commands, &mut commands);
                        if !buttons.is_empty() && kind == "PrimaryCommandSurface" {
                            ui.ribbon.push(RibbonGroupDecl { tab: tab.clone(), group: group_label, buttons });
                        }
                    }
                    if kind == "ContextMenu" && container.name == "OfficeMenu" {
                        let menu = match container.attr("id") {
                            Some("ContextMenuText") => Some(MenuKind::Text),
                            _ => None,
                        };
                        for b in controls(container, &res, &function_file, &mut office.commands, &mut commands) {
                            if let Some(menu) = menu {
                                ui.menus.push(MenuItemDecl { menu, command: b.command, label: b.label, params: Json::Null });
                            }
                        }
                    }
                }
            }
        }
    }
    if office.commands.is_empty() {
        return Err(format!("{MANIFEST_FILE}: {name} has no task pane (no <SourceLocation> or add-in commands with an https:// page)"));
    }
    let manifest = json!({
        "id": format!("office.{}", office.office_id),
        "name": name,
        "version": app.child_value("Version"),
        "description": app.child_value("Description").chars().take(2000).collect::<String>(),
        "author": app.child_value("ProviderName").chars().take(200).collect::<String>(),
        "homepage": https_url(&app.child_value("SupportUrl")).unwrap_or_default(),
        "apps": apps,
        "run": RunSpec::default(),
        "commands": commands,
        "ui": ui,
    });
    let mut m = Manifest::parse_with(&manifest.to_string(), true)?;
    m.office = Some(office);
    Ok(m)
}

/// The buttons in a group or menu (`<Control xsi:type="Button">`, and the items of a
/// `<Control xsi:type="Menu">`), registering their commands.
fn controls(
    parent: &Node,
    res: &Resources,
    function_file: &str,
    office: &mut BTreeMap<String, OfficeCommand>,
    commands: &mut Vec<CommandDecl>,
) -> Vec<ButtonDecl> {
    let mut out = Vec::new();
    let mut items: Vec<&Node> = Vec::new();
    for c in parent.children("Control") {
        if c.xsi_type() == "Menu" {
            items.extend(c.child("Items").into_iter().flat_map(|i| i.children("Item")));
        } else {
            items.push(c);
        }
    }
    for c in items.into_iter().take(200) {
        let Some(action) = c.child("Action") else { continue };
        let what = match action.xsi_type() {
            "ShowTaskpane" => match https_url(&res.get(action.child("SourceLocation"))) {
                Some(url) => OfficeCommand::ShowTaskpane { url, taskpane_id: action.child("TaskpaneId").map(Node::value).filter(|v| !v.is_empty()) },
                None => continue,
            },
            "ExecuteFunction" if !function_file.is_empty() => {
                let function = action.child_value("FunctionName");
                if function.is_empty() {
                    continue;
                }
                OfficeCommand::ExecuteFunction { function, url: function_file.to_string() }
            }
            _ => continue,
        };
        let id = command_id(c.attr("id").unwrap_or("command"), office);
        let text = label(&res.get(c.child("Label")));
        let tip = c.child("Supertip").map(|s| res.get(s.child("Description"))).unwrap_or_default();
        office.insert(id.clone(), what);
        commands.push(CommandDecl { id: id.clone(), label: text.clone(), description: tip.clone(), params: String::new() });
        out.push(ButtonDecl {
            command: id,
            label: Some(text),
            tooltip: (!tip.is_empty()).then_some(tip),
            icon: Some("script".into()),
            size: ButtonSize::Large,
            params: Json::Null,
        });
    }
    out
}

// ---------------------------------------------------------------- the bridge's messages

/// One step from the root object (the document, or the add-in's settings) to another
/// object: a property (`{"p": "body"}`) or a method returning an object
/// (`{"m": "getItem", "a": ["Sheet1"]}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Step {
    Prop {
        p: String,
    },
    Method {
        m: String,
        #[serde(default)]
        a: Vec<Json>,
    },
}

/// What the task pane asks of the document in one `context.sync()`, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op")]
pub enum Action {
    /// Read properties of an object (`paragraph.load("text,style")`).
    Load {
        path: Vec<Step>,
        #[serde(default)]
        props: Vec<String>,
    },
    /// Set a property (`paragraph.style = "Heading 1"`, `range.font.bold = true`).
    Set { path: Vec<Step>, prop: String, value: Json },
    /// Call a method for its effect (`paragraph.delete()`, `body.insertParagraph("Hi", "End")`), or one whose
    /// result comes back (`paragraphs.getFirst()` is a step instead).
    Call {
        path: Vec<Step>,
        method: String,
        #[serde(default)]
        args: Vec<Json>,
    },
}

/// Most actions one sync may carry.
pub const MAX_ACTIONS: usize = 10_000;
