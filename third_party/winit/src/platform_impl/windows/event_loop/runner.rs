use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::{mem, panic};

use windows_sys::Win32::Foundation::HWND;

use crate::dpi::PhysicalSize;
use crate::event::{Event, InnerSizeWriter, StartCause, WindowEvent};
use crate::platform_impl::platform::event_loop::{WindowData, GWL_USERDATA};
use crate::platform_impl::platform::get_window_long;
use crate::window::WindowId;

use super::ControlFlow;

pub(crate) type EventLoopRunnerShared<T> = Rc<EventLoopRunner<T>>;

type EventHandler<T> = Cell<Option<Box<dyn FnMut(Event<T>)>>>;

pub(crate) struct EventLoopRunner<T: 'static> {
    // The event loop's win32 handles
    pub(super) thread_msg_target: HWND,

    // Setting this will ensure pump_events will return to the external
    // loop asap. E.g. set after each RedrawRequested to ensure pump_events
    // can't stall an external loop beyond a frame
    pub(super) interrupt_msg_dispatch: Cell<bool>,

    control_flow: Cell<ControlFlow>,
    exit: Cell<Option<i32>>,
    runner_state: Cell<RunnerState>,
    last_events_cleared: Cell<Instant>,
    event_handler: EventHandler<T>,
    event_buffer: RefCell<VecDeque<BufferedEvent<T>>>,

    panic_error: Cell<Option<PanicError>>,
}

pub type PanicError = Box<dyn Any + Send + 'static>;

/// See `move_state_to` function for details on how the state loop works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RunnerState {
    /// The event loop has just been created, and an `Init` event must be sent.
    Uninitialized,
    /// The event loop is idling.
    Idle,
    /// The event loop is handling the OS's events and sending them to the user's callback.
    /// `NewEvents` has been sent, and `AboutToWait` hasn't.
    HandlingMainEvents,
    /// The event loop has been destroyed. No other events will be emitted.
    Destroyed,
}

enum BufferedEvent<T: 'static> {
    Event(Event<T>),
    ImeResetCompletion(Event<T>, Box<dyn FnOnce() -> bool>),
    ScaleFactorChanged(WindowId, f64, PhysicalSize<u32>),
}

impl<T> EventLoopRunner<T> {
    pub(crate) fn new(thread_msg_target: HWND) -> EventLoopRunner<T> {
        EventLoopRunner {
            thread_msg_target,
            interrupt_msg_dispatch: Cell::new(false),
            runner_state: Cell::new(RunnerState::Uninitialized),
            control_flow: Cell::new(ControlFlow::default()),
            exit: Cell::new(None),
            panic_error: Cell::new(None),
            last_events_cleared: Cell::new(Instant::now()),
            event_handler: Cell::new(None),
            event_buffer: RefCell::new(VecDeque::new()),
        }
    }

    /// Associate the application's event handler with the runner
    ///
    /// # Safety
    /// This is ignoring the lifetime of the application handler (which may not
    /// outlive the EventLoopRunner) and can lead to undefined behaviour if
    /// the handler is not cleared before the end of real lifetime.
    ///
    /// All public APIs that take an event handler (`run`, `run_on_demand`,
    /// `pump_events`) _must_ pair a call to `set_event_handler` with
    /// a call to `clear_event_handler` before returning to avoid
    /// undefined behaviour.
    pub(crate) unsafe fn set_event_handler<F>(&self, f: F)
    where
        F: FnMut(Event<T>),
    {
        // Erase closure lifetime.
        // SAFETY: Caller upholds that the lifetime of the closure is upheld.
        let f = unsafe {
            mem::transmute::<Box<dyn FnMut(Event<T>)>, Box<dyn FnMut(Event<T>)>>(Box::new(f))
        };
        let old_event_handler = self.event_handler.replace(Some(f));
        assert!(old_event_handler.is_none());
    }

    pub(crate) fn clear_event_handler(&self) {
        self.event_handler.set(None);
    }

    pub(crate) fn reset_runner(&self) {
        let EventLoopRunner {
            thread_msg_target: _,
            interrupt_msg_dispatch,
            runner_state,
            panic_error,
            control_flow: _,
            exit,
            last_events_cleared: _,
            event_handler,
            event_buffer: _,
        } = self;
        interrupt_msg_dispatch.set(false);
        runner_state.set(RunnerState::Uninitialized);
        panic_error.set(None);
        exit.set(None);
        event_handler.set(None);
    }
}

/// State retrieval functions.
impl<T> EventLoopRunner<T> {
    #[allow(unused)]
    pub fn thread_msg_target(&self) -> HWND {
        self.thread_msg_target
    }

    pub fn take_panic_error(&self) -> Result<(), PanicError> {
        match self.panic_error.take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    pub fn set_control_flow(&self, control_flow: ControlFlow) {
        self.control_flow.set(control_flow)
    }

    pub fn control_flow(&self) -> ControlFlow {
        self.control_flow.get()
    }

    pub fn set_exit_code(&self, code: i32) {
        self.exit.set(Some(code))
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit.get()
    }

    pub fn clear_exit(&self) {
        self.exit.set(None);
    }

    pub fn should_buffer(&self) -> bool {
        let handler = self.event_handler.take();
        let should_buffer = handler.is_none();
        self.event_handler.set(handler);
        should_buffer
    }
}

/// Misc. functions
impl<T> EventLoopRunner<T> {
    pub fn catch_unwind<R>(&self, f: impl FnOnce() -> R) -> Option<R> {
        let panic_error = self.panic_error.take();
        if panic_error.is_none() {
            let result = panic::catch_unwind(panic::AssertUnwindSafe(f));

            // Check to see if the panic error was set in a re-entrant call to catch_unwind inside
            // of `f`. If it was, that error takes priority. If it wasn't, check if our call to
            // catch_unwind caught any panics and set panic_error appropriately.
            match self.panic_error.take() {
                None => match result {
                    Ok(r) => Some(r),
                    Err(e) => {
                        self.panic_error.set(Some(e));
                        None
                    },
                },
                Some(e) => {
                    self.panic_error.set(Some(e));
                    None
                },
            }
        } else {
            self.panic_error.set(panic_error);
            None
        }
    }
}

/// Event dispatch functions.
impl<T> EventLoopRunner<T> {
    pub(crate) fn prepare_wait(&self) {
        self.move_state_to(RunnerState::Idle);
    }

    pub(crate) fn wakeup(&self) {
        self.move_state_to(RunnerState::HandlingMainEvents);
    }

    pub(crate) fn send_event(&self, event: Event<T>) {
        if let Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } = event {
            self.call_event_handler(event);
            // As a rule, to ensure that `pump_events` can't block an external event loop
            // for too long, we always guarantee that `pump_events` will return control to
            // the external loop asap after a `RedrawRequested` event is dispatched.
            self.interrupt_msg_dispatch.set(true);
        } else if self.should_buffer() {
            // If the runner is already borrowed, we're in the middle of an event loop invocation.
            // Add the event to a buffer to be processed later.
            self.event_buffer.borrow_mut().push_back(BufferedEvent::from_event(event))
        } else {
            self.call_event_handler(event);
            self.dispatch_buffered_events();
        }
    }

    /// Publish the input events accepted by one native IME message.
    pub(crate) fn publish_ime_batch(&self, events: impl IntoIterator<Item = Event<T>>) {
        // Accept the whole native message before exposing its first callback. A reset from
        // that callback must follow a result already copied by this message, including its
        // optional next preedit or END notification.
        self.event_buffer.borrow_mut().extend(events.into_iter().map(BufferedEvent::from_event));
        if !self.should_buffer() {
            self.dispatch_buffered_events();
        }
    }

    pub(crate) fn send_ime_reset_completion(
        &self,
        event: Event<T>,
        valid: impl FnOnce() -> bool + 'static,
    ) {
        self.event_buffer
            .borrow_mut()
            .push_back(BufferedEvent::ImeResetCompletion(event, Box::new(valid)));
        if !self.should_buffer() {
            self.dispatch_buffered_events();
        }
    }

    pub(crate) fn loop_destroyed(&self) {
        self.move_state_to(RunnerState::Destroyed);
    }

    fn call_event_handler(&self, event: Event<T>) {
        self.catch_unwind(|| {
            let mut event_handler = self.event_handler.take().expect(
                "either event handler is re-entrant (likely), or no event handler is registered \
                 (very unlikely)",
            );

            event_handler(event);

            assert!(self.event_handler.replace(Some(event_handler)).is_none());
        });
    }

    fn dispatch_buffered_events(&self) {
        loop {
            // We do this instead of using a `while let` loop because if we use a `while let`
            // loop the reference returned `borrow_mut()` doesn't get dropped until the end
            // of the loop's body and attempts to add events to the event buffer while in
            // `process_event` will fail.
            let buffered_event_opt = self.event_buffer.borrow_mut().pop_front();
            match buffered_event_opt {
                Some(e) => e.dispatch_event(|e| self.call_event_handler(e)),
                None => break,
            }
        }
    }

    /// Dispatch control flow events (`NewEvents`, `AboutToWait`, and
    /// `LoopExiting`) as necessary to bring the internal `RunnerState` to the
    /// new runner state.
    ///
    /// The state transitions are defined as follows:
    ///
    /// ```text
    ///    Uninitialized
    ///          |
    ///          V
    ///        Idle
    ///       ^    |
    ///       |    V
    /// HandlingMainEvents
    ///         |
    ///         V
    ///     Destroyed
    /// ```
    ///
    /// Attempting to transition back to `Uninitialized` will result in a panic. Attempting to
    /// transition *from* `Destroyed` will also result in a panic. Transitioning to the current
    /// state is a no-op. Even if the `new_runner_state` isn't the immediate next state in the
    /// runner state machine (e.g. `self.runner_state == HandlingMainEvents` and
    /// `new_runner_state == Idle`), the intermediate state transitions will still be executed.
    fn move_state_to(&self, new_runner_state: RunnerState) {
        use RunnerState::{Destroyed, HandlingMainEvents, Idle, Uninitialized};

        match (self.runner_state.replace(new_runner_state), new_runner_state) {
            (Uninitialized, Uninitialized)
            | (Idle, Idle)
            | (HandlingMainEvents, HandlingMainEvents)
            | (Destroyed, Destroyed) => (),

            // State transitions that initialize the event loop.
            (Uninitialized, HandlingMainEvents) => {
                self.call_new_events(true);
            },
            (Uninitialized, Idle) => {
                self.call_new_events(true);
                self.call_event_handler(Event::AboutToWait);
                self.last_events_cleared.set(Instant::now());
            },
            (Uninitialized, Destroyed) => {
                self.call_new_events(true);
                self.call_event_handler(Event::AboutToWait);
                self.last_events_cleared.set(Instant::now());
                self.call_event_handler(Event::LoopExiting);
            },
            (_, Uninitialized) => panic!("cannot move state to Uninitialized"),

            // State transitions that start the event handling process.
            (Idle, HandlingMainEvents) => {
                self.call_new_events(false);
            },
            (Idle, Destroyed) => {
                self.call_event_handler(Event::LoopExiting);
            },

            (HandlingMainEvents, Idle) => {
                // This is always the last event we dispatch before waiting for new events
                self.call_event_handler(Event::AboutToWait);
                self.last_events_cleared.set(Instant::now());
            },
            (HandlingMainEvents, Destroyed) => {
                self.call_event_handler(Event::AboutToWait);
                self.last_events_cleared.set(Instant::now());
                self.call_event_handler(Event::LoopExiting);
            },

            (Destroyed, _) => panic!("cannot move state from Destroyed"),
        }
    }

    fn call_new_events(&self, init: bool) {
        let start_cause = match (init, self.control_flow(), self.exit.get()) {
            (true, ..) => StartCause::Init,
            (false, ControlFlow::Poll, None) => StartCause::Poll,
            (false, _, Some(_)) | (false, ControlFlow::Wait, None) => StartCause::WaitCancelled {
                requested_resume: None,
                start: self.last_events_cleared.get(),
            },
            (false, ControlFlow::WaitUntil(requested_resume), None) => {
                if Instant::now() < requested_resume {
                    StartCause::WaitCancelled {
                        requested_resume: Some(requested_resume),
                        start: self.last_events_cleared.get(),
                    }
                } else {
                    StartCause::ResumeTimeReached {
                        requested_resume,
                        start: self.last_events_cleared.get(),
                    }
                }
            },
        };
        self.call_event_handler(Event::NewEvents(start_cause));
        // NB: For consistency all platforms must emit a 'resumed' event even though Windows
        // applications don't themselves have a formal suspend/resume lifecycle.
        if init {
            self.call_event_handler(Event::Resumed);
        }
        self.dispatch_buffered_events();
    }
}

impl<T> BufferedEvent<T> {
    pub fn from_event(event: Event<T>) -> BufferedEvent<T> {
        match event {
            Event::WindowEvent {
                event: WindowEvent::ScaleFactorChanged { scale_factor, inner_size_writer },
                window_id,
            } => BufferedEvent::ScaleFactorChanged(
                window_id,
                scale_factor,
                *inner_size_writer.new_inner_size.upgrade().unwrap().lock().unwrap(),
            ),
            event => BufferedEvent::Event(event),
        }
    }

    pub fn dispatch_event(self, dispatch: impl FnOnce(Event<T>)) {
        match self {
            Self::Event(event) => dispatch(event),
            Self::ImeResetCompletion(event, valid) => {
                if valid() {
                    dispatch(event);
                }
            },
            Self::ScaleFactorChanged(window_id, scale_factor, new_inner_size) => {
                let user_new_inner_size = Arc::new(Mutex::new(new_inner_size));
                dispatch(Event::WindowEvent {
                    window_id,
                    event: WindowEvent::ScaleFactorChanged {
                        scale_factor,
                        inner_size_writer: InnerSizeWriter::new(Arc::downgrade(
                            &user_new_inner_size,
                        )),
                    },
                });
                let inner_size = *user_new_inner_size.lock().unwrap();

                drop(user_new_inner_size);

                if inner_size != new_inner_size {
                    let window_flags = unsafe {
                        let userdata =
                            get_window_long(window_id.0.into(), GWL_USERDATA) as *mut WindowData;
                        (*userdata).window_state_lock().window_flags
                    };

                    window_flags.set_size((window_id.0).0, inner_size);
                }
            },
        }
    }
}

#[cfg(test)]
mod ime_reset_queue_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::EventLoopRunner;
    use crate::event::{Event, Ime, WindowEvent};

    #[test]
    fn reentrant_input_and_reset_fence_keep_fifo_order() {
        let runner = Rc::new(EventLoopRunner::new(0));
        let output = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&output);
        let nested = Rc::clone(&runner);
        let window_id = crate::window::WindowId::dummy();
        // The handler owns everything it captures and is cleared before runner teardown.
        unsafe {
            runner.set_event_handler(move |event: Event<()>| {
                if matches!(event, Event::UserEvent(())) {
                    for event in [
                        WindowEvent::Ime(Ime::Commit("accepted old".into())),
                        WindowEvent::Ime(Ime::Preedit(String::new(), None)),
                        WindowEvent::ImeResetCompleted { request_id: u64::MAX },
                        WindowEvent::Ime(Ime::Preedit("new".into(), Some((3, 3)))),
                    ] {
                        nested.send_event(Event::WindowEvent { window_id, event });
                    }
                    assert!(observed.borrow().is_empty(), "reset callback reentered app code");
                } else if let Event::WindowEvent { event, .. } = event {
                    observed.borrow_mut().push(event);
                }
            });
        }
        runner.send_event(Event::UserEvent(()));
        runner.clear_event_handler();
        assert!(runner.take_panic_error().is_ok());
        assert_eq!(*output.borrow(), vec![
            WindowEvent::Ime(Ime::Commit("accepted old".into())),
            WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            WindowEvent::ImeResetCompleted { request_id: u64::MAX },
            WindowEvent::Ime(Ime::Preedit("new".into(), Some((3, 3)))),
        ]);
    }
}

#[cfg(test)]
mod ime_native_batch_boundary_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::EventLoopRunner;
    use crate::event::{Event, Ime, WindowEvent};

    #[test]
    fn reset_from_native_clear_callback_cannot_overtake_copied_result_or_next_preedit() {
        for suffix in
            [None, Some(Ime::Preedit("old next".into(), Some((8, 8)))), Some(Ime::Disabled)]
        {
            let runner = Rc::new(EventLoopRunner::new(0));
            let output = Rc::new(RefCell::new(Vec::new()));
            let nested = Rc::clone(&runner);
            let observed = Rc::clone(&output);
            let window_id = crate::window::WindowId::dummy();
            let mut requested = false;
            unsafe {
                runner.set_event_handler(move |event: Event<()>| {
                    if let Event::WindowEvent { event, .. } = event {
                        let clear = matches!(&event, WindowEvent::Ime(Ime::Preedit(text, _)) if text.is_empty());
                        observed.borrow_mut().push(event);
                        if clear && !requested {
                            requested = true;
                            // This is where the real reset-message handler queues completion
                            // when invoked synchronously from this callback.
                            nested.send_event(Event::WindowEvent {
                                window_id, event: WindowEvent::ImeResetCompleted { request_id: 7 },
                            });
                        }
                    }
                });
            }
            let mut native = vec![
                WindowEvent::Ime(Ime::Preedit(String::new(), None)),
                WindowEvent::Ime(Ime::Commit("already copied old result".into())),
            ];
            if let Some(suffix) = suffix {
                native.push(WindowEvent::Ime(suffix));
            }
            // Both WM_IME_COMPOSITION and END use this production publication boundary.
            runner.publish_ime_batch(
                native.iter().cloned().map(|event| Event::WindowEvent { window_id, event }),
            );
            runner.clear_event_handler();
            assert!(runner.take_panic_error().is_ok());
            native.push(WindowEvent::ImeResetCompleted { request_id: 7 });
            assert_eq!(*output.borrow(), native);
        }
    }
}

#[cfg(test)]
mod ime_completion_cleanup_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::EventLoopRunner;
    use crate::event::{Event, Ime, WindowEvent};

    #[test]
    fn closing_from_clear_callback_filters_completion_but_keeps_accepted_result() {
        let runner = Rc::new(EventLoopRunner::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let output = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&output);
        let nested = Rc::clone(&runner);
        let closing = Arc::clone(&alive);
        let window_id = crate::window::WindowId::dummy();
        unsafe {
            runner.set_event_handler(move |event: Event<()>| {
                if let Event::WindowEvent { event, .. } = event {
                    let clear = matches!(&event, WindowEvent::Ime(Ime::Preedit(text, _)) if text.is_empty());
                    observed.borrow_mut().push(event);
                    if clear {
                        let valid = Arc::clone(&closing);
                        nested.send_ime_reset_completion(
                            Event::WindowEvent { window_id, event: WindowEvent::ImeResetCompleted { request_id: 7 } },
                            move || valid.load(Ordering::Acquire),
                        );
                        closing.store(false, Ordering::Release);
                    }
                }
            });
        }
        runner.publish_ime_batch([
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            },
            Event::WindowEvent { window_id, event: WindowEvent::Ime(Ime::Commit("copied".into())) },
        ]);
        runner.clear_event_handler();
        assert!(runner.take_panic_error().is_ok());
        assert_eq!(*output.borrow(), vec![
            WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            WindowEvent::Ime(Ime::Commit("copied".into())),
        ]);
        assert!(runner.event_buffer.borrow().is_empty());
    }

    #[test]
    fn panic_in_first_callback_does_not_leave_a_stuck_batch_or_run_completion() {
        let runner = Rc::new(EventLoopRunner::new(0));
        let nested = Rc::clone(&runner);
        let window_id = crate::window::WindowId::dummy();
        unsafe {
            runner.set_event_handler(move |_: Event<()>| {
                nested.send_ime_reset_completion(
                    Event::WindowEvent {
                        window_id,
                        event: WindowEvent::ImeResetCompleted { request_id: 7 },
                    },
                    || true,
                );
                panic!("app callback failed");
            });
        }
        runner.publish_ime_batch([
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            },
            Event::WindowEvent { window_id, event: WindowEvent::Ime(Ime::Commit("copied".into())) },
        ]);
        runner.clear_event_handler();
        assert!(runner.take_panic_error().is_err());
        assert!(runner.event_buffer.borrow().is_empty());
    }
}
