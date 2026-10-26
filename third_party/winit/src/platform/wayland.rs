//! # Wayland
//!
//! **Note:** Windows don't appear on Wayland until you draw/present to them.
//!
//! By default, Winit loads system libraries using `dlopen`. This can be
//! disabled by disabling the `"wayland-dlopen"` cargo feature.
//!
//! ## Client-side decorations
//!
//! Winit provides client-side decorations by default, but the behaviour can
//! be controlled with the following feature flags:
//!
//! * `wayland-csd-adwaita` (default).
//! * `wayland-csd-adwaita-crossfont`.
//! * `wayland-csd-adwaita-notitle`.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Weak};
use std::{fmt, io};

use crate::event_loop::{ActiveEventLoop, EventLoop, EventLoopBuilder};
use crate::monitor::MonitorHandle;
use crate::window::{Window, WindowAttributes, WindowId};

pub use crate::window::Theme;

pub use crate::platform_impl::wayland::clipboard::{
    ClipboardContent, ClipboardError, ImageClaimReservation, InputOriginToken, PngBytes,
    PngBytesWriter, RegularTransferHandle, SelectionClaim, TransferHandle, TransferOptions,
    TransferWaker, IMAGE_PNG_MIME, MAX_TOTAL_DEADLINE, MAX_TRANSFER_BYTES,
};

/// Additional methods on [`ActiveEventLoop`] that are specific to Wayland.
pub trait ActiveEventLoopExtWayland {
    /// True if the [`ActiveEventLoop`] uses Wayland.
    fn is_wayland(&self) -> bool;

    /// The clipboard of this event loop, or [`None`] if it doesn't use Wayland.
    fn clipboard(&self) -> Option<Clipboard>;
}

impl ActiveEventLoopExtWayland for ActiveEventLoop {
    #[inline]
    fn is_wayland(&self) -> bool {
        self.p.is_wayland()
    }

    fn clipboard(&self) -> Option<Clipboard> {
        #[allow(clippy::single_match)]
        match &self.p {
            crate::platform_impl::ActiveEventLoop::Wayland(event_loop) => {
                Some(Clipboard { state: Arc::downgrade(&event_loop.state.borrow().clipboard) })
            },
            #[cfg(x11_platform)]
            _ => None,
        }
    }
}

/// The clipboard of a Wayland event loop.
///
/// The regular selection goes through the `wl_data_device` winit binds for each seat to receive
/// file drops, so a toolkit should use it instead of binding a data device of its own: some
/// compositors, such as Hyprland, send the selection and drags to only one data device of a
/// client. PRIMARY uses its own `zwp_primary_selection_device_v1` per seat and does not require a
/// regular data device manager.
///
/// A handle may be cloned and shared between threads. The bounded reads (`load_*`,
/// `begin_load_*`) and dropping their handles may run off the event-loop thread: they use bounded
/// workers and report through a caller-owned [`TransferHandle`], and a peer that trickles bytes
/// cannot extend the total deadline. Reading our own selection returns it directly and also
/// obeys the requested byte cap. Deadlines above 60 seconds and byte caps above 16 MiB (or zero)
/// are rejected with `InvalidInput`, and converted text is capped after lossy UTF-8/CRLF
/// normalization. A clipboard outliving its event loop reports errors.
///
/// Selection ownership calls (`store_text`, `store_primary_text`, `claim_text`,
/// `claim_primary_text`, `claim_reserved_image_png`) and `invalidate_image_reservations` follow
/// the event loop's discipline: invoke them on the thread that runs that Wayland event loop, from
/// its input/output processing, because they enqueue protocol requests or flush the connection.
/// `input_origin` and `reserve_image_claim` are synchronous state transitions used at that same
/// input/output boundary. This is a caller contract; the type does not inspect which thread is
/// currently calling it.
#[derive(Clone)]
pub struct Clipboard {
    state: Weak<crate::platform_impl::wayland::ClipboardState>,
}

impl Clipboard {
    /// The clipboard of the winit event loop connected through `display`, a `wl_display`
    /// pointer such as the one in its `WaylandDisplayHandle`, if that event loop still exists.
    pub fn for_display(display: NonNull<c_void>) -> Option<Self> {
        crate::platform_impl::wayland::ClipboardState::for_display(display.as_ptr())
            .map(|state| Self { state: Arc::downgrade(&state) })
    }

    /// The text on the clipboard of the seat the application last received input from.
    ///
    /// This compatibility wrapper is bounded but blocks the calling thread. Use
    /// [`Self::begin_load_text`] from a user-interface event path.
    pub fn load_text(&self) -> io::Result<String> {
        self.state()?.load_text()
    }

    /// Puts `text` on the clipboard of the seat the application last received input from.
    ///
    /// This compatibility wrapper has no trigger token. New code should obtain a token with
    /// [`Self::input_origin`] and call [`Self::claim_text`] in the same input turn. Call it on the
    /// event-loop thread; it creates a protocol source and flushes the connection.
    pub fn store_text(&self, text: impl Into<String>) -> io::Result<()> {
        self.state()?.store_text(text.into())
    }

    /// Returns a fresh trigger token for the active input seat focused on `target_window`.
    ///
    /// Call this while handling the keyboard or pointer input that asks for the copy. The token
    /// records the seat, window, focus epoch, triggering serial and input origin. It is
    /// single-use per selection family: the same input may set the regular clipboard and PRIMARY
    /// once each, but a second claim in either family returns `InvalidInput`. If no fresh
    /// claimable input remains on the active seat, or that seat no longer focuses
    /// `target_window`, this returns `InvalidInput` rather than choosing another focused seat by
    /// insertion order. Call it on the event-loop thread from the input turn whose serial it
    /// should carry.
    pub fn input_origin(&self, target_window: WindowId) -> io::Result<InputOriginToken> {
        self.state()?.input_origin(target_window)
    }

    /// Asynchronously reads only text from the regular clipboard of the active input seat
    /// focused on `target_window`.
    ///
    /// An absent offer is `NotFound`, a non-text (including image-only) offer is `InvalidData`,
    /// and a present zero-byte text stream is a successful empty string. Our own selection returns
    /// immediately. Invalid limits are `InvalidInput`; an offer invalidated during receive is
    /// `Interrupted`, never a successful empty string. Use [`Self::begin_load_regular`] for a
    /// text-or-image read from the same single offer.
    pub fn begin_load_text(&self, options: TransferOptions) -> io::Result<TransferHandle> {
        self.state()?.begin_load_text(options)
    }

    /// Asynchronously reads the regular clipboard as text or exact `image/png`.
    ///
    /// Exactly one active-input-seat offer is resolved at admission. Text MIME types are preferred
    /// in their existing order; only when none is offered is [`IMAGE_PNG_MIME`] accepted. A text
    /// transfer never falls back to an image after malformed bytes, timeout or another failure.
    ///
    /// The returned handle freezes the seat, focus epoch, active-input-seat epoch, window and
    /// offer/source identity. A result completed before the owner, focus, seat or window changes
    /// is revoked and [`ClipboardError::Interrupted`] is returned instead. Moving input to
    /// another seat and back (A-B-A) also invalidates a focus-bound read. The transfer deadline
    /// covers admission, queue wait, pipe I/O and text conversion. PNG bytes are immutable, carry
    /// their image-budget permit, and remain charged until the last clone is dropped. Use
    /// [`Self::reserve_image_claim`] / [`Self::claim_reserved_image_png`] for an asynchronous copy
    /// whose encoding finishes after the triggering input turn.
    ///
    /// An ended event loop returns [`ClipboardError::Disconnected`]. The caller's generation from
    /// [`TransferOptions`] is retained on the handle and available through
    /// [`RegularTransferHandle::caller_generation`].
    pub fn begin_load_regular(
        &self,
        options: TransferOptions,
    ) -> Result<RegularTransferHandle, ClipboardError> {
        // The typed API distinguishes an ended event loop from a generic backend failure. Only
        // `NotConnected` is produced here for a dead event loop; keep the legacy `begin_load_text`
        // mapping untouched.
        let state = self.state().map_err(|error| match error.kind() {
            io::ErrorKind::NotConnected => ClipboardError::Disconnected,
            _ => ClipboardError::Backend(error),
        })?;
        state.begin_load_regular(options)
    }

    /// Synchronously claims the regular clipboard on the triggering input serial.
    ///
    /// Call it on the event-loop thread: it creates a protocol source, sends `set_selection` and
    /// flushes the connection synchronously.
    ///
    /// Validation, per-family token consumption, byte reservation, source creation and
    /// `set_selection` happen before this returns; only later pipe serving uses bounded workers.
    /// A pending own text is returned by a self-paste until the compositor echoes it, for at
    /// most one second; an authoritative selection event for another owner ends it sooner.
    pub fn claim_text(&self, text: Arc<str>, claim: SelectionClaim) -> io::Result<()> {
        self.state()?.claim_text(text, claim)
    }

    /// Reserves the regular selection for a deferred PNG copy from the triggering input.
    ///
    /// This consumes the regular-family trigger now and records its exact serial, seat, window,
    /// focus epoch, active-seat epoch and a per-seat regular ownership epoch. Encoding may run on
    /// a worker; installation must call [`Self::claim_reserved_image_png`] from the event loop.
    /// Later pointer/keyboard input and pointer release do not replace the reserved serial.
    ///
    /// A newer regular text claim, compatibility text store or image reservation invalidates this
    /// reservation even if that newer attempt fails. Focus/window/seat loss, expiry, explicit
    /// [`Self::invalidate_image_reservations`] and dropping the returned value also reject it.
    /// PRIMARY claims never invalidate it. The reservation is non-cloneable. Call this at the
    /// event-loop output boundary that owns the triggering input turn.
    pub fn reserve_image_claim(
        &self,
        claim: SelectionClaim,
    ) -> Result<ImageClaimReservation, ClipboardError> {
        self.typed_state()?.reserve_image_claim(claim)
    }

    /// Installs a reserved PNG as the regular selection with its original triggering serial.
    ///
    /// Validation and `set_selection` run synchronously on the calling thread, which must be the
    /// event-loop thread. A stale, expired or superseded reservation returns an error and leaves
    /// the current selection and its serving source unchanged. The source advertises exactly
    /// [`IMAGE_PNG_MIME`] plus the opaque winit marker. The bytes keep their image-budget permit
    /// while installed and while any bounded serving write still references them.
    pub fn claim_reserved_image_png(
        &self,
        png: PngBytes,
        reservation: ImageClaimReservation,
    ) -> Result<(), ClipboardError> {
        self.typed_state()?.claim_reserved_image_png(png, reservation)
    }

    /// Invalidates pending regular image reservations for `target_window`.
    ///
    /// It resolves the active seat/window but does not require a claimable input serial. Only that
    /// seat's regular ownership/reservation epoch advances; the installed source and PRIMARY are
    /// untouched. Call this on the event-loop thread for a winning regular output before resolving
    /// its fresh origin.
    pub fn invalidate_image_reservations(
        &self,
        target_window: WindowId,
    ) -> Result<(), ClipboardError> {
        self.typed_state()?.invalidate_image_reservations(target_window)
    }

    /// The text on the PRIMARY selection of the seat the application last received input from.
    ///
    /// This compatibility wrapper is bounded but blocks the calling thread. Use
    /// [`Self::begin_load_primary_text`] from a user-interface event path.
    pub fn load_primary_text(&self) -> io::Result<String> {
        self.state()?.load_primary_text()
    }

    /// Puts `text` on PRIMARY through the compatibility API without a trigger token.
    ///
    /// New code should use [`Self::claim_primary_text`] with a token from
    /// [`Self::input_origin`]. Call it on the event-loop thread; it creates a protocol source and
    /// flushes the connection.
    pub fn store_primary_text(&self, text: impl Into<String>) -> io::Result<()> {
        self.state()?.store_primary_text(text.into())
    }

    /// Asynchronously reads the PRIMARY selection of the active input seat focused on
    /// `target_window`.
    ///
    /// PRIMARY uses a distinct `zwp_primary_selection_device_v1` per seat. An absent global is
    /// `Unsupported`, an absent offer is `NotFound`, and non-text is `InvalidData`. It shares the
    /// same hard deadline/byte limits, checked self-paste cap, and invalidation semantics as the
    /// regular clipboard.
    pub fn begin_load_primary_text(&self, options: TransferOptions) -> io::Result<TransferHandle> {
        self.state()?.begin_load_primary_text(options)
    }

    /// Synchronously claims PRIMARY on the triggering keyboard or pointer input serial.
    ///
    /// Call it on the event-loop thread: it creates a protocol source, sends `set_selection` and
    /// flushes the connection synchronously.
    pub fn claim_primary_text(&self, text: Arc<str>, claim: SelectionClaim) -> io::Result<()> {
        self.state()?.claim_primary_text(text, claim)
    }

    fn state(&self) -> io::Result<Arc<crate::platform_impl::wayland::ClipboardState>> {
        self.state
            .upgrade()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "the event loop has ended"))
    }

    fn typed_state(
        &self,
    ) -> Result<Arc<crate::platform_impl::wayland::ClipboardState>, ClipboardError> {
        self.state().map_err(|error| match error.kind() {
            io::ErrorKind::NotConnected => ClipboardError::Disconnected,
            _ => ClipboardError::Backend(error),
        })
    }
}

impl fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Clipboard").finish_non_exhaustive()
    }
}

/// Additional methods on [`EventLoop`] that are specific to Wayland.
pub trait EventLoopExtWayland {
    /// True if the [`EventLoop`] uses Wayland.
    fn is_wayland(&self) -> bool;
}

impl<T: 'static> EventLoopExtWayland for EventLoop<T> {
    #[inline]
    fn is_wayland(&self) -> bool {
        self.event_loop.is_wayland()
    }
}

/// Additional methods on [`EventLoopBuilder`] that are specific to Wayland.
pub trait EventLoopBuilderExtWayland {
    /// Force using Wayland.
    fn with_wayland(&mut self) -> &mut Self;

    /// Whether to allow the event loop to be created off of the main thread.
    ///
    /// By default, the window is only allowed to be created on the main
    /// thread, to make platform compatibility easier.
    fn with_any_thread(&mut self, any_thread: bool) -> &mut Self;
}

impl<T> EventLoopBuilderExtWayland for EventLoopBuilder<T> {
    #[inline]
    fn with_wayland(&mut self) -> &mut Self {
        self.platform_specific.forced_backend = Some(crate::platform_impl::Backend::Wayland);
        self
    }

    #[inline]
    fn with_any_thread(&mut self, any_thread: bool) -> &mut Self {
        self.platform_specific.any_thread = any_thread;
        self
    }
}

/// Why the client changed the surrounding text, cursor or anchor.
///
/// text-input-v3 starts every commit with an implicit [`InputMethod`](Self::InputMethod) cause.
/// An application-side edit (keyboard typing, model changes, selection movement) must be reported
/// as [`Other`](Self::Other) so the input method can update or remove a stale preedit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImeTextChangeCause {
    /// The change was caused by applying text or a deletion from the input method.
    #[default]
    InputMethod,
    /// Something other than the input method caused the change.
    Other,
}

/// Text an input method asks to delete around the cursor, in bytes of UTF-8 text.
///
/// The counts exclude the selection and the preedit: `before_bytes` ends where the preedit or
/// selection starts and `after_bytes` starts where it ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ImeDeleteSurrounding {
    /// Bytes to delete before the cursor.
    pub before_bytes: usize,
    /// Bytes to delete after the cursor.
    pub after_bytes: usize,
}

/// Additional methods on [`Window`] that are specific to Wayland.
///
/// This fork adds required surrounding-text methods without default no-op implementations. That
/// is a source-breaking change for any external `WindowExtWayland` implementor: the API carries
/// the E21b outgoing-context contract, and a silent no-op would make an unsupported window look
/// like an IME context that had published text. Implementors must forward
/// [`Self::set_ime_surrounding_text_with_change_cause`] and
/// [`Self::clear_ime_surrounding_text`] (X11 may ignore both).
///
/// [`Window`]: crate::window::Window
pub trait WindowExtWayland {
    /// Returns `xdg_toplevel` of the window or [`None`] if the window is X11 window.
    fn xdg_toplevel(&self) -> Option<NonNull<c_void>>;
    /// Tells the input method the text around the cursor of the focused text field, so it can
    /// base suggestions and corrections on it (text-input-v3 surrounding text).
    ///
    /// `cursor` and `anchor` are byte offsets into `text`, the other end of the selection being
    /// `anchor`; without a selection they are equal. Leave the preedit out of `text`. Offsets
    /// are clamped to the text and moved back to a character boundary, and text longer than
    /// 4000 bytes is cut around the cursor and selection. Call it again whenever the text, cursor
    /// or selection changes; a call with the same text, cursor and anchor as the last one sends
    /// nothing. Disallowing IME forgets the text. X11 ignores it.
    ///
    /// This is the application-originated setter: it reports [`ImeTextChangeCause::Other`] with
    /// the next commit. Use [`Self::set_ime_surrounding_text_with_change_cause`] when the change
    /// was caused by applying an IME commit or deletion.
    ///
    /// An empty document is represented internally as an available snapshot and is sent with an
    /// empty `set_surrounding_text` request; [`Self::clear_ime_surrounding_text`] omits that
    /// request after a committed reset. text-input-v3 has no separate availability bit: the
    /// protocol specifies empty initial values as "unsupported", and warns that if empty values
    /// are applied, later attempts to change them may have no effect. The available-empty
    /// distinction is therefore internal and best-effort on the wire, not a guarantee that a
    /// compositor will retain it. A native compositor trace is required before relying on it.
    ///
    /// Calls while the window's [`ImePurpose`] is [`ImePurpose::Password`] are ignored.
    ///
    /// [`ImePurpose`]: crate::window::ImePurpose
    /// [`ImePurpose::Password`]: crate::window::ImePurpose::Password
    fn set_ime_surrounding_text(&self, text: &str, cursor: usize, anchor: usize) {
        self.set_ime_surrounding_text_with_change_cause(
            text,
            cursor,
            anchor,
            ImeTextChangeCause::Other,
        );
    }

    /// Like [`Self::set_ime_surrounding_text`], but also carries the reason for the change.
    ///
    /// Pass [`ImeTextChangeCause::InputMethod`] when publishing text produced by applying the
    /// input method's commit/deletion; pass [`ImeTextChangeCause::Other`] for application edits.
    /// text-input-v3 resets the cause to `input_method` after each commit. X11 ignores it.
    fn set_ime_surrounding_text_with_change_cause(
        &self,
        text: &str,
        cursor: usize,
        anchor: usize,
        cause: ImeTextChangeCause,
    );

    /// Forgets the surrounding text and makes the IME context unavailable.
    ///
    /// Use this when the IME context changes to something that must not inherit the old text
    /// (another editor, a password field, focus loss) or when the editor is unavailable. Winit
    /// forgets the cached text and, if text was published or an incoming generation is active or
    /// buffered, sends a committed disable/enable reset so the input method cancels the old
    /// context. It preserves whether IME is allowed and never enables a window whose IME is
    /// disabled. Call
    /// [`Self::set_ime_surrounding_text`] afterwards for the new context; an available empty
    /// document is that call with `""` rather than this one.
    ///
    /// X11 ignores it.
    fn clear_ime_surrounding_text(&self);

    /// The surrounding text to delete before inserting the [`Ime::Commit`] being handled.
    ///
    /// Call this when handling each `Ime::Commit`, after removing the preedit and before
    /// inserting the committed text, which may be empty when an input method only deletes.
    /// Returns `None` when the commit deletes nothing, and always on X11.
    ///
    /// [`Ime::Commit`]: crate::event::Ime::Commit
    fn take_ime_delete_surrounding(&self) -> Option<ImeDeleteSurrounding>;
}

impl WindowExtWayland for Window {
    fn set_ime_surrounding_text_with_change_cause(
        &self,
        text: &str,
        cursor: usize,
        anchor: usize,
        cause: ImeTextChangeCause,
    ) {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => {},
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => {
                window.set_ime_surrounding_text_with_change_cause(text, cursor, anchor, cause)
            },
        }
    }

    fn clear_ime_surrounding_text(&self) {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => {},
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.clear_ime_surrounding_text(),
        }
    }
    fn take_ime_delete_surrounding(&self) -> Option<ImeDeleteSurrounding> {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => None,
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.take_ime_delete_surrounding(),
        }
    }
    #[inline]
    fn xdg_toplevel(&self) -> Option<NonNull<c_void>> {
        #[allow(clippy::single_match)]
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => None,
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.xdg_toplevel(),
        }
    }
}

/// Additional methods on [`WindowAttributes`] that are specific to Wayland.
pub trait WindowAttributesExtWayland {
    /// Build window with the given name.
    ///
    /// The `general` name sets an application ID, which should match the `.desktop`
    /// file distributed with your program. The `instance` is a `no-op`.
    ///
    /// For details about application ID conventions, see the
    /// [Desktop Entry Spec](https://specifications.freedesktop.org/desktop-entry-spec/desktop-entry-spec-latest.html#desktop-file-id)
    fn with_name(self, general: impl Into<String>, instance: impl Into<String>) -> Self;
}

impl WindowAttributesExtWayland for WindowAttributes {
    #[inline]
    fn with_name(mut self, general: impl Into<String>, instance: impl Into<String>) -> Self {
        self.platform_specific.name =
            Some(crate::platform_impl::ApplicationName::new(general.into(), instance.into()));
        self
    }
}

/// Additional methods on `MonitorHandle` that are specific to Wayland.
pub trait MonitorHandleExtWayland {
    /// Returns the inner identifier of the monitor.
    fn native_id(&self) -> u32;
}

impl MonitorHandleExtWayland for MonitorHandle {
    #[inline]
    fn native_id(&self) -> u32 {
        self.inner.native_identifier()
    }
}
