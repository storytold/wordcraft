//! DBusMenu worker and nonblocking per-window binding. Local menus remain the caller's responsibility.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use crate::{Click, Item, Menu};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::platform::wayland::WindowExtWayland;
use x11rb::{
    connection::Connection as _,
    protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode},
    wrapper::ConnectionExt as _,
};
use zbus::{
    blocking::{Connection, connection::Builder},
    fdo,
    zvariant::{OwnedValue, Str, Structure, Value},
};

const PATH: &str = "/MenuBar";
const INTERFACE: &str = "com.canonical.dbusmenu";
const REGISTRAR: &str = "com.canonical.AppMenu.Registrar";
const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";
const POLL: Duration = Duration::from_secs(2);
type Properties = HashMap<String, OwnedValue>;
type Layout = (i32, Properties, Vec<OwnedValue>);

type Repaint = Arc<dyn Fn() + Send + Sync>;

/// Main-thread handle. Dropping it closes the worker channel and the window association.
pub struct Bridge {
    pending: Arc<Mutex<Menu>>,
    wake: mpsc::SyncSender<()>,
    clicks: mpsc::Receiver<Click>,
    addresses: mpsc::Receiver<Option<String>>,
    wayland: Option<crate::wayland::Host>,
    // Keep the borrowed native surface alive until the association has been released.
    _window: Arc<winit::window::Window>,
}

impl Bridge {
    /// Export this window's snapshots unless disabled or no safe native handle is available.
    /// Connection/retry work runs off the UI thread; callers retain their local menus.
    pub fn start(
        window: Arc<winit::window::Window>,
        application_id: &str,
        env_prefix: &str,
        menu: Menu,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Option<Self> {
        if std::env::var_os(format!("{env_prefix}_NO_GLOBAL_MENU")).is_some()
            || std::env::var_os("KDE_NO_GLOBAL_MENU").is_some()
        {
            return None;
        }
        let repaint: Repaint = Arc::new(repaint);
        let (x11, wayland) = match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Xlib(h) => (Some(u32::try_from(h.window).ok()?), None),
            RawWindowHandle::Xcb(h) => (Some(h.window.get()), None),
            RawWindowHandle::Wayland(_) => {
                let surface = window.wayland_surface()?;
                let host = crate::wayland::Host::new(surface, repaint.clone())
                    .map_err(|e| {
                        log::warn!("global menu: Wayland association unavailable: {e}");
                    })
                    .ok()?;
                (None, Some(host))
            }
            _ => return None,
        };
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let service = format!("{application_id}.AppMenu.p{}.w{serial}", std::process::id());
        let pending = Arc::new(Mutex::new(menu));
        let shared = pending.clone();
        let (wake, updates) = mpsc::sync_channel(1);
        let (tx, clicks) = mpsc::sync_channel(32);
        let (ready, addresses) = mpsc::channel();
        if let Err(e) = std::thread::Builder::new()
            .name("craft-appmenu".into())
            .spawn(move || worker(x11, service, shared, updates, tx, ready, repaint))
        {
            log::warn!("global menu: cannot start worker: {e}");
            return None;
        }
        Some(Self {
            pending,
            wake,
            clicks,
            addresses,
            wayland,
            _window: window,
        })
    }

    /// Coalesce a newer snapshot and wake the worker without blocking on transport I/O.
    pub fn publish(&self, next: Menu) {
        if let Ok(mut pending) = self.pending.lock()
            && *pending != next
        {
            *pending = next;
            let _ = self.wake.try_send(());
        }
    }

    /// Pump our guest Wayland queue without reading winit's socket. Revalidate all returned
    /// clicks against the current document/modal/command state before executing them.
    pub fn take_events(&mut self) -> Vec<Click> {
        // Drain even on X11 or after a Wayland failure: reconnect notifications must
        // not accumulate indefinitely in an unused receiver.
        for address in self.addresses.try_iter() {
            if let Some(host) = &mut self.wayland {
                match address {
                    Some(service) => {
                        if let Err(e) = host.set_address(&service, PATH) {
                            log::debug!("global menu: {e}");
                        }
                    }
                    None => {
                        if let Err(e) = host.clear_address() {
                            log::debug!("global menu: {e}");
                        }
                    }
                }
            }
        }
        if let Some(host) = &mut self.wayland
            && let Err(e) = host.poll()
        {
            log::warn!("global menu: Wayland binding disabled: {e}");
            self.wayland = None;
        }
        self.clicks.try_iter().take(32).collect()
    }
}

#[derive(Clone)]
struct Node {
    item: Item,
    children: Vec<i32>,
}

#[derive(Clone, Default)]
struct Tree {
    document: u64,
    revision: u32,
    nodes: BTreeMap<i32, Node>,
    ids: HashMap<Vec<String>, i32>,
    next_id: i32,
}

struct Changes {
    layouts: Vec<i32>,
    properties: Vec<(i32, Properties)>,
    removed: Vec<(i32, Vec<String>)>,
}

impl Tree {
    /// Begin a new export generation without recycling any previously advertised ID.
    /// The worker retains this tree even when the D-Bus connection is replaced.
    fn restart(&mut self, menu: &Menu) -> Result<(), String> {
        self.ids.clear();
        self.update(menu)
    }

    fn update(&mut self, menu: &Menu) -> Result<(), String> {
        fn add(tree: &mut Tree, parent: &[String], item: &Item) -> Result<i32, String> {
            // Separate path components cannot collide when keys contain '/' (for
            // example user paths). A retargeted action must also retire its ID:
            // delayed D-Bus events contain only that ID, not the original action.
            let action = item.action.as_ref().map(|a| (&a.command, &a.params));
            let mut key = parent.to_vec();
            key.push(item.key.clone());
            key.push(serde_json::to_string(&action).map_err(|e| e.to_string())?);
            let id = if let Some(id) = tree.ids.get(&key) {
                *id
            } else {
                // IDs are never recycled, so a delayed event can't hit a different command.
                tree.next_id = tree
                    .next_id
                    .checked_add(1)
                    .ok_or("menu item IDs exhausted")?;
                tree.ids.insert(key.clone(), tree.next_id);
                tree.next_id
            };
            let children = item
                .children
                .iter()
                .map(|i| add(tree, &key, i))
                .collect::<Result<Vec<_>, _>>()?;
            let mut leaf = item.clone();
            leaf.children.clear();
            tree.nodes.insert(
                id,
                Node {
                    item: leaf,
                    children,
                },
            );
            Ok(id)
        }
        self.nodes.clear();
        if self.document != menu.document {
            self.ids.clear();
        }
        self.document = menu.document;
        self.revision = self.revision.wrapping_add(1);
        let children = match menu
            .items
            .iter()
            .map(|i| add(self, &[], i))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(children) => children,
            Err(error) => {
                // A failed rebuild must not expose an actionable partial generation.
                // Keep next_id advanced: even failed attempts must never recycle IDs.
                self.nodes.clear();
                self.ids.clear();
                return Err(error);
            }
        };
        self.nodes.insert(
            0,
            Node {
                item: Item {
                    key: String::new(),
                    label: String::new(),
                    enabled: true,
                    checked: None,
                    shortcut: String::new(),
                    action: None,
                    children: vec![],
                    separator: false,
                },
                children,
            },
        );
        // Keep only live keys. Removed items receive fresh IDs if they reappear, without
        // retaining every custom tab name the user has ever tried during this session.
        self.ids.retain(|_, id| self.nodes.contains_key(id));
        Ok(())
    }

    fn changes(&self, previous: &Self) -> fdo::Result<Changes> {
        let mut changes = Changes {
            layouts: vec![],
            properties: vec![],
            removed: vec![],
        };
        for (id, node) in &self.nodes {
            // New nodes are discovered through the changed child list of an existing parent.
            let Some(old) = previous.nodes.get(id) else {
                continue;
            };
            if node.children != old.children {
                changes.layouts.push(*id);
            }
            if node.item != old.item || node.children.is_empty() != old.children.is_empty() {
                let mut properties = self.properties(*id, &[])?;
                let old_properties = previous.properties(*id, &[])?;
                let removed: Vec<String> = old_properties
                    .keys()
                    .filter(|key| !properties.contains_key(*key))
                    .cloned()
                    .collect();
                properties.retain(|key, value| old_properties.get(key) != Some(value));
                if !properties.is_empty() {
                    changes.properties.push((*id, properties));
                }
                if !removed.is_empty() {
                    changes.removed.push((*id, removed));
                }
            }
        }
        Ok(changes)
    }

    fn properties(&self, id: i32, names: &[String]) -> fdo::Result<Properties> {
        let n = self
            .nodes
            .get(&id)
            .ok_or_else(|| invalid("unknown menu item"))?;
        let mut p = HashMap::new();
        let mut put = |key: &str, v: OwnedValue| {
            if names.is_empty() || names.iter().any(|name| name == key) {
                p.insert(key.into(), v);
            }
        };
        // DBusMenu uses '_' as its mnemonic marker. Preserve literal underscores in user labels.
        put("label", Str::from(n.item.label.replace('_', "__")).into());
        put("enabled", n.item.enabled.into());
        put("visible", true.into());
        if n.item.separator {
            put("type", Str::from("separator").into());
        }
        if !n.children.is_empty() {
            put("children-display", Str::from("submenu").into());
        }
        if let Some(checked) = n.item.checked {
            put("toggle-type", Str::from("checkmark").into());
            put("toggle-state", i32::from(checked).into());
        }
        if !n.item.shortcut.is_empty() {
            let keys = shortcut(&n.item.shortcut);
            let value = OwnedValue::try_from(Value::from(vec![keys])).map_err(failed)?;
            put("shortcut", value);
        }
        Ok(p)
    }

    fn layout(&self, id: i32, depth: i32, names: &[String]) -> fdo::Result<Layout> {
        let n = self
            .nodes
            .get(&id)
            .ok_or_else(|| invalid("unknown menu item"))?;
        let mut children = Vec::new();
        if depth != 0 {
            let next = if depth < 0 { -1 } else { depth - 1 };
            for child in &n.children {
                let layout = self.layout(*child, next, names)?;
                children.push(OwnedValue::try_from(Structure::from(layout)).map_err(failed)?);
            }
        }
        Ok((id, self.properties(id, names)?, children))
    }
}

fn shortcut(s: &str) -> Vec<String> {
    s.split('+')
        .map(|s| {
            match s {
                "Ctrl" | "Mod" => "Control",
                "PageUp" => "Page_Up",
                "PageDown" => "Page_Down",
                "Enter" => "Return",
                "Plus" => "plus",
                "Minus" | "-" => "minus",
                "Equals" | "=" => "equal",
                other => other,
            }
            .to_string()
        })
        .collect()
}
fn invalid(msg: &str) -> fdo::Error {
    fdo::Error::InvalidArgs(msg.into())
}
fn failed(e: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}

struct Exporter {
    tree: Arc<RwLock<Tree>>,
    clicks: mpsc::SyncSender<Click>,
    repaint: Repaint,
}

impl Exporter {
    fn read(&self) -> fdo::Result<std::sync::RwLockReadGuard<'_, Tree>> {
        self.tree.read().map_err(failed)
    }
    fn click(&self, id: i32, event: &str) -> fdo::Result<()> {
        let tree = self.read()?;
        let n = tree
            .nodes
            .get(&id)
            .ok_or_else(|| invalid("unknown menu item"))?;
        if event != "clicked" || !n.item.enabled {
            return Ok(());
        }
        if let Some(action) = &n.item.action {
            // Backpressure must not block a D-Bus executor while holding the tree lock.
            // A full queue rejects this request explicitly; accepted clicks stay FIFO.
            self.clicks
                .try_send(Click {
                    document: tree.document,
                    action: action.clone(),
                })
                .map_err(failed)?;
            (self.repaint)();
        }
        Ok(())
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Exporter {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(out_args("revision", "layout"))]
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> fdo::Result<(u32, Layout)> {
        let tree = self.read()?;
        Ok((
            tree.revision,
            tree.layout(parent_id, recursion_depth, &property_names)?,
        ))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> fdo::Result<Vec<(i32, Properties)>> {
        let tree = self.read()?;
        let ids = if ids.is_empty() {
            tree.nodes.keys().copied().collect()
        } else {
            ids
        };
        // Missing IDs can occur when a contextual menu disappears between the two calls.
        ids.into_iter()
            .filter(|id| tree.nodes.contains_key(id))
            .map(|id| tree.properties(id, &property_names).map(|p| (id, p)))
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedValue> {
        self.read()?
            .properties(id, std::slice::from_ref(&name))?
            .remove(&name)
            .ok_or_else(|| invalid("unknown menu property"))
    }

    fn event(
        &self,
        id: i32,
        event_id: &str,
        _data: OwnedValue,
        _timestamp: u32,
    ) -> fdo::Result<()> {
        self.click(id, event_id)
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        events
            .into_iter()
            .filter_map(|(id, event, _, _)| self.click(id, &event).err().map(|_| id))
            .collect()
    }

    fn about_to_show(&self, id: i32) -> fdo::Result<bool> {
        if !self.read()?.nodes.contains_key(&id) {
            return Err(invalid("unknown menu item"));
        }
        (self.repaint)();
        Ok(false)
    }

    #[zbus(out_args("updatesNeeded", "idErrors"))]
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (
            vec![],
            ids.into_iter()
                .filter(|id| self.about_to_show(*id).is_err())
                .collect(),
        )
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        updated_props: Vec<(i32, Properties)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn item_activation_requested(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: i32,
        timestamp: u32,
    ) -> zbus::Result<()>;
}

fn registrar_owner(bus: &Connection) -> Option<String> {
    bus.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetNameOwner",
        &(REGISTRAR,),
    )
    .ok()?
    .body()
    .deserialize()
    .ok()
}

fn register(bus: &Connection, window: u32) -> Result<(), String> {
    let path = zbus::zvariant::ObjectPath::try_from(PATH).map_err(|e| e.to_string())?;
    bus.call_method(
        Some(REGISTRAR),
        REGISTRAR_PATH,
        Some(REGISTRAR),
        "RegisterWindow",
        &(window, path),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

struct WindowProperties {
    connection: x11rb::rust_connection::RustConnection,
    window: u32,
    atoms: Vec<u32>,
}

impl WindowProperties {
    fn install(window: u32, service: &str) -> Result<Self, String> {
        let (connection, _) = x11rb::connect(None).map_err(|e| e.to_string())?;
        let mut installed = Self {
            connection,
            window,
            atoms: Vec::new(),
        };
        for (name, value) in [
            ("_KDE_NET_WM_APPMENU_SERVICE_NAME", service),
            ("_KDE_NET_WM_APPMENU_OBJECT_PATH", PATH),
        ] {
            let atom = installed
                .connection
                .intern_atom(false, name.as_bytes())
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?
                .atom;
            installed
                .connection
                .change_property8(
                    PropMode::REPLACE,
                    window,
                    atom,
                    AtomEnum::STRING,
                    value.as_bytes(),
                )
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|e| e.to_string())?;
            installed.atoms.push(atom);
        }
        installed.connection.flush().map_err(|e| e.to_string())?;
        Ok(installed)
    }
}

impl Drop for WindowProperties {
    fn drop(&mut self) {
        for atom in &self.atoms {
            let _ = self.connection.delete_property(self.window, *atom);
        }
        let _ = self.connection.flush();
    }
}

/// Menu changes are coalesced in `pending`; they must not bypass reconnect backoff.
/// Returns false when the bridge is dropped, including during the wait.
fn wait_for_retry(updates: &mpsc::Receiver<()>, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        match updates.recv_timeout(remaining) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return true,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// Retry transport failures while retaining the lifetime-wide item-ID namespace.
fn worker(
    window: Option<u32>,
    service: String,
    pending: Arc<Mutex<Menu>>,
    updates: mpsc::Receiver<()>,
    clicks: mpsc::SyncSender<Click>,
    addresses: mpsc::Sender<Option<String>>,
    repaint: Repaint,
) {
    let tree = Arc::new(RwLock::new(Tree::default()));
    loop {
        if let Err(e) = serve(
            window,
            &service,
            &pending,
            &updates,
            clicks.clone(),
            &addresses,
            repaint.clone(),
            &tree,
        ) {
            log::warn!("global menu: {e}; retrying in a few seconds");
            let _ = addresses.send(None);
            (repaint)();
        } else {
            return;
        }
        if !wait_for_retry(&updates, POLL) {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Serve one connection generation, retiring old IDs before advertising its address.
fn serve(
    window: Option<u32>,
    service: &str,
    pending: &Mutex<Menu>,
    updates: &mpsc::Receiver<()>,
    clicks: mpsc::SyncSender<Click>,
    addresses: &mpsc::Sender<Option<String>>,
    repaint: Repaint,
    tree: &Arc<RwLock<Tree>>,
) -> Result<(), String> {
    let mut last = pending.lock().map_err(|e| e.to_string())?.clone();
    tree.write().map_err(|e| e.to_string())?.restart(&last)?;
    let bus = Builder::session()
        .map_err(|e| e.to_string())?
        .method_timeout(Duration::from_secs(1))
        .name(service)
        .map_err(|e| e.to_string())?
        .serve_at(
            PATH,
            Exporter {
                tree: tree.clone(),
                clicks,
                repaint: repaint.clone(),
            },
        )
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let _properties = window
        .map(|w| WindowProperties::install(w, service))
        .transpose()?;
    let _ = addresses.send(Some(service.to_owned()));
    (repaint)();
    log::info!("global menu exported: {service} {PATH}, X11 window {window:?}");
    let mut owner = None;
    let mut next_probe = Instant::now();
    loop {
        if bus.is_closed() {
            return Err("session bus connection closed".into());
        }
        if let Some(window) = window
            && Instant::now() >= next_probe
        {
            let now_owner = registrar_owner(&bus);
            if now_owner != owner {
                if now_owner.is_some() {
                    register(&bus, window)?;
                }
                owner = now_owner;
            }
            next_probe = Instant::now() + POLL;
        }
        match updates.recv_timeout(POLL) {
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(window) = window {
                    let _ = bus.call_method(
                        Some(REGISTRAR),
                        REGISTRAR_PATH,
                        Some(REGISTRAR),
                        "UnregisterWindow",
                        &(window,),
                    );
                }
                return Ok(());
            }
            _ => {
                let next = pending.lock().map_err(|e| e.to_string())?.clone();
                if next != last {
                    let (revision, changes) = {
                        let mut tree = tree.write().map_err(|e| e.to_string())?;
                        let previous = tree.clone();
                        tree.update(&next)?;
                        (
                            tree.revision,
                            tree.changes(&previous).map_err(|e| e.to_string())?,
                        )
                    };
                    // Plasma fetches one level at a time. Updating only the root would leave
                    // cached submenu labels/checkmarks stale, even on the next opening.
                    if !changes.properties.is_empty() || !changes.removed.is_empty() {
                        bus.emit_signal(
                            None::<&str>,
                            PATH,
                            INTERFACE,
                            "ItemsPropertiesUpdated",
                            &(changes.properties, changes.removed),
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    for parent in changes.layouts {
                        bus.emit_signal(
                            None::<&str>,
                            PATH,
                            INTERFACE,
                            "LayoutUpdated",
                            &(revision, parent),
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    last = next;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::Action;
    use zbus::zvariant::Type;

    fn menu() -> Menu {
        let item = |command: &str, enabled, checked| Item {
            key: command.into(),
            label: command.into(),
            enabled,
            checked,
            shortcut: "Ctrl+B".into(),
            action: Some(Action {
                command: command.into(),
                params: serde_json::json!({}),
            }),
            ..Default::default()
        };
        Menu {
            document: 1,
            items: vec![
                Item {
                    key: "file".into(),
                    label: "File".into(),
                    enabled: true,
                    children: vec![
                        item("file.save", true, None),
                        item("edit.undo", false, None),
                        item("format.bold", true, Some(false)),
                    ],
                    ..Default::default()
                },
                Item {
                    key: "edit".into(),
                    label: "Edit".into(),
                    enabled: true,
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn appmenu_shortcuts_export_xkb_names_for_zoom_keys() {
        for (input, expected) in [
            ("Ctrl+Plus", vec!["Control", "plus"]),
            ("Ctrl+-", vec!["Control", "minus"]),
            ("Ctrl+Minus", vec!["Control", "minus"]),
            ("Ctrl+=", vec!["Control", "equal"]),
            ("Mod+Shift+Equals", vec!["Control", "Shift", "equal"]),
            ("Alt+PageUp", vec!["Alt", "Page_Up"]),
            ("Ctrl+B", vec!["Control", "B"]),
        ] {
            let mut m = menu();
            m.items[0].children[0].shortcut = input.into();
            let mut tree = Tree::default();
            tree.update(&m).unwrap();
            let id = *tree
                .nodes
                .iter()
                .find(|(_, n)| n.item.key == "file.save")
                .unwrap()
                .0;
            let mut properties = tree.properties(id, &[]).unwrap();
            let exported: Vec<Vec<String>> =
                properties.remove("shortcut").unwrap().try_into().unwrap();
            assert_eq!(exported, vec![expected], "{input}");
        }
    }

    #[test]
    fn appmenu_layout_has_protocol_signature_depth_and_property_filter() {
        assert_eq!(Layout::SIGNATURE.to_string(), "(ia{sv}av)");
        let mut t = Tree::default();
        t.update(&menu()).unwrap();
        assert!(t.layout(0, 0, &[]).unwrap().2.is_empty());
        let layout = t.layout(0, 1, &["label".into()]).unwrap();
        assert!(layout.1.keys().all(|k| k == "label"));
        assert!(!layout.2.is_empty());
        for child in layout.2 {
            let s = Structure::try_from(child).unwrap();
            let (_, _, grandchildren): Layout = s.try_into().unwrap();
            assert!(grandchildren.is_empty());
        }
        assert!(t.layout(0, -1, &[]).is_ok());
        assert!(t.layout(-123, -1, &[]).is_err());
    }

    #[test]
    fn appmenu_ids_survive_reordering_and_removed_ids_do_not_activate() {
        let mut m = menu();
        let mut t = Tree::default();
        t.update(&m).unwrap();
        let first = t.nodes.get(&0).unwrap().children[0];
        let key = t.nodes.get(&first).unwrap().item.key.clone();
        m.items.reverse();
        t.update(&m).unwrap();
        assert_eq!(t.nodes.get(&first).unwrap().item.key, key);
        m.items.retain(|i| i.key != key);
        t.update(&m).unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let server = Exporter {
            tree: Arc::new(RwLock::new(t)),
            clicks: tx,
            repaint: Arc::new(|| {}),
        };
        assert!(server.click(first, "clicked").is_err());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn appmenu_clicks_are_queued_and_disabled_commands_are_not() {
        let mut t = Tree::default();
        t.update(&menu()).unwrap();
        let id = |cmd: &str| {
            *t.nodes
                .iter()
                .find(|(_, n)| n.item.action.as_ref().is_some_and(|a| a.command == cmd))
                .unwrap()
                .0
        };
        let (save, undo) = (id("file.save"), id("edit.undo"));
        let (tx, rx) = mpsc::sync_channel(1);
        let server = Exporter {
            tree: Arc::new(RwLock::new(t)),
            clicks: tx,
            repaint: Arc::new(|| {}),
        };
        server.click(undo, "clicked").unwrap();
        server.click(save, "hovered").unwrap();
        assert!(rx.try_recv().is_err());
        server.click(save, "clicked").unwrap();
        assert_eq!(rx.try_recv().unwrap().action.command, "file.save");
    }

    /// Overload returns errors without dropping accepted events or blocking layout updates.
    #[test]
    fn appmenu_full_queue_rejects_explicitly_and_preserves_fifo() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut t = Tree::default();
        t.update(&menu()).unwrap();
        let id = |cmd: &str| {
            *t.nodes
                .iter()
                .find(|(_, n)| n.item.action.as_ref().is_some_and(|a| a.command == cmd))
                .unwrap()
                .0
        };
        let (save, bold) = (id("file.save"), id("format.bold"));
        let (tx, rx) = mpsc::sync_channel(32);
        let paints = Arc::new(AtomicUsize::new(0));
        let counter = paints.clone();
        let server = Exporter {
            tree: Arc::new(RwLock::new(t)),
            clicks: tx,
            repaint: Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        };
        for i in 0..32 {
            server
                .click(if i % 2 == 0 { save } else { bold }, "clicked")
                .unwrap();
        }
        assert!(server.event(save, "clicked", 0i32.into(), 0).is_err());
        assert_eq!(
            server.event_group(vec![(bold, "clicked".into(), 0i32.into(), 0)]),
            vec![bold]
        );
        assert_eq!(paints.load(Ordering::SeqCst), 32);
        assert!(
            server.tree.try_write().is_ok(),
            "overload must release the tree lock"
        );
        for i in 0..32 {
            assert_eq!(
                rx.try_recv().unwrap().action.command,
                if i % 2 == 0 {
                    "file.save"
                } else {
                    "format.bold"
                }
            );
        }
        assert!(
            rx.try_recv().is_err(),
            "rejected requests are not acknowledged as queued"
        );
        server.click(save, "clicked").unwrap();
        assert_eq!(rx.try_recv().unwrap().action.command, "file.save");
        drop(rx);
        assert!(server.click(save, "clicked").is_err());
    }

    /// Every recovery retires the previous generation, even if its document is unchanged.
    #[test]
    fn appmenu_restart_never_recycles_ids_and_exhaustion_fails_closed() {
        let mut t = Tree::default();
        let mut m = menu();
        t.restart(&m).unwrap();
        let first: Vec<_> = t.nodes.keys().copied().filter(|id| *id != 0).collect();
        t.restart(&m).unwrap();
        assert!(first.iter().all(|id| !t.nodes.contains_key(id)));
        let second: Vec<_> = t.nodes.keys().copied().filter(|id| *id != 0).collect();
        m.document += 1;
        t.restart(&m).unwrap();
        assert!(second.iter().all(|id| !t.nodes.contains_key(id)));
        t.next_id = i32::MAX;
        assert!(t.restart(&m).is_err());
        assert!(
            t.nodes.is_empty(),
            "exhaustion must not wrap or leave stale actions enabled"
        );
        // Also fail closed when exhaustion happens after some new nodes were built.
        t.next_id = i32::MAX - 2;
        assert!(t.restart(&m).is_err());
        assert!(
            t.nodes.is_empty(),
            "partial generation must not remain actionable"
        );
        assert!(
            t.ids.is_empty(),
            "failed generation must not retain key mappings"
        );
    }

    #[test]
    fn appmenu_old_document_item_ids_are_retired() {
        let mut m = menu();
        let mut tree = Tree::default();
        tree.update(&m).unwrap();
        let old_ids: Vec<i32> = tree.nodes.keys().copied().filter(|id| *id != 0).collect();
        m.document += 1;
        tree.update(&m).unwrap();
        assert!(old_ids.iter().all(|id| !tree.nodes.contains_key(id)));
    }

    #[test]
    fn appmenu_path_components_do_not_alias() {
        let m = Menu {
            items: vec![
                Item {
                    key: "a".into(),
                    children: vec![Item {
                        key: "b/c".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Item {
                    key: "a/b".into(),
                    children: vec![Item {
                        key: "c".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut tree = Tree::default();
        tree.update(&m).unwrap();
        assert_eq!(tree.nodes.len(), 5); // root and four distinct nodes
        let parents = &tree.nodes[&0].children;
        assert_ne!(
            tree.nodes[&parents[0]].children[0],
            tree.nodes[&parents[1]].children[0]
        );
    }

    #[test]
    fn appmenu_fixed_key_retarget_retires_old_id_but_translation_does_not() {
        let mut m = menu();
        let mut tree = Tree::default();
        tree.update(&m).unwrap();
        let parent = tree.nodes[&0].children[0];
        let original = tree.nodes[&parent].children[0];
        m.items[0].children[0].label = "Сохранить".into();
        tree.update(&m).unwrap();
        assert_eq!(tree.nodes[&parent].children[0], original);
        m.items[0].children[0].action.as_mut().unwrap().params = serde_json::json!({"target": 2});
        tree.update(&m).unwrap();
        let changed_params = tree.nodes[&parent].children[0];
        assert_ne!(original, changed_params);
        assert!(!tree.nodes.contains_key(&original));
        m.items[0].children[0].action.as_mut().unwrap().command = "file.saveAs".into();
        tree.update(&m).unwrap();
        assert_ne!(changed_params, tree.nodes[&parent].children[0]);
        let (tx, rx) = mpsc::sync_channel(1);
        let server = Exporter {
            tree: Arc::new(RwLock::new(tree)),
            clicks: tx,
            repaint: Arc::new(|| {}),
        };
        assert!(server.click(original, "clicked").is_err());
        assert!(server.click(changed_params, "clicked").is_err());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn appmenu_empty_submenu_updates_children_display_property() {
        let mut m = menu();
        let mut tree = Tree::default();
        tree.update(&m).unwrap();
        let parent = tree.nodes[&0].children[1];
        let empty = tree.clone();
        m.items[1].children.push(Item {
            key: "new".into(),
            ..Default::default()
        });
        tree.update(&m).unwrap();
        let added = tree.changes(&empty).unwrap();
        assert!(added.layouts.contains(&parent));
        assert!(
            added
                .properties
                .iter()
                .any(|(id, props)| *id == parent && props.contains_key("children-display"))
        );
        let filled = tree.clone();
        m.items[1].children.clear();
        tree.update(&m).unwrap();
        let removed = tree.changes(&filled).unwrap();
        assert!(removed.layouts.contains(&parent));
        assert!(
            removed
                .removed
                .iter()
                .any(|(id, props)| *id == parent
                    && props.iter().any(|key| key == "children-display"))
        );
    }

    #[test]
    fn appmenu_retry_wait_coalesces_updates_and_stops_on_disconnect() {
        let (tx, rx) = mpsc::channel();
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        let delay = Duration::from_millis(20);
        let started = Instant::now();
        assert!(wait_for_retry(&rx, delay));
        assert!(
            started.elapsed() >= delay,
            "queued updates must not bypass backoff"
        );
        assert!(rx.try_recv().is_err());
        drop(tx);
        assert!(!wait_for_retry(&rx, Duration::from_secs(60)));
    }

    #[test]
    fn appmenu_cached_submenus_receive_property_and_structure_updates() {
        let mut previous = Tree::default();
        previous.update(&menu()).unwrap();
        let bold = *previous
            .nodes
            .iter()
            .find(|(_, n)| {
                n.item
                    .action
                    .as_ref()
                    .is_some_and(|a| a.command == "format.bold")
            })
            .unwrap()
            .0;
        let mut next = previous.clone();
        let node = next.nodes.get_mut(&bold).unwrap();
        node.item.checked = Some(!node.item.checked.unwrap());
        node.item.label = "Changed label".into();
        node.item.shortcut.clear();
        node.item.enabled = false;
        let changes = next.changes(&previous).unwrap();
        assert!(changes.layouts.is_empty());
        assert_eq!(changes.properties.len(), 1);
        assert_eq!(changes.properties[0].0, bold);
        assert_eq!(changes.properties[0].1.len(), 3); // label, enabled, toggle-state; no immutable properties
        assert!(!bool::try_from(&changes.properties[0].1["enabled"]).unwrap());
        assert_eq!(changes.removed, vec![(bold, vec!["shortcut".into()])]);
        next.nodes.get_mut(&0).unwrap().children.pop();
        assert_eq!(next.changes(&previous).unwrap().layouts, vec![0]);
    }

    /// From an app checkout: `dbus-run-session -- cargo test --manifest-path
    /// vendor/craft-desktop/Cargo.toml --features linux,portal appmenu_dbus -- --ignored`.
    #[test]
    #[ignore = "requires a private D-Bus session"]
    fn appmenu_dbus_wire_layout_and_activation() {
        let mut tree = Tree::default();
        tree.update(&menu()).unwrap();
        let save = *tree
            .nodes
            .iter()
            .find(|(_, n)| {
                n.item
                    .action
                    .as_ref()
                    .is_some_and(|a| a.command == "file.save")
            })
            .unwrap()
            .0;
        let (tx, rx) = mpsc::sync_channel(1);
        let server = Builder::session()
            .unwrap()
            .serve_at(
                PATH,
                Exporter {
                    tree: Arc::new(RwLock::new(tree)),
                    clicks: tx,
                    repaint: Arc::new(|| {}),
                },
            )
            .unwrap()
            .build()
            .unwrap();
        let client = Builder::session()
            .unwrap()
            .method_timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let dest = server.unique_name().unwrap().as_str();
        let reply = client
            .call_method(
                Some(dest),
                PATH,
                Some(INTERFACE),
                "GetLayout",
                &(0i32, -1i32, Vec::<String>::new()),
            )
            .unwrap();
        // zvariant represents the message's argument list as a structure, including the
        // outer parentheses; the D-Bus method has separate `u` and `(ia{sv}av)` outputs.
        assert_eq!(reply.body().signature(), <(u32, Layout)>::SIGNATURE);
        let (revision, layout): (u32, Layout) = reply.body().deserialize().unwrap();
        assert_eq!(revision, 1);
        assert_eq!(layout.0, 0);
        assert!(!layout.2.is_empty());
        client
            .call_method(
                Some(dest),
                PATH,
                Some(INTERFACE),
                "Event",
                &(save, "clicked", Value::from(0i32), 0u32),
            )
            .unwrap();
        let overloaded = client
            .call_method(
                Some(dest),
                PATH,
                Some(INTERFACE),
                "Event",
                &(save, "clicked", Value::from(0i32), 0u32),
            )
            .unwrap_err();
        assert!(
            matches!(overloaded, zbus::Error::MethodError(..)),
            "full queue must return a D-Bus error, not wait for a client timeout"
        );
        let rejected: Vec<i32> = client
            .call_method(
                Some(dest),
                PATH,
                Some(INTERFACE),
                "EventGroup",
                &(vec![(save, "clicked", Value::from(0i32), 0u32)],),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        assert_eq!(rejected, vec![save]);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap()
                .action
                .command,
            "file.save"
        );
    }

    /// Reacquiring the same bus address must reject a consumer's pre-recovery ID.
    #[test]
    #[ignore = "requires a private D-Bus session"]
    fn appmenu_dbus_recovery_rejects_cached_ids_at_same_address() {
        const SERVICE: &str = "org.craft.Desktop.RecoveryTest";
        let mut m = menu();
        let tree = Arc::new(RwLock::new(Tree::default()));
        tree.write().unwrap().restart(&m).unwrap();
        let save_id = || {
            *tree
                .read()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, n)| n.item.key == "file.save")
                .unwrap()
                .0
        };
        let old = save_id();
        let (tx, rx) = mpsc::sync_channel(32);
        let export = || {
            Builder::session()
                .unwrap()
                .name(SERVICE)
                .unwrap()
                .serve_at(
                    PATH,
                    Exporter {
                        tree: tree.clone(),
                        clicks: tx.clone(),
                        repaint: Arc::new(|| {}),
                    },
                )
                .unwrap()
                .build()
                .unwrap()
        };
        let client = Builder::session()
            .unwrap()
            .method_timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let layout = || -> (u32, Layout) {
            client
                .call_method(
                    Some(SERVICE),
                    PATH,
                    Some(INTERFACE),
                    "GetLayout",
                    &(0i32, -1i32, Vec::<String>::new()),
                )
                .unwrap()
                .body()
                .deserialize()
                .unwrap()
        };
        let event = |id| {
            client.call_method(
                Some(SERVICE),
                PATH,
                Some(INTERFACE),
                "Event",
                &(id, "clicked", Value::from(0i32), 0u32),
            )
        };
        let server = export();
        let (old_revision, _) = layout();
        event(old).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().document, 1);
        server.close().unwrap();
        m.document = 2;
        m.items[0].children[0].action.as_mut().unwrap().command = "file.saveAs".into();
        tree.write().unwrap().restart(&m).unwrap();
        let new = save_id();
        assert_ne!(old, new);
        let server = export();
        // Deliberately send the cached ID before refreshing the client's layout.
        assert!(event(old).is_err());
        assert!(rx.try_recv().is_err());
        let (new_revision, _) = layout();
        assert!(new_revision > old_revision);
        event(new).unwrap();
        let click = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(click.document, 2);
        assert_eq!(click.action.command, "file.saveAs");
        server.close().unwrap();
    }
}
