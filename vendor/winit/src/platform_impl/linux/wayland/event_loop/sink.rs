//! An event loop's sink to deliver events from the Wayland event callbacks.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::vec::Drain;

use crate::event::{DeviceEvent, DeviceId as RootDeviceId, Event, Ime, WindowEvent};
use crate::platform::wayland::ImeDeleteSurrounding;
use crate::platform_impl::platform::DeviceId as PlatformDeviceId;
use crate::window::WindowId as RootWindowId;

use super::{DeviceId, WindowId};

/// Whether one text-input-v3 lifecycle generation may still deliver events.
///
/// Every enable commit creates a distinct token. A disable, reset or leave retires the token it
/// replaces: queued composition state (`Enabled`, `Preedit`) from that generation is dropped, but
/// a `Commit` that was already accepted from a protocol `done` is committed application input and
/// is still delivered with its deletion. Destroying the window discards the token completely.
/// Distinct tokens are used instead of a wrapping generation counter so an old event can never
/// compare equal to a newer generation.
#[derive(Clone, Debug)]
pub struct ImeLifecycleToken(Arc<ImeLifecycleState>);

#[derive(Debug)]
struct ImeLifecycleState {
    valid: AtomicBool,
    deliver_commits: AtomicBool,
}

impl Default for ImeLifecycleToken {
    fn default() -> Self {
        Self::new()
    }
}

impl ImeLifecycleToken {
    pub fn new() -> Self {
        Self(Arc::new(ImeLifecycleState {
            valid: AtomicBool::new(true),
            deliver_commits: AtomicBool::new(false),
        }))
    }

    /// Retires the generation after a disable, reset or leave.
    ///
    /// Queued composition events are dropped at delivery, while commits accepted before the
    /// boundary keep their place in the stream.
    pub(crate) fn invalidate(&self) {
        self.0.deliver_commits.store(true, Ordering::Release);
        self.0.valid.store(false, Ordering::Release);
    }

    /// Drops every queued event of this generation, including commits.
    ///
    /// Kept for state cleanup when the window is destroyed. The event loop also rejects every
    /// queued IME event for a closed window before delivery, so this is not the only guard for a
    /// destroyed window.
    pub(crate) fn discard(&self) {
        self.0.deliver_commits.store(false, Ordering::Release);
        self.0.valid.store(false, Ordering::Release);
    }

    /// Whether events tagged with this generation may still be delivered.
    #[cfg(test)]
    pub fn is_valid(&self) -> bool {
        self.0.valid.load(Ordering::Acquire)
    }

    /// Whether an event of the given kind may still be delivered.
    fn allows(&self, is_commit: bool) -> bool {
        if self.0.valid.load(Ordering::Acquire) {
            true
        } else {
            is_commit && self.0.deliver_commits.load(Ordering::Acquire)
        }
    }

    /// Whether both tokens are the same generation, not two generations with equal state.
    pub(crate) fn is_same_generation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A queued event with the IME lifecycle it belongs to, if any.
pub struct QueuedEvent {
    /// Process-wide push order.
    ///
    /// Window-originated (synthetic) events and compositor events live in two sinks so the window
    /// side can push without borrowing `WinitState`. The sequence is assigned when either sink
    /// accepts the event, and the event loop merges both sinks by it before delivery. This keeps
    /// the two queues one ordered stream without holding a lock across callbacks.
    sequence: u64,
    event: Event<()>,
    ime: Option<ImeEventMeta>,
}

/// The lifecycle generation and commit association of a queued compositor IME event.
pub(crate) struct ImeEventMeta {
    token: ImeLifecycleToken,
    /// Includes the mandatory clear immediately preceding an accepted commit.
    committed_input: bool,
    /// The surrounding-text deletion that goes with this `Ime::Commit`.
    ///
    /// Carrying it on the event itself keeps the deletion associated with its commit even when
    /// events from several windows or seats interleave.
    delete: Option<ImeDeleteSurrounding>,
}

impl QueuedEvent {
    /// The window an IME event belongs to, including synthetic lifecycle events.
    ///
    /// Synthetic `Enabled`/`Disabled`/clear events have no lifecycle token, so the closed-window
    /// tombstone must be checked for them too.
    pub(crate) fn ime_window(&self) -> Option<WindowId> {
        ime_event_window(&self.event)
    }

    /// Returns the event and the deletion that goes with it, or `None` if its lifecycle
    /// generation ended before delivery.
    ///
    /// Validity is checked here, at delivery, not when the event is queued: a callback can
    /// disable or reset IME after the compositor event was queued. Committed text stays
    /// deliverable; only composition state is retracted.
    pub(crate) fn into_deliverable(self) -> Option<(Event<()>, Option<ImeDeleteSurrounding>)> {
        let delete = match self.ime {
            Some(meta) if !meta.token.allows(meta.committed_input) => return None,
            Some(meta) => meta.delete,
            None => None,
        };
        Some((self.event, delete))
    }
}

/// Classify native IME input and completion controls at both drain and callback boundaries.
pub(crate) fn ime_event_window<T: 'static>(event: &Event<T>) -> Option<WindowId> {
    match event {
        Event::WindowEvent {
            window_id,
            event: WindowEvent::Ime(_) | WindowEvent::ImeResetCompleted { .. },
        } => Some(window_id.0),
        _ => None,
    }
}

/// Process-wide order of events pushed into any [`EventSink`].
///
/// The counter never wraps: wrapping would make a new event sort before an older queued one and
/// `sort_unstable_by_key` would then reorder the merged queue. Exhaustion is unreachable in a
/// process lifetime, but it is checked rather than silently corrupting the order.
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

// `try_update` is newer than this crate's Rust 1.70 minimum.
#[allow(deprecated)]
pub(crate) fn next_sequence() -> u64 {
    NEXT_SEQUENCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, checked_next_sequence)
        .expect("the Wayland event sequence space is exhausted")
}

fn checked_next_sequence(current: u64) -> Option<u64> {
    current.checked_add(1)
}

/// An event loop's sink to deliver events from the Wayland event callbacks
/// to the winit's user.
#[derive(Default)]
pub struct EventSink {
    pub(crate) window_events: Vec<QueuedEvent>,
}

impl EventSink {
    pub fn new() -> Self {
        Default::default()
    }

    /// Return `true` if there're pending events.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.window_events.is_empty()
    }

    /// Add new device event to a queue.
    #[inline]
    pub fn push_device_event(&mut self, event: DeviceEvent, device_id: DeviceId) {
        self.window_events.push(QueuedEvent {
            sequence: next_sequence(),
            event: Event::DeviceEvent {
                event,
                device_id: RootDeviceId(PlatformDeviceId::Wayland(device_id)),
            },
            ime: None,
        });
    }

    /// Add new window event to a queue.
    ///
    /// Used for control events that are not tied to an incoming IME generation.
    #[inline]
    pub fn push_window_event(&mut self, event: WindowEvent, window_id: WindowId) {
        self.window_events.push(QueuedEvent {
            sequence: next_sequence(),
            event: Event::WindowEvent { event, window_id: RootWindowId(window_id) },
            ime: None,
        });
    }

    /// Add a synthetic window event at a lifecycle boundary's reserved sequence.
    ///
    /// A lifecycle transition reserves this sequence while it still holds the text-input data
    /// lock, so a compositor event accepted after the transition always sorts after the synthetic
    /// boundary even if it reaches its sink first.
    #[inline]
    pub(crate) fn push_window_event_at(
        &mut self,
        sequence: u64,
        event: WindowEvent,
        window_id: WindowId,
    ) {
        self.window_events.push(QueuedEvent {
            sequence,
            event: Event::WindowEvent { event, window_id: RootWindowId(window_id) },
            ime: None,
        });
    }

    /// Queue a compositor IME event tagged with its generation.
    ///
    /// Composition events are dropped at delivery when the generation was retired; an accepted
    /// commit is still delivered.
    #[inline]
    pub(crate) fn push_ime_event(
        &mut self,
        ime: Ime,
        window_id: WindowId,
        token: &ImeLifecycleToken,
        delete: Option<ImeDeleteSurrounding>,
    ) {
        let committed_input = matches!(ime, Ime::Commit(_));
        self.push_classified_ime_event(
            next_sequence(),
            ime,
            window_id,
            token,
            delete,
            committed_input,
        );
    }

    /// Queue an accepted commit and the clear required by `Ime::Commit`'s contract.
    ///
    /// `Ime::Preedit` has one public slot for the window, not one per seat. An accepted commit
    /// from any seat ends the old visible composition, so this always queues the empty preedit
    /// before the commit and releases the old owner in the window state. The clear is part of the
    /// accepted transaction: both records survive leave/reset/disable and are discarded together
    /// only when the window is destroyed.
    pub(crate) fn push_ime_commit(
        &mut self,
        text: String,
        window_id: WindowId,
        token: &ImeLifecycleToken,
        delete: Option<ImeDeleteSurrounding>,
    ) {
        self.push_classified_ime_event(
            next_sequence(),
            Ime::Preedit(String::new(), None),
            window_id,
            token,
            None,
            true,
        );
        self.push_ime_event(Ime::Commit(text), window_id, token, delete);
    }

    fn push_classified_ime_event(
        &mut self,
        sequence: u64,
        ime: Ime,
        window_id: WindowId,
        token: &ImeLifecycleToken,
        delete: Option<ImeDeleteSurrounding>,
        committed_input: bool,
    ) {
        self.window_events.push(QueuedEvent {
            sequence,
            event: Event::WindowEvent {
                event: WindowEvent::Ime(ime),
                window_id: RootWindowId(window_id),
            },
            ime: Some(ImeEventMeta { token: token.clone(), committed_input, delete }),
        });
    }

    /// Append another sink and restore the process-wide push order.
    #[inline]
    pub(crate) fn append_ordered(&mut self, other: &mut Self) {
        self.window_events.append(&mut other.window_events);
        self.window_events.sort_unstable_by_key(|queued| queued.sequence);
    }

    /// Drain events the event loop can deliver, dropping IME events for closed windows first.
    ///
    /// `is_window_closed` is the window-scoped tombstone. Destruction is asynchronous with
    /// respect to event dispatch, so this check has to happen immediately before delivery rather
    /// than after the queue has already been handed to the application.
    pub(crate) fn drain_deliverable_for<F>(
        &mut self,
        mut is_window_closed: F,
    ) -> Vec<(Event<()>, Option<ImeDeleteSurrounding>)>
    where
        F: FnMut(WindowId) -> bool,
    {
        self.drain()
            .filter_map(|queued| {
                if let Some(window_id) = queued.ime_window() {
                    if is_window_closed(window_id) {
                        return None;
                    }
                }
                queued.into_deliverable()
            })
            .collect()
    }

    #[inline]
    pub fn drain(&mut self) -> Drain<'_, QueuedEvent> {
        self.window_events.drain(..)
    }
}

#[cfg(test)]
impl EventSink {
    /// Number of queued events, for tests of the production queue.
    pub(crate) fn queued_len(&self) -> usize {
        self.window_events.len()
    }

    /// Drains the queue through the delivery-time validity check the event loop uses.
    pub(crate) fn drain_deliverable(&mut self) -> Vec<(Event<()>, Option<ImeDeleteSurrounding>)> {
        self.drain_deliverable_for(|_| false)
    }
}

#[cfg(test)]
mod tests {
    use super::checked_next_sequence;

    #[test]
    fn sequence_exhaustion_is_rejected_instead_of_wrapping() {
        assert_eq!(checked_next_sequence(0), Some(1));
        assert_eq!(checked_next_sequence(u64::MAX - 1), Some(u64::MAX));
        assert_eq!(checked_next_sequence(u64::MAX), None);
    }

    #[test]
    fn reset_completion_is_window_scoped_and_dropped_after_window_closure() {
        use crate::event::{Event, WindowEvent};
        use crate::platform_impl::wayland::WindowId;

        let closed = WindowId::from(1);
        let live = WindowId::from(2);
        let mut sink = super::EventSink::new();
        sink.push_window_event(WindowEvent::ImeResetCompleted { request_id: 7 }, closed);
        sink.push_window_event(WindowEvent::ImeResetCompleted { request_id: 8 }, live);
        assert_eq!(sink.drain_deliverable_for(|window| window == closed), vec![(
            Event::WindowEvent {
                window_id: crate::window::WindowId(live),
                event: WindowEvent::ImeResetCompleted { request_id: 8 },
            },
            None,
        )]);
    }
}
