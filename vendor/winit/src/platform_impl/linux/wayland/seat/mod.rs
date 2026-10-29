//! Seat handling.

use std::sync::Arc;

use ahash::AHashMap;
use tracing::warn;

use sctk::reexports::client::backend::ObjectId;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_touch::WlTouch;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use sctk::reexports::protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::ZwpRelativePointerV1;
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::ZwpTextInputV3;

use sctk::data_device_manager::data_device::DataDevice;
use sctk::seat::pointer::{ThemeSpec, ThemedPointer};
use sctk::seat::{Capability as SeatCapability, SeatHandler, SeatState};

use crate::event::WindowEvent;
use crate::keyboard::ModifiersState;
use crate::platform_impl::wayland::state::WinitState;
use crate::platform_impl::wayland::window::state::TextInputLeft;
use crate::platform_impl::wayland::WindowId;

mod dnd;
mod keyboard;
mod pointer;
mod text_input;
mod touch;

pub use pointer::relative_pointer::RelativePointerState;
pub use pointer::{PointerConstraintsState, WinitPointerData, WinitPointerDataExt};
pub use text_input::{
    context_reset_needed, surrounding_text_excerpt, OutgoingCommit, OutgoingImeState,
    TextInputState, ZwpTextInputV3Ext,
};
pub(crate) use text_input::{reset_ime_with_fence, TextInputData};

use keyboard::{KeyboardData, KeyboardState};
use text_input::push_ime_teardown;
use touch::TouchPoint;

#[derive(Debug, Default)]
pub struct WinitSeatState {
    /// The pointer bound on the seat.
    pointer: Option<Arc<ThemedPointer<WinitPointerData>>>,

    /// The touch bound on the seat.
    touch: Option<WlTouch>,

    /// The mapping from touched points to the surfaces they're present.
    touch_map: AHashMap<i32, TouchPoint>,

    /// The text input bound on the seat.
    text_input: Option<Arc<ZwpTextInputV3>>,

    /// The relative pointer bound on the seat.
    relative_pointer: Option<ZwpRelativePointerV1>,

    /// The keyboard bound on the seat.
    keyboard_state: Option<KeyboardState>,

    /// The current modifiers state on the seat.
    modifiers: ModifiersState,

    /// Whether we have pending modifiers.
    modifiers_pending: bool,

    /// The data device bound on the seat, which receives file drags.
    data_device: Option<DataDevice>,

    /// The file drag over one of our windows.
    file_drag: Option<dnd::FileDrag>,
}

impl WinitSeatState {
    pub fn new(data_device: Option<DataDevice>) -> Self {
        Self { data_device, ..Default::default() }
    }
}

impl SeatHandler for WinitState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
        capability: SeatCapability,
    ) {
        let seat_state = match self.seats.get_mut(&seat.id()) {
            Some(seat_state) => seat_state,
            None => {
                warn!("Received wl_seat::new_capability for unknown seat");
                return;
            },
        };

        match capability {
            SeatCapability::Touch if seat_state.touch.is_none() => {
                seat_state.touch = self.seat_state.get_touch(queue_handle, &seat).ok();
            },
            SeatCapability::Keyboard if seat_state.keyboard_state.is_none() => {
                let keyboard = seat.get_keyboard(queue_handle, KeyboardData::new(seat.clone()));
                seat_state.keyboard_state =
                    Some(KeyboardState::new(keyboard, self.loop_handle.clone()));
            },
            SeatCapability::Pointer if seat_state.pointer.is_none() => {
                let surface = self.compositor_state.create_surface(queue_handle);
                let viewport = self
                    .viewporter_state
                    .as_ref()
                    .map(|state| state.get_viewport(&surface, queue_handle));
                let surface_id = surface.id();
                let pointer_data = WinitPointerData::new(seat.clone(), viewport);
                let themed_pointer = self
                    .seat_state
                    .get_pointer_with_theme_and_data(
                        queue_handle,
                        &seat,
                        self.shm.wl_shm(),
                        surface,
                        ThemeSpec::System,
                        pointer_data,
                    )
                    .expect("failed to create pointer with present capability.");

                seat_state.relative_pointer = self.relative_pointer.as_ref().map(|manager| {
                    manager.get_relative_pointer(
                        themed_pointer.pointer(),
                        queue_handle,
                        sctk::globals::GlobalData,
                    )
                });

                let themed_pointer = Arc::new(themed_pointer);

                // Register cursor surface.
                self.pointer_surfaces.insert(surface_id, themed_pointer.clone());

                seat_state.pointer = Some(themed_pointer);
            },
            _ => (),
        }

        if let Some(text_input_state) =
            seat_state.text_input.is_none().then_some(self.text_input_state.as_ref()).flatten()
        {
            seat_state.text_input = Some(Arc::new(text_input_state.get_text_input(
                &seat,
                queue_handle,
                TextInputData::default(),
            )));
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
        capability: SeatCapability,
    ) {
        let seat_state = match self.seats.get_mut(&seat.id()) {
            Some(seat_state) => seat_state,
            None => {
                warn!("Received wl_seat::remove_capability for unknown seat");
                return;
            },
        };

        // WordCraft patch: the text input belongs to the seat, not to a capability, so it is
        // destroyed in `remove_seat` instead (rust-windowing/winit#4747). Destroying it here
        // whenever any capability went away (a touch screen or tablet unplugged, a remote-desktop
        // session ending) left the window without IME until restart: `new_capability` only
        // recreates it when a capability is added.
        match capability {
            SeatCapability::Touch => {
                if let Some(touch) = seat_state.touch.take() {
                    if touch.version() >= 3 {
                        touch.release();
                    }
                }
            },
            SeatCapability::Pointer => {
                if let Some(relative_pointer) = seat_state.relative_pointer.take() {
                    relative_pointer.destroy();
                }

                if let Some(pointer) = seat_state.pointer.take() {
                    let pointer_data = pointer.pointer().winit_data();

                    // Remove the cursor from the mapping.
                    let surface_id = pointer.surface().id();
                    let _ = self.pointer_surfaces.remove(&surface_id);

                    // Remove the inner locks/confines before dropping the pointer.
                    pointer_data.unlock_pointer();
                    pointer_data.unconfine_pointer();

                    if pointer.pointer().version() >= 3 {
                        pointer.pointer().release();
                    }
                }
            },
            SeatCapability::Keyboard => {
                seat_state.keyboard_state = None;
                self.on_keyboard_destroy(&seat.id());
            },
            _ => (),
        }
    }

    fn new_seat(
        &mut self,
        _connection: &Connection,
        queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
    ) {
        let data_device = self
            .data_device_manager_state
            .as_ref()
            .map(|manager| manager.get_data_device(queue_handle, &seat));
        self.clipboard.add_seat(
            &seat,
            data_device.as_ref().map(|device| device.inner().clone()),
            queue_handle,
        );
        self.seats.insert(seat.id(), WinitSeatState::new(data_device));
    }

    fn remove_seat(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
    ) {
        // W15 (winit#4729): end a file drag in progress on this seat; W2 (winit#4747): the text
        // input is destroyed only here, when the seat itself goes away.
        self.cancel_file_drag(&seat.id());
        self.clipboard.remove_seat(&seat.id());
        let removed_text_input =
            self.seats.remove(&seat.id()).and_then(|mut seat| seat.text_input.take());
        if let Some(text_input) = removed_text_input {
            // Detach the proxy from every window that still listed it *before* sampling whether
            // it was active. A worker `set_ime_allowed` takes a window lock first, so removing it
            // under that lock prevents an `Enabled` from slipping in after we sampled. Sampling
            // afterwards also means a worker that won the lock earlier is observed as active and
            // gets the matching `Disabled`; an earlier sampling could leave Enabled without one.
            // Without this detach, `set_ime_allowed` keeps reporting success and sends requests
            // to a destroyed object.
            let mut teardown = Vec::new();
            for (window_id, window) in self.windows.get_mut() {
                let mut window = window.lock().unwrap();
                // The removed seat may own the window's visible preedit even when another seat
                // remains entered; that clear must still be emitted. A non-owner must not clear
                // a newer seat's composition.
                let clear_preedit = window.release_ime_preedit(&text_input);
                let left = window.text_input_left(&text_input);
                if left == TextInputLeft::Last {
                    // No other seat remains in this window, so this seat's leftover deletion
                    // belongs to no pending commit.
                    window.ime_delete_surrounding = None;
                }
                if clear_preedit || left == TextInputLeft::Last {
                    teardown.push((*window_id, left, clear_preedit));
                }
            }

            // The seat is going away. No window listing remains through which another thread can
            // re-enable it, so this sample is stable. Stop accepting incoming state and clear its
            // owner and composition before destroying the protocol object. A commit already
            // accepted from a `done` stays deliverable as committed input.
            let was_active = match text_input.data::<TextInputData>() {
                Some(data) => {
                    let was_active = data.is_active();
                    data.end_owner();
                    was_active
                },
                None => false,
            };

            let mut disabled_queued = false;
            for (window_id, left, clear_preedit) in teardown {
                let emit_disabled = left == TextInputLeft::Last && was_active && !disabled_queued;
                disabled_queued |= emit_disabled;
                push_ime_teardown(
                    &mut self.events_sink,
                    window_id,
                    left,
                    clear_preedit,
                    emit_disabled,
                );
            }

            text_input.destroy();
        }
        self.on_keyboard_destroy(&seat.id());
    }
}

impl WinitState {
    fn on_keyboard_destroy(&mut self, seat: &ObjectId) {
        // Capability destruction also ends the focus epoch even without a `wl_keyboard.leave`.
        // Clear the per-seat claim serial so the compatibility store path cannot use it later.
        self.clipboard.keyboard_focus_leave(seat);
        for (window_id, window) in self.windows.get_mut() {
            let mut window = window.lock().unwrap();
            let had_focus = window.has_focus();
            window.remove_seat_focus(seat);
            if had_focus != window.has_focus() {
                self.events_sink.push_window_event(WindowEvent::Focused(false), *window_id);
            }
        }
    }
}

/// Drops text-input state owned by a window that is being destroyed.
///
/// Unlike a lifecycle boundary, destroying the window discards its queued
/// `Enabled`/`Preedit`/`Commit` events too, together with any deletion attached to a commit.
pub(crate) fn clear_ime_batch_for_window(
    seats: &AHashMap<ObjectId, WinitSeatState>,
    window_id: WindowId,
) {
    for seat in seats.values() {
        if let Some(text_input) = &seat.text_input {
            if let Some(data) = text_input.data::<TextInputData>() {
                data.clear_if_owner(window_id);
            }
        }
    }
}

sctk::delegate_seat!(WinitState);
