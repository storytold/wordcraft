//! Optional same-origin file capability. No provider is registered by default.
//!
//! The UI owns serialization and document state; the host only transports immutable captures.
//! A resource ID is an opaque provider identity, never a path or permission to run a command.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_BYTES: usize = 80 * 1024 * 1024;
const MAX_PENDING: usize = 16;
const MAX_EVENTS: usize = 64;
const MAX_RESOURCES: usize = 100;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resource {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub updated_at: String,
}

pub struct Event {
    pub metadata: Value,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Clone, Copy)]
enum TerminalOwner {
    Open,
    Save,
}

#[derive(Clone)]
struct Terminal {
    request: String,
    metadata: Value,
    owner: TerminalOwner,
}

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    id: String,
    label: String,
    namespace: String,
}

#[derive(Clone)]
struct Binding {
    resource: String,
    name: String,
}

#[derive(Clone)]
pub(crate) struct Capture {
    pub doc: u64,
    pub revision: u64,
    pub name: String,
    pub save_as: bool,
}

struct Dialog {
    doc: u64,
    name: String,
    save_as: bool,
}

/// Owned requests consumed only on the application's logic frame (never in a JS callback).
pub(crate) enum Command {
    Save { request: String, doc: u64, destination: String, name: String, save_as: bool },
    Result { capture: Capture, resource: String, status: String },
    Open { request: String, name: String, bytes: Vec<u8> },
}

#[derive(Default)]
struct State {
    identity: Option<Identity>,
    connection: String,
    nonce: String,
    serial: u64,
    enabled: bool,
    dialogs: HashMap<String, Dialog>,
    captures: HashMap<String, Capture>,
    reads: HashMap<String, String>,
    opening: HashMap<String, String>,
    working: HashMap<String, u64>,
    bindings: HashMap<u64, Binding>,
    resources: Vec<Resource>,
    commands: Vec<Command>,
    // A failed delivery transfers an already admitted capture here, even if commands is full.
    // Together with captures this is bounded by MAX_PENDING; it never admits new work.
    delivery_failures: Vec<Capture>,
    events: Vec<Event>,
    publishing: bool,
    // Owners stay in opening/working until delivery can be queued: at most 16 of each.
    finished: VecDeque<Terminal>,
    documents: Value,
}

impl State {
    /// Report unresolved transport ownership, queued terminals and publication in progress.
    /// A taken batch remains busy even when its last terminal is no longer in the event queue.
    fn busy(&self) -> bool {
        !self.dialogs.is_empty()
            || !self.captures.is_empty()
            || !self.reads.is_empty()
            || !self.opening.is_empty()
            || !self.working.is_empty()
            || !self.commands.is_empty()
            || !self.delivery_failures.is_empty()
            || !self.finished.is_empty()
            || self.publishing
            || self.events.iter().any(|event| matches!(event.metadata["type"].as_str(), Some("opened" | "openFailed" | "saveFailed")))
    }

    /// Allocate a nonce-scoped request ID, rejecting an uninitialized or exhausted counter.
    fn next(&mut self, kind: &str) -> Result<String, String> {
        if self.nonce.is_empty() {
            return Err("file provider session is not initialized".into());
        }
        self.serial = self.serial.checked_add(1).ok_or("file provider request counter exhausted")?;
        Ok(format!("{}-{kind}-{}", self.nonce, self.serial))
    }

    /// Queue one event within the event-count and aggregate event-byte budgets.
    /// Attach the current account/connection identity without changing request ownership on rejection.
    fn event(&mut self, mut metadata: Value, bytes: Option<Vec<u8>>) -> Result<(), String> {
        let total = self.events.iter().fold(0usize, |n, e| n.saturating_add(e.bytes.as_ref().map_or(0, Vec::len)));
        if self.events.len() >= MAX_EVENTS || total.saturating_add(bytes.as_ref().map_or(0, Vec::len)) > MAX_BYTES {
            return Err("file provider event queue is full; consume events before retrying".into());
        }
        if let (Some(identity), Some(fields)) = (&self.identity, metadata.as_object_mut()) {
            fields.insert("providerId".into(), identity.id.clone().into());
            fields.insert("namespace".into(), identity.namespace.clone().into());
            fields.insert("connectionId".into(), self.connection.clone().into());
        }
        self.events.push(Event { metadata, bytes });
        Ok(())
    }

    /// Find this document's save ownership across choices, captures, commands and recovery.
    /// Independent reads do not count as a pending save for this document.
    fn pending(&self, doc: u64) -> bool {
        self.dialogs.values().any(|d| d.doc == doc)
            || self.captures.values().any(|c| c.doc == doc)
            || self.working.values().any(|d| *d == doc)
            || self.delivery_failures.iter().any(|capture| capture.doc == doc)
            || self.commands.iter().any(|c| match c {
                Command::Save { doc: d, .. } => *d == doc,
                Command::Result { capture, .. } => capture.doc == doc,
                Command::Open { .. } => false,
            })
    }

    /// Move retained terminals into available event slots in order.
    /// Retire each working/opening owner only after its terminal has been queued successfully.
    fn flush_finished(&mut self) {
        while let Some(terminal) = self.finished.front().cloned() {
            if self.event(terminal.metadata, None).is_err() {
                break;
            }
            self.finished.pop_front();
            match terminal.owner {
                TerminalOwner::Open => {
                    self.opening.remove(&terminal.request);
                }
                TerminalOwner::Save => {
                    self.working.remove(&terminal.request);
                }
            }
        }
    }
}

/// One explicitly registered account namespace. Clones share the bounded transport state.
#[derive(Clone, Default)]
pub struct Host(Arc<Mutex<State>>);

struct Publication(Host);

impl Drop for Publication {
    /// Release publication ownership when the outer batch leaves its callback boundary.
    fn drop(&mut self) {
        self.0.state().publishing = false;
    }
}

impl Host {
    /// Lock shared transport state, retaining access to the state if an earlier holder poisoned the mutex.
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Initialize the per-instance request nonce exactly once after validating its ID and minimum length.
    pub fn set_session(&self, nonce: String) -> Result<(), String> {
        let mut s = self.state();
        if !s.nonce.is_empty() || !valid_id(&nonce) || nonce.len() < 16 {
            return Err("invalid or already initialized file provider session".into());
        }
        s.nonce = nonce;
        Ok(())
    }

    /// Register and enable a valid provider identity, returning its nonce-scoped connection ID.
    /// Re-registering the identical identity is idempotent; a different identity requires unregistering first.
    pub fn connect(&self, id: String, label: String, namespace: String) -> Result<String, String> {
        if !valid_id(&id) || !valid_text(&label, 100) || !valid_id(&namespace) {
            return Err("invalid file provider identity".into());
        }
        let mut s = self.state();
        let next = Identity { id, label, namespace };
        if let Some(current) = &s.identity {
            return if *current == next { Ok(s.connection.clone()) } else { Err("unregister the current provider before changing identity".into()) };
        }
        let connection = s.next("connection")?;
        s.connection = connection.clone();
        s.identity = Some(next);
        s.enabled = true;
        Ok(connection)
    }

    /// Clear provider identity, resources, bindings and metadata only when transport ownership is idle.
    /// Unresolved commands, retained/queued terminals and a taken publisher batch prevent account changes.
    pub fn disconnect(&self) -> Result<(), String> {
        let mut s = self.state();
        if s.busy() {
            return Err("finish or cancel pending operations before unregistering".into());
        }
        s.identity = None;
        s.connection.clear();
        s.enabled = false;
        s.resources.clear();
        s.bindings.clear();
        s.events.clear();
        s.documents = Value::Null;
        Ok(())
    }

    /// Report registration independently of whether new provider operations are enabled.
    pub fn connected(&self) -> bool {
        self.state().identity.is_some()
    }

    /// Report whether a registered provider currently admits new choices and reads.
    pub fn enabled(&self) -> bool {
        let s = self.state();
        s.identity.is_some() && s.enabled
    }

    /// Pause or resume new operations for a registered provider without discarding outstanding owners.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut s = self.state();
        if s.identity.is_none() {
            return Err("file provider is not connected".into());
        }
        s.enabled = enabled;
        Ok(())
    }

    /// Report whether account unregistering must wait for pending work or terminal publication.
    pub fn busy(&self) -> bool {
        self.state().busy()
    }

    /// Report unresolved save ownership for the specified document, including failed-delivery recovery.
    pub fn pending_for(&self, doc: u64) -> bool {
        self.state().pending(doc)
    }

    /// Report whether the document has a provider resource binding in this account.
    pub(crate) fn bound(&self, doc: u64) -> bool {
        self.state().bindings.contains_key(&doc)
    }

    /// Return the bound provider filename without treating its resource ID as a local path.
    pub(crate) fn filename(&self, doc: u64) -> Option<String> {
        self.state().bindings.get(&doc).map(|b| b.name.clone())
    }

    /// Start a destination choice. Repeated Save while one is unresolved does not recapture.
    /// Use the save-choice admission count, which includes ordinary commands and reserved failed captures.
    /// The separate read-owner count is not a single aggregate budget shared with saves.
    pub(crate) fn choose(&self, doc: u64, revision: u64, name: String, save_as: bool) -> Result<(), String> {
        if !valid_name(&name) {
            return Err("invalid provider filename".into());
        }
        let mut s = self.state();
        if s.identity.is_none() || !s.enabled {
            return Err("file provider is disabled".into());
        }
        if s.pending(doc) {
            return Ok(());
        }
        if s.dialogs.len() + s.captures.len() + s.commands.len() + s.working.len() + s.delivery_failures.len() >= MAX_PENDING {
            return Err("too many pending file provider operations".into());
        }
        let request = s.next("dialog")?;
        let resource = if save_as { None } else { s.bindings.get(&doc).map(|b| b.resource.clone()) };
        s.event(
            json!({"type":"saveDialog", "requestId":request, "documentId":doc.to_string(), "revision":revision,
            "name":name, "saveAs":save_as, "resourceId":resource}),
            None,
        )?;
        s.dialogs.insert(request, Dialog { doc, name, save_as });
        Ok(())
    }

    /// Resolve an owned choice to provider, device or cancellation after validating the destination/name.
    /// A full ordinary command queue leaves a non-cancelled choice owned so its callback can be retried.
    pub fn resolve_dialog(&self, request: &str, destination: &str, name: &str) -> Result<(), String> {
        if !matches!(destination, "provider" | "device" | "cancel") || (!name.is_empty() && !valid_name(name)) {
            return Err("invalid save destination or filename".into());
        }
        let mut s = self.state();
        if destination != "cancel" && s.commands.len() >= MAX_PENDING {
            return Err("file provider command queue is full".into());
        }
        let dialog = s.dialogs.remove(request).ok_or("unknown or completed save dialog")?;
        if destination != "cancel" {
            s.commands.push(Command::Save {
                request: request.into(),
                doc: dialog.doc,
                destination: destination.into(),
                name: if name.is_empty() { dialog.name } else { name.into() },
                save_as: dialog.save_as,
            });
        }
        Ok(())
    }

    /// Only the app may create captures, after committing drafts and encoding the selected format.
    /// Require the matching working document and preserve it if queueing the immutable payload fails.
    /// Save As or a changed filename cannot inherit an existing overwrite resource.
    pub(crate) fn capture(&self, request: &str, capture: Capture, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.len() > MAX_BYTES || !valid_name(&capture.name) {
            return Err("file exceeds the provider limit or has an invalid name".into());
        }
        let mut s = self.state();
        if s.identity.is_none() || s.captures.len() + s.delivery_failures.len() >= MAX_PENDING || s.working.get(request) != Some(&capture.doc) {
            return Err("file provider is disconnected or busy".into());
        }
        let resource =
            if capture.save_as { None } else { s.bindings.get(&capture.doc).filter(|b| b.name == capture.name).map(|b| b.resource.clone()) };
        s.event(
            json!({"type":"save", "requestId":request, "documentId":capture.doc.to_string(), "revision":capture.revision,
            "name":capture.name, "resourceId":resource, "destination":"provider", "bytesLength":bytes.len()}),
            Some(bytes),
        )?;
        s.captures.insert(request.into(), capture);
        s.working.remove(request);
        Ok(())
    }

    /// Validate and consume one still-owned capture into an app-thread terminal result.
    /// A full command queue preserves the capture; only the app consumer can acknowledge the captured revision.
    pub fn result(&self, request: String, status: String, resource: String) -> Result<(), String> {
        if !matches!(status.as_str(), "saved" | "failed" | "cancel" | "download")
            || (status == "saved" && !valid_id(&resource))
            || (!resource.is_empty() && !valid_id(&resource))
        {
            return Err("invalid file provider result".into());
        }
        let mut s = self.state();
        let capture = s.captures.get(&request).ok_or("unknown or completed provider save")?.clone();
        if s.commands.len() >= MAX_PENDING {
            return Err("file provider command queue is full".into());
        }
        s.captures.remove(&request);
        s.commands.push(Command::Result { capture, resource, status });
        Ok(())
    }

    /// Returns other documents bound to the overwritten resource. Their clean state is stale.
    /// Remove those aliases and refresh provider Recents; the application advances their document revisions.
    pub(crate) fn bind(&self, capture: &Capture, resource: String) -> Vec<u64> {
        let mut s = self.state();
        let aliases: Vec<u64> = s.bindings.iter().filter(|(doc, b)| **doc != capture.doc && b.resource == resource).map(|(doc, _)| *doc).collect();
        for doc in &aliases {
            s.bindings.remove(doc);
        }
        s.bindings.insert(capture.doc, Binding { resource: resource.clone(), name: capture.name.clone() });
        s.resources.retain(|r| r.id != resource);
        s.resources.insert(0, Resource { id: resource, name: capture.name.clone(), updated_at: String::new() });
        s.resources.truncate(MAX_RESOURCES);
        aliases
    }

    /// Replace current-account Recent metadata after validating count, fields and unique IDs.
    /// Reject a stale connection without changing the existing list.
    pub fn set_resources(&self, connection: &str, resources: Vec<Resource>) -> Result<(), String> {
        if resources.len() > MAX_RESOURCES {
            return Err("too many provider resources".into());
        }
        let mut ids = HashSet::new();
        if resources.iter().any(|r| !valid_id(&r.id) || !valid_name(&r.name) || !valid_optional_text(&r.updated_at, 100) || !ids.insert(&r.id)) {
            return Err("invalid or duplicate provider resource".into());
        }
        let mut s = self.state();
        if s.identity.is_none() || connection != s.connection {
            return Err("stale provider connection".into());
        }
        s.resources = resources;
        Ok(())
    }

    /// Request bytes for an enabled listed resource and return the correlated read ID.
    /// Read admission counts reads plus opening owners separately from the save-choice admission budget.
    pub fn request_open(&self, resource: &str) -> Result<String, String> {
        let mut s = self.state();
        if !s.enabled || s.identity.is_none() || !s.resources.iter().any(|r| r.id == resource) {
            return Err("unknown or unavailable provider resource".into());
        }
        if s.reads.len() + s.opening.len() >= MAX_PENDING {
            return Err("too many pending provider reads".into());
        }
        let request = s.next("open")?;
        s.event(json!({"type":"openRequest", "requestId":request, "resourceId":resource}), None)?;
        s.reads.insert(request.clone(), resource.into());
        Ok(request)
    }

    /// Admit bounded bytes for a still-owned read into the ordinary app command queue.
    /// Validate filename, queue count and aggregate open bytes before retiring callback ownership.
    pub fn open(&self, request: String, name: String, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.len() > MAX_BYTES || !valid_name(&name) {
            return Err("file exceeds the provider limit or has an invalid name".into());
        }
        let mut s = self.state();
        let queued = s.commands.iter().fold(0usize, |n, c| {
            n.saturating_add(match c {
                Command::Open { bytes, .. } => bytes.len(),
                _ => 0,
            })
        });
        if s.commands.len() >= MAX_PENDING || queued.saturating_add(bytes.len()) > MAX_BYTES {
            return Err("file provider open queue is full".into());
        }
        let resource = s.reads.remove(&request).ok_or("unknown or completed provider read")?;
        s.opening.insert(request.clone(), resource);
        s.commands.push(Command::Open { request, name, bytes });
        Ok(())
    }

    /// Queue a correlated failure for an outstanding read and then retire its callback ownership.
    /// Invalid error text or a full event queue leaves that read outstanding.
    pub fn cancel_open(&self, request: String, error: String) -> Result<(), String> {
        if !valid_optional_text(&error, 1024) {
            return Err("invalid provider read error".into());
        }
        let mut s = self.state();
        let resource = s.reads.get(&request).ok_or("unknown or completed provider read")?.clone();
        s.event(json!({"type":"openFailed", "requestId":request, "resourceId":resource, "error":error}), None)?;
        s.reads.remove(&request);
        Ok(())
    }

    /// Complete an app-admitted read once, binding only a successfully imported document.
    /// Retain its opened/openFailed terminal and owner until event capacity is available.
    pub(crate) fn opened(&self, request: &str, capture: Option<Capture>, error: &str) {
        let mut s = self.state();
        let Some(resource) = s.opening.get(request).cloned() else { return };
        if s.finished.iter().any(|terminal| terminal.request == request) {
            return;
        }
        let metadata = if let Some(capture) = capture {
            s.bindings.insert(capture.doc, Binding { resource: resource.clone(), name: capture.name.clone() });
            json!({"type":"opened", "requestId":request, "resourceId":resource,
                "documentId":capture.doc.to_string(), "revision":capture.revision})
        } else {
            json!({"type":"openFailed", "requestId":request, "resourceId":resource, "error":bounded_error(error)})
        };
        s.finished.push_back(Terminal { request: request.into(), metadata, owner: TerminalOwner::Open });
        s.flush_finished();
    }

    /// Fail a still-working save once without acknowledging or binding it.
    /// Bound the error and retain its terminal until it can enter the event queue.
    pub(crate) fn failed(&self, request: &str, message: &str) {
        let mut s = self.state();
        if !s.working.contains_key(request) || s.finished.iter().any(|terminal| terminal.request == request) {
            return;
        }
        s.finished.push_back(Terminal {
            request: request.into(),
            metadata: json!({"type":"saveFailed", "requestId":request, "error":bounded_error(message)}),
            owner: TerminalOwner::Save,
        });
        s.flush_finished();
    }

    /// Transfer ordinary commands and reserved failed captures to the application logic frame.
    /// Failed captures become non-durable results; Save commands acquire working ownership before serialization.
    pub(crate) fn take_commands(&self) -> Vec<Command> {
        let mut s = self.state();
        let mut commands = std::mem::take(&mut s.commands);
        commands.extend(std::mem::take(&mut s.delivery_failures).into_iter().map(|capture| Command::Result {
            capture,
            resource: String::new(),
            status: "failed".into(),
        }));
        for command in &commands {
            if let Command::Save { request, doc, .. } = command {
                s.working.insert(request.clone(), *doc);
            }
        }
        commands
    }

    /// Recover only an unresolved owner identified by the failed event's connection and request.
    /// Accepted synchronous replies win; failed reads lose callback authority before their retained terminal drains.
    /// Failed save payloads use reserved metadata-only space for later application consumption.
    fn delivery_failed(&self, metadata: &Value) {
        let mut s = self.state();
        if s.identity.is_none() || metadata["connectionId"].as_str() != Some(s.connection.as_str()) {
            return;
        }
        let Some(request) = metadata["requestId"].as_str() else { return };
        match metadata["type"].as_str() {
            Some("save") => {
                // A synchronous listener may already have accepted a terminal result. In that
                // case the capture is no longer ours and must never be failed a second time.
                if let Some(capture) = s.captures.remove(request) {
                    s.delivery_failures.push(capture);
                }
            }
            Some("saveDialog") => {
                if let Some(dialog) = s.dialogs.remove(request) {
                    s.working.insert(request.into(), dialog.doc);
                    s.finished.push_back(Terminal {
                        request: request.into(),
                        metadata: json!({"type":"saveFailed", "requestId":request, "error":"Provider event delivery failed."}),
                        owner: TerminalOwner::Save,
                    });
                }
            }
            Some("openRequest") => {
                if let Some(resource) = s.reads.remove(request) {
                    s.opening.insert(request.into(), resource.clone());
                    s.finished.push_back(Terminal {
                        request: request.into(),
                        metadata: json!({"type":"openFailed", "requestId":request, "resourceId":resource,
                            "error":"Provider event delivery failed."}),
                        owner: TerminalOwner::Open,
                    });
                }
            }
            _ => {}
        }
        s.flush_finished();
    }

    /// Publish bounded batches without holding the state lock during callbacks. Failed delivery
    /// releases only an owner still held by this connection; a save is consumed on a logic frame.
    /// Returns whether the caller must schedule that frame. Listener exceptions swallowed by
    /// the browser's event dispatcher are not observable delivery failures here.
    pub fn publish_events_with<E>(&self, mut dispatch: impl FnMut(&Value, Option<Vec<u8>>) -> Result<(), E>, mut on_error: impl FnMut(E)) -> bool {
        {
            let mut s = self.state();
            if s.publishing {
                return false;
            }
            s.publishing = true;
        }
        let _publication = Publication(self.clone());
        let mut repaint = false;
        // Keep account ownership across both drains, including a last terminal event already
        // removed from the queue. A reentrant publisher leaves the outer batch in charge.
        for _ in 0..2 {
            let events = self.take_events();
            if events.is_empty() {
                break;
            }
            for event in events {
                if let Err(error) = dispatch(&event.metadata, event.bytes) {
                    self.delivery_failed(&event.metadata);
                    repaint = true;
                    on_error(error);
                }
            }
        }
        repaint || {
            let s = self.state();
            !s.events.is_empty() || !s.finished.is_empty() || !s.commands.is_empty() || !s.delivery_failures.is_empty()
        }
    }

    /// Release a device choice after the platform has taken its bytes. This never ACKs a provider.
    pub(crate) fn finish_device(&self, request: &str) {
        self.state().working.remove(request);
    }

    /// Take the current event batch and refill newly available slots from retained terminals.
    /// A caller must finish its taken batch before permitting an account change; the publisher guard enforces this.
    pub fn take_events(&self) -> Vec<Event> {
        let mut s = self.state();
        let events = std::mem::take(&mut s.events);
        s.flush_finished();
        if events.is_empty() { std::mem::take(&mut s.events) } else { events }
    }

    /// Replace the document snapshot even when no provider is registered.
    /// Retire closed-document bindings and choices; captured saves still require their own terminal result.
    pub(crate) fn publish_documents(&self, documents: Value, open_ids: &[u64]) {
        let mut s = self.state();
        s.documents = documents;
        s.bindings.retain(|doc, _| open_ids.contains(doc));
        // A closed tab cannot retain an unresolvable chooser. Captures still require a result.
        s.dialogs.retain(|_, d| open_ids.contains(&d.doc));
    }

    /// Return protocol, connection, resource, document and pending metadata without taking queued work.
    /// This trusted same-origin metadata read is available before provider registration; it grants no file operation.
    pub fn snapshot(&self) -> Value {
        let s = self.state();
        json!({"protocolVersion":1, "connected":s.identity.is_some(), "enabled":s.identity.is_some() && s.enabled,
            "providerId":s.identity.as_ref().map(|i| &i.id), "label":s.identity.as_ref().map(|i| &i.label),
            "namespace":s.identity.as_ref().map(|i| &i.namespace), "connectionId":s.connection,
            "resources":s.resources, "documents":s.documents, "pending":s.busy()})
    }

    /// Native Home/Open view, separate from paths belonging to this device.
    /// Only enabled provider entries issue correlated reads; resource names never enter device Recents.
    pub(crate) fn recent_ui(&self, ui: &mut egui::Ui) {
        let (label, resources) = {
            let s = self.state();
            let Some(identity) = s.identity.as_ref().filter(|_| s.enabled) else { return };
            (identity.label.clone(), s.resources.clone())
        };
        ui.heading(crate::i18n::fmt(crate::tl!("Recent — {label}"), &[("label", &label)]));
        if resources.is_empty() {
            ui.weak(crate::tl!("No recent provider files."));
        }
        for resource in resources {
            if ui.button(&resource.name).on_hover_text(&resource.updated_at).clicked()
                && let Err(error) = self.request_open(&resource.id)
            {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        }
        ui.separator();
        ui.weak(crate::tl!("Recent on this device"));
    }
}

/// Remove control characters and truncate error text at a UTF-8 boundary within 1024 bytes.
fn bounded_error(message: &str) -> String {
    let mut error = String::new();
    for ch in message.chars().filter(|ch| !ch.is_control()) {
        if error.len().saturating_add(ch.len_utf8()) > 1024 {
            break;
        }
        error.push(ch);
    }
    error
}

/// Accept only nonempty IDs of at most 160 ASCII letters, digits, hyphens or underscores.
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 160 && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Require nonblank text within the byte limit and without control characters.
fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && valid_optional_text(value, max)
}

/// Validate byte length and control characters while allowing an empty optional field.
fn valid_optional_text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.chars().any(char::is_control)
}

/// Validate a bounded filename, excluding path separators and the special dot directory names.
pub(crate) fn valid_name(value: &str) -> bool {
    valid_text(value, 255) && !matches!(value, "." | "..") && !value.contains(['/', '\\'])
}

#[cfg(test)]
mod tests;
