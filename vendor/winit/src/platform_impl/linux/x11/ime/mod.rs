// Important: all XIM calls need to happen from the same thread!

mod callbacks;
mod context;
mod inner;
mod input_method;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use self::callbacks::*;
use self::context::ImeContext;
pub use self::context::ImeContextCreationError;
use self::inner::{close_im, ImeInner};
use self::input_method::PotentialInputMethods;
use super::{ffi, util, XConnection, XError};
use crate::event::{Ime as PublicIme, WindowEvent};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ImeEvent {
    Enabled,
    Start,
    Update(String, usize),
    End,
    Disabled,
    /// Enqueued after XmbResetIC and its callbacks, never delivered ahead of this channel.
    Reset {
        request_id: Option<u64>,
    },
}

/// Composition belongs to a window; resetting one context must not clear another's preview.
#[derive(Default)]
pub(crate) struct CompositionState(std::collections::HashSet<ffi::Window>);

impl CompositionState {
    pub(crate) fn is_composing(&self, window: ffi::Window) -> bool {
        self.0.contains(&window)
    }

    pub(crate) fn end(&mut self, window: ffi::Window) -> bool {
        self.0.remove(&window)
    }

    pub(crate) fn handle(
        &mut self,
        window: ffi::Window,
        event: ImeEvent,
    ) -> [Option<WindowEvent>; 2] {
        let event = match event {
            ImeEvent::Enabled => PublicIme::Enabled,
            ImeEvent::Start => {
                self.0.insert(window);
                PublicIme::Preedit(String::new(), None)
            },
            ImeEvent::Update(text, position) if self.is_composing(window) => {
                PublicIme::Preedit(text, Some((position, position)))
            },
            ImeEvent::End => {
                self.end(window);
                PublicIme::Preedit(String::new(), None)
            },
            ImeEvent::Disabled => {
                self.end(window);
                PublicIme::Disabled
            },
            ImeEvent::Reset { request_id } => {
                let clear = self
                    .end(window)
                    .then(|| WindowEvent::Ime(PublicIme::Preedit(String::new(), None)));
                let completion =
                    request_id.map(|request_id| WindowEvent::ImeResetCompleted { request_id });
                return [clear, completion];
            },
            ImeEvent::Update(..) => return [None, None],
        };
        [Some(WindowEvent::Ime(event)), None]
    }
}

pub type ImeReceiver = Receiver<ImeRequest>;
pub type ImeSender = Sender<ImeRequest>;
pub type ImeEventReceiver = Receiver<(ffi::Window, ImeEvent)>;
pub type ImeEventSender = Sender<(ffi::Window, ImeEvent)>;

fn queue_reset_fence(
    sender: &ImeEventSender,
    window: ffi::Window,
    request_id: Option<u64>,
    reset: Result<bool, XError>,
) -> Result<bool, XError> {
    let reset = reset?;
    if reset {
        let _ = sender.send((window, ImeEvent::Reset { request_id }));
    }
    Ok(reset)
}

/// Request to control XIM handler from the window.
pub enum ImeRequest {
    /// Set IME spot position for given `window_id`.
    Position(ffi::Window, i16, i16),

    /// Allow IME input for the given `window_id`.
    Allow(ffi::Window, bool),

    /// Discard the composition of the given `window_id`.
    Reset(ffi::Window),

    /// Discard composition and queue the corresponding ordered completion fence.
    ResetWithRequest(ffi::Window, u64),
}

#[derive(Debug)]
pub(crate) enum ImeCreationError {
    // Boxed to prevent large error type
    OpenFailure(Box<PotentialInputMethods>),
    SetDestroyCallbackFailed(#[allow(dead_code)] XError),
}

pub(crate) struct Ime {
    xconn: Arc<XConnection>,
    // The actual meat of this struct is boxed away, since it needs to have a fixed location in
    // memory so we can pass a pointer to it around.
    inner: Box<ImeInner>,
}

impl Ime {
    pub fn new(
        xconn: Arc<XConnection>,
        event_sender: ImeEventSender,
    ) -> Result<Self, ImeCreationError> {
        let potential_input_methods = PotentialInputMethods::new(&xconn);

        let (mut inner, client_data) = {
            let mut inner = Box::new(ImeInner::new(xconn, potential_input_methods, event_sender));
            let inner_ptr = Box::into_raw(inner);
            let client_data = inner_ptr as _;
            let destroy_callback =
                ffi::XIMCallback { client_data, callback: Some(xim_destroy_callback) };
            inner = unsafe { Box::from_raw(inner_ptr) };
            inner.destroy_callback = destroy_callback;
            (inner, client_data)
        };

        let xconn = Arc::clone(&inner.xconn);

        let input_method = inner.potential_input_methods.open_im(
            &xconn,
            Some(&|| {
                let _ = unsafe { set_instantiate_callback(&xconn, client_data) };
            }),
        );

        let is_fallback = input_method.is_fallback();
        if let Some(input_method) = input_method.ok() {
            inner.is_fallback = is_fallback;
            unsafe {
                let result = set_destroy_callback(&xconn, input_method.im, &inner)
                    .map_err(ImeCreationError::SetDestroyCallbackFailed);
                if result.is_err() {
                    let _ = close_im(&xconn, input_method.im);
                }
                result?;
            }
            inner.im = Some(input_method);
            Ok(Ime { xconn, inner })
        } else {
            Err(ImeCreationError::OpenFailure(Box::new(inner.potential_input_methods)))
        }
    }

    pub fn is_destroyed(&self) -> bool {
        self.inner.is_destroyed
    }

    // This pattern is used for various methods here:
    // Ok(_) indicates that nothing went wrong internally
    // Ok(true) indicates that the action was actually performed
    // Ok(false) indicates that the action is not presently applicable
    pub fn create_context(
        &mut self,
        window: ffi::Window,
        with_ime: bool,
    ) -> Result<bool, ImeContextCreationError> {
        let context = if self.is_destroyed() {
            // Create empty entry in map, so that when IME is rebuilt, this window has a context.
            None
        } else {
            let im = self.inner.im.as_ref().unwrap();

            let context = unsafe {
                ImeContext::new(
                    &self.inner.xconn,
                    im,
                    window,
                    None,
                    self.inner.event_sender.clone(),
                    with_ime,
                )?
            };

            let event = if context.is_allowed() { ImeEvent::Enabled } else { ImeEvent::Disabled };
            self.inner.event_sender.send((window, event)).expect("Failed to send enabled event");

            Some(context)
        };

        self.inner.contexts.insert(window, context);
        Ok(!self.is_destroyed())
    }

    pub fn get_context(&self, window: ffi::Window) -> Option<ffi::XIC> {
        if self.is_destroyed() {
            return None;
        }
        if let Some(Some(context)) = self.inner.contexts.get(&window) {
            Some(context.ic)
        } else {
            None
        }
    }

    pub fn remove_context(&mut self, window: ffi::Window) -> Result<bool, XError> {
        if let Some(Some(context)) = self.inner.contexts.remove(&window) {
            unsafe {
                self.inner.destroy_ic_if_necessary(context.ic)?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn focus(&mut self, window: ffi::Window) -> Result<bool, XError> {
        if self.is_destroyed() {
            return Ok(false);
        }
        match self.inner.contexts.get_mut(&window) {
            // A context with IME disallowed stays unfocused, so the input method doesn't show
            // itself for a window that doesn't take text.
            Some(Some(context)) if context.is_allowed() => context.focus(&self.xconn).map(|_| true),
            _ => Ok(false),
        }
    }

    pub fn unfocus(&mut self, window: ffi::Window) -> Result<bool, XError> {
        if self.is_destroyed() {
            return Ok(false);
        }
        if let Some(&mut Some(ref mut context)) = self.inner.contexts.get_mut(&window) {
            context.unfocus(&self.xconn).map(|_| true)
        } else {
            Ok(false)
        }
    }

    pub fn send_xim_spot(&mut self, window: ffi::Window, x: i16, y: i16) {
        if self.is_destroyed() {
            return;
        }
        if let Some(&mut Some(ref mut context)) = self.inner.contexts.get_mut(&window) {
            context.set_spot(&self.xconn, x as _, y as _);
        }
    }

    /// Discards the window's composition, if it has a context.
    pub fn reset(&mut self, window: ffi::Window) -> Result<bool, XError> {
        if self.is_destroyed() {
            return Ok(false);
        }
        if let Some(&mut Some(ref mut context)) = self.inner.contexts.get_mut(&window) {
            context.reset(&self.xconn).map(|_| true)
        } else {
            Ok(false)
        }
    }

    pub fn reset_with_request(
        &mut self,
        window: ffi::Window,
        request_id: Option<u64>,
    ) -> Result<bool, XError> {
        // XmbResetIC may synchronously enqueue End or other callbacks. The fence must follow
        // those and callbacks already waiting, not bypass the receiver's FIFO.
        let reset = self.reset(window);
        queue_reset_fence(&self.inner.event_sender, window, request_id, reset)
    }

    /// Returns whether IME was turned on or off. The window keeps its input context either way;
    /// when the window has the focus, the caller focuses the context, or resets and unfocuses it.
    pub fn set_ime_allowed(
        &mut self,
        window: ffi::Window,
        allowed: bool,
    ) -> Result<bool, ImeContextCreationError> {
        if self.is_destroyed() {
            return Ok(false);
        }

        match self.inner.contexts.get_mut(&window) {
            Some(Some(context)) => {
                if allowed == context.is_allowed() {
                    return Ok(false);
                }
                context.set_allowed(allowed);
                let event = if allowed { ImeEvent::Enabled } else { ImeEvent::Disabled };
                self.inner.event_sender.send((window, event)).expect("Failed to send IME event");
                Ok(true)
            },
            _ => self.create_context(window, allowed),
        }
    }

    pub fn is_ime_allowed(&self, window: ffi::Window) -> bool {
        if self.is_destroyed() {
            false
        } else if let Some(Some(context)) = self.inner.contexts.get(&window) {
            context.is_allowed()
        } else {
            false
        }
    }
}

impl Drop for Ime {
    fn drop(&mut self) {
        unsafe {
            let _ = self.inner.destroy_all_contexts_if_necessary();
            let _ = self.inner.close_im_if_necessary();
        }
    }
}

#[cfg(test)]
mod reset_fence_tests {
    use std::sync::mpsc::channel;

    use super::{queue_reset_fence, CompositionState, ImeEvent, XError};
    use crate::event::{Ime, WindowEvent};

    #[test]
    fn reset_fence_follows_queued_callbacks_and_precedes_new_composition() {
        let (sender, receiver) = channel();
        sender.send((1, ImeEvent::Start)).unwrap();
        sender.send((1, ImeEvent::Update("old".into(), 3))).unwrap();
        assert!(queue_reset_fence(&sender, 1, Some(u64::MAX), Ok(true)).unwrap());
        // A late update without a new Start cannot revive the reset composition.
        sender.send((1, ImeEvent::Update("obsolete".into(), 8))).unwrap();
        sender.send((1, ImeEvent::Start)).unwrap();
        sender.send((1, ImeEvent::Update("new".into(), 3))).unwrap();

        let mut state = CompositionState::default();
        let actual: Vec<_> = receiver
            .try_iter()
            .flat_map(|(window, event)| state.handle(window, event).into_iter().flatten())
            .collect();
        assert_eq!(actual, vec![
            WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            WindowEvent::Ime(Ime::Preedit("old".into(), Some((3, 3)))),
            WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            WindowEvent::ImeResetCompleted { request_id: u64::MAX },
            WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            WindowEvent::Ime(Ime::Preedit("new".into(), Some((3, 3)))),
        ]);
    }

    #[test]
    fn terminal_pending_fence_completes_without_clearing_another_window() {
        let mut state = CompositionState::default();
        state.handle(2, ImeEvent::Start);
        assert_eq!(state.handle(1, ImeEvent::Reset { request_id: Some(7) }), [
            None,
            Some(WindowEvent::ImeResetCompleted { request_id: 7 })
        ],);
        assert!(state.is_composing(2));
        assert_eq!(state.handle(2, ImeEvent::Update("other".into(), 5)), [
            Some(WindowEvent::Ime(Ime::Preedit("other".into(), Some((5, 5))))),
            None
        ],);
        state.handle(1, ImeEvent::Start);
        state.handle(1, ImeEvent::End);
        assert_eq!(state.handle(1, ImeEvent::Reset { request_id: Some(8) }), [
            None,
            Some(WindowEvent::ImeResetCompleted { request_id: 8 })
        ],);
    }

    #[test]
    fn failed_or_missing_context_does_not_queue_a_successful_fence() {
        let (sender, receiver) = channel();
        assert!(!queue_reset_fence(&sender, 1, Some(1), Ok(false)).unwrap());
        let failure = XError {
            description: "reset failed".into(),
            error_code: 1,
            request_code: 0,
            minor_code: 0,
        };
        assert!(queue_reset_fence(&sender, 1, Some(2), Err(failure)).is_err());
        assert!(receiver.try_recv().is_err());
    }
}
