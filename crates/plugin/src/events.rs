//! Protocol 1.2: document events. A plugin whose manifest declares `events` hears, after the
//! fact, when a document was opened from a local file, saved to one, or closed.
//!
//! Events never hold up the app: each goes to each listening plugin on a thread of its own, and
//! what comes back (a message, an error) is queued for the app to show ([`take_event_outcomes`]).
//! During an event a plugin may log and send messages, but `host/sync` is refused
//! (`AccessDenied`): events are notifications, not edits. A timeout counts as a failure of the
//! plugin, as for commands.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::{HostError, lock};

/// A document event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentEvent {
    /// `events/documentOpened {path, name}`.
    Opened,
    /// `events/documentSaved {path, name, format}`.
    Saved,
    /// `events/documentClosed {path, name}`.
    Closed,
}

/// The event names a manifest's `events` may hold.
pub const EVENT_NAMES: [&str; 3] = ["documentOpened", "documentSaved", "documentClosed"];

/// Most outcomes kept for the app to show; older ones are dropped (the CLI never reads them).
const MAX_OUTCOMES: usize = 64;

impl DocumentEvent {
    /// The name in a manifest's `events`.
    pub fn name(self) -> &'static str {
        match self {
            DocumentEvent::Opened => "documentOpened",
            DocumentEvent::Saved => "documentSaved",
            DocumentEvent::Closed => "documentClosed",
        }
    }

    /// The JSON-RPC method.
    pub fn method(self) -> &'static str {
        match self {
            DocumentEvent::Opened => "events/documentOpened",
            DocumentEvent::Saved => "events/documentSaved",
            DocumentEvent::Closed => "events/documentClosed",
        }
    }

    /// How long a plugin has to answer: 30 s, or 5 s for a close (the app may be quitting).
    pub fn timeout(self) -> Duration {
        match self {
            DocumentEvent::Opened | DocumentEvent::Saved => Duration::from_secs(30),
            DocumentEvent::Closed => Duration::from_secs(5),
        }
    }
}

/// What a plugin answered an event: a message for the status bar, or an error.
#[derive(Clone, Debug, PartialEq)]
pub struct EventOutcome {
    pub plugin: String,
    /// The plugin's name, for the user.
    pub name: String,
    pub event: DocumentEvent,
    pub message: Option<String>,
    pub error: Option<String>,
}

fn outcomes() -> &'static Mutex<VecDeque<EventOutcome>> {
    static OUTCOMES: Mutex<VecDeque<EventOutcome>> = Mutex::new(VecDeque::new());
    &OUTCOMES
}

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// One event request in flight: counted while it lives, whatever way its thread ends.
struct Flight;

impl Flight {
    fn start() -> Flight {
        IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
        Flight
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
    }
}

fn push(outcome: EventOutcome) {
    let mut q = lock(outcomes());
    if q.len() >= MAX_OUTCOMES {
        q.pop_front();
    }
    q.push_back(outcome);
}

/// The messages and errors plugins answered events with since the last call, oldest first.
pub fn take_event_outcomes() -> Vec<EventOutcome> {
    lock(outcomes()).drain(..).collect()
}

/// How many event requests are still waiting for their plugin.
pub fn events_in_flight() -> usize {
    IN_FLIGHT.load(Ordering::Relaxed)
}

/// Waits until no event is waiting for its plugin, or `timeout` has passed. True when none is.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)] // off wasm only: the clock works here
pub fn wait_for_events(timeout: Duration) -> bool {
    let end = std::time::Instant::now() + timeout;
    while events_in_flight() > 0 {
        if std::time::Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// The web build has no plugins, so no events.
#[cfg(target_arch = "wasm32")]
pub fn wait_for_events(_timeout: Duration) -> bool {
    true
}

/// The loaded, active (on, not blocked) plugins that declared `event` (Office add-ins hear none).
pub fn listeners(event: DocumentEvent) -> Vec<std::sync::Arc<crate::Plugin>> {
    crate::list().into_iter().filter(|p| p.active() && p.manifest.office.is_none() && p.manifest.events.iter().any(|e| e == event.name())).collect()
}

/// The answer to a plugin's host request during an event.
fn host_during_event(plugin: &str, name: &str, event: DocumentEvent, method: &str, params: &Json) -> Result<Json, HostError> {
    let text = || -> String { crate::dialog::cut(params.get("text").and_then(Json::as_str).unwrap_or(""), 4000) };
    match method {
        "host/log" => {
            log::info!("add-in {plugin}: {}", text());
            Ok(json!({}))
        }
        "host/message" => {
            push(EventOutcome { plugin: plugin.into(), name: name.into(), event, message: Some(text()), error: None });
            Ok(json!({"result": 1}))
        }
        "host/sync" | "host/vbaProject/get" | "host/vbaProject/set" => Err(HostError {
            code: -32000,
            message: format!("{} is a notification: the document can't be read or changed during it", event.method()),
            data: json!({"code": "AccessDenied"}),
        }),
        _ => Err(HostError::unknown()),
    }
}

/// Sends `event` with `params` to every plugin that declared it, each on a thread of its own, and
/// returns at once with how many plugins it went to.
pub fn send_event(event: DocumentEvent, params: Json) -> usize {
    let targets = listeners(event);
    if cfg!(target_arch = "wasm32") {
        return 0;
    }
    let mut sent = 0;
    for plugin in targets {
        let params = params.clone();
        let flight = Flight::start();
        let id = plugin.manifest.id.clone();
        let spawned = std::thread::Builder::new().name(format!("plugin-{id}-event")).spawn(move || {
            let _flight = flight;
            let (pid, name) = (plugin.manifest.id.clone(), plugin.manifest.name.clone());
            let r = plugin.request_event(event.method(), params, event.timeout(), &mut |m, p| host_during_event(&pid, &name, event, m, p));
            match r {
                Ok(answer) => {
                    let message = answer.get("message").and_then(Json::as_str).map(|m| crate::dialog::cut(m, 4000)).filter(|m| !m.trim().is_empty());
                    if let Some(m) = message {
                        push(EventOutcome { plugin: pid, name, event, message: Some(m), error: None });
                    }
                }
                Err(e) => {
                    log::warn!("add-in {pid}: {}: {e}", event.method());
                    push(EventOutcome { plugin: pid, name, event, message: None, error: Some(crate::dialog::cut(&e, 4000)) });
                }
            }
        });
        match spawned {
            Ok(_) => sent += 1,
            Err(e) => {
                // The closure (and the flight it holds) is dropped with the error.
                log::warn!("add-in {id}: can't send {}: {e}", event.method());
            }
        }
    }
    sent
}
