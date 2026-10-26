//! Copy and paste through the seat's `wl_data_device` and `zwp_primary_selection_device_v1`.
//!
//! A client should keep one regular data device per seat. Some compositors, Hyprland among them,
//! send the selection and drags to only one data device of a client, so a toolkit that binds a
//! second device for its clipboard stops receiving the selection once winit binds one for drops.
//! Winit therefore offers the regular clipboard on its own device. PRIMARY is a different protocol
//! object on the same connection and queue; it is never another `wl_data_device`.
//!
//! Reads and source-serving writes never run on the event-loop thread. A caller owns a
//! [`TransferHandle`] for a read and receives exactly one completion wake; dropping the handle
//! cancels the operation and prevents a later result or wake.
//!
//! The selection ownership state is protocol free. [`Inner`] holds one [`SeatFamily`] per seat and
//! selection family, and the SCTK objects live beside it. Tests drive the same transitions, claim
//! admission, window/seat teardown and worker queues with detached protocol handles.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, ErrorKind};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use rustix::event::{poll, PollFd, PollFlags};
use rustix::fs::{fcntl_getfl, fcntl_setfl, OFlags};
use rustix::pipe::{pipe_with, PipeFlags};
use tracing::warn;

use sctk::data_device_manager::data_device::DataDeviceData;
use sctk::data_device_manager::data_offer::SelectionOffer;
use sctk::data_device_manager::data_source::DataSourceData;
use sctk::data_device_manager::WritePipe;
use sctk::primary_selection::device::{
    PrimarySelectionDevice, PrimarySelectionDeviceData, PrimarySelectionDeviceHandler,
};
use sctk::globals::GlobalData;
use sctk::primary_selection::offer::{PrimarySelectionOffer, PrimarySelectionOfferData};
use sctk::primary_selection::selection::{PrimarySelectionSource, PrimarySelectionSourceHandler};
use sctk::primary_selection::PrimarySelectionManagerState;
use sctk::reexports::client::backend::{ObjectData, ObjectId};
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::WlDataDeviceManager;
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::backend::WaylandError;
use sctk::reexports::client::protocol::wl_data_offer;
use sctk::reexports::client::{delegate_dispatch, Connection, Dispatch, Proxy, QueueHandle};
use sctk::reexports::protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_manager_v1::{
    ZwpPrimarySelectionDeviceManagerV1,
};
use sctk::reexports::protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_v1::{
    Event as PrimarySelectionDeviceEvent, ZwpPrimarySelectionDeviceV1,
};
use sctk::reexports::protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1::{
    self as primary_offer_v1, ZwpPrimarySelectionOfferV1,
};
use sctk::reexports::protocols::wp::primary_selection::zv1::client::zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1;

use crate::platform_impl::wayland::state::WinitState;
use crate::window::WindowId;

/// The text types winit offers, and accepts in this order of preference.
const TEXT_MIME_TYPES: [&str; 5] =
    ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"];

/// Prefix of the opaque marker mime type that identifies one winit source.
///
/// The rest of the marker is a process nonce, a per-clipboard nonce and a per-source counter. It
/// deliberately contains no PID, display handle, address or other host identifier; another client
/// can see it, so it must not reveal anything beyond "this source came from some winit process".
const MARKER_PREFIX: &str = "application/x-winit-selection;v=1;";

/// How long a claim stays pending before an unconfirmed selection event becomes authoritative.
///
/// Wayland echoes `set_selection` on the next round trip. A claim must tolerate the selection
/// event that was already in flight when it was sent, but it must not keep serving its text
/// forever if the compositor never echoes it.
const CLAIM_ECHO_GRACE: Duration = Duration::from_secs(1);

/// Default total deadline for one read or serving write.
pub const DEFAULT_TOTAL_DEADLINE: Duration = Duration::from_secs(2);
/// Hard upper bound for a caller-supplied transfer deadline (60 seconds).
pub const MAX_TOTAL_DEADLINE: Duration = Duration::from_secs(60);
/// Hard per-transfer byte cap (16 MiB).
pub const MAX_TRANSFER_BYTES: usize = 16 * 1024 * 1024;
/// Default per-transfer byte cap. This accepts the 10 MiB suite fixture.
pub const DEFAULT_MAX_TRANSFER_BYTES: usize = MAX_TRANSFER_BYTES;
/// Maximum bytes in one selection we own.
const MAX_SOURCE_BYTES: usize = MAX_TRANSFER_BYTES;
/// Hard admission cap for payloads retained by the clipboard's owned-source ledger.
///
/// Every installed source, every replaced source held for rollback until its `set_selection`
/// flush completes, and every serving-writer clone of those payloads stays charged. A new source
/// must fit beside all of them; a replacement never discounts the source it is replacing. This is
/// not a process-RSS bound and excludes read results already handed to a caller (including
/// self-paste clones); it charges payload length, not allocation capacity. PNG capacity is
/// bounded separately by [`IMAGE_BUDGET_BYTES`].
const MAX_TOTAL_OWNED_BYTES: usize = 32 * 1024 * 1024;
/// How long an input serial remains a valid ownership trigger.
const INPUT_ORIGIN_MAX_AGE: Duration = Duration::from_secs(5);

const MAX_ACTIVE_READS: usize = 4;
const MAX_QUEUED_READS: usize = 8;
const MAX_ACTIVE_WRITES: usize = 2;
const MAX_QUEUED_WRITES: usize = 8;
const WORKER_POLL_SLICE: Duration = Duration::from_millis(50);

/// The exact MIME type a combined regular read accepts for an image.
///
/// It is deliberately the single PNG MIME type, not a generic image type. Image reads only select
/// `image/png`; the decoder in the toolkit verifies the payload.
pub const IMAGE_PNG_MIME: &str = "image/png";

/// Process-global budget for encoded PNG bytes retained by winit, including queued reads.
///
/// A full-cap image read reserves `2 * 16 MiB` at admission, so at most four full-cap reads
/// (active, queued or completed-but-unconsumed) fit at once. Queue slots never bypass this byte
/// budget. Text reads do not consume it. This is an admission bound on retained PNG capacity,
/// separate from the source-byte ledger, not a hard process-RSS ceiling.
const IMAGE_BUDGET_BYTES: usize = 128 * 1024 * 1024;

/// Why a regular or PRIMARY read did not produce content.
///
/// [`Self::io_kind`] is the compatibility mapping for the older `io::Result` APIs. It preserves
/// the kinds those APIs returned: absence was `NotFound`, a non-text offer was `InvalidData`, a
/// full source-byte or worker budget was `WouldBlock`, and backend failures kept their kind.
#[derive(Debug)]
pub enum ClipboardError {
    /// There is no offer on the selected seat and family.
    Absent,
    /// The offer exists but has neither text nor (for a combined regular read) exact `image/png`.
    NotOffered,
    /// The offer carries a marker from an older source of this clipboard.
    StaleSelfSource,
    /// The owner, seat, focus, window or offer changed during the transfer.
    Interrupted,
    /// The total transfer deadline expired, including queue wait and conversion.
    TimedOut,
    /// The protocol family, data device or PRIMARY manager is unavailable.
    Unsupported,
    /// A worker queue or the image byte budget is full.
    Busy,
    /// The payload exceeds the transfer or source byte cap.
    TooLarge,
    /// The caller supplied an invalid limit, target or window.
    InvalidInput,
    /// The event loop or clipboard connection has ended.
    Disconnected,
    /// A backend failure, preserving its original `io::ErrorKind` and message.
    Backend(io::Error),
}

impl ClipboardError {
    /// Classifies an I/O error without discarding its original kind.
    pub fn from_io(error: io::Error) -> Self {
        match error.kind() {
            ErrorKind::NotFound => Self::Absent,
            ErrorKind::Interrupted => Self::Interrupted,
            ErrorKind::TimedOut => Self::TimedOut,
            ErrorKind::Unsupported => Self::Unsupported,
            ErrorKind::WouldBlock => Self::Busy,
            ErrorKind::InvalidInput => Self::InvalidInput,
            ErrorKind::BrokenPipe => Self::Disconnected,
            // `NotConnected` and every other kind stay exact as backend errors; the old text
            // worker-shutdown path returned `NotConnected` and must not become `BrokenPipe`.
            _ => Self::Backend(error),
        }
    }

    /// The `io::ErrorKind` the legacy text/PRIMARY APIs use for this failure.
    pub fn io_kind(&self) -> ErrorKind {
        match self {
            Self::Absent => ErrorKind::NotFound,
            Self::NotOffered | Self::StaleSelfSource | Self::TooLarge => ErrorKind::InvalidData,
            Self::Interrupted => ErrorKind::Interrupted,
            Self::TimedOut => ErrorKind::TimedOut,
            Self::Unsupported => ErrorKind::Unsupported,
            Self::Busy => ErrorKind::WouldBlock,
            Self::InvalidInput => ErrorKind::InvalidInput,
            Self::Disconnected => ErrorKind::BrokenPipe,
            Self::Backend(error) => error.kind(),
        }
    }

    /// Converts to the compatibility I/O error. A backend error keeps its message and kind.
    pub fn into_io_error(self) -> io::Error {
        match self {
            Self::Backend(error) => error,
            other => {
                let kind = other.io_kind();
                io::Error::new(kind, other)
            },
        }
    }
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("the selection is empty"),
            Self::NotOffered => f.write_str("the selection does not offer the requested type"),
            Self::StaleSelfSource => {
                f.write_str("the selection belongs to a stale winit source from this clipboard")
            },
            Self::Interrupted => f.write_str("the clipboard transfer was invalidated"),
            Self::TimedOut => f.write_str("the clipboard transfer deadline expired"),
            Self::Unsupported => f.write_str("the selection protocol is unavailable"),
            Self::Busy => f.write_str("the clipboard transfer budget is full"),
            Self::TooLarge => f.write_str("the clipboard payload exceeds its byte cap"),
            Self::InvalidInput => f.write_str("the clipboard request is invalid"),
            Self::Disconnected => f.write_str("the event loop has ended"),
            Self::Backend(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ClipboardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ClipboardError> for io::Error {
    fn from(error: ClipboardError) -> Self {
        error.into_io_error()
    }
}

/// One process-global admission budget for retained PNG bytes.
#[derive(Debug)]
struct ImageBudget {
    used: Mutex<usize>,
    limit: usize,
}

impl ImageBudget {
    const fn new(limit: usize) -> Self {
        Self { used: Mutex::new(0), limit }
    }

    fn try_acquire(self: &Arc<Self>, bytes: usize) -> Option<ImagePermit> {
        let mut used = self.used.lock().unwrap();
        let next = used.checked_add(bytes)?;
        if next > self.limit {
            return None;
        }
        *used = next;
        Some(ImagePermit { _inner: Arc::new(ImagePermitInner { budget: Arc::clone(self), bytes }) })
    }

    #[cfg(test)]
    fn used(&self) -> usize {
        *self.used.lock().unwrap()
    }
}

/// A cloned permit shares one reservation; the allocation is released exactly once.
#[derive(Clone, Debug)]
struct ImagePermit {
    _inner: Arc<ImagePermitInner>,
}

#[derive(Debug)]
struct ImagePermitInner {
    budget: Arc<ImageBudget>,
    bytes: usize,
}

impl Drop for ImagePermitInner {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock().unwrap();
        *used = used.saturating_sub(self.bytes);
    }
}

impl ImagePermit {
    fn acquire_global(bytes: usize) -> Result<Self, ClipboardError> {
        image_budget().try_acquire(bytes).ok_or(ClipboardError::Busy)
    }
}

fn image_budget() -> &'static Arc<ImageBudget> {
    static BUDGET: OnceLock<Arc<ImageBudget>> = OnceLock::new();
    BUDGET.get_or_init(|| Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES)))
}

/// Accounting for every payload allocation retained by the clipboard as an owned source.
///
/// A charge is shared by the installed source, a replaced source held for rollback, and every
/// source-serving writer that clones the payload. Cloning a payload therefore does not
/// double-count it; the bytes are released only when the last reference to that particular
/// allocation is gone. Replacement admission does not discount the old source while it is held.
///
/// This ledger excludes read results already handed to a caller (including self-paste clones),
/// charges payload length rather than allocation capacity, and is not an RSS bound. Encoded PNG
/// capacity is covered by the separate [`IMAGE_BUDGET_BYTES`] permit.
#[derive(Debug)]
struct SourceBytesBudget {
    used: Mutex<usize>,
    limit: usize,
}

impl SourceBytesBudget {
    const fn new(limit: usize) -> Self {
        Self { used: Mutex::new(0), limit }
    }

    #[cfg(test)]
    fn used(&self) -> usize {
        *self.used.lock().unwrap()
    }
}

/// One lifetime charge for a distinct retained source payload allocation.
///
/// Every clone of the same payload holds a clone of this value. The last one to drop subtracts
/// its bytes from [`SourceBytesBudget`] exactly once. The charge contains no payload and does not
/// acquire the clipboard lock on drop.
#[derive(Debug)]
struct SourceBytesCharge {
    budget: Arc<SourceBytesBudget>,
    bytes: usize,
}

impl SourceBytesCharge {
    #[cfg(test)]
    fn new(budget: Arc<SourceBytesBudget>, bytes: usize) -> Self {
        let mut used = budget.used.lock().unwrap();
        *used = used.saturating_add(bytes);
        drop(used);
        Self { budget, bytes }
    }

    /// Atomically admits `bytes` against `budget` and returns the lifetime charge.
    ///
    /// The check and increment share one lock, so concurrent admissions cannot jointly exceed
    /// the cap. Callers must not pass a replacement discount: a previous source remains charged
    /// until its owner drops it after the protocol flush.
    fn try_new(budget: Arc<SourceBytesBudget>, bytes: usize) -> io::Result<Self> {
        let mut used = budget.used.lock().unwrap();
        admit_source_bytes(*used, bytes, budget.limit)?;
        *used = used.saturating_add(bytes);
        debug_assert!(*used <= budget.limit, "source-byte admission exceeded its cap");
        drop(used);
        Ok(Self { budget, bytes })
    }
}

impl Drop for SourceBytesCharge {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock().unwrap();
        *used = used.saturating_sub(self.bytes);
    }
}

/// Immutable, permit-bearing PNG bytes.
///
/// The permit is acquired before the backing vector is retained and is released when the last
/// clone is dropped. There is no accessor that detaches the bytes from the permit. [`Self::admit`]
/// admits bytes that a caller already allocated. Encoder workers should use [`PngBytesWriter`],
/// which acquires the permit before allocating or growing its output.
#[derive(Clone)]
pub struct PngBytes {
    inner: Arc<PngBytesInner>,
}

struct PngBytesInner {
    bytes: Vec<u8>,
    /// Released only when this allocation and every shared clone are gone.
    #[allow(dead_code)]
    _permit: ImagePermit,
}

impl PngBytes {
    /// Admits already-encoded PNG bytes against the process-global image budget.
    ///
    /// The backing allocation must fit the existing 16 MiB transfer cap. The admission reserves
    /// twice its capacity (the retained buffer plus transient encoder/reader storage). A full
    /// budget returns [`ClipboardError::Busy`].
    pub fn admit(bytes: Vec<u8>) -> Result<Self, ClipboardError> {
        if bytes.capacity() > MAX_TRANSFER_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        let permit = ImagePermit::acquire_global(bytes.capacity().saturating_mul(2))?;
        Ok(Self::from_vec_with_permit(bytes, permit))
    }

    fn from_vec_with_permit(bytes: Vec<u8>, permit: ImagePermit) -> Self {
        Self { inner: Arc::new(PngBytesInner { bytes, _permit: permit }) }
    }

    /// The encoded PNG payload.
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner.bytes
    }

    /// The encoded PNG payload length in bytes.
    pub fn len(&self) -> usize {
        self.inner.bytes.len()
    }

    /// Whether the encoded PNG payload is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.bytes.is_empty()
    }
}

impl fmt::Debug for PngBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PngBytes").field("len", &self.len()).finish()
    }
}

/// A bounded, fallible PNG encoder output buffer that owns its image-budget permit.
///
/// The permit is acquired in [`Self::new`] / [`Self::with_limit`] **before** the output vector is
/// allocated. It covers the retained buffer plus the reader/encoder transient allowance, and it
/// is moved into the resulting [`PngBytes`] by [`Self::finish`]. If the writer is dropped without
/// finishing, the permit is released and no partial PNG escapes.
///
/// This type implements [`io::Write`] so an image encoder can stream into it off the event-loop
/// thread. A write that would take the payload past its byte limit fails before extending the
/// vector, and allocation growth uses `try_reserve_exact` rather than an infallible reserve.
pub struct PngBytesWriter {
    bytes: Vec<u8>,
    permit: Option<ImagePermit>,
    max_bytes: usize,
}

impl PngBytesWriter {
    /// A writer capped at the existing 16 MiB transfer limit.
    pub fn new() -> Result<Self, ClipboardError> {
        Self::with_limit(MAX_TRANSFER_BYTES)
    }

    /// A writer capped at `max_bytes`, acquiring `2 * max_bytes` of image budget first.
    ///
    /// `max_bytes` must be non-zero and no larger than [`MAX_TRANSFER_BYTES`]. A full image
    /// budget returns [`ClipboardError::Busy`] without allocating the output vector.
    pub fn with_limit(max_bytes: usize) -> Result<Self, ClipboardError> {
        Self::with_budget(image_budget(), max_bytes)
    }

    fn with_budget(budget: &Arc<ImageBudget>, max_bytes: usize) -> Result<Self, ClipboardError> {
        if max_bytes == 0 || max_bytes > MAX_TRANSFER_BYTES {
            return Err(ClipboardError::InvalidInput);
        }
        let reserved = max_bytes.checked_mul(2).ok_or(ClipboardError::TooLarge)?;
        // Acquire before `Vec::new` grows any backing allocation. The vector starts empty and
        // every later growth is checked against both `max_bytes` and this reservation.
        let permit = budget.try_acquire(reserved).ok_or(ClipboardError::Busy)?;
        Ok(Self { bytes: Vec::new(), permit: Some(permit), max_bytes })
    }

    /// Number of encoded bytes written so far.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether no encoded bytes have been written yet.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The configured encoded-byte limit.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Finish encoding and transfer the permit into immutable [`PngBytes`].
    pub fn finish(mut self) -> Result<PngBytes, ClipboardError> {
        let permit = self.permit.take().expect("an unfinished writer retains its image permit");
        Ok(PngBytes::from_vec_with_permit(std::mem::take(&mut self.bytes), permit))
    }
}

impl io::Write for PngBytesWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self.bytes.len().checked_add(buffer.len()).ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidData, "the encoded PNG is too large")
        })?;
        if next > self.max_bytes {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "the encoded PNG exceeds its byte limit",
            ));
        }
        // Grow fallibly and before copying. `try_reserve_exact` reports allocator failure as an
        // error instead of aborting the encoder worker.
        self.bytes.try_reserve_exact(buffer.len()).map_err(|_| {
            io::Error::new(ErrorKind::Other, "the encoded PNG could not be allocated")
        })?;
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl fmt::Debug for PngBytesWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PngBytesWriter")
            .field("len", &self.bytes.len())
            .field("max_bytes", &self.max_bytes)
            .finish_non_exhaustive()
    }
}

/// The result of one combined regular read.
///
/// Text is preferred when an offer advertises both text and `image/png`; otherwise the read
/// returns the exact PNG payload. PRIMARY reads remain text-only.
#[derive(Clone, Debug)]
pub enum ClipboardContent {
    /// UTF-8 text after the existing lossy/CRLF normalization and byte-cap checks.
    Text(Arc<str>),
    /// Exact `image/png` bytes with their image-budget permit still held.
    ImagePng(PngBytes),
}

/// A caller-supplied completion callback.
pub type TransferWaker = Arc<dyn Fn() + Send + Sync + 'static>;

/// Options for one asynchronous regular or PRIMARY read.
///
/// The deadline starts at admission and covers queue wait, pipe I/O and text/PNG conversion; a
/// trickling peer cannot extend it. The caller owns the returned [`TransferHandle`] or
/// [`RegularTransferHandle`]; its waker should schedule a repaint/input pass for `target_window`,
/// not broadcast a generic winit event. `total_deadline` is rejected with `InvalidInput` above
/// [`MAX_TOTAL_DEADLINE`], and `max_bytes` is rejected with `InvalidInput` when zero or above
/// [`MAX_TRANSFER_BYTES`]. The same byte cap applies to a self-paste and to the UTF-8 text
/// produced after lossy conversion, so invalid input cannot expand past it.
///
/// A self-paste can complete synchronously during `begin_load_*`; a waker must therefore tolerate
/// being invoked once before the returned [`TransferHandle`] is stored by the caller. Admission
/// failure follows the same contract: a concurrent transition or worker/coordinator failure may
/// invoke the waker once before `begin_load_*` returns `Err`. The bridge caller must therefore
/// ignore a wake for which it has no stored handle and take any later `Err` as the authoritative
/// admission result. Winit does not add a second synchronization step for that edge.
pub struct TransferOptions {
    /// The window whose active input seat owns this read.
    pub target_window: WindowId,
    /// The caller's request/input generation. It is retained on the returned handle and
    /// available through `caller_generation()` for the caller's own validation; winit does not
    /// compare it with any internal generation.
    pub caller_generation: u64,
    /// Total time allowed for admission and the whole peer transfer.
    pub total_deadline: Duration,
    /// Largest accepted byte count, checked cumulatively while reading.
    pub max_bytes: usize,
    /// Invoked once after a result is stored, unless the handle was cancelled.
    pub completion_waker: TransferWaker,
}

impl TransferOptions {
    /// Options with the two-second total deadline and 16 MiB byte cap.
    pub fn new(
        target_window: WindowId,
        caller_generation: u64,
        completion_waker: TransferWaker,
    ) -> Self {
        Self {
            target_window,
            caller_generation,
            total_deadline: DEFAULT_TOTAL_DEADLINE,
            max_bytes: DEFAULT_MAX_TRANSFER_BYTES,
            completion_waker,
        }
    }

    /// Override the total deadline and byte cap. Values above the hard limits are rejected
    /// by `begin_load_*` with `InvalidInput`.
    pub fn with_limits(mut self, total_deadline: Duration, max_bytes: usize) -> Self {
        self.total_deadline = total_deadline;
        self.max_bytes = max_bytes;
        self
    }
}

impl fmt::Debug for TransferOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferOptions")
            .field("target_window", &self.target_window)
            .field("caller_generation", &self.caller_generation)
            .field("total_deadline", &self.total_deadline)
            .field("max_bytes", &self.max_bytes)
            .finish_non_exhaustive()
    }
}

/// An opaque winit-issued input/seat/focus-generation token.
///
/// Obtain one from [`crate::platform::wayland::Clipboard::input_origin`] while handling the input
/// that triggers a copy. It is deliberately not constructible outside winit; cloning an old token
/// does not make it fresh again.
///
/// A token is single-use per selection family: the same triggering input may claim the regular
/// clipboard once and PRIMARY once, but a second claim in either family returns `InvalidInput`.
#[derive(Clone)]
pub struct InputOriginToken {
    inner: OriginTokenInner,
}

impl fmt::Debug for InputOriginToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputOriginToken")
            .field("serial", &self.inner.serial)
            .field("origin", &self.inner.origin)
            .field("window", &self.inner.window)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputOrigin {
    KeyboardKey,
    PointerPress,
    PointerRelease,
}

#[derive(Clone)]
struct OriginTokenInner {
    seat_token: u64,
    serial: u32,
    origin: InputOrigin,
    at: Instant,
    epoch: u64,
    window: WindowId,
}

#[derive(Clone, Copy, Debug)]
struct InputSerial {
    serial: u32,
    origin: InputOrigin,
    at: Instant,
    epoch: u64,
    window: WindowId,
    regular_consumed: bool,
    primary_consumed: bool,
}

impl InputSerial {
    fn fresh(self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) <= INPUT_ORIGIN_MAX_AGE
    }

    fn claimable(self) -> bool {
        matches!(
            self.origin,
            InputOrigin::KeyboardKey | InputOrigin::PointerPress | InputOrigin::PointerRelease
        )
    }

    fn consumed(self, kind: SelectionKind) -> bool {
        match kind {
            SelectionKind::Regular => self.regular_consumed,
            SelectionKind::Primary => self.primary_consumed,
        }
    }

    fn consume(&mut self, kind: SelectionKind) {
        match kind {
            SelectionKind::Regular => self.regular_consumed = true,
            SelectionKind::Primary => self.primary_consumed = true,
        }
    }
}

/// Pure per-seat input policy, kept separate from Wayland objects so its origin/epoch rules can
/// be tested directly.
#[derive(Debug, Default)]
struct SeatInputState {
    keyboard_focus: bool,
    focused_window: Option<WindowId>,
    epoch: u64,
    keyboard: Option<InputSerial>,
    pointer: Option<InputSerial>,
}

impl SeatInputState {
    fn focuses(&self, target: WindowId) -> bool {
        self.keyboard_focus && self.focused_window == Some(target)
    }

    fn enter(&mut self, _serial: u32, window: WindowId, _now: Instant) {
        // A click-to-focus compositor may dispatch the pointer button before `wl_keyboard.enter`.
        // Keep that pre-focus pointer serial only for the window that just gained focus, and move
        // it into the new focus epoch so a claim can validate it once the epoch becomes current.
        let pre_focus_pointer = self.pointer.filter(|pointer| pointer.window == window);
        self.epoch = self.epoch.wrapping_add(1);
        self.keyboard_focus = true;
        self.focused_window = Some(window);
        self.keyboard = None;
        self.pointer = pre_focus_pointer.map(|mut pointer| {
            pointer.epoch = self.epoch;
            pointer
        });
    }

    fn leave(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.keyboard_focus = false;
        self.focused_window = None;
        self.keyboard = None;
        self.pointer = None;
    }

    fn key(&mut self, serial: u32, window: WindowId, now: Instant) {
        if !self.keyboard_focus || self.focused_window != Some(window) {
            return;
        }
        self.keyboard = Some(InputSerial {
            serial,
            origin: InputOrigin::KeyboardKey,
            at: now,
            epoch: self.epoch,
            window,
            regular_consumed: false,
            primary_consumed: false,
        });
        // Only the latest input turn may trigger ownership.
        self.pointer = None;
    }

    fn pointer(&mut self, serial: u32, window: WindowId, pressed: bool, now: Instant) {
        // Record the pointer serial even before keyboard focus arrives. The claim path still
        // requires `keyboard_focus` and `focused_window == target`, and `enter` only carries a
        // pre-focus pointer forward for the window that actually gained focus.
        self.pointer = Some(InputSerial {
            serial,
            origin: if pressed { InputOrigin::PointerPress } else { InputOrigin::PointerRelease },
            at: now,
            epoch: self.epoch,
            window,
            regular_consumed: false,
            primary_consumed: false,
        });
        self.keyboard = None;
    }

    fn latest(&self, target: WindowId, now: Instant) -> Option<InputSerial> {
        if !self.keyboard_focus || self.focused_window != Some(target) {
            return None;
        }

        let keyboard = self
            .keyboard
            .filter(|serial| serial.claimable() && serial.fresh(now) && serial.window == target);
        let pointer = self
            .pointer
            .filter(|serial| serial.claimable() && serial.fresh(now) && serial.window == target);
        match (keyboard, pointer) {
            (Some(keyboard), Some(pointer)) => {
                if pointer.at >= keyboard.at {
                    Some(pointer)
                } else {
                    Some(keyboard)
                }
            },
            (Some(keyboard), None) => Some(keyboard),
            (None, Some(pointer)) => Some(pointer),
            (None, None) => None,
        }
    }

    fn current_origin(
        &self,
        seat_token: u64,
        target: WindowId,
        now: Instant,
    ) -> Option<InputOriginToken> {
        let serial = self.latest(target, now)?;
        if serial.consumed(SelectionKind::Regular) && serial.consumed(SelectionKind::Primary) {
            return None;
        }
        Some(InputOriginToken {
            inner: OriginTokenInner {
                seat_token,
                serial: serial.serial,
                origin: serial.origin,
                at: serial.at,
                epoch: serial.epoch,
                window: serial.window,
            },
        })
    }

    fn validates(
        &self,
        token: &OriginTokenInner,
        target: WindowId,
        kind: SelectionKind,
        now: Instant,
    ) -> bool {
        if token.window != target
            || !self.keyboard_focus
            || self.focused_window != Some(target)
            || token.epoch != self.epoch
            || !token.fresh(now)
        {
            return false;
        }

        // Latest origin wins: a retained older keyboard token is no longer a valid claim after
        // pointer input in the same focus epoch.
        let Some(current) = self.latest(target, now) else {
            return false;
        };
        current.origin == token.origin
            && current.serial == token.serial
            && current.epoch == token.epoch
            && current.window == token.window
            && !current.consumed(kind)
    }

    /// Consume the exact latest trigger accepted by `validates` for one selection family.
    ///
    /// Returns false if the serial was already claimed in that family or invalidated. This is a
    /// real state transition, not a debug assertion: release builds must reject a replayed token.
    fn consume(&mut self, token: &OriginTokenInner, kind: SelectionKind) -> bool {
        let matches = |serial: &InputSerial| {
            serial.origin == token.origin
                && serial.serial == token.serial
                && serial.epoch == token.epoch
                && serial.window == token.window
        };
        let serial = match token.origin {
            InputOrigin::KeyboardKey => self.keyboard.as_mut(),
            InputOrigin::PointerPress | InputOrigin::PointerRelease => self.pointer.as_mut(),
        };
        match serial {
            Some(serial) if matches(serial) && !serial.consumed(kind) => {
                serial.consume(kind);
                true
            },
            _ => false,
        }
    }
}

impl OriginTokenInner {
    fn fresh(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) <= INPUT_ORIGIN_MAX_AGE
    }
}

/// A synchronous selection ownership claim.
///
/// `trigger` must come from [`crate::platform::wayland::Clipboard::input_origin`] for the same
/// input turn that asks for the copy. Claims validate that the trigger's seat is still the active
/// input seat, that it focuses the target window in the same focus epoch, and that the triggering
/// keyboard or pointer serial is still the latest one before replacing any owned source. Each
/// family consumes its own copy of the trigger, so one input may set the regular and PRIMARY
/// selections, once each.
#[derive(Clone, Debug)]
pub struct SelectionClaim {
    pub target_window: WindowId,
    pub caller_generation: u64,
    pub trigger: InputOriginToken,
}

/// A non-cloneable reservation for one asynchronous regular PNG copy.
///
/// [`crate::platform::wayland::Clipboard::reserve_image_claim`] validates and consumes the
/// regular-family trigger, retains the exact input serial, and fences the reservation with the
/// seat, window, focus epoch, active-input-seat epoch, and a per-seat regular ownership/reservation
/// epoch. Encoding may then happen off the event-loop thread. The reservation is installed only
/// through [`crate::platform::wayland::Clipboard::claim_reserved_image_png`], which uses the
/// original serial even if unrelated input arrived afterwards.
///
/// Dropping this value cancels the reservation and removes its pending accounting. Seat/window/
/// focus loss, another regular ownership attempt (including one that fails), explicit
/// invalidation, or expiry at the triggering input's original five-second freshness deadline
/// also make it unusable. The deadline is not extended by the time at which the reservation was
/// requested. PRIMARY claims never invalidate a regular reservation.
pub struct ImageClaimReservation {
    inner: Option<ImageReservationInner>,
}

struct ImageReservationInner {
    id: u64,
    target_window: WindowId,
    caller_generation: u64,
    seat_token: u64,
    serial: u32,
    focus_epoch: u64,
    active_seat_epoch: u64,
    ownership_epoch: u64,
    deadline: Instant,
    validation: Weak<Mutex<Inner>>,
}

impl ImageClaimReservation {
    /// The target window captured by the triggering copy.
    pub fn target_window(&self) -> WindowId {
        self.inner.as_ref().expect("a live reservation has its captured route").target_window
    }

    /// The caller generation supplied with the triggering [`SelectionClaim`].
    ///
    /// Winit only retains this value for the caller's own validation.
    pub fn caller_generation(&self) -> u64 {
        self.inner.as_ref().expect("a live reservation has its caller generation").caller_generation
    }

    fn take(&mut self) -> ImageReservationInner {
        self.inner.take().expect("a reservation can be claimed only once")
    }

    #[cfg(test)]
    fn serial(&self) -> u32 {
        self.inner.as_ref().expect("a live reservation has its original serial").serial
    }
}

impl fmt::Debug for ImageClaimReservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            Some(inner) => f
                .debug_struct("ImageClaimReservation")
                .field("target_window", &inner.target_window)
                .field("serial", &inner.serial)
                .field("seat_token", &inner.seat_token)
                .field("ownership_epoch", &inner.ownership_epoch)
                .finish_non_exhaustive(),
            None => f.write_str("ImageClaimReservation(consumed)"),
        }
    }
}

impl Drop for ImageClaimReservation {
    fn drop(&mut self) {
        // Cancellation must not wake or call any user code. The reservation owns no payload; it
        // only removes its small pending record so invalidation/drop accounting stays exact.
        if let Some(inner) = self.inner.take() {
            if let Some(state) = inner.validation.upgrade() {
                if let Ok(mut state) = state.lock() {
                    state.release_image_reservation(inner.id);
                }
            }
        }
    }
}

/// A caller-owned asynchronous text read.
///
/// The result is available at most once. Dropping or cancelling the handle invalidates a late
/// result and releases the completion waker. The handle is `Send + Sync`; use it from the target
/// window's next input preparation and revalidate the caller's generation there.
///
/// Winit itself can also complete a transfer with `Interrupted` when the selection owner changes,
/// the window closes or the seat goes away. That completion wakes once like a successful read;
/// `try_take` then returns the error exactly once.
pub struct TransferHandle {
    slot: Arc<TransferSlot>,
}

impl TransferHandle {
    /// Take the completed text result, if any. A cancelled handle never yields a result.
    ///
    /// The frozen offer, source, seat, focus epoch and window are validated again before the
    /// stored result is taken. A result completed before the selection owner, focus, seat or
    /// window changed therefore becomes [`ErrorKind::Interrupted`] instead of escaping.
    pub fn try_take(&self) -> Option<io::Result<Arc<str>>> {
        self.slot.try_take().map(|result| match result {
            Ok(ClipboardContent::Text(text)) => Ok(text),
            Ok(ClipboardContent::ImagePng(_)) => Err(ClipboardError::NotOffered.into_io_error()),
            Err(error) => Err(error.into_io_error()),
        })
    }

    /// The caller generation supplied to the `TransferOptions` that admitted this read.
    ///
    /// Winit retains it only so the caller can compare it with its own request/input state;
    /// winit never validates or rewrites it.
    pub fn caller_generation(&self) -> u64 {
        self.slot.caller_generation
    }

    /// Cancel the operation. Idempotent; releases the result and waker without waking it.
    pub fn cancel(&self) {
        self.slot.cancel();
    }
}

/// A caller-owned combined regular read.
///
/// It completes with text when the one selected offer advertises any existing text type, or with
/// exact `image/png` bytes otherwise. The route is resolved once at admission; there is no second
/// offer or fallback after a transfer failure. `try_take` consumes the result at most once, and
/// dropping the handle cancels it without waking.
pub struct RegularTransferHandle {
    slot: Arc<TransferSlot>,
}

impl RegularTransferHandle {
    /// Take the completed content, if any. A cancelled handle never yields a result.
    pub fn try_take(&self) -> Option<Result<ClipboardContent, ClipboardError>> {
        self.slot.try_take()
    }

    /// The caller generation supplied to the `TransferOptions` that admitted this read.
    ///
    /// Winit retains it only so the caller can compare it with its own request/input state;
    /// winit never validates or rewrites it.
    pub fn caller_generation(&self) -> u64 {
        self.slot.caller_generation
    }

    /// Cancel the operation. Idempotent; releases the result, permit and waker without waking.
    pub fn cancel(&self) {
        self.slot.cancel();
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TransferHandle>();
    assert_send_sync::<RegularTransferHandle>();
    assert_send_sync::<ClipboardContent>();
};

impl fmt::Debug for TransferHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferHandle")
            .field("request_id", &self.slot.request_id)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for RegularTransferHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegularTransferHandle")
            .field("request_id", &self.slot.request_id)
            .finish_non_exhaustive()
    }
}

impl Drop for TransferHandle {
    fn drop(&mut self) {
        self.slot.cancel();
    }
}

impl Drop for RegularTransferHandle {
    fn drop(&mut self) {
        self.slot.cancel();
    }
}

struct TransferInner {
    result: Option<Result<ClipboardContent, ClipboardError>>,
    waker: Option<TransferWaker>,
    /// A result or internal error has been stored, or the caller consumed it.
    completed: bool,
    /// The stored result was taken. An internal invalidation cannot retroactively revoke it.
    consumed: bool,
}

impl fmt::Debug for TransferInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferInner")
            .field("result_present", &self.result.is_some())
            .field("waker_present", &self.waker.is_some())
            .field("completed", &self.completed)
            .field("consumed", &self.consumed)
            .finish()
    }
}

/// The exact route one transfer was admitted for.
///
/// `try_take` compares it with the clipboard state, so a result stored before a focus, seat,
/// window, owner, source or active-input-seat change cannot escape. A legacy blocking read has no
/// target window and therefore only freezes the seat and offer/source identity.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FrozenRoute {
    target_window: Option<WindowId>,
    seat_token: u64,
    selection_kind: SelectionKind,
    expected_offer: Option<OfferKey>,
    expected_source: Option<u64>,
    focus_epoch: u64,
    /// Monotonic epoch of the active input seat. It changes whenever input moves to a different
    /// seat, so an A-B-A transfer still invalidates even though the seat token returns to A.
    active_seat_epoch: u64,
    check_focus: bool,
}

impl FrozenRoute {
    fn unchecked(
        target_window: Option<WindowId>,
        seat_token: u64,
        selection_kind: SelectionKind,
    ) -> Self {
        Self {
            target_window,
            seat_token,
            selection_kind,
            expected_offer: None,
            expected_source: None,
            focus_epoch: 0,
            active_seat_epoch: 0,
            check_focus: false,
        }
    }
}

#[derive(Debug)]
struct TransferSlot {
    request_id: u64,
    /// The caller's own generation as supplied to `begin_load_*`. Winit only retains it; the
    /// handle exposes it for the caller's later validation and never compares it internally.
    caller_generation: u64,
    target_window: Option<WindowId>,
    seat_token: u64,
    selection_kind: SelectionKind,
    route: FrozenRoute,
    /// The clipboard state this route is validated against. Weak, so a slot cannot keep the
    /// clipboard alive. `None` is only used by the protocol-free queue tests.
    validation: Option<Weak<Mutex<Inner>>>,
    /// The admission deadline shared with the queued/active read job. The blocking compatibility
    /// wait observes it so a missing coordinator cannot strand the caller forever.
    deadline: Mutex<Instant>,
    /// Set by an explicit caller `cancel`/drop: result and waker are discarded silently.
    caller_cancelled: AtomicBool,
    /// Set once a result or internal error is stored, so queued jobs can be pruned.
    finished: AtomicBool,
    inner: Mutex<TransferInner>,
    result_ready: Condvar,
}

impl TransferSlot {
    fn new(
        request_id: u64,
        target_window: Option<WindowId>,
        seat_token: u64,
        selection_kind: SelectionKind,
        waker: Option<TransferWaker>,
    ) -> Self {
        Self {
            request_id,
            caller_generation: 0,
            target_window,
            seat_token,
            selection_kind,
            route: FrozenRoute::unchecked(target_window, seat_token, selection_kind),
            validation: None,
            deadline: Mutex::new(Instant::now() + DEFAULT_TOTAL_DEADLINE),
            caller_cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            inner: Mutex::new(TransferInner {
                result: None,
                waker,
                completed: false,
                consumed: false,
            }),
            result_ready: Condvar::new(),
        }
    }

    /// Worker completion: store the result and wake once unless the caller cancelled.
    fn set_result(&self, result: Result<ClipboardContent, ClipboardError>) {
        let waker = {
            let mut inner = self.inner.lock().unwrap();
            if self.caller_cancelled.load(Ordering::Acquire) || inner.completed || inner.consumed {
                return;
            }
            inner.completed = true;
            inner.result = Some(result);
            self.finished.store(true, Ordering::Release);
            inner.waker.take()
        };
        self.result_ready.notify_all();
        self.invoke_waker(waker);
    }

    /// Internal invalidation (owner change, focus loss, window close, seat removal, clipboard
    /// drop): revoke even a completed-but-unconsumed result.
    ///
    /// If the one completion wake already ran, this does not wake again: it replaces the stored
    /// result with `Interrupted`, so the single `try_take` observes the invalidation. If no result
    /// was stored yet, it stores the error and wakes exactly once. Must not be called while the
    /// clipboard lock is held.
    fn fail_internal(&self, error: ClipboardError) {
        let waker = {
            let mut inner = self.inner.lock().unwrap();
            if self.caller_cancelled.load(Ordering::Acquire) || inner.consumed {
                return;
            }
            let already_completed = inner.result.is_some();
            inner.completed = true;
            inner.result = Some(Err(error));
            self.finished.store(true, Ordering::Release);
            if already_completed {
                None
            } else {
                inner.waker.take()
            }
        };
        self.result_ready.notify_all();
        self.invoke_waker(waker);
    }

    fn invoke_waker(&self, waker: Option<TransferWaker>) {
        // A wake requested just before an explicit cancel may still run after `cancel` returns.
        // It only asks the caller for a repaint; `try_take` stays empty after a cancel.
        if self.caller_cancelled.load(Ordering::Acquire) {
            return;
        }
        if let Some(waker) = waker {
            // Never call user code while holding a slot lock. A panicking caller callback must
            // not leak the worker permit that will release this slot.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| waker()));
            if result.is_err() {
                warn!("clipboard completion callback panicked");
            }
        }
    }

    /// Records the admission deadline for the blocking compatibility wait. Called before the slot
    /// is published, so no waiter can observe a half-updated deadline.
    fn set_deadline(&self, deadline: Instant) {
        *self.deadline.lock().unwrap() = deadline;
    }

    fn cancel(&self) {
        self.caller_cancelled.store(true, Ordering::Release);
        let mut inner = self.inner.lock().unwrap();
        inner.result = None;
        inner.waker = None;
        inner.completed = true;
        inner.consumed = true;
        drop(inner);
        self.result_ready.notify_all();
    }

    /// Takes at most one result and validates the route while the clipboard lock is held.
    fn try_take(&self) -> Option<Result<ClipboardContent, ClipboardError>> {
        if self.caller_cancelled.load(Ordering::Acquire) {
            return None;
        }
        let validation = self.validation.as_ref().and_then(Weak::upgrade);
        match validation {
            None if self.validation.is_some() => {
                // The clipboard is gone. Revoke any stored result exactly once.
                let mut inner = self.inner.lock().unwrap();
                if self.caller_cancelled.load(Ordering::Acquire) {
                    return None;
                }
                let result = inner.result.take();
                if result.is_some() {
                    inner.consumed = true;
                }
                result.map(|_| Err(ClipboardError::Interrupted))
            },
            Some(state) => {
                // Lock order: clipboard state, then the slot. Transitions release the clipboard
                // lock before calling `fail_internal`, so they never invert this order.
                let state = state.lock().unwrap();
                let route_current = state.frozen_route_is_current(&self.route);
                let mut inner = self.inner.lock().unwrap();
                if self.caller_cancelled.load(Ordering::Acquire) {
                    return None;
                }
                let result = inner.result.take();
                if result.is_some() {
                    inner.consumed = true;
                }
                result.map(
                    |result| {
                        if route_current {
                            result
                        } else {
                            Err(ClipboardError::Interrupted)
                        }
                    },
                )
            },
            None => {
                let mut inner = self.inner.lock().unwrap();
                if self.caller_cancelled.load(Ordering::Acquire) {
                    return None;
                }
                let result = inner.result.take();
                if result.is_some() {
                    inner.consumed = true;
                }
                result
            },
        }
    }

    fn wait(&self) -> Result<ClipboardContent, ClipboardError> {
        let deadline = *self.deadline.lock().unwrap();
        loop {
            if self.caller_cancelled.load(Ordering::Acquire) {
                return Err(ClipboardError::Interrupted);
            }
            if let Some(result) = self.try_take() {
                return result;
            }
            let inner = self.inner.lock().unwrap();
            if self.caller_cancelled.load(Ordering::Acquire) {
                return Err(ClipboardError::Interrupted);
            }
            if inner.result.is_some() {
                continue;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ClipboardError::TimedOut);
            }
            let (guard, _) = self
                .result_ready
                .wait_timeout(inner, deadline.saturating_duration_since(now))
                .unwrap();
            drop(guard);
        }
    }

    fn is_caller_cancelled(&self) -> bool {
        self.caller_cancelled.load(Ordering::Acquire)
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

#[derive(Debug, Default)]
struct TransferRegistry {
    next_request_id: u64,
    slots: Vec<Weak<TransferSlot>>,
}

impl TransferRegistry {
    #[cfg(test)]
    fn create(
        &mut self,
        target_window: Option<WindowId>,
        seat_token: u64,
        selection_kind: SelectionKind,
        waker: Option<TransferWaker>,
    ) -> Arc<TransferSlot> {
        let route = FrozenRoute::unchecked(target_window, seat_token, selection_kind);
        self.create_inner(0, target_window, route, None, waker)
    }

    fn create_with_route(
        &mut self,
        caller_generation: u64,
        target_window: Option<WindowId>,
        route: FrozenRoute,
        validation: Weak<Mutex<Inner>>,
        waker: Option<TransferWaker>,
    ) -> Arc<TransferSlot> {
        self.create_inner(caller_generation, target_window, route, Some(validation), waker)
    }

    fn create_inner(
        &mut self,
        caller_generation: u64,
        target_window: Option<WindowId>,
        route: FrozenRoute,
        validation: Option<Weak<Mutex<Inner>>>,
        waker: Option<TransferWaker>,
    ) -> Arc<TransferSlot> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.slots.retain(|slot| slot.strong_count() > 0);
        let seat_token = route.seat_token;
        let selection_kind = route.selection_kind;
        let mut slot =
            TransferSlot::new(request_id, target_window, seat_token, selection_kind, waker);
        slot.caller_generation = caller_generation;
        if let Some(validation) = validation {
            slot.route = route;
            slot.validation = Some(validation);
        }
        let slot = Arc::new(slot);
        self.slots.push(Arc::downgrade(&slot));
        slot
    }

    /// Invalidates every live transfer bound to one window.
    fn interrupt_target(&mut self, target: WindowId, slots: &mut Vec<Arc<TransferSlot>>) {
        self.slots.retain(|weak| {
            let Some(slot) = weak.upgrade() else { return false };
            if slot.target_window == Some(target) {
                slots.push(slot);
                false
            } else {
                true
            }
        });
    }

    /// Invalidates every live transfer for one seat and selection family.
    fn interrupt_seat(
        &mut self,
        seat_token: u64,
        selection_kind: SelectionKind,
        slots: &mut Vec<Arc<TransferSlot>>,
    ) {
        self.slots.retain(|weak| {
            let Some(slot) = weak.upgrade() else { return false };
            if slot.seat_token == seat_token && slot.selection_kind == selection_kind {
                slots.push(slot);
                false
            } else {
                true
            }
        });
    }

    fn interrupt_all(&mut self, slots: &mut Vec<Arc<TransferSlot>>) {
        for weak in self.slots.drain(..) {
            if let Some(slot) = weak.upgrade() {
                slots.push(slot);
            }
        }
    }
}

/// Collects transfers that production transitions invalidated under the clipboard lock.
///
/// The caller must drop the [`Inner`] lock and then call [`TransitionBatch::complete`]; that is
/// what guarantees user wakers never run under the clipboard lock.
#[derive(Debug, Default)]
struct TransitionBatch {
    interrupted: Vec<Arc<TransferSlot>>,
}

impl TransitionBatch {
    fn merge(&mut self, other: Self) {
        self.interrupted.extend(other.interrupted);
    }

    fn complete(self) {
        for slot in self.interrupted {
            slot.fail_internal(ClipboardError::Interrupted);
        }
    }
}

/// Runs one production transition under the clipboard lock, then completes invalidations after
/// the lock is released.
fn run_transition<R>(
    inner: &Arc<Mutex<Inner>>,
    transition: impl FnOnce(&mut Inner) -> (R, TransitionBatch),
) -> R {
    let (result, batch) = {
        let mut guard = inner.lock().unwrap();
        transition(&mut guard)
    };
    batch.complete();
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionKind {
    Regular,
    Primary,
}

/// What one admission may accept from its single selected offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadPreference {
    /// Existing text-only reads and all PRIMARY reads.
    TextOnly,
    /// The combined regular read: text in the existing order, else exact `image/png`.
    TextThenImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct OfferKey(u64);

/// One snapshot of the compositor's selection state for a seat family.
///
/// `key` is assigned by [`Inner`] when a distinct protocol offer is observed and is `None` for an
/// absent offer. The type is protocol free so the transition tests drive exactly the code the
/// SCTK handlers drive.
#[derive(Clone, Debug)]
struct ObservedOffer {
    key: Option<OfferKey>,
    mime_types: Vec<String>,
}

impl ObservedOffer {
    #[cfg(test)]
    fn present(key: OfferKey, mime_types: Vec<String>) -> Self {
        Self { key: Some(key), mime_types }
    }

    fn marker(&self) -> Option<&str> {
        // Every winit process and every clipboard owns a distinct marker identity. Keep the
        // marker raw here; `Inner` decides whether it is this clipboard's current source, one of
        // its stale sources, or a foreign winit clipboard whose text we may read.
        self.mime_types.iter().find(|mime| mime.starts_with(MARKER_PREFIX)).map(String::as_str)
    }

    fn to_view(&self, marker: Option<String>) -> Option<OfferView> {
        self.key.map(|key| OfferView { key, mime_types: self.mime_types.clone(), marker })
    }
}

#[derive(Clone, Debug)]
struct OfferView {
    key: OfferKey,
    mime_types: Vec<String>,
    /// The exact winit marker mime advertised by this offer, if any.
    marker: Option<String>,
}

#[derive(Clone, Debug)]
enum SourcePhase {
    /// `set_selection` was sent; the compositor has not echoed this source yet. `predecessor` is
    /// the offer that was current when the claim was installed and may still be replayed.
    Pending { predecessor: Option<OfferKey>, since: Instant },
    /// The compositor announced this exact source marker.
    Echoed,
}

/// The payload of one source we own. A regular image source carries permit-bearing PNG bytes;
/// PRIMARY and text sources carry text. Keeping the payload in the source lets self-reads return
/// the exact owned content without a compositor round trip.
#[derive(Clone)]
enum OwnedPayload {
    Text(Arc<str>),
    Png(PngBytes),
}

impl OwnedPayload {
    fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Png(png) => png.len(),
        }
    }

    fn serves(&self, mime_type: &str) -> bool {
        match self {
            Self::Text(_) => TEXT_MIME_TYPES.contains(&mime_type),
            Self::Png(_) => mime_type == IMAGE_PNG_MIME,
        }
    }

    #[cfg(test)]
    fn as_text(&self) -> Option<&Arc<str>> {
        match self {
            Self::Text(text) => Some(text),
            Self::Png(_) => None,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(text) => text.as_bytes(),
            Self::Png(png) => png.as_bytes(),
        }
    }
}

impl fmt::Debug for OwnedPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(&text.len()).finish(),
            Self::Png(png) => f.debug_tuple("Png").field(png).finish(),
        }
    }
}

impl PartialEq for OwnedPayload {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(left), Self::Text(right)) => left == right,
            (Self::Png(left), Self::Png(right)) => left.as_bytes() == right.as_bytes(),
            _ => false,
        }
    }
}

impl Eq for OwnedPayload {}

/// One owned regular or PRIMARY source. The protocol object (if any) lives with it, but the
/// lifecycle state is usable without a Wayland connection so tests can exercise the real
/// transitions.
struct OwnedSource {
    token: u64,
    marker: String,
    payload: OwnedPayload,
    /// The lifetime charge shared by this payload allocation and every serving writer that
    /// currently references it.
    source_bytes: Arc<SourceBytesCharge>,
    /// The exact protocol serial used for `set_selection` (or the compatibility store's serial).
    selection_serial: u32,
    target_window: WindowId,
    caller_generation: u64,
    serving: Arc<SourceServeState>,
    phase: SourcePhase,
    protocol: Option<ProtocolSource>,
}

impl fmt::Debug for OwnedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedSource")
            .field("token", &self.token)
            .field("target_window", &self.target_window)
            .field("caller_generation", &self.caller_generation)
            .field("payload_bytes", &self.payload.len())
            .field("payload", &self.payload)
            .field("selection_serial", &self.selection_serial)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl Drop for OwnedSource {
    fn drop(&mut self) {
        self.serving.cancel();
        // Dropping the protocol source destroys the Wayland object (including a source whose
        // claim failed before it was ever installed).
        drop(self.protocol.take());
    }
}

enum ProtocolSource {
    Regular(WlDataSource),
    Primary(PrimarySelectionSource),
}

impl fmt::Debug for ProtocolSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtocolSource")
            .field("family", &match self {
                Self::Regular(_) => "regular",
                Self::Primary(_) => "primary",
            })
            .finish()
    }
}

impl Drop for ProtocolSource {
    fn drop(&mut self) {
        match self {
            Self::Regular(source) => {
                if source.is_alive() {
                    source.destroy();
                }
            },
            // Dropping the wrapper destroys the PRIMARY protocol object.
            Self::Primary(_) => {},
        }
    }
}

/// The regular or PRIMARY selection state of one seat, with no Wayland objects.
#[derive(Debug, Default)]
struct SeatFamily {
    /// Last observed compositor offer.
    offer: Option<OfferView>,
    /// The text we currently own, if any.
    owned: Option<OwnedSource>,
}

impl SeatFamily {
    /// Production offer classification for a text-only read.
    fn classify(
        &self,
        kind: SelectionKind,
        identity: &ClipboardIdentity,
    ) -> Result<OfferRead, ClipboardError> {
        self.classify_with(kind, identity, ReadPreference::TextOnly)
    }

    /// Classifies one offer snapshot before any receive is requested.
    ///
    /// A pending claim always prefers the owned bytes: the compositor may not have echoed the
    /// selection yet, and reading the previous owner's offer would return stale text. Once the
    /// source is confirmed or the observation proves another owner, `offer` decides. A stale
    /// marker from this clipboard fails before text/image preference is considered. Text uses the
    /// existing order; only a regular combined read may then select exact `image/png`.
    fn classify_with(
        &self,
        kind: SelectionKind,
        identity: &ClipboardIdentity,
        preference: ReadPreference,
    ) -> Result<OfferRead, ClipboardError> {
        if let Some(owned) = &self.owned {
            match &owned.phase {
                SourcePhase::Pending { .. } => {
                    return match (&owned.payload, preference) {
                        (OwnedPayload::Text(_), _)
                        | (OwnedPayload::Png(_), ReadPreference::TextThenImage) => {
                            Ok(OfferRead::SelfContent(owned.payload.clone()))
                        },
                        (OwnedPayload::Png(_), ReadPreference::TextOnly) => {
                            Err(ClipboardError::NotOffered)
                        },
                    };
                },
                SourcePhase::Echoed => {
                    if self.offer.as_ref().and_then(|offer| offer.marker.as_deref())
                        == Some(owned.marker.as_str())
                    {
                        return match (&owned.payload, preference) {
                            (OwnedPayload::Text(_), _)
                            | (OwnedPayload::Png(_), ReadPreference::TextThenImage) => {
                                Ok(OfferRead::SelfContent(owned.payload.clone()))
                            },
                            (OwnedPayload::Png(_), ReadPreference::TextOnly) => {
                                Err(ClipboardError::NotOffered)
                            },
                        };
                    }
                },
            }
        }

        let Some(offer) = &self.offer else {
            return Err(ClipboardError::Absent);
        };

        if offer.marker.as_deref().is_some_and(|marker| identity.owns(marker)) {
            return Err(ClipboardError::StaleSelfSource);
        }

        if let Some(mime) = TEXT_MIME_TYPES
            .iter()
            .find(|mime| offer.mime_types.iter().any(|offered| offered == *mime))
        {
            return Ok(OfferRead::Request((*mime).to_owned()));
        }

        if preference == ReadPreference::TextThenImage
            && kind == SelectionKind::Regular
            && offer.mime_types.iter().any(|mime| mime == IMAGE_PNG_MIME)
        {
            return Ok(OfferRead::RequestImage(IMAGE_PNG_MIME.to_owned()));
        }

        Err(ClipboardError::NotOffered)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum OfferRead {
    /// The content of a source this clipboard still owns; no compositor round trip is needed.
    SelfContent(OwnedPayload),
    /// A text MIME type. A text transfer never falls back to an image.
    Request(String),
    /// The exact `image/png` MIME type from the same selected offer.
    RequestImage(String),
}

/// The exact seat, focus epoch and offer snapshot selected for one asynchronous read.
#[derive(Debug)]
struct ReadRoute {
    read: OfferRead,
    offer: Option<SelectionOfferKind>,
    frozen: FrozenRoute,
}

/// The payload and serving state behind one owned source.
type ServingSource = (OwnedPayload, Arc<SourceBytesCharge>, Arc<SourceServeState>);

/// One source-serving state shared by its owned text and every queued/active write.
#[derive(Debug, Default)]
struct SourceServeState {
    cancelled: AtomicBool,
    error: Mutex<Option<String>>,
}

impl SourceServeState {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn record_error(&self, error: impl fmt::Display) {
        let message = error.to_string();
        let mut current = self.error.lock().unwrap();
        if current.is_none() {
            warn!("clipboard selection serving failed: {message}");
            *current = Some(message);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

#[derive(Debug)]
struct SeatClipboard {
    seat: ObjectId,
    seat_token: u64,
    /// The regular `wl_data_device`, when the `wl_data_device_manager` global exists. PRIMARY
    /// does not depend on this field.
    device: Option<WlDataDevice>,
    /// The serial of the latest input event on this seat for the compatibility API.
    serial: Option<u32>,
    input: SeatInputState,
    /// Monotonic regular ownership/reservation epoch. Every new regular claim or reservation
    /// attempt advances it before validation, so a failed attempt still supersedes an older
    /// asynchronous image reservation. PRIMARY and paste routing do not touch it.
    regular_ownership_epoch: u64,
    /// Protocol offer last reported for the regular device (identity + receive endpoint).
    regular_offer: Option<SelectionOffer>,
    /// A predecessor offer invalidated by source cancellation. Reconciliation may keep
    /// reporting the same SCTK object until a new selection event arrives; it must not revive
    /// the logical offer in the meantime.
    suppressed_regular_offer: Option<ObjectId>,
    /// Lifecycle state of the regular selection.
    regular: SeatFamily,
    /// The distinct per-seat PRIMARY protocol object.
    primary_device: Option<PrimarySelectionDevice>,
    /// Protocol offer last reported for the PRIMARY device.
    primary_offer: Option<PrimarySelectionOffer>,
    /// Raw PRIMARY offer paired with `primary_offer` for a checked `receive` request. SCTK still
    /// owns the offer's lifetime; this clone is dead as soon as SCTK destroys it and is cleared
    /// whenever the wrapper is cleared.
    primary_raw_offer: Option<Arc<dyn PrimaryOfferProxy>>,
    /// The PRIMARY predecessor invalidated by source cancellation; see
    /// `suppressed_regular_offer`.
    suppressed_primary_offer: Option<PrimarySelectionOffer>,
    /// Lifecycle state of the PRIMARY selection.
    primary: SeatFamily,
}

impl SeatClipboard {
    fn family(&self, kind: SelectionKind) -> &SeatFamily {
        match kind {
            SelectionKind::Regular => &self.regular,
            SelectionKind::Primary => &self.primary,
        }
    }

    fn family_mut(&mut self, kind: SelectionKind) -> &mut SeatFamily {
        match kind {
            SelectionKind::Regular => &mut self.regular,
            SelectionKind::Primary => &mut self.primary,
        }
    }

    fn primary_receive_offer(&self) -> Option<Arc<dyn PrimaryOfferProxy>> {
        self.primary_raw_offer.clone()
    }

    /// Applies one raw-offer side-table update.
    ///
    /// `Replace` stores the proxy from the same event (or clears it for a NULL selection).
    /// Reconciliation cannot supply an event-time proxy, so it keeps an existing pair only while
    /// the wrapper identity is unchanged and present and otherwise clears it.
    fn update_primary_raw_offer(
        &mut self,
        same_object: bool,
        offer_present: bool,
        update: PrimaryRawOfferUpdate,
    ) {
        match update {
            PrimaryRawOfferUpdate::Replace(raw) => self.primary_raw_offer = raw,
            PrimaryRawOfferUpdate::Reconcile if same_object && offer_present => {},
            PrimaryRawOfferUpdate::Reconcile => self.primary_raw_offer = None,
        }
    }
}

/// How a PRIMARY selection observation updates the raw offer side table.
///
/// `Reconcile` only re-reads SCTK's wrapper, so the raw clone paired with it stays authoritative.
/// An event dispatch supplies the raw offer from that same event, including `None` for a NULL
/// selection, so wrapper and raw proxy change under one clipboard lock.
enum PrimaryRawOfferUpdate {
    Reconcile,
    Replace(Option<Arc<dyn PrimaryOfferProxy>>),
}

#[derive(Debug)]
struct Inner {
    seats: Vec<SeatClipboard>,
    /// The seat token that last gave one of our windows input.
    ///
    /// This identifies the exact seat entry, not merely its protocol object, and is cleared when
    /// that entry is removed or replaced. Routing never falls back to insertion order.
    active_seat: Option<u64>,
    /// Monotonic epoch bumped on every active-seat change. A-B-A input returns the same token but
    /// advances this epoch, so a focus-bound admitted read cannot survive an intervening seat.
    active_seat_epoch: u64,
    next_seat_token: u64,
    next_offer_key: u64,
    /// Identity embedded in markers for sources created by this clipboard instance.
    marker_identity: ClipboardIdentity,
    /// Highest source token this clipboard has allocated. This is monotonic and never evicted.
    last_source_token: u64,
    /// Small accounting records for live image reservations. They contain no payload; the
    /// permit-bearing PNG is owned by the caller until the claim consumes it.
    image_reservations: Vec<PendingImageReservation>,
    next_image_reservation_id: u64,
    /// Lifetime accounting for every source payload allocation still retained by an installed
    /// source or by a source-serving write.
    source_bytes: Arc<SourceBytesBudget>,
    windows: HashSet<WindowId>,
    transfers: TransferRegistry,
}

#[derive(Clone, Copy, Debug)]
struct PendingImageReservation {
    id: u64,
    target_window: WindowId,
    seat_token: u64,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            seats: Vec::new(),
            active_seat: None,
            active_seat_epoch: 0,
            next_seat_token: 1,
            next_offer_key: 1,
            marker_identity: ClipboardIdentity::new(),
            last_source_token: 0,
            image_reservations: Vec::new(),
            next_image_reservation_id: 1,
            source_bytes: Arc::new(SourceBytesBudget::new(MAX_TOTAL_OWNED_BYTES)),
            windows: HashSet::new(),
            transfers: TransferRegistry::default(),
        }
    }
}

/// One protocol source that is being installed, or `None` for a detached (test) claim.
struct ClaimInstall {
    kind: SelectionKind,
    token: u64,
    marker: String,
    payload: OwnedPayload,
    target_window: WindowId,
    caller_generation: u64,
    protocol: Option<ProtocolSource>,
}

/// One reserved image waiting for installation on the event-loop thread.
struct ReservedImageInstall {
    token: u64,
    marker: String,
    png: PngBytes,
    target_window: WindowId,
    protocol: Option<ProtocolSource>,
}

struct InstalledClaim {
    token: u64,
    previous: Option<OwnedSource>,
}

impl Inner {
    fn allocate_offer_key(&mut self) -> OfferKey {
        let key = OfferKey(self.next_offer_key);
        self.next_offer_key = self.next_offer_key.wrapping_add(1);
        key
    }

    /// Makes `token` the active input seat and advances the active-seat epoch when it changed.
    ///
    /// Repeated input on the already-active seat keeps the epoch, so an ordinary keystroke does
    /// not invalidate an in-flight read. Moving to another seat (and therefore returning later,
    /// even A-B-A) does.
    fn set_active_seat(&mut self, token: u64) {
        if self.active_seat != Some(token) {
            self.active_seat = Some(token);
            self.active_seat_epoch = self.active_seat_epoch.wrapping_add(1);
        }
    }

    /// Starts one regular ownership attempt and supersedes every older image reservation for
    /// this seat.
    ///
    /// This runs before validation on every new regular text claim, compatibility text store and
    /// image reservation. A failed attempt therefore still prevents an older asynchronous encode
    /// from installing content later. The currently installed source is deliberately untouched.
    fn begin_regular_ownership_attempt_at(&mut self, position: usize) {
        let seat = &mut self.seats[position];
        seat.regular_ownership_epoch = seat.regular_ownership_epoch.wrapping_add(1);
        let seat_token = seat.seat_token;
        self.image_reservations.retain(|reservation| reservation.seat_token != seat_token);
    }

    fn begin_regular_ownership_attempt_for_seat(&mut self, seat_token: u64) -> bool {
        let Some(position) = self.seats.iter().position(|entry| entry.seat_token == seat_token)
        else {
            return false;
        };
        self.begin_regular_ownership_attempt_at(position);
        true
    }

    /// Admits one new source payload against the hard lifetime-retained source-byte budget.
    ///
    /// The installed source being replaced, a previous source held for rollback, and any serving
    /// writer clones all remain charged during this admission. There is no replacement discount:
    /// the new payload must fit beside them, or the claim is refused before replacing ownership.
    ///
    /// Runs under the clipboard lock and before `set_selection` or `OwnedSource` construction.
    /// The returned charge is dropped automatically if a later validation fails.
    fn admit_source_payload(&self, new_bytes: usize) -> io::Result<Arc<SourceBytesCharge>> {
        let charge = SourceBytesCharge::try_new(Arc::clone(&self.source_bytes), new_bytes)?;
        Ok(Arc::new(charge))
    }

    fn release_image_reservation(&mut self, id: u64) {
        self.image_reservations.retain(|reservation| reservation.id != id);
    }

    fn allocate_source_identity(&mut self) -> (u64, String) {
        let token = NEXT_SOURCE_TOKEN.fetch_add(1, Ordering::Relaxed);
        self.last_source_token = self.last_source_token.max(token);
        (token, source_marker(&self.marker_identity, token))
    }

    fn mark_source_token(&mut self, token: u64) {
        self.last_source_token = self.last_source_token.max(token);
    }

    fn marker_is_stale(&self, marker: &str, current_token: Option<u64>) -> bool {
        let Some(token) = self.marker_identity.token(marker) else {
            return false;
        };
        match current_token {
            Some(current) => token < current,
            None => token <= self.last_source_token,
        }
    }

    fn add_seat_entry(
        &mut self,
        seat_id: ObjectId,
        device: Option<WlDataDevice>,
        primary_device: Option<PrimarySelectionDevice>,
    ) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        if let Some(position) = self.seats.iter().position(|entry| entry.seat == seat_id) {
            let mut previous = self.seats.remove(position);
            if self.active_seat == Some(previous.seat_token) {
                self.active_seat = None;
            }
            self.release_seat_state(&mut previous, &mut batch);
        }
        let seat_token = self.next_seat_token;
        self.next_seat_token = self.next_seat_token.wrapping_add(1).max(1);
        self.seats.push(SeatClipboard {
            seat: seat_id,
            seat_token,
            device,
            serial: None,
            input: SeatInputState::default(),
            regular_ownership_epoch: 0,
            regular_offer: None,
            suppressed_regular_offer: None,
            regular: SeatFamily::default(),
            primary_device,
            primary_offer: None,
            primary_raw_offer: None,
            suppressed_primary_offer: None,
            primary: SeatFamily::default(),
        });
        batch
    }

    fn release_seat_state(&mut self, entry: &mut SeatClipboard, batch: &mut TransitionBatch) {
        self.image_reservations.retain(|reservation| reservation.seat_token != entry.seat_token);
        if let Some(owned) = entry.regular.owned.take() {
            drop(owned);
        }
        if let Some(owned) = entry.primary.owned.take() {
            drop(owned);
        }
        entry.regular_offer = None;
        entry.suppressed_regular_offer = None;
        entry.primary_offer = None;
        entry.primary_raw_offer = None;
        entry.suppressed_primary_offer = None;
        self.transfers.interrupt_seat(
            entry.seat_token,
            SelectionKind::Regular,
            &mut batch.interrupted,
        );
        self.transfers.interrupt_seat(
            entry.seat_token,
            SelectionKind::Primary,
            &mut batch.interrupted,
        );
    }

    fn remove_seat(&mut self, seat: &ObjectId) -> TransitionBatch {
        if let Some(position) = self.seats.iter().position(|entry| &entry.seat == seat) {
            self.remove_seat_at(position)
        } else {
            TransitionBatch::default()
        }
    }

    fn remove_seat_at(&mut self, position: usize) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        let mut entry = self.seats.remove(position);
        if self.active_seat == Some(entry.seat_token) {
            self.active_seat = None;
        }
        self.release_seat_state(&mut entry, &mut batch);
        batch
    }

    fn remove_window(&mut self, window: WindowId) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        self.windows.remove(&window);
        self.transfers.interrupt_target(window, &mut batch.interrupted);

        for position in 0..self.seats.len() {
            if self.seats[position].input.focused_window == Some(window) {
                // Closing/hiding the focused window invalidates a pending image reservation even
                // if no owned source or read is bound to it. The installed source is untouched.
                self.begin_regular_ownership_attempt_at(position);
            }
            if self.seats[position]
                .regular
                .owned
                .as_ref()
                .is_some_and(|owned| owned.target_window == window)
            {
                // A pending claim has not replaced the compositor's selection yet. If the
                // logical offer is still the claim's predecessor, dropping our not-yet-echoed
                // source must not tombstone that predecessor: the compositor may keep sending
                // the same protocol object, and suppressing it would make the still-current
                // selection unreadable. An echoed source, or an offer that is no longer the
                // predecessor, is still suppressed because it can only be a stale clone.
                let pending_predecessor =
                    match self.seats[position].regular.owned.as_ref().map(|owned| &owned.phase) {
                        Some(SourcePhase::Pending { predecessor: Some(predecessor), .. }) => {
                            Some(*predecessor)
                        },
                        _ => None,
                    };
                let preserve_predecessor = pending_predecessor.is_some_and(|predecessor| {
                    self.seats[position]
                        .regular
                        .offer
                        .as_ref()
                        .is_some_and(|offer| offer.key == predecessor)
                });

                if let Some(owned) = self.seats[position].regular.owned.take() {
                    // Dropping the owned source cancels its serving jobs and destroys the
                    // protocol source. Legacy sources use `WindowId::dummy()` and stay.
                    drop(owned);
                }
                if preserve_predecessor {
                    self.seats[position].suppressed_regular_offer = None;
                } else {
                    self.seats[position].regular.offer = None;
                    let suppressed =
                        self.seats[position].regular_offer.as_ref().map(|offer| offer.inner().id());
                    self.seats[position].suppressed_regular_offer = suppressed;
                    self.seats[position].regular_offer = None;
                }
                let seat_token = self.seats[position].seat_token;
                self.transfers.interrupt_seat(
                    seat_token,
                    SelectionKind::Regular,
                    &mut batch.interrupted,
                );
            }

            if self.seats[position]
                .primary
                .owned
                .as_ref()
                .is_some_and(|owned| owned.target_window == window)
            {
                let pending_predecessor =
                    match self.seats[position].primary.owned.as_ref().map(|owned| &owned.phase) {
                        Some(SourcePhase::Pending { predecessor: Some(predecessor), .. }) => {
                            Some(*predecessor)
                        },
                        _ => None,
                    };
                let preserve_predecessor = pending_predecessor.is_some_and(|predecessor| {
                    self.seats[position]
                        .primary
                        .offer
                        .as_ref()
                        .is_some_and(|offer| offer.key == predecessor)
                });

                if let Some(owned) = self.seats[position].primary.owned.take() {
                    drop(owned);
                }
                if preserve_predecessor {
                    self.seats[position].suppressed_primary_offer = None;
                } else {
                    self.seats[position].primary.offer = None;
                    let suppressed = self.seats[position].primary_offer.take();
                    self.seats[position].primary_raw_offer = None;
                    self.seats[position].suppressed_primary_offer = suppressed;
                }
                let seat_token = self.seats[position].seat_token;
                self.transfers.interrupt_seat(
                    seat_token,
                    SelectionKind::Primary,
                    &mut batch.interrupted,
                );
            }
        }
        batch
    }

    fn add_window(&mut self, window: WindowId) {
        self.windows.insert(window);
    }

    fn keyboard_focus_enter(&mut self, seat: &ObjectId, serial: u32, window: WindowId) {
        if let Some(position) = self.seats.iter().position(|entry| &entry.seat == seat) {
            self.keyboard_focus_enter_at(position, serial, window);
        }
    }

    fn keyboard_focus_enter_at(&mut self, position: usize, serial: u32, window: WindowId) {
        // Focus entry is a new ownership/input context for an async image copy too.
        self.begin_regular_ownership_attempt_at(position);
        let entry = &mut self.seats[position];
        entry.serial = Some(serial);
        entry.input.enter(serial, window, Instant::now());
        let seat_token = entry.seat_token;
        self.set_active_seat(seat_token);
    }

    fn keyboard_focus_leave(&mut self, seat: &ObjectId) -> TransitionBatch {
        match self.seats.iter().position(|entry| &entry.seat == seat) {
            Some(position) => self.keyboard_focus_leave_at(position),
            None => TransitionBatch::default(),
        }
    }

    /// Clears focus and interrupts every live transfer for both families of this seat.
    ///
    /// Focus loss changes the focus epoch, so a result completed just before it must not escape
    /// through `try_take`; the returned batch must be completed after the clipboard lock is
    /// released.
    fn keyboard_focus_leave_at(&mut self, position: usize) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        {
            let entry = &mut self.seats[position];
            entry.input.leave();
            // The compatibility API must not reuse a serial from a focus epoch that ended.
            entry.serial = None;
        }
        // A pending asynchronous image copy belongs to the focus it was reserved in.
        self.begin_regular_ownership_attempt_at(position);
        let seat_token = self.seats[position].seat_token;
        self.transfers.interrupt_seat(seat_token, SelectionKind::Regular, &mut batch.interrupted);
        self.transfers.interrupt_seat(seat_token, SelectionKind::Primary, &mut batch.interrupted);
        batch
    }

    fn keyboard_key(&mut self, seat: &ObjectId, serial: u32, window: WindowId) {
        if let Some(position) = self.seats.iter().position(|entry| &entry.seat == seat) {
            self.keyboard_key_at(position, serial, window);
        }
    }

    fn keyboard_key_at(&mut self, position: usize, serial: u32, window: WindowId) {
        let entry = &mut self.seats[position];
        entry.serial = Some(serial);
        entry.input.key(serial, window, Instant::now());
        let seat_token = entry.seat_token;
        self.set_active_seat(seat_token);
    }

    fn pointer_input(&mut self, seat: &ObjectId, serial: u32, window: WindowId, pressed: bool) {
        if let Some(position) = self.seats.iter().position(|entry| &entry.seat == seat) {
            self.pointer_input_at(position, serial, window, pressed);
        }
    }

    fn pointer_input_at(&mut self, position: usize, serial: u32, window: WindowId, pressed: bool) {
        let entry = &mut self.seats[position];
        entry.serial = Some(serial);
        entry.input.pointer(serial, window, pressed, Instant::now());
        let seat_token = entry.seat_token;
        self.set_active_seat(seat_token);
    }

    /// The seat that last gave input and still owns keyboard focus on `target`.
    ///
    /// A seat merely having focus is not enough: with two seats focused on the same window only
    /// the input-triggering seat may route a claim or read. We deliberately do not scan the seat
    /// list for another focused seat, because insertion order is not evidence of which input
    /// asked for the selection. The no-active and stale-active cases fail instead of falling back.
    fn active_focused_seat(&self, target: WindowId) -> Result<&SeatClipboard, ClipboardError> {
        let Some(active) = self.active_seat else {
            return Err(ClipboardError::InvalidInput);
        };
        let seat = self
            .seats
            .iter()
            .find(|entry| entry.seat_token == active)
            .ok_or(ClipboardError::InvalidInput)?;
        if !seat.input.focuses(target) {
            return Err(ClipboardError::InvalidInput);
        }
        Ok(seat)
    }

    fn input_origin(&self, target: WindowId) -> io::Result<InputOriginToken> {
        self.input_origin_typed(target).map_err(ClipboardError::into_io_error)
    }

    fn input_origin_typed(&self, target: WindowId) -> Result<InputOriginToken, ClipboardError> {
        if !self.windows.contains(&target) {
            return Err(ClipboardError::InvalidInput);
        }
        let seat = self.active_focused_seat(target)?;
        seat.input
            .current_origin(seat.seat_token, target, Instant::now())
            .ok_or(ClipboardError::InvalidInput)
    }

    /// Resolves the one seat and text offer snapshot an asynchronous text read will use.
    #[cfg(test)]
    fn read_route(
        &self,
        target: WindowId,
        kind: SelectionKind,
    ) -> Result<ReadRoute, ClipboardError> {
        self.read_route_with(target, kind, ReadPreference::TextOnly)
    }

    /// Resolves the one seat, focus epoch and offer snapshot an asynchronous read will use.
    ///
    /// The returned [`FrozenRoute`] is what admission freezes and `try_take` revalidates. No
    /// second begin call, offer or seat is considered later.
    fn read_route_with(
        &self,
        target: WindowId,
        kind: SelectionKind,
        preference: ReadPreference,
    ) -> Result<ReadRoute, ClipboardError> {
        if !self.windows.contains(&target) {
            return Err(ClipboardError::InvalidInput);
        }
        let seat = self.active_focused_seat(target)?;
        let read = seat.family(kind).classify_with(kind, &self.marker_identity, preference)?;
        let offer = match kind {
            SelectionKind::Regular => seat.regular_offer.clone().map(SelectionOfferKind::Regular),
            SelectionKind::Primary => seat
                .primary_offer
                .as_ref()
                .map(|_| SelectionOfferKind::Primary { raw: seat.primary_receive_offer() }),
        };
        let (expected_offer, expected_source) = match &read {
            OfferRead::SelfContent(_) => {
                (None, seat.family(kind).owned.as_ref().map(|owned| owned.token))
            },
            OfferRead::Request(_) | OfferRead::RequestImage(_) => {
                (seat.family(kind).offer.as_ref().map(|offer| offer.key), None)
            },
        };
        let frozen = FrozenRoute {
            target_window: Some(target),
            seat_token: seat.seat_token,
            selection_kind: kind,
            expected_offer,
            expected_source,
            focus_epoch: seat.input.epoch,
            active_seat_epoch: self.active_seat_epoch,
            check_focus: true,
        };
        Ok(ReadRoute { read, offer, frozen })
    }

    /// Resolves the active seat's offer for the blocking compatibility text path.
    fn read_route_legacy(&self, kind: SelectionKind) -> Result<ReadRoute, ClipboardError> {
        let seat = self.active_seat().ok_or(ClipboardError::Absent)?;
        let read = seat.family(kind).classify(kind, &self.marker_identity)?;
        let offer = match kind {
            SelectionKind::Regular => seat.regular_offer.clone().map(SelectionOfferKind::Regular),
            SelectionKind::Primary => seat
                .primary_offer
                .as_ref()
                .map(|_| SelectionOfferKind::Primary { raw: seat.primary_receive_offer() }),
        };
        let (expected_offer, expected_source) = match &read {
            OfferRead::SelfContent(_) => {
                (None, seat.family(kind).owned.as_ref().map(|owned| owned.token))
            },
            OfferRead::Request(_) | OfferRead::RequestImage(_) => {
                (seat.family(kind).offer.as_ref().map(|offer| offer.key), None)
            },
        };
        let frozen = FrozenRoute {
            target_window: None,
            seat_token: seat.seat_token,
            selection_kind: kind,
            expected_offer,
            expected_source,
            focus_epoch: seat.input.epoch,
            active_seat_epoch: self.active_seat_epoch,
            check_focus: false,
        };
        Ok(ReadRoute { read, offer, frozen })
    }

    /// Validates a frozen route against the current windows, seat, focus epoch, active-seat
    /// epoch and offer/source.
    fn frozen_route_is_current(&self, route: &FrozenRoute) -> bool {
        if let Some(target) = route.target_window {
            if !self.windows.contains(&target) {
                return false;
            }
        }
        let Some(seat) = self.seat_by_token(route.seat_token) else {
            return false;
        };
        if route.check_focus {
            let Some(target) = route.target_window else {
                return false;
            };
            if !seat.input.focuses(target) || seat.input.epoch != route.focus_epoch {
                return false;
            }
            // A focus-bound read belongs to the active input seat, not merely to a seat that is
            // still focused on the window. The epoch also rejects A-B-A seat movement.
            if self.active_seat != Some(route.seat_token)
                || self.active_seat_epoch != route.active_seat_epoch
            {
                return false;
            }
        }
        if let Some(expected) = route.expected_offer {
            if seat.family(route.selection_kind).offer.as_ref().map(|offer| offer.key)
                != Some(expected)
            {
                return false;
            }
        }
        if let Some(expected) = route.expected_source {
            if seat.family(route.selection_kind).owned.as_ref().map(|owned| owned.token)
                != Some(expected)
            {
                return false;
            }
        }
        true
    }

    fn seat_by_token(&self, token: u64) -> Option<&SeatClipboard> {
        self.seats.iter().find(|entry| entry.seat_token == token)
    }

    /// Validates and consumes one regular trigger for asynchronous PNG encoding.
    ///
    /// The returned route deliberately retains the triggering serial instead of resolving the
    /// latest input later. Every new regular ownership attempt advances the per-seat epoch first,
    /// so even an attempt that fails below supersedes older reservations.
    fn reserve_image_claim_at(
        &mut self,
        claim: &SelectionClaim,
        now: Instant,
    ) -> Result<ImageReservationInner, ClipboardError> {
        let position = self
            .seats
            .iter()
            .position(|entry| entry.seat_token == claim.trigger.inner.seat_token)
            .ok_or(ClipboardError::InvalidInput)?;
        // This attempt invalidates older reservations before any of its own validation.
        self.begin_regular_ownership_attempt_at(position);

        if !self.windows.contains(&claim.target_window) {
            return Err(ClipboardError::InvalidInput);
        }
        if self.active_seat != Some(self.seats[position].seat_token) {
            return Err(ClipboardError::InvalidInput);
        }
        if !self.seats[position].input.validates(
            &claim.trigger.inner,
            claim.target_window,
            SelectionKind::Regular,
            now,
        ) {
            return Err(ClipboardError::InvalidInput);
        }
        // Consume the regular family now. The asynchronous completion never looks up another
        // trigger, and a later PRIMARY claim can still use the same input independently.
        if !self.seats[position].input.consume(&claim.trigger.inner, SelectionKind::Regular) {
            return Err(ClipboardError::InvalidInput);
        }

        // Bound the reservation by the original trigger's freshness window, not by the time the
        // reservation happens to be made. A token accepted near the end of that window must not
        // let a deferred claim use a serial older than five seconds.
        let deadline = claim
            .trigger
            .inner
            .at
            .checked_add(INPUT_ORIGIN_MAX_AGE)
            .ok_or(ClipboardError::InvalidInput)?;
        let id = self.next_image_reservation_id;
        self.next_image_reservation_id = self.next_image_reservation_id.wrapping_add(1).max(1);
        let seat = &self.seats[position];
        let route = ImageReservationInner {
            id,
            target_window: claim.target_window,
            caller_generation: claim.caller_generation,
            seat_token: seat.seat_token,
            serial: claim.trigger.inner.serial,
            focus_epoch: seat.input.epoch,
            active_seat_epoch: self.active_seat_epoch,
            ownership_epoch: seat.regular_ownership_epoch,
            deadline,
            validation: Weak::new(),
        };
        self.image_reservations.push(PendingImageReservation {
            id,
            target_window: claim.target_window,
            seat_token: route.seat_token,
        });
        Ok(route)
    }

    /// Validates all reservation fences except the latest input serial.
    ///
    /// Pointer/keyboard input after reservation may replace the latest serial, but the reserved
    /// original serial remains valid while its focus and ownership epochs are current. Focus,
    /// window, active-seat or ownership changes, expiry, or another regular attempt reject it.
    fn validate_image_reservation_at(
        &self,
        reservation: &ImageReservationInner,
        now: Instant,
    ) -> Result<(), ClipboardError> {
        if !self.image_reservations.iter().any(|record| {
            record.id == reservation.id
                && record.target_window == reservation.target_window
                && record.seat_token == reservation.seat_token
        }) {
            return Err(ClipboardError::Interrupted);
        }
        if !self.windows.contains(&reservation.target_window) {
            return Err(ClipboardError::Interrupted);
        }
        let seat = self.seat_by_token(reservation.seat_token).ok_or(ClipboardError::Interrupted)?;
        if !seat.input.focuses(reservation.target_window)
            || seat.input.epoch != reservation.focus_epoch
            || self.active_seat != Some(reservation.seat_token)
            || self.active_seat_epoch != reservation.active_seat_epoch
            || seat.regular_ownership_epoch != reservation.ownership_epoch
        {
            return Err(ClipboardError::Interrupted);
        }
        if now >= reservation.deadline {
            return Err(ClipboardError::TimedOut);
        }
        Ok(())
    }

    /// Supersedes every image reservation for the active focused seat of `target_window`.
    ///
    /// This is the event-loop invalidation entry point used by a winning regular output before it
    /// resolves an origin. It does not require a claimable input serial, does not interrupt
    /// reads, and never clears the installed source.
    fn invalidate_image_reservations_at(
        &mut self,
        target_window: WindowId,
    ) -> Result<(), ClipboardError> {
        if !self.windows.contains(&target_window) {
            return Err(ClipboardError::InvalidInput);
        }
        let Some(active) = self.active_seat else {
            return Ok(());
        };
        let Some(position) = self.seats.iter().position(|entry| entry.seat_token == active) else {
            return Ok(());
        };
        if !self.seats[position].input.focuses(target_window) {
            return Ok(());
        }
        self.begin_regular_ownership_attempt_at(position);
        Ok(())
    }

    fn admit_reserved_image_claim(
        &mut self,
        install: ReservedImageInstall,
        reservation: ImageReservationInner,
        now: Instant,
        batch: &mut TransitionBatch,
    ) -> Result<InstalledClaim, ClipboardError> {
        self.validate_image_reservation_at(&reservation, now)?;
        let ReservedImageInstall { token, marker, png, target_window, protocol } = install;
        if target_window != reservation.target_window {
            return Err(ClipboardError::InvalidInput);
        }
        let position = self
            .seats
            .iter()
            .position(|entry| entry.seat_token == reservation.seat_token)
            .ok_or(ClipboardError::Interrupted)?;
        if let Some(protocol) = &protocol {
            let fits = match protocol {
                ProtocolSource::Regular(_) => self.seats[position].device.is_some(),
                ProtocolSource::Primary(_) => false,
            };
            if !fits {
                return Err(ClipboardError::Unsupported);
            }
        }

        let source_bytes = self.admit_source_payload(png.len()).map_err(typed_clipboard_error)?;
        self.mark_source_token(token);

        let predecessor = self.seats[position].regular.offer.as_ref().map(|offer| offer.key);
        if let Some(protocol) = &protocol {
            match protocol {
                ProtocolSource::Regular(source) => {
                    let device =
                        self.seats[position].device.as_ref().expect("the device was checked above");
                    // Use the serial captured by the reservation, not a later input event.
                    device.set_selection(Some(source), reservation.serial);
                },
                ProtocolSource::Primary(_) => unreachable!("an image claim is regular only"),
            }
        }

        let owned = OwnedSource {
            token,
            marker,
            payload: OwnedPayload::Png(png),
            source_bytes,
            selection_serial: reservation.serial,
            target_window,
            caller_generation: reservation.caller_generation,
            serving: Arc::new(SourceServeState::default()),
            phase: SourcePhase::Pending { predecessor, since: now },
            protocol,
        };
        let previous = self.seats[position].regular.owned.replace(owned);
        self.release_image_reservation(reservation.id);
        let seat_token = self.seats[position].seat_token;
        self.transfers.interrupt_seat(seat_token, SelectionKind::Regular, &mut batch.interrupted);
        Ok(InstalledClaim { token, previous })
    }

    fn admit_claim(
        &mut self,
        install: ClaimInstall,
        claim: &SelectionClaim,
        now: Instant,
        batch: &mut TransitionBatch,
    ) -> io::Result<InstalledClaim> {
        let ClaimInstall {
            kind,
            token,
            marker,
            payload,
            target_window,
            caller_generation,
            protocol,
        } = install;
        // A new regular attempt supersedes an older asynchronous image reservation even if this
        // claim is stale or fails below. PRIMARY does not touch the regular ownership epoch.
        if kind == SelectionKind::Regular {
            self.begin_regular_ownership_attempt_for_seat(claim.trigger.inner.seat_token);
        }
        if target_window != claim.target_window {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "the claim target does not match its request",
            ));
        }
        if !self.windows.contains(&claim.target_window) {
            return Err(io::Error::new(ErrorKind::InvalidInput, "the target window is not live"));
        }
        let position = self
            .seats
            .iter()
            .position(|entry| entry.seat_token == claim.trigger.inner.seat_token)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "the trigger seat is gone"))?;
        // The token may have been obtained before another seat received input. Admission must
        // follow the current input-triggering seat, not merely a seat that still happens to be
        // focused and has a retained serial.
        if self.active_seat != Some(self.seats[position].seat_token) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "the trigger seat is no longer the active input seat",
            ));
        }
        if !self.seats[position].input.validates(
            &claim.trigger.inner,
            claim.target_window,
            kind,
            now,
        ) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "the selection trigger is stale, already used, or belongs to another focus owner",
            ));
        }
        if let Some(protocol) = &protocol {
            let fits = match (kind, protocol) {
                (SelectionKind::Regular, ProtocolSource::Regular(_)) => {
                    self.seats[position].device.is_some()
                },
                (SelectionKind::Primary, ProtocolSource::Primary(_)) => {
                    self.seats[position].primary_device.is_some()
                },
                _ => false,
            };
            if !fits {
                return Err(io::Error::new(ErrorKind::Unsupported, match kind {
                    SelectionKind::Regular => "no data device for the seat",
                    SelectionKind::Primary => "no primary selection device for the seat",
                }));
            }
        }

        let source_bytes = self.admit_source_payload(payload.len())?;

        self.mark_source_token(token);

        // Single-use per family. This runs in every build, before any source is replaced.
        if !self.seats[position].input.consume(&claim.trigger.inner, kind) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "this input already claimed that selection family",
            ));
        }

        let predecessor = self.seats[position].family(kind).offer.as_ref().map(|offer| offer.key);
        let serial = claim.trigger.inner.serial;
        if let Some(protocol) = &protocol {
            match (kind, protocol) {
                (SelectionKind::Regular, ProtocolSource::Regular(source)) => {
                    let device =
                        self.seats[position].device.as_ref().expect("the device was checked above");
                    device.set_selection(Some(source), serial);
                },
                (SelectionKind::Primary, ProtocolSource::Primary(source)) => {
                    let device = self.seats[position]
                        .primary_device
                        .as_ref()
                        .expect("the primary device was checked above");
                    source.set_selection(device, serial);
                },
                _ => unreachable!("the protocol family was checked above"),
            }
        }

        let owned = OwnedSource {
            token,
            marker,
            payload,
            source_bytes,
            selection_serial: serial,
            target_window,
            caller_generation,
            serving: Arc::new(SourceServeState::default()),
            phase: SourcePhase::Pending { predecessor, since: now },
            protocol,
        };
        let previous = self.seats[position].family_mut(kind).owned.replace(owned);
        let seat_token = self.seats[position].seat_token;
        self.transfers.interrupt_seat(seat_token, kind, &mut batch.interrupted);
        Ok(InstalledClaim { token, previous })
    }

    /// Rolls back a claim whose `set_selection` could not be queued on the connection.
    ///
    /// The new protocol source is dropped deliberately. If the claim had replaced an owned
    /// source, restore that previous source so a fatal flush does not silently destroy both.
    /// Returns a previous source that could not be restored because another claim won the race;
    /// the caller drops it outside the clipboard lock.
    fn rollback_claim(
        &mut self,
        kind: SelectionKind,
        token: u64,
        previous: Option<OwnedSource>,
    ) -> Option<OwnedSource> {
        let Some(position) = self.seats.iter().position(|seat| {
            seat.family(kind).owned.as_ref().is_some_and(|owned| owned.token == token)
        }) else {
            return previous;
        };
        let failed = self.seats[position].family_mut(kind).owned.take();
        drop(failed);
        if let Some(previous) = previous {
            self.seats[position].family_mut(kind).owned = Some(previous);
            None
        } else {
            None
        }
    }

    fn install_legacy_source(
        &mut self,
        kind: SelectionKind,
        text: Arc<str>,
        token: u64,
        marker: String,
        protocol: Option<ProtocolSource>,
        batch: &mut TransitionBatch,
    ) -> io::Result<Option<OwnedSource>> {
        let Some(position) =
            self.seats.iter().position(|entry| self.active_seat == Some(entry.seat_token))
        else {
            return Err(io::Error::new(ErrorKind::NotFound, "no seat"));
        };
        // Compatibility regular text stores are ownership attempts too: a new one must
        // supersede an older async image reservation even when it fails before installing.
        if kind == SelectionKind::Regular {
            self.begin_regular_ownership_attempt_at(position);
        }
        let serial = self.seats[position]
            .serial
            .ok_or_else(|| io::Error::new(ErrorKind::Other, "no input to set the selection for"))?;
        if let Some(protocol) = &protocol {
            let fits = match (kind, protocol) {
                (SelectionKind::Regular, ProtocolSource::Regular(_)) => {
                    self.seats[position].device.is_some()
                },
                (SelectionKind::Primary, ProtocolSource::Primary(_)) => {
                    self.seats[position].primary_device.is_some()
                },
                _ => false,
            };
            if !fits {
                return Err(io::Error::new(ErrorKind::Unsupported, match kind {
                    SelectionKind::Regular => "no data device for the seat",
                    SelectionKind::Primary => "no primary selection device for the seat",
                }));
            }
        }
        self.mark_source_token(token);
        let source_bytes = self.admit_source_payload(text.len())?;

        if let Some(protocol) = &protocol {
            match (kind, protocol) {
                (SelectionKind::Regular, ProtocolSource::Regular(source)) => {
                    let device =
                        self.seats[position].device.as_ref().expect("the device was checked above");
                    device.set_selection(Some(source), serial);
                },
                (SelectionKind::Primary, ProtocolSource::Primary(source)) => {
                    let device = self.seats[position]
                        .primary_device
                        .as_ref()
                        .expect("the primary device was checked above");
                    source.set_selection(device, serial);
                },
                _ => unreachable!("the protocol family was checked above"),
            }
        }

        let predecessor = self.seats[position].family(kind).offer.as_ref().map(|offer| offer.key);
        let owned = OwnedSource {
            token,
            marker,
            payload: OwnedPayload::Text(text),
            source_bytes,
            selection_serial: serial,
            target_window: WindowId::dummy(),
            caller_generation: 0,
            serving: Arc::new(SourceServeState::default()),
            phase: SourcePhase::Pending { predecessor, since: Instant::now() },
            protocol,
        };
        let previous = self.seats[position].family_mut(kind).owned.replace(owned);
        let seat_token = self.seats[position].seat_token;
        self.transfers.interrupt_seat(seat_token, kind, &mut batch.interrupted);
        Ok(previous)
    }

    /// Ends a pending claim that the suppressed predecessor proves was never echoed.
    ///
    /// The predecessor itself stays suppressed: cancellation invalidated it, so an expired claim
    /// becomes absent rather than readable as the old owner's text.
    fn expire_suppressed_pending(
        &mut self,
        position: usize,
        kind: SelectionKind,
        now: Instant,
        batch: &mut TransitionBatch,
    ) {
        let expired = matches!(
            self.seats[position].family(kind).owned.as_ref().map(|owned| &owned.phase),
            Some(SourcePhase::Pending { since, .. })
                if now.saturating_duration_since(*since) > CLAIM_ECHO_GRACE
        );
        if !expired {
            return;
        }
        let seat_token = self.seats[position].seat_token;
        if let Some(owned) = self.seats[position].family_mut(kind).owned.take() {
            drop(owned);
        }
        self.transfers.interrupt_seat(seat_token, kind, &mut batch.interrupted);
    }

    /// Applies one offer snapshot to the lifecycle state.
    fn observe_selection(
        &mut self,
        position: usize,
        kind: SelectionKind,
        observed: ObservedOffer,
        now: Instant,
        batch: &mut TransitionBatch,
    ) {
        let stored_key = self.seats[position].family(kind).offer.as_ref().map(|offer| offer.key);
        let marker = observed.marker().map(str::to_owned);
        let owned_token = self.seats[position].family(kind).owned.as_ref().map(|owned| owned.token);
        let owned_marker =
            self.seats[position].family(kind).owned.as_ref().map(|owned| owned.marker.clone());
        let is_current = marker.is_some() && marker == owned_marker;
        let is_stale_own =
            marker.as_deref().is_some_and(|marker| self.marker_is_stale(marker, owned_token));

        let pending =
            match self.seats[position].family(kind).owned.as_ref().map(|owned| &owned.phase) {
                Some(SourcePhase::Pending { predecessor, since }) => Some((*predecessor, *since)),
                _ => None,
            };

        // A stale announce of one of this clipboard's older sources must not destroy the
        // current source. While a claim is pending it can still be the pre-claim selection the
        // compositor has not replaced yet; after the grace period it is authoritative evidence
        // that the claim was not echoed. Monotonic source tokens make this independent of any
        // fixed-size retirement list.
        if is_stale_own && owned_marker.is_some() {
            match pending {
                Some((_, since)) if now.saturating_duration_since(since) <= CLAIM_ECHO_GRACE => {
                    self.seats[position].family_mut(kind).offer = observed.to_view(marker);
                    return;
                },
                Some(_) => {},
                // An echoed current source ignores an older announcement.
                None => return,
            }
        }

        let mut authoritative = false;
        if let Some((predecessor, since)) = pending {
            let is_replay = observed.key == predecessor
                && now.saturating_duration_since(since) <= CLAIM_ECHO_GRACE;
            if is_replay && !is_current {
                // The compositor has not echoed the claim yet. Refresh the offer snapshot but
                // keep the pending source and do not cancel a read started after the claim.
                self.seats[position].family_mut(kind).offer = observed.to_view(marker);
                return;
            }
            // An unconfirmed claim older than the grace period, or an event for another offer, is
            // authoritative: it decides whether this source is still ours.
            authoritative = true;
        }

        let duplicate = match (stored_key, observed.key) {
            (None, None) => true,
            (Some(stored), Some(seen)) => stored == seen,
            _ => false,
        };
        if !authoritative && duplicate {
            self.seats[position].family_mut(kind).offer = observed.to_view(marker);
            return;
        }

        let seat_token = self.seats[position].seat_token;
        self.seats[position].family_mut(kind).offer = observed.to_view(marker);
        // Every changed owner cancels in-flight reads, including a change to our own source.
        self.transfers.interrupt_seat(seat_token, kind, &mut batch.interrupted);

        if is_current {
            if let Some(owned) = self.seats[position].family_mut(kind).owned.as_mut() {
                owned.phase = SourcePhase::Echoed;
            }
        } else if let Some(owned) = self.seats[position].family_mut(kind).owned.take() {
            // Another owner (or an authoritative empty selection) replaced us. The source and
            // its serving jobs end with the owned text.
            drop(owned);
        }
    }

    fn apply_regular_offer_at(
        &mut self,
        position: usize,
        offer: Option<SelectionOffer>,
        now: Instant,
    ) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        if let Some(suppressed) = self.seats[position].suppressed_regular_offer.clone() {
            if offer.as_ref().is_some_and(|offer| offer.inner().id() == suppressed) {
                self.seats[position].regular_offer = None;
                self.seats[position].regular.offer = None;
                self.expire_suppressed_pending(position, SelectionKind::Regular, now, &mut batch);
                return batch;
            }
            self.seats[position].suppressed_regular_offer = None;
        }
        let same_object = match (&self.seats[position].regular_offer, &offer) {
            (Some(old), Some(new)) => old.inner().id() == new.inner().id(),
            (None, None) => true,
            _ => false,
        };
        let stored_key = self.seats[position].regular.offer.as_ref().map(|view| view.key);
        let pending = matches!(
            self.seats[position].regular.owned.as_ref().map(|owned| &owned.phase),
            Some(SourcePhase::Pending { .. })
        );
        if same_object && stored_key.is_some() && !pending {
            // The same protocol offer was observed again (the per-turn reconciliation): its mime
            // types and lifecycle state cannot have changed. A pending claim still observes the
            // repeated predecessor so its grace period can expire.
            return TransitionBatch::default();
        }
        let key = if same_object {
            stored_key
        } else {
            offer.as_ref().map(|_| self.allocate_offer_key())
        };
        let mime_types = offer
            .as_ref()
            .map(|offer| offer.with_mime_types(|types| types.to_vec()))
            .unwrap_or_default();
        self.seats[position].regular_offer = offer;
        self.observe_selection(
            position,
            SelectionKind::Regular,
            ObservedOffer { key, mime_types },
            now,
            &mut batch,
        );
        batch
    }

    fn apply_primary_offer_at(
        &mut self,
        position: usize,
        offer: Option<PrimarySelectionOffer>,
        raw: PrimaryRawOfferUpdate,
        now: Instant,
    ) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        if let Some(suppressed) = self.seats[position].suppressed_primary_offer.clone() {
            if offer.as_ref() == Some(&suppressed) {
                self.seats[position].primary_offer = None;
                self.seats[position].primary_raw_offer = None;
                self.seats[position].primary.offer = None;
                self.expire_suppressed_pending(position, SelectionKind::Primary, now, &mut batch);
                return batch;
            }
            self.seats[position].suppressed_primary_offer = None;
        }
        let same_object = match (&self.seats[position].primary_offer, &offer) {
            (Some(old), Some(new)) => old == new,
            (None, None) => true,
            _ => false,
        };
        // Pair the raw proxy with the wrapper in the same state transition. A same-object
        // observation still refreshes the clone so the side table tracks the latest dispatch
        // handle. Reconciliation has no event-provided raw offer, so it fails closed instead of
        // bracketing a new wrapper with an old proxy.
        self.seats[position].update_primary_raw_offer(same_object, offer.is_some(), raw);
        let stored_key = self.seats[position].primary.offer.as_ref().map(|view| view.key);
        let pending = matches!(
            self.seats[position].primary.owned.as_ref().map(|owned| &owned.phase),
            Some(SourcePhase::Pending { .. })
        );
        if same_object && stored_key.is_some() && !pending {
            return TransitionBatch::default();
        }
        let key = if same_object {
            stored_key
        } else {
            offer.as_ref().map(|_| self.allocate_offer_key())
        };
        let mime_types = offer
            .as_ref()
            .map(|offer| offer.with_mime_types(|types| types.to_vec()))
            .unwrap_or_default();
        self.seats[position].primary_offer = offer;
        self.observe_selection(
            position,
            SelectionKind::Primary,
            ObservedOffer { key, mime_types },
            now,
            &mut batch,
        );
        batch
    }

    fn apply_regular_selection_change(
        &mut self,
        seat: &ObjectId,
        offer: Option<SelectionOffer>,
        now: Instant,
    ) -> TransitionBatch {
        match self.seats.iter().position(|entry| &entry.seat == seat) {
            Some(position) => self.apply_regular_offer_at(position, offer, now),
            None => TransitionBatch::default(),
        }
    }

    fn apply_primary_selection_change(
        &mut self,
        seat: &ObjectId,
        offer: Option<PrimarySelectionOffer>,
        raw: Option<Arc<dyn PrimaryOfferProxy>>,
        now: Instant,
    ) -> TransitionBatch {
        match self.seats.iter().position(|entry| &entry.seat == seat) {
            Some(position) => self.apply_primary_offer_at(
                position,
                offer,
                PrimaryRawOfferUpdate::Replace(raw),
                now,
            ),
            None => TransitionBatch::default(),
        }
    }

    fn reconcile(&mut self) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        let now = Instant::now();
        for position in 0..self.seats.len() {
            let regular = self.seats[position]
                .device
                .as_ref()
                .and_then(|device| device.data::<DataDeviceData>())
                .and_then(|data| data.selection_offer());
            batch.merge(self.apply_regular_offer_at(position, regular, now));
            let primary = self.seats[position]
                .primary_device
                .as_ref()
                .and_then(|device| device.data().selection_offer());
            batch.merge(self.apply_primary_offer_at(
                position,
                primary,
                PrimaryRawOfferUpdate::Reconcile,
                now,
            ));
        }
        batch
    }

    fn regular_serving(&self, source: &WlDataSource) -> Option<ServingSource> {
        self.seats.iter().find_map(|seat| {
            let owned = seat.regular.owned.as_ref()?;
            match owned.protocol.as_ref() {
                Some(ProtocolSource::Regular(protocol)) if protocol == source => {
                    Some((owned.payload.clone(), owned.source_bytes.clone(), owned.serving.clone()))
                },
                _ => None,
            }
        })
    }

    fn primary_serving(&self, source: &ZwpPrimarySelectionSourceV1) -> Option<ServingSource> {
        self.seats.iter().find_map(|seat| {
            let owned = seat.primary.owned.as_ref()?;
            match owned.protocol.as_ref() {
                Some(ProtocolSource::Primary(protocol)) if protocol.inner() == source => {
                    Some((owned.payload.clone(), owned.source_bytes.clone(), owned.serving.clone()))
                },
                _ => None,
            }
        })
    }

    /// Forgets an owned source after the compositor explicitly cancels it.
    ///
    /// The predecessor offer is cleared even while the claim is still pending: the cancellation
    /// has invalidated it, and retaining it would let the next read return the previous owner's
    /// text. The active protocol offer clone is cleared together with the logical view; the same
    /// SCTK object is remembered only as a tombstone so reconciliation cannot revive it.
    fn cancel_owned_source_at(
        &mut self,
        position: usize,
        kind: SelectionKind,
        batch: &mut TransitionBatch,
    ) {
        let seat_token = self.seats[position].seat_token;
        let owned = self.seats[position].family_mut(kind).owned.take();
        let Some(owned) = owned else {
            return;
        };
        {
            let seat = &mut self.seats[position];
            seat.family_mut(kind).offer = None;
            match kind {
                SelectionKind::Regular => {
                    let suppressed = seat.regular_offer.as_ref().map(|offer| offer.inner().id());
                    seat.suppressed_regular_offer = suppressed;
                    seat.regular_offer = None;
                },
                SelectionKind::Primary => {
                    seat.suppressed_primary_offer = seat.primary_offer.take();
                    seat.primary_raw_offer = None;
                },
            }
        }
        self.transfers.interrupt_seat(seat_token, kind, &mut batch.interrupted);
        drop(owned);
    }

    fn regular_source_cancelled(&mut self, source: &WlDataSource) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        let Some(position) = self.seats.iter().position(|seat| {
            seat.regular
                .owned
                .as_ref()
                .and_then(|owned| owned.protocol.as_ref())
                .is_some_and(|protocol| {
                    matches!(protocol, ProtocolSource::Regular(protocol) if protocol == source)
                })
        }) else {
            if source.is_alive() {
                source.destroy();
            }
            return batch;
        };
        self.cancel_owned_source_at(position, SelectionKind::Regular, &mut batch);
        batch
    }

    fn primary_source_cancelled(
        &mut self,
        source: &ZwpPrimarySelectionSourceV1,
    ) -> TransitionBatch {
        let mut batch = TransitionBatch::default();
        let Some(position) = self.seats.iter().position(|seat| {
            seat.primary
                .owned
                .as_ref()
                .and_then(|owned| owned.protocol.as_ref())
                .is_some_and(|protocol| {
                    matches!(protocol, ProtocolSource::Primary(protocol) if protocol.inner() == source)
                })
        }) else {
            if source.is_alive() {
                source.destroy();
            }
            return batch;
        };
        self.cancel_owned_source_at(position, SelectionKind::Primary, &mut batch);
        batch
    }

    fn active_seat(&self) -> Option<&SeatClipboard> {
        self.active_seat.and_then(|token| self.seat_by_token(token))
    }

    #[cfg(test)]
    fn add_test_seat(&mut self) -> u64 {
        let seat_token = self.next_seat_token;
        self.next_seat_token = self.next_seat_token.wrapping_add(1).max(1);
        self.seats.push(SeatClipboard {
            seat: ObjectId::null(),
            seat_token,
            device: None,
            serial: None,
            input: SeatInputState::default(),
            regular_ownership_epoch: 0,
            regular_offer: None,
            suppressed_regular_offer: None,
            regular: SeatFamily::default(),
            primary_device: None,
            primary_offer: None,
            primary_raw_offer: None,
            suppressed_primary_offer: None,
            primary: SeatFamily::default(),
        });
        seat_token
    }
}

/// Checks whether a new payload fits the hard source ledger cap.
///
/// Callers pass the current ledger total while holding the budget lock. There is deliberately no
/// replacement release: the old source stays charged until its owner drops it after the flush.
fn admit_source_bytes(total: usize, new_bytes: usize, limit: usize) -> io::Result<()> {
    if new_bytes > MAX_SOURCE_BYTES {
        return Err(io::Error::new(ErrorKind::WouldBlock, "the selection is too large"));
    }
    let next = total.saturating_add(new_bytes);
    if next > limit {
        return Err(io::Error::new(
            ErrorKind::WouldBlock,
            "the owned selection byte budget is full",
        ));
    }
    Ok(())
}

fn validate_transfer_options(options: &TransferOptions) -> Result<(), ClipboardError> {
    if options.total_deadline.is_zero() {
        return Err(ClipboardError::TimedOut);
    }
    if options.total_deadline > MAX_TOTAL_DEADLINE {
        return Err(ClipboardError::InvalidInput);
    }
    if options.max_bytes == 0 || options.max_bytes > MAX_TRANSFER_BYTES {
        return Err(ClipboardError::InvalidInput);
    }
    Instant::now().checked_add(options.total_deadline).ok_or(ClipboardError::InvalidInput)?;
    Ok(())
}

/// Turns a Wayland flush result into an I/O result while preserving queue semantics.
///
/// `WouldBlock` means the requests are already queued in the backend output buffer and will be
/// flushed later; it is not a failed claim or receive. Any other error is fatal for this call and
/// the caller deliberately rolls back.
fn flush_queued(result: Result<(), WaylandError>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(error) => Err(io::Error::new(ErrorKind::Other, error)),
    }
}

/// Which error taxonomy an already-admitted read reports.
///
/// The typed `begin_load_regular` path uses [`Self::Typed`]; the compatibility
/// `load_text`/`begin_load_text` paths keep [`Self::Legacy`] so their existing `io::ErrorKind`
/// mapping, including the deliberate flush `Other`, does not change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorMode {
    Typed,
    Legacy,
}

/// Maps an admission or worker I/O error for the typed clipboard API.
///
/// `NotConnected` is only produced by this module for an ended event loop or clipboard worker,
/// so it becomes [`ClipboardError::Disconnected`]. Every other kind keeps its exact identity;
/// in particular a broken pipe becomes `Disconnected` and an unexpected I/O kind stays
/// `Backend(kind)`.
fn typed_clipboard_error(error: io::Error) -> ClipboardError {
    if error.kind() == ErrorKind::NotConnected {
        ClipboardError::Disconnected
    } else {
        ClipboardError::from_io(error)
    }
}

fn map_io_error(mode: ErrorMode, error: io::Error) -> ClipboardError {
    match mode {
        ErrorMode::Typed => typed_clipboard_error(error),
        ErrorMode::Legacy => ClipboardError::from_io(error),
    }
}

/// Typed variant of [`flush_queued`].
///
/// Queue semantics are unchanged (`WouldBlock` means the requests are buffered and will be
/// flushed later). Unlike the legacy helper, a fatal `WaylandError::Io` keeps its original kind
/// so the typed caller sees `Disconnected` for a broken pipe instead of `Backend(Other)`.
fn flush_queued_typed(result: Result<(), WaylandError>) -> Result<(), ClipboardError> {
    match result {
        Ok(()) => Ok(()),
        Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(WaylandError::Io(error)) => Err(typed_clipboard_error(error)),
        Err(error) => Err(ClipboardError::Backend(io::Error::new(ErrorKind::Other, error))),
    }
}

/// Whether one admission deadline already expired. Checked after route resolution and before
/// any peer request or synchronous result publication.
fn transfer_deadline_passed(deadline: Instant) -> bool {
    Instant::now() >= deadline
}

fn transfer_interrupted(message: &'static str) -> io::Error {
    io::Error::new(ErrorKind::Interrupted, message)
}

fn checked_self_text(text: Arc<str>, max_bytes: usize) -> Result<Arc<str>, ClipboardError> {
    if text.len() > max_bytes {
        return Err(ClipboardError::TooLarge);
    }
    Ok(text)
}

/// Checks a self-read against the admitted byte cap and returns exact owned content.
///
/// A PNG source is returned by cloning the permit-bearing [`PngBytes`]; that shares the source's
/// existing allocation and budget reservation instead of allocating another buffer.
fn checked_self_content(
    payload: OwnedPayload,
    max_bytes: usize,
) -> Result<ClipboardContent, ClipboardError> {
    match payload {
        OwnedPayload::Text(text) => checked_self_text(text, max_bytes).map(ClipboardContent::Text),
        OwnedPayload::Png(png) => {
            if png.len() > max_bytes {
                Err(ClipboardError::TooLarge)
            } else {
                Ok(ClipboardContent::ImagePng(png))
            }
        },
    }
}

/// Create a pipe and submit a checked receive request.
///
/// The liveness check brackets `send`, so a destroyed proxy cannot silently create an EOF pipe
/// that is later mistaken for a genuine empty selection. The production regular-data-offer path
/// passes `WlDataOffer::send_request`, which returns `InvalidId` instead of the scanner-generated
/// method's silently ignored error.
fn checked_receive(
    mut alive: impl FnMut() -> bool,
    send: impl FnOnce(BorrowedFd<'_>) -> io::Result<()>,
) -> io::Result<OwnedFd> {
    let (read, write) = pipe_with(PipeFlags::CLOEXEC).map_err(io::Error::from)?;
    if !alive() {
        return Err(transfer_interrupted("the selection offer is no longer valid"));
    }
    send(write.as_fd())?;
    if !alive() {
        return Err(transfer_interrupted("the selection offer was destroyed during receive"));
    }
    Ok(read)
}

fn receive_regular_offer(offer: &SelectionOffer, mime: String) -> io::Result<OwnedFd> {
    checked_receive(
        || offer.inner().is_alive(),
        |fd| {
            offer
                .inner()
                .send_request(wl_data_offer::Request::Receive { mime_type: mime, fd })
                .map_err(|_| transfer_interrupted("the selection offer is no longer valid"))
        },
    )
}

/// The raw protocol operations a checked PRIMARY receive needs.
///
/// The production implementation wraps SCTK's private `ZwpPrimarySelectionOfferV1`. Keeping the
/// surface behind a small trait lets tests drive the same checked-receive path with deterministic
/// live/dead offers; it does not add a second protocol object or another source of offer lifetime.
trait PrimaryOfferProxy: fmt::Debug + Send + Sync {
    fn is_alive(&self) -> bool;
    fn request_receive(&self, mime: String, fd: BorrowedFd<'_>) -> io::Result<()>;
}

/// The raw offer captured from SCTK's device dispatch, beside the wrapper SCTK still owns.
#[derive(Clone, Debug)]
struct RawPrimaryOffer(ZwpPrimarySelectionOfferV1);

impl PrimaryOfferProxy for RawPrimaryOffer {
    fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    fn request_receive(&self, mime: String, fd: BorrowedFd<'_>) -> io::Result<()> {
        self.0
            .send_request(primary_offer_v1::Request::Receive { mime_type: mime, fd })
            .map_err(|_| transfer_interrupted("the selection offer is no longer valid"))
    }
}

fn receive_primary_offer(raw: &dyn PrimaryOfferProxy, mime: String) -> io::Result<OwnedFd> {
    // `Proxy::send_request` on the raw PRIMARY offer is the checked request. SCTK 0.19.2's
    // `PrimarySelectionOffer::receive_to_fd` discards that result, which let a dead offer look
    // like a present empty selection. The interception that supplies `raw` is documented in
    // docs/egui-037-migration-plan.md section 5.3.2b.
    checked_receive(|| raw.is_alive(), |fd| raw.request_receive(mime, fd))
}

fn deadline_after(duration: Duration) -> io::Result<Instant> {
    Instant::now().checked_add(duration).ok_or_else(|| {
        io::Error::new(ErrorKind::InvalidInput, "the transfer deadline is out of range")
    })
}

/// The two selection families have independent globals. PRIMARY must not be reported as missing
/// just because `wl_data_device_manager` is absent, and its absence must win over seat/focus
/// errors.
fn require_selection_family(
    primary: bool,
    regular_available: bool,
    primary_available: bool,
) -> io::Result<()> {
    if primary && !primary_available {
        Err(io::Error::new(ErrorKind::Unsupported, "zwp_primary_selection_v1 is unavailable"))
    } else if !primary && !regular_available {
        Err(io::Error::new(ErrorKind::Unsupported, "no data device manager"))
    } else {
        Ok(())
    }
}

/// Every source in the process gets a distinct token, including sources of other event loops and
/// displays. Markers therefore never collide between two winit-owned selections.
static NEXT_SOURCE_TOKEN: AtomicU64 = AtomicU64::new(1);

/// A nonce that distinguishes this process's markers from another winit process's markers.
fn marker_nonce() -> u64 {
    static NONCE: OnceLock<u64> = OnceLock::new();
    *NONCE.get_or_init(|| {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(0x9e37_79b9_7f4a_7c15);
        hasher.finish()
    })
}

/// Marker ownership for one [`ClipboardState`].
///
/// The process nonce prevents another winit process from being mistaken for ours. The clipboard
/// nonce keeps two `ClipboardState` instances in this process distinct: a marker from the other
/// instance is a foreign text offer, not an invalid stale source. Source tokens are monotonic per
/// process and the owning clipboard remembers the highest one it allocated, so stale-source
/// recognition does not depend on a bounded retirement list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ClipboardIdentity(String);

impl ClipboardIdentity {
    fn new() -> Self {
        static NEXT_CLIPBOARD: AtomicU64 = AtomicU64::new(1);
        let clipboard = NEXT_CLIPBOARD.fetch_add(1, Ordering::Relaxed);
        Self(format!("{:016x}-{clipboard:016x}", marker_nonce()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn owns(&self, marker: &str) -> bool {
        self.token(marker).is_some()
    }

    fn token(&self, marker: &str) -> Option<u64> {
        let rest = marker.strip_prefix(MARKER_PREFIX)?;
        let (owner, token) = rest.rsplit_once('-')?;
        if owner != self.0 || token.len() != 16 {
            return None;
        }
        u64::from_str_radix(token, 16).ok()
    }
}

/// Opaque, per-source marker mime type. It contains a process nonce, a clipboard nonce and a
/// source counter, and no PID, display pointer or other host identifier.
fn source_marker(identity: &ClipboardIdentity, token: u64) -> String {
    format!("{MARKER_PREFIX}{}-{token:016x}", identity.as_str())
}

/// The clipboard of one event loop, shared between the loop and its users.
#[derive(Debug)]
pub struct ClipboardState {
    connection: Connection,
    queue_handle: QueueHandle<WinitState>,
    manager: Option<WlDataDeviceManager>,
    primary_manager: Option<Arc<PrimarySelectionManagerState>>,
    /// The `wl_display` pointer, to match a raw display handle.
    display: usize,
    workers: Workers,
    /// Shared with transfer slots so `try_take` can validate its frozen route. Slots hold a
    /// `Weak` reference, so this does not create a cycle.
    inner: Arc<Mutex<Inner>>,
}

/// Every live clipboard, so one can be found from the display handle a toolkit is given.
static CLIPBOARDS: Mutex<Vec<Weak<ClipboardState>>> = Mutex::new(Vec::new());

impl ClipboardState {
    pub fn new(
        connection: &Connection,
        queue_handle: &QueueHandle<WinitState>,
        manager: Option<WlDataDeviceManager>,
        primary_manager: Option<Arc<PrimarySelectionManagerState>>,
    ) -> Arc<Self> {
        let display = connection.display().id().as_ptr() as usize;
        let state = Arc::new(Self {
            connection: connection.clone(),
            queue_handle: queue_handle.clone(),
            manager,
            primary_manager,
            display,
            workers: Workers::default(),
            inner: Arc::new(Mutex::new(Inner::default())),
        });

        let mut clipboards = CLIPBOARDS.lock().unwrap();
        clipboards.retain(|clipboard| clipboard.strong_count() > 0);
        clipboards.push(Arc::downgrade(&state));
        state
    }

    /// The clipboard of the event loop connected through this `wl_display`.
    pub fn for_display(display: *mut std::ffi::c_void) -> Option<Arc<Self>> {
        let clipboards = CLIPBOARDS.lock().unwrap();
        clipboards
            .iter()
            .filter_map(Weak::upgrade)
            .find(|clipboard| clipboard.display == display as usize)
    }

    /// Allocates an opaque per-source identity and its marker mime type.
    fn new_source(&self) -> (u64, String) {
        self.inner.lock().unwrap().allocate_source_identity()
    }

    pub fn add_seat(
        &self,
        seat: &WlSeat,
        device: Option<WlDataDevice>,
        queue_handle: &QueueHandle<WinitState>,
    ) {
        let primary_device = self
            .primary_manager
            .as_ref()
            .map(|manager| manager.get_selection_device(queue_handle, seat));
        let batch = {
            let mut inner = self.inner.lock().unwrap();
            inner.add_seat_entry(seat.id(), device, primary_device)
        };
        batch.complete();
    }

    pub fn remove_seat(&self, seat: &ObjectId) {
        let batch = {
            let mut inner = self.inner.lock().unwrap();
            inner.remove_seat(seat)
        };
        batch.complete();
    }

    /// Registers a live window with the admission/cancellation checks.
    pub fn add_window(&self, window: WindowId) {
        self.inner.lock().unwrap().add_window(window);
    }

    /// Cancels reads bound to a closed window and releases any selection source it owned.
    pub fn remove_window(&self, window: WindowId) {
        let batch = {
            let mut inner = self.inner.lock().unwrap();
            inner.remove_window(window)
        };
        batch.complete();
    }

    /// Records keyboard focus entry on a seat.
    pub fn keyboard_focus_enter(&self, seat: &ObjectId, serial: u32, window: WindowId) {
        self.inner.lock().unwrap().keyboard_focus_enter(seat, serial, window);
    }

    /// Records keyboard focus loss; this invalidates all earlier trigger serials and every live
    /// transfer for the seat.
    pub fn keyboard_focus_leave(&self, seat: &ObjectId) {
        run_transition(&self.inner, |inner| ((), inner.keyboard_focus_leave(seat)));
    }

    /// Records a pressed keyboard key as a possible ownership trigger.
    pub fn keyboard_key(&self, seat: &ObjectId, serial: u32, window: WindowId) {
        self.inner.lock().unwrap().keyboard_key(seat, serial, window);
    }

    /// Records a pointer press or release as a possible ownership trigger.
    pub fn pointer_input(&self, seat: &ObjectId, serial: u32, window: WindowId, pressed: bool) {
        self.inner.lock().unwrap().pointer_input(seat, serial, window, pressed);
    }

    /// Returns a fresh trigger token for the active input seat focused on `target`.
    pub fn input_origin(&self, target: WindowId) -> io::Result<InputOriginToken> {
        self.inner.lock().unwrap().input_origin(target)
    }

    /// Marks a regular selection change and applies the authoritative offer lifecycle.
    pub fn regular_selection_changed(&self, seat: &ObjectId, offer: Option<SelectionOffer>) {
        run_transition(&self.inner, |inner| {
            let batch = inner.apply_regular_selection_change(seat, offer, Instant::now());
            ((), batch)
        });
    }

    /// Marks a PRIMARY selection change and applies the authoritative offer lifecycle.
    ///
    /// `raw` is captured by the device dispatch from the same event as `offer`; the two are
    /// stored together so a checked receive can never bracket a wrapper with a different proxy.
    fn primary_selection_changed(
        &self,
        seat: &ObjectId,
        offer: Option<PrimarySelectionOffer>,
        raw: Option<Arc<dyn PrimaryOfferProxy>>,
    ) {
        run_transition(&self.inner, |inner| {
            let batch = inner.apply_primary_selection_change(seat, offer, raw, Instant::now());
            ((), batch)
        });
    }

    /// Re-reads the compositor's current offers from the protocol objects. SCTK 0.19.2 does not
    /// invoke `DataDeviceHandler::selection` for `wl_data_device.selection(NULL)`, so a regular
    /// selection clear is otherwise invisible. PRIMARY already reports `None`, but the same
    /// reconciliation keeps both families coherent after any dropped or coalesced event.
    pub fn reconcile_selections(&self) {
        run_transition(&self.inner, |inner| {
            let batch = inner.reconcile();
            ((), batch)
        });
    }

    /// Sends our regular selection's text or exact PNG through the pipe the compositor gave us.
    pub fn send(&self, source: &WlDataSource, mime_type: &str, pipe: WritePipe) {
        let serving = {
            let inner = self.inner.lock().unwrap();
            inner.regular_serving(source)
        };
        let Some((payload, source_bytes, serving)) = serving else {
            return;
        };
        // A text source serves only its text types and an image source only exact `image/png`.
        // This prevents a compositor request for an unadvertised type from escaping the source.
        if !payload.serves(mime_type) {
            return;
        }
        self.serve_source(serving, payload, source_bytes, pipe);
    }

    /// Sends our PRIMARY selection's text through the pipe the compositor gave us.
    pub fn send_primary(
        &self,
        source: &ZwpPrimarySelectionSourceV1,
        mime_type: &str,
        pipe: WritePipe,
    ) {
        let serving = {
            let inner = self.inner.lock().unwrap();
            inner.primary_serving(source)
        };
        let Some((payload, source_bytes, serving)) = serving else {
            return;
        };
        if !payload.serves(mime_type) {
            return;
        }
        self.serve_source(serving, payload, source_bytes, pipe);
    }

    fn serve_source(
        &self,
        serving: Arc<SourceServeState>,
        payload: OwnedPayload,
        source_bytes: Arc<SourceBytesCharge>,
        pipe: WritePipe,
    ) {
        let deadline = match deadline_after(DEFAULT_TOTAL_DEADLINE) {
            Ok(deadline) => deadline,
            Err(error) => {
                serving.record_error(error);
                // Dropping the pipe closes the compositor's read side.
                return;
            },
        };
        let job = WriteJob {
            serving: Arc::downgrade(&serving),
            fd: OwnedFd::from(pipe),
            payload,
            _source_bytes: source_bytes,
            deadline,
        };
        if let Err(error) = self.workers.submit_write(job) {
            serving.record_error(error);
            // The dropped pipe closes the compositor's read side.
        }
    }

    /// Forgets a regular source once another client takes the selection.
    pub fn source_cancelled(&self, source: &WlDataSource) {
        run_transition(&self.inner, |inner| ((), inner.regular_source_cancelled(source)));
    }

    /// Forgets a PRIMARY source once the compositor cancels it.
    pub fn primary_source_cancelled(&self, source: &ZwpPrimarySelectionSourceV1) {
        run_transition(&self.inner, |inner| ((), inner.primary_source_cancelled(source)));
    }

    /// Synchronously claims the regular clipboard on the triggering input serial.
    pub fn claim_text(&self, text: Arc<str>, claim: SelectionClaim) -> io::Result<()> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| io::Error::new(ErrorKind::Unsupported, "no data device manager"))?
            .clone();
        let (token, marker) = self.new_source();
        let source = manager.create_data_source(&self.queue_handle, DataSourceData::default());
        for mime in TEXT_MIME_TYPES {
            source.offer(mime.to_owned());
        }
        source.offer(marker.clone());
        self.install_claim(
            ClaimInstall {
                kind: SelectionKind::Regular,
                token,
                marker,
                payload: OwnedPayload::Text(text),
                target_window: claim.target_window,
                caller_generation: claim.caller_generation,
                protocol: Some(ProtocolSource::Regular(source)),
            },
            claim,
        )
    }

    /// Synchronously claims PRIMARY on the triggering keyboard or pointer input serial.
    pub fn claim_primary_text(&self, text: Arc<str>, claim: SelectionClaim) -> io::Result<()> {
        let manager = self.primary_manager.as_ref().ok_or_else(|| {
            io::Error::new(ErrorKind::Unsupported, "zwp_primary_selection_v1 is unavailable")
        })?;
        let (token, marker) = self.new_source();
        let mut mime_types: Vec<String> =
            TEXT_MIME_TYPES.iter().map(|mime| (*mime).to_owned()).collect();
        mime_types.push(marker.clone());
        let source = manager.create_selection_source(&self.queue_handle, mime_types);
        self.install_claim(
            ClaimInstall {
                kind: SelectionKind::Primary,
                token,
                marker,
                payload: OwnedPayload::Text(text),
                target_window: claim.target_window,
                caller_generation: claim.caller_generation,
                protocol: Some(ProtocolSource::Primary(source)),
            },
            claim,
        )
    }

    /// Reserves the regular selection for an asynchronous PNG encode.
    ///
    /// This validates target window, active input seat, focus epoch and the latest regular trigger,
    /// then consumes that trigger. It is the synchronous half of a deferred image copy; encoding
    /// may run on a worker, but installation must return through
    /// [`Self::claim_reserved_image_png`]. The reservation retains the triggering serial, so later
    /// pointer/keyboard input cannot substitute its own serial. Expiry is bounded by the original
    /// trigger's five-second freshness deadline, not by `now`. A new regular text claim,
    /// compatibility text store or image reservation invalidates this one even if that attempt
    /// later fails. PRIMARY is independent.
    pub fn reserve_image_claim(
        &self,
        claim: SelectionClaim,
    ) -> Result<ImageClaimReservation, ClipboardError> {
        if self.manager.is_none() {
            return Err(ClipboardError::Unsupported);
        }
        let validation = Arc::downgrade(&self.inner);
        let mut route = {
            let mut inner = self.inner.lock().unwrap();
            let route = inner.reserve_image_claim_at(&claim, Instant::now())?;
            if inner.seat_by_token(route.seat_token).is_some_and(|seat| seat.device.is_none()) {
                inner.release_image_reservation(route.id);
                return Err(ClipboardError::Unsupported);
            }
            route
        };
        route.validation = validation;
        Ok(ImageClaimReservation { inner: Some(route) })
    }

    /// Installs a previously reserved PNG as the regular selection.
    ///
    /// The reservation's original serial is used for `set_selection`; this method never consults
    /// the latest input. Focus, window, active-seat, ownership-epoch and original-trigger
    /// five-second expiry fences are revalidated on the event-loop thread. A failed claim leaves
    /// any installed source unchanged. The bytes retain their image-budget permit through
    /// installation and serving.
    pub fn claim_reserved_image_png(
        &self,
        png: PngBytes,
        mut reservation: ImageClaimReservation,
    ) -> Result<(), ClipboardError> {
        let manager = self.manager.as_ref().ok_or(ClipboardError::Unsupported)?.clone();
        let route = reservation.take();
        let reservation_id = route.id;
        {
            let mut inner = self.inner.lock().unwrap();
            if let Err(error) = inner.validate_image_reservation_at(&route, Instant::now()) {
                inner.release_image_reservation(reservation_id);
                return Err(error);
            }
        }

        let (token, marker) = self.new_source();
        let source = manager.create_data_source(&self.queue_handle, DataSourceData::default());
        // An image source advertises exactly PNG plus the opaque winit marker. It never offers a
        // text type, so a reader cannot accidentally treat encoded PNG bytes as text.
        source.offer(IMAGE_PNG_MIME.to_owned());
        source.offer(marker.clone());

        let installed = run_transition(&self.inner, |inner| {
            let mut batch = TransitionBatch::default();
            let installed = inner.admit_reserved_image_claim(
                ReservedImageInstall {
                    token,
                    marker,
                    png,
                    target_window: route.target_window,
                    protocol: Some(ProtocolSource::Regular(source)),
                },
                route,
                Instant::now(),
                &mut batch,
            );
            (installed, batch)
        });
        let installed = match installed {
            Ok(installed) => installed,
            Err(error) => {
                self.inner.lock().unwrap().release_image_reservation(reservation_id);
                return Err(error);
            },
        };
        if let Err(error) = flush_queued_typed(self.connection.flush()) {
            let failed_previous = run_transition(&self.inner, |inner| {
                (
                    inner.rollback_claim(
                        SelectionKind::Regular,
                        installed.token,
                        installed.previous,
                    ),
                    TransitionBatch::default(),
                )
            });
            drop(failed_previous);
            return Err(error);
        }
        drop(installed.previous);
        Ok(())
    }

    /// Invalidates pending regular image reservations for `target_window`.
    ///
    /// The active seat that focuses the target takes one new ownership epoch. No claimable input
    /// serial is required and the installed source is left unchanged. PRIMARY and reads are not
    /// affected. Intended for a winning regular output before it resolves a fresh origin.
    pub fn invalidate_image_reservations(
        &self,
        target_window: WindowId,
    ) -> Result<(), ClipboardError> {
        self.inner.lock().unwrap().invalidate_image_reservations_at(target_window)
    }

    fn install_claim(&self, install: ClaimInstall, claim: SelectionClaim) -> io::Result<()> {
        let kind = install.kind;
        let installed = run_transition(&self.inner, |inner| {
            let mut batch = TransitionBatch::default();
            let installed = inner.admit_claim(install, &claim, Instant::now(), &mut batch);
            (installed, batch)
        })?;
        if let Err(error) = flush_queued(self.connection.flush()) {
            let failed_previous = run_transition(&self.inner, |inner| {
                (
                    inner.rollback_claim(kind, installed.token, installed.previous),
                    TransitionBatch::default(),
                )
            });
            drop(failed_previous);
            return Err(error);
        }
        // The compositor now owns the new source; the previous one ends here. A `WouldBlock`
        // flush still means the request is queued, so this is not a rollback point.
        drop(installed.previous);
        Ok(())
    }

    fn legacy_claim(&self, kind: SelectionKind, text: Arc<str>) -> io::Result<()> {
        let (token, marker) = self.new_source();
        let protocol = match kind {
            SelectionKind::Regular => {
                let manager = self.manager.as_ref().ok_or_else(|| {
                    io::Error::new(ErrorKind::Unsupported, "no data device manager")
                })?;
                let source =
                    manager.create_data_source(&self.queue_handle, DataSourceData::default());
                for mime in TEXT_MIME_TYPES {
                    source.offer(mime.to_owned());
                }
                source.offer(marker.clone());
                ProtocolSource::Regular(source)
            },
            SelectionKind::Primary => {
                let manager = self.primary_manager.as_ref().ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::Unsupported,
                        "zwp_primary_selection_v1 is unavailable",
                    )
                })?;
                let mut mime_types: Vec<String> =
                    TEXT_MIME_TYPES.iter().map(|mime| (*mime).to_owned()).collect();
                mime_types.push(marker.clone());
                ProtocolSource::Primary(
                    manager.create_selection_source(&self.queue_handle, mime_types),
                )
            },
        };
        let previous = run_transition(&self.inner, |inner| {
            let mut batch = TransitionBatch::default();
            let previous =
                inner.install_legacy_source(kind, text, token, marker, Some(protocol), &mut batch);
            (previous, batch)
        })?;
        if let Err(error) = flush_queued(self.connection.flush()) {
            let failed_previous = run_transition(&self.inner, |inner| {
                (inner.rollback_claim(kind, token, previous), TransitionBatch::default())
            });
            drop(failed_previous);
            return Err(error);
        }
        // Flush before dropping the replaced source: on a fatal error the previous source is
        // restored rather than destroying both selections.
        drop(previous);
        Ok(())
    }

    /// The text on the active seat's regular clipboard. This compatibility wrapper is bounded but
    /// blocks the calling thread; egui-winit should use [`Self::begin_load_text`] instead.
    pub fn load_text(&self) -> io::Result<String> {
        self.load_text_on_seat(false)
    }

    /// The text on the active seat's PRIMARY selection. This compatibility wrapper is bounded but
    /// blocks the calling thread; egui-winit should use PRIMARY's asynchronous path instead.
    pub fn load_primary_text(&self) -> io::Result<String> {
        self.load_text_on_seat(true)
    }

    fn load_text_on_seat(&self, primary: bool) -> io::Result<String> {
        let handle = self.begin_legacy_load(primary)?;
        let content = handle.slot.wait().map_err(ClipboardError::into_io_error)?;
        match content {
            ClipboardContent::Text(text) => Ok(text.to_string()),
            ClipboardContent::ImagePng(_) => Err(ClipboardError::NotOffered.into_io_error()),
        }
    }

    pub fn store_text(&self, text: String) -> io::Result<()> {
        self.legacy_claim(SelectionKind::Regular, Arc::from(text))
    }

    pub fn store_primary_text(&self, text: String) -> io::Result<()> {
        self.legacy_claim(SelectionKind::Primary, Arc::from(text))
    }

    pub fn begin_load_text(&self, options: TransferOptions) -> io::Result<TransferHandle> {
        self.begin_load_typed(
            options,
            SelectionKind::Regular,
            ReadPreference::TextOnly,
            ErrorMode::Legacy,
        )
        .map(|slot| TransferHandle { slot })
        .map_err(ClipboardError::into_io_error)
    }

    pub fn begin_load_primary_text(&self, options: TransferOptions) -> io::Result<TransferHandle> {
        self.begin_load_typed(
            options,
            SelectionKind::Primary,
            ReadPreference::TextOnly,
            ErrorMode::Legacy,
        )
        .map(|slot| TransferHandle { slot })
        .map_err(ClipboardError::into_io_error)
    }

    /// Begins one combined regular read: text in the existing order, else exact `image/png`, from
    /// exactly one resolved seat/focus/offer snapshot.
    pub fn begin_load_regular(
        &self,
        options: TransferOptions,
    ) -> Result<RegularTransferHandle, ClipboardError> {
        self.begin_load_typed(
            options,
            SelectionKind::Regular,
            ReadPreference::TextThenImage,
            ErrorMode::Typed,
        )
        .map(|slot| RegularTransferHandle { slot })
    }

    fn begin_load_typed(
        &self,
        options: TransferOptions,
        kind: SelectionKind,
        preference: ReadPreference,
        error_mode: ErrorMode,
    ) -> Result<Arc<TransferSlot>, ClipboardError> {
        // An absent protocol family is `Unsupported` even if another argument is invalid.
        require_selection_family(
            kind == SelectionKind::Primary,
            self.manager.is_some(),
            self.primary_manager.is_some(),
        )
        .map_err(ClipboardError::from_io)?;
        validate_transfer_options(&options)?;
        let deadline = deadline_after(options.total_deadline).map_err(ClipboardError::from_io)?;
        let target = options.target_window;
        self.reconcile_selections();

        let max_bytes = options.max_bytes;
        let (read, offer, frozen, slot) = {
            let mut inner = self.inner.lock().unwrap();
            let route = inner.read_route_with(target, kind, preference)?;
            let seat = inner
                .seat_by_token(route.frozen.seat_token)
                .expect("the routed seat is present under the clipboard lock");
            if kind == SelectionKind::Primary && seat.primary_device.is_none() {
                return Err(ClipboardError::Unsupported);
            }
            if kind == SelectionKind::Regular && seat.device.is_none() {
                return Err(ClipboardError::Unsupported);
            }
            let frozen = route.frozen.clone();
            // Create the slot under the same lock that validated the window and focused seat, so
            // a concurrent close/seat removal cannot leave a transfer bound to stale state.
            let slot = inner.transfers.create_with_route(
                options.caller_generation,
                Some(target),
                frozen.clone(),
                Arc::downgrade(&self.inner),
                Some(options.completion_waker),
            );
            slot.set_deadline(deadline);
            (route.read, route.offer, frozen, slot)
        };

        match read {
            OfferRead::SelfContent(payload) => {
                let route_current = self.inner.lock().unwrap().frozen_route_is_current(&frozen);
                if !route_current {
                    slot.cancel();
                    return Err(ClipboardError::Interrupted);
                }
                let content = match checked_self_content(payload, max_bytes) {
                    Ok(content) => content,
                    Err(error) => {
                        slot.cancel();
                        return Err(error);
                    },
                };
                // Admission itself is covered by the total deadline. A self-paste must not
                // succeed after it expired just because no peer I/O was needed.
                if transfer_deadline_passed(deadline) {
                    slot.cancel();
                    return Err(ClipboardError::TimedOut);
                }
                slot.set_result(Ok(content));
                Ok(slot)
            },
            OfferRead::Request(mime) => self.start_read(
                slot,
                offer,
                mime,
                ReadKind::Text,
                frozen,
                deadline,
                max_bytes,
                error_mode,
            ),
            OfferRead::RequestImage(mime) => self.start_read(
                slot,
                offer,
                mime,
                ReadKind::ImagePng,
                frozen,
                deadline,
                max_bytes,
                error_mode,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_read(
        &self,
        slot: Arc<TransferSlot>,
        offer: Option<SelectionOfferKind>,
        mime: String,
        read_kind: ReadKind,
        frozen: FrozenRoute,
        deadline: Instant,
        max_bytes: usize,
        error_mode: ErrorMode,
    ) -> Result<Arc<TransferSlot>, ClipboardError> {
        if slot.is_caller_cancelled() {
            slot.cancel();
            return Err(ClipboardError::Interrupted);
        }
        // The absolute deadline starts at admission. Do not reserve a worker or ask the peer
        // for data after it has already expired.
        if transfer_deadline_passed(deadline) {
            slot.cancel();
            return Err(ClipboardError::TimedOut);
        }
        // Reserve worker capacity before asking the peer for its data; a full queue must not
        // consume a peer transfer and a pipe. Queue slots do not bypass the image byte budget.
        let reservation = match self.workers.reserve_read() {
            Ok(reservation) => reservation,
            Err(error) => {
                slot.cancel();
                return Err(map_io_error(error_mode, error));
            },
        };
        let image_permit = match read_kind {
            ReadKind::ImagePng => match ImagePermit::acquire_global(max_bytes.saturating_mul(2)) {
                Ok(permit) => Some(permit),
                Err(error) => {
                    self.workers.release_read(reservation);
                    slot.cancel();
                    return Err(error);
                },
            },
            ReadKind::Text => None,
        };
        if !self.inner.lock().unwrap().frozen_route_is_current(&frozen) {
            self.workers.release_read(reservation);
            slot.cancel();
            return Err(ClipboardError::Interrupted);
        }
        // Reservation and permit acquisition can take a measurable moment. Recheck the deadline
        // immediately before issuing the peer request so expiry never emits protocol I/O.
        if transfer_deadline_passed(deadline) {
            self.workers.release_read(reservation);
            slot.cancel();
            return Err(ClipboardError::TimedOut);
        }

        let fd = match receive_selection_offer(offer.expect("a request requires an offer"), mime) {
            Ok(fd) => fd,
            Err(error) => {
                self.workers.release_read(reservation);
                slot.cancel();
                return Err(map_io_error(error_mode, error));
            },
        };
        let flushed = match error_mode {
            ErrorMode::Typed => flush_queued_typed(self.connection.flush()),
            ErrorMode::Legacy => {
                flush_queued(self.connection.flush()).map_err(ClipboardError::from_io)
            },
        };
        if let Err(error) = flushed {
            self.workers.release_read(reservation);
            slot.cancel();
            return Err(error);
        }
        // A dispatched selection change can destroy the offer between classification and
        // receive. Recheck the frozen route, not just the offer, before the worker is allowed to
        // turn an EOF pipe into a successful empty paste.
        let route_current = self.inner.lock().unwrap().frozen_route_is_current(&frozen);
        if !route_current || slot.is_caller_cancelled() || slot.is_finished() {
            self.workers.release_read(reservation);
            slot.cancel();
            return Err(ClipboardError::Interrupted);
        }

        let job = ReadJob {
            slot: Arc::downgrade(&slot),
            fd,
            deadline,
            max_bytes,
            kind: read_kind,
            image_permit,
        };
        if let Err(error) = self.workers.submit_reserved_read(reservation, job) {
            slot.cancel();
            return Err(map_io_error(error_mode, error));
        }
        Ok(slot)
    }

    fn begin_legacy_load(&self, primary: bool) -> io::Result<TransferHandle> {
        require_selection_family(primary, self.manager.is_some(), self.primary_manager.is_some())?;
        self.reconcile_selections();
        let kind = if primary { SelectionKind::Primary } else { SelectionKind::Regular };
        let deadline = deadline_after(DEFAULT_TOTAL_DEADLINE)?;
        let (read, offer, frozen, slot) = {
            let mut inner = self.inner.lock().unwrap();
            let route = inner.read_route_legacy(kind)?;
            let seat =
                inner.seat_by_token(route.frozen.seat_token).ok_or(ClipboardError::Absent)?;
            if kind == SelectionKind::Primary && seat.primary_device.is_none() {
                return Err(ClipboardError::Unsupported.into_io_error());
            }
            if kind == SelectionKind::Regular && seat.device.is_none() {
                return Err(ClipboardError::Unsupported.into_io_error());
            }
            let frozen = route.frozen.clone();
            let slot = inner.transfers.create_with_route(
                0,
                None,
                frozen.clone(),
                Arc::downgrade(&self.inner),
                None,
            );
            slot.set_deadline(deadline);
            (route.read, route.offer, frozen, slot)
        };

        match read {
            OfferRead::SelfContent(payload) => {
                let route_current = self.inner.lock().unwrap().frozen_route_is_current(&frozen);
                if !route_current {
                    slot.cancel();
                    return Err(ClipboardError::Interrupted.into_io_error());
                }
                let content = checked_self_content(payload, DEFAULT_MAX_TRANSFER_BYTES)?;
                slot.set_result(Ok(content));
                Ok(TransferHandle { slot })
            },
            OfferRead::Request(mime) => {
                let slot = self
                    .start_read(
                        slot,
                        offer,
                        mime,
                        ReadKind::Text,
                        frozen,
                        deadline,
                        DEFAULT_MAX_TRANSFER_BYTES,
                        ErrorMode::Legacy,
                    )
                    .map_err(ClipboardError::into_io_error)?;
                Ok(TransferHandle { slot })
            },
            OfferRead::RequestImage(_) => {
                slot.cancel();
                Err(ClipboardError::NotOffered.into_io_error())
            },
        }
    }
}

impl Drop for ClipboardState {
    fn drop(&mut self) {
        let batch = {
            let mut inner = self.inner.lock().unwrap();
            let mut batch = TransitionBatch::default();
            inner.transfers.interrupt_all(&mut batch.interrupted);
            for seat in &mut inner.seats {
                drop(seat.regular.owned.take());
                drop(seat.primary.owned.take());
            }
            // Destroy the per-seat protocol devices before the manager field is dropped.
            inner.seats.clear();
            batch
        };
        batch.complete();
        self.workers.shutdown();
    }
}

#[derive(Debug)]
enum SelectionOfferKind {
    Regular(SelectionOffer),
    /// The raw PRIMARY offer paired with the logical offer when the transfer started.
    Primary {
        raw: Option<Arc<dyn PrimaryOfferProxy>>,
    },
}

/// Submit the checked receive for a snapshot. A PRIMARY snapshot without a paired raw proxy is
/// refused; it must never fall back to SCTK's unchecked `receive_to_fd`.
fn receive_selection_offer(offer: SelectionOfferKind, mime: String) -> io::Result<OwnedFd> {
    match offer {
        SelectionOfferKind::Regular(offer) => receive_regular_offer(&offer, mime),
        SelectionOfferKind::Primary { raw } => match raw {
            Some(raw) => receive_primary_offer(raw.as_ref(), mime),
            None => Err(transfer_interrupted(
                "the PRIMARY selection offer has no tracked protocol object",
            )),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdmissionDecision {
    Run,
    Queue,
    Full,
}

/// FIFO admission: a new request never jumps ahead of an older queued job, even when an active
/// slot happens to be free at reservation time.
fn admission_decision(
    active: usize,
    queued: usize,
    max_active: usize,
    max_queued: usize,
) -> AdmissionDecision {
    if queued == 0 && active < max_active {
        AdmissionDecision::Run
    } else if queued < max_queued {
        AdmissionDecision::Queue
    } else {
        AdmissionDecision::Full
    }
}

#[derive(Debug, Default)]
struct Workers {
    inner: Arc<WorkerInner>,
}

#[derive(Debug, Default)]
struct ReadQueue {
    active: usize,
    reserved: usize,
    jobs: VecDeque<ReadJob>,
}

#[derive(Debug, Default)]
struct WriteQueue {
    active: usize,
    reserved: usize,
    jobs: VecDeque<WriteJob>,
}

#[derive(Debug, Default)]
struct WorkerInner {
    reads: Mutex<ReadQueue>,
    writes: Mutex<WriteQueue>,
    shutting_down: AtomicBool,
    /// Whether the single queued-job coordinator thread is running.
    coordinator: Mutex<bool>,
    /// Test-only deterministic spawn-failure injection. The flag belongs to one worker pool, so
    /// tests never exhaust real threads or interfere with another clipboard instance.
    #[cfg(test)]
    coordinator_spawn_failure: AtomicBool,
    /// Test-only interleaving hooks. They run while the corresponding queue lock is held, after
    /// the queue has been partitioned but before the surviving jobs are put back. A test can use
    /// them to prove that pruning never exposes an empty queue to admission.
    #[cfg(test)]
    read_prune_probe: Mutex<Option<PruneProbe>>,
    #[cfg(test)]
    write_prune_probe: Mutex<Option<PruneProbe>>,
}

#[cfg(test)]
struct PruneProbe(Box<dyn FnOnce() + Send>);

#[cfg(test)]
impl fmt::Debug for PruneProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PruneProbe")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reservation {
    Active,
    Queued,
}

/// What the worker must produce from the one admitted offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadKind {
    Text,
    ImagePng,
}

#[derive(Debug)]
struct ReadJob {
    slot: Weak<TransferSlot>,
    fd: OwnedFd,
    deadline: Instant,
    max_bytes: usize,
    kind: ReadKind,
    /// The image-budget reservation, held while active or queued. Text jobs have none.
    image_permit: Option<ImagePermit>,
}

struct WriteJob {
    serving: Weak<SourceServeState>,
    fd: OwnedFd,
    /// The exact source payload. A PNG clone retains the image-budget permit until this queued or
    /// active writer is gone, even if the source is replaced meanwhile.
    payload: OwnedPayload,
    /// Shared lifetime charge for `payload`. It stays charged across source replacement and is
    /// released only when this job (and every other reference) is dropped.
    _source_bytes: Arc<SourceBytesCharge>,
    deadline: Instant,
}

impl fmt::Debug for WriteJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteJob")
            .field("serving_alive", &self.serving.strong_count())
            .field("payload_bytes", &self.payload.len())
            .field("payload", &self.payload)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

enum JobStatus {
    Runnable,
    Discard,
    Expired(Arc<TransferSlot>),
}

fn timed_out_error() -> ClipboardError {
    ClipboardError::TimedOut
}

fn read_job_status(job: &ReadJob) -> JobStatus {
    match job.slot.upgrade() {
        None => JobStatus::Discard,
        Some(slot) => {
            if slot.is_caller_cancelled() || slot.is_finished() {
                JobStatus::Discard
            } else if Instant::now() >= job.deadline {
                JobStatus::Expired(slot)
            } else {
                JobStatus::Runnable
            }
        },
    }
}

/// Retires a queued read that will never run.
///
/// The pipe endpoint is closed immediately and the image-budget permit is returned to the
/// caller. Callers that hold the queue lock drop the permit there too: the image budget is a leaf
/// mutex, so the queue-to-budget order cannot deadlock, and a waiter that observes the queue slot
/// as free must also observe the read end closed. Callers without the queue lock simply drop it.
fn retire_read_job(job: ReadJob) -> Option<ImagePermit> {
    let ReadJob { fd, image_permit, .. } = job;
    drop(fd);
    image_permit
}

enum WriteStatus {
    Runnable,
    Discard,
    Expired,
}

/// One write-queue decision that may own a payload.
///
/// The job is returned to the caller so it can be dropped after the queue mutex is released. A
/// discarded payload can be the last reference to a large PNG or to a replaced text source; its
/// destructor and source-byte release must not run while admission is blocked on the queue lock.
enum WriteNext {
    Spawn(WriteJob),
    Discard(WriteJob),
    Expire(WriteJob),
    Stop,
}

fn write_job_status(job: &WriteJob) -> WriteStatus {
    match job.serving.upgrade() {
        None => WriteStatus::Discard,
        Some(serving) => {
            if serving.is_cancelled() {
                WriteStatus::Discard
            } else if Instant::now() >= job.deadline {
                WriteStatus::Expired
            } else {
                WriteStatus::Runnable
            }
        },
    }
}

impl Workers {
    fn reserve_read(&self) -> io::Result<Reservation> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(io::Error::new(ErrorKind::NotConnected, "the clipboard has ended"));
        }
        let mut reads = self.inner.reads.lock().unwrap();
        match admission_decision(
            reads.active,
            reads.jobs.len().saturating_add(reads.reserved),
            MAX_ACTIVE_READS,
            MAX_QUEUED_READS,
        ) {
            AdmissionDecision::Run => {
                reads.active += 1;
                Ok(Reservation::Active)
            },
            AdmissionDecision::Queue => {
                reads.reserved += 1;
                Ok(Reservation::Queued)
            },
            AdmissionDecision::Full => {
                Err(io::Error::new(ErrorKind::WouldBlock, "too many clipboard reads are in flight"))
            },
        }
    }

    fn release_read(&self, reservation: Reservation) {
        {
            let mut reads = self.inner.reads.lock().unwrap();
            match reservation {
                Reservation::Active => reads.active = reads.active.saturating_sub(1),
                Reservation::Queued => reads.reserved = reads.reserved.saturating_sub(1),
            }
        }
        if !self.inner.shutting_down.load(Ordering::Acquire) {
            self.promote_read();
        }
    }

    fn submit_reserved_read(&self, reservation: Reservation, job: ReadJob) -> io::Result<()> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            self.release_read(reservation);
            return Err(io::Error::new(ErrorKind::NotConnected, "the clipboard has ended"));
        }
        let mut pending = Some(job);
        let mut queued = false;
        {
            let mut reads = self.inner.reads.lock().unwrap();
            if let Reservation::Queued = reservation {
                reads.reserved = reads.reserved.saturating_sub(1);
                if reads.jobs.is_empty() && reads.active < MAX_ACTIVE_READS {
                    reads.active += 1;
                } else {
                    reads.jobs.push_back(pending.take().unwrap());
                    queued = true;
                }
            }
        }
        if let Some(job) = pending {
            WorkerInner::spawn_reader(&self.inner, job);
        }
        if queued {
            self.inner.ensure_coordinator()?;
        }
        Ok(())
    }

    fn promote_read(&self) {
        loop {
            let (next, stale_permit) = {
                let mut reads = self.inner.reads.lock().unwrap();
                if reads.active >= MAX_ACTIVE_READS {
                    return;
                }
                match reads.jobs.pop_front() {
                    None => return,
                    Some(job) => match read_job_status(&job) {
                        JobStatus::Runnable => {
                            reads.active += 1;
                            (Some(Ok(job)), None)
                        },
                        JobStatus::Discard => (None, retire_read_job(job)),
                        JobStatus::Expired(slot) => (Some(Err(slot)), retire_read_job(job)),
                    },
                }
            };
            // The pipe was closed under the queue lock; release the image-budget permit here.
            drop(stale_permit);
            match next {
                Some(Ok(job)) => {
                    WorkerInner::spawn_reader(&self.inner, job);
                    return;
                },
                Some(Err(slot)) => {
                    slot.fail_internal(timed_out_error());
                },
                None => continue,
            }
        }
    }

    fn reserve_write(&self) -> io::Result<Reservation> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(io::Error::new(ErrorKind::NotConnected, "the clipboard has ended"));
        }
        let mut writes = self.inner.writes.lock().unwrap();
        match admission_decision(
            writes.active,
            writes.jobs.len().saturating_add(writes.reserved),
            MAX_ACTIVE_WRITES,
            MAX_QUEUED_WRITES,
        ) {
            AdmissionDecision::Run => {
                writes.active += 1;
                Ok(Reservation::Active)
            },
            AdmissionDecision::Queue => {
                writes.reserved += 1;
                Ok(Reservation::Queued)
            },
            AdmissionDecision::Full => Err(io::Error::new(
                ErrorKind::WouldBlock,
                "too many clipboard writes are in flight",
            )),
        }
    }

    fn release_write(&self, reservation: Reservation) {
        {
            let mut writes = self.inner.writes.lock().unwrap();
            match reservation {
                Reservation::Active => writes.active = writes.active.saturating_sub(1),
                Reservation::Queued => writes.reserved = writes.reserved.saturating_sub(1),
            }
        }
        if !self.inner.shutting_down.load(Ordering::Acquire) {
            self.promote_write();
        }
    }

    fn submit_reserved_write(&self, reservation: Reservation, job: WriteJob) -> io::Result<()> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            self.release_write(reservation);
            return Err(io::Error::new(ErrorKind::NotConnected, "the clipboard has ended"));
        }
        let mut pending = Some(job);
        let mut queued = false;
        {
            let mut writes = self.inner.writes.lock().unwrap();
            if let Reservation::Queued = reservation {
                writes.reserved = writes.reserved.saturating_sub(1);
                if writes.jobs.is_empty() && writes.active < MAX_ACTIVE_WRITES {
                    writes.active += 1;
                } else {
                    writes.jobs.push_back(pending.take().unwrap());
                    queued = true;
                }
            }
        }
        if let Some(job) = pending {
            WorkerInner::spawn_writer(&self.inner, job);
        }
        if queued {
            self.inner.ensure_coordinator()?;
        }
        Ok(())
    }

    fn submit_write(&self, job: WriteJob) -> io::Result<()> {
        let reservation = self.reserve_write()?;
        self.submit_reserved_write(reservation, job)
    }

    fn promote_write(&self) {
        loop {
            let next = {
                let mut writes = self.inner.writes.lock().unwrap();
                if writes.active >= MAX_ACTIVE_WRITES {
                    return;
                }
                match writes.jobs.pop_front() {
                    None => return,
                    Some(job) => match write_job_status(&job) {
                        WriteStatus::Runnable => {
                            writes.active += 1;
                            WriteNext::Spawn(job)
                        },
                        WriteStatus::Discard => WriteNext::Discard(job),
                        WriteStatus::Expired => WriteNext::Expire(job),
                    },
                }
            };
            match next {
                WriteNext::Spawn(job) => {
                    WorkerInner::spawn_writer(&self.inner, job);
                    return;
                },
                WriteNext::Expire(job) => {
                    if let Some(serving) = job.serving.upgrade() {
                        serving.record_error(timed_out_error());
                    }
                    drop(job);
                },
                WriteNext::Discard(job) => drop(job),
                WriteNext::Stop => return,
            }
        }
    }

    fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::Release);
        // Move jobs out before dropping them. A queued image read releases its budget permit on
        // drop, and that destructor must not run while a queue lock is held.
        let reads = {
            let mut reads = self.inner.reads.lock().unwrap();
            std::mem::take(&mut reads.jobs)
        };
        let writes = {
            let mut writes = self.inner.writes.lock().unwrap();
            std::mem::take(&mut writes.jobs)
        };
        drop(reads);
        drop(writes);
    }
}

impl WorkerInner {
    /// Starts the single bounded coordinator if queued jobs need deadline/cancellation checks.
    fn ensure_coordinator(self: &Arc<Self>) -> io::Result<()> {
        self.start_coordinator(|inner| {
            #[cfg(test)]
            if inner.coordinator_spawn_failure.load(Ordering::Acquire) {
                return Err(io::Error::new(ErrorKind::Other, "forced coordinator spawn failure"));
            }
            std::thread::Builder::new()
                .name("winit-clipboard-coordinator".into())
                .spawn(move || inner.coordinate())
                .map(|_| ())
                .map_err(|error| io::Error::new(ErrorKind::Other, error))
        })
    }

    /// Shared failure path for coordinator startup.
    ///
    /// On spawn failure the queues are drained and every queued read/write is completed or
    /// recorded with an explicit error before the submission returns. That makes the failure
    /// deterministic even when no active worker is left to promote the queued jobs. Active workers
    /// keep their permits and simply observe empty queues when they finish.
    fn start_coordinator(
        self: &Arc<Self>,
        spawn: impl FnOnce(Arc<Self>) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }
        {
            let mut running = self.coordinator.lock().unwrap();
            if *running {
                return Ok(());
            }
            *running = true;
        }
        if let Err(error) = spawn(Arc::clone(self)) {
            *self.coordinator.lock().unwrap() = false;
            let message = format!("clipboard worker coordinator failed to start: {error}");
            self.fail_queued_jobs(&message);
            return Err(io::Error::new(ErrorKind::Other, message));
        }
        Ok(())
    }

    fn fail_queued_jobs(&self, message: &str) {
        let reads: Vec<ReadJob> = self.reads.lock().unwrap().jobs.drain(..).collect();
        let writes: Vec<WriteJob> = self.writes.lock().unwrap().jobs.drain(..).collect();
        for job in reads {
            if let Some(slot) = job.slot.upgrade() {
                slot.fail_internal(ClipboardError::Backend(io::Error::new(
                    ErrorKind::Other,
                    message.to_owned(),
                )));
            }
        }
        for job in writes {
            if let Some(serving) = job.serving.upgrade() {
                serving.record_error(message);
            }
        }
    }

    /// Wakes once for each queued job whose caller cancelled it, whose deadline expired or whose
    /// selection source was cancelled. One thread per clipboard, never one per queued job.
    fn coordinate(self: Arc<Self>) {
        loop {
            self.prune_queued();
            if self.shutting_down.load(Ordering::Acquire) {
                break;
            }
            {
                let mut running = self.coordinator.lock().unwrap();
                if self.queues_empty() {
                    *running = false;
                    return;
                }
            }
            std::thread::sleep(WORKER_POLL_SLICE);
        }
        if let Ok(mut running) = self.coordinator.lock() {
            *running = false;
        }
    }

    fn queues_empty(&self) -> bool {
        let reads_empty = self.reads.lock().unwrap().jobs.is_empty();
        let writes_empty = self.writes.lock().unwrap().jobs.is_empty();
        reads_empty && writes_empty
    }

    fn prune_queued(self: &Arc<Self>) {
        // Partition each queue under its own lock. Admission decisions count `jobs.len()`, so
        // releasing the lock with a temporarily drained queue would let a concurrent reserve see
        // free queued capacity and admit more work than the hard caps allow. Keep the lock for the
        // whole partition and reinsertion; completion callbacks run only after unlocking.
        let mut expired_reads = Vec::new();
        {
            let mut reads = self.reads.lock().unwrap();
            let mut keep = VecDeque::with_capacity(reads.jobs.len());
            while let Some(job) = reads.jobs.pop_front() {
                match read_job_status(&job) {
                    JobStatus::Runnable => keep.push_back(job),
                    // `retire_read_job` closes the pipe immediately and releases the image-budget
                    // permit. The budget mutex is a leaf: no path holds it while taking a queue
                    // lock, so this preserves the existing "queue slot free means pipe closed"
                    // ordering without a lock cycle.
                    JobStatus::Discard => drop(retire_read_job(job)),
                    JobStatus::Expired(slot) => {
                        drop(retire_read_job(job));
                        expired_reads.push(slot);
                    },
                }
            }
            #[cfg(test)]
            Self::run_prune_probe(&self.read_prune_probe);
            // Reinsert the survivors in their original order while the queue lock still makes
            // jobs and reservations one atomic accounting unit for admission.
            reads.jobs = keep;
        }
        // A worker can finish and decrement `active` while the queue lock is released above.
        // Promote requeued runnable work after re-adding it so that race cannot strand jobs.
        Workers { inner: Arc::clone(self) }.promote_read();
        for slot in expired_reads {
            slot.fail_internal(timed_out_error());
        }

        let mut discarded_writes = Vec::new();
        let mut failed_writes = Vec::new();
        {
            let mut writes = self.writes.lock().unwrap();
            let mut keep = VecDeque::with_capacity(writes.jobs.len());
            while let Some(job) = writes.jobs.pop_front() {
                match write_job_status(&job) {
                    WriteStatus::Runnable => keep.push_back(job),
                    WriteStatus::Discard => discarded_writes.push(job),
                    WriteStatus::Expired => {
                        if let Some(serving) = job.serving.upgrade() {
                            failed_writes.push(serving);
                        }
                        discarded_writes.push(job);
                    },
                }
            }
            #[cfg(test)]
            Self::run_prune_probe(&self.write_prune_probe);
            writes.jobs = keep;
        }
        // Close expired/cancelled pipes and record serving errors outside the queue lock.
        drop(discarded_writes);
        Workers { inner: Arc::clone(self) }.promote_write();
        for serving in failed_writes {
            serving.record_error(timed_out_error());
        }
    }

    #[cfg(test)]
    fn run_prune_probe(probe: &Mutex<Option<PruneProbe>>) {
        let probe = probe.lock().unwrap().take();
        if let Some(PruneProbe(probe)) = probe {
            probe();
        }
    }

    fn spawn_reader(self: &Arc<Self>, job: ReadJob) {
        let queues = Arc::clone(self);
        let failed_slot = job.slot.clone();
        let spawned =
            std::thread::Builder::new().name("winit-clipboard-read".into()).spawn(move || {
                run_read_job(job);
                WorkerInner::reader_finished(&queues);
            });
        if let Err(error) = spawned {
            if let Some(slot) = failed_slot.upgrade() {
                slot.set_result(Err(ClipboardError::Backend(io::Error::new(
                    ErrorKind::Other,
                    error,
                ))));
            }
            WorkerInner::reader_finished(self);
        }
    }

    fn spawn_writer(self: &Arc<Self>, job: WriteJob) {
        let queues = Arc::clone(self);
        let failed_serving = job.serving.clone();
        let spawned =
            std::thread::Builder::new().name("winit-clipboard-write".into()).spawn(move || {
                run_write_job(job);
                WorkerInner::writer_finished(&queues);
            });
        if let Err(error) = spawned {
            if let Some(serving) = failed_serving.upgrade() {
                serving.record_error(error);
            }
            WorkerInner::writer_finished(self);
        }
    }

    fn reader_finished(self: &Arc<Self>) {
        enum Next {
            Spawn(ReadJob),
            Expire(Arc<TransferSlot>, Option<ImagePermit>),
            Discard(Option<ImagePermit>),
            Stop,
        }

        loop {
            if self.shutting_down.load(Ordering::Acquire) {
                let jobs = {
                    let mut reads = self.reads.lock().unwrap();
                    reads.active = 0;
                    reads.reserved = 0;
                    std::mem::take(&mut reads.jobs)
                };
                drop(jobs);
                return;
            }
            let next = {
                let mut reads = self.reads.lock().unwrap();
                match reads.jobs.pop_front() {
                    Some(job) => match read_job_status(&job) {
                        JobStatus::Runnable => Next::Spawn(job),
                        JobStatus::Discard => Next::Discard(retire_read_job(job)),
                        JobStatus::Expired(slot) => Next::Expire(slot, retire_read_job(job)),
                    },
                    None => {
                        reads.active = reads.active.saturating_sub(1);
                        Next::Stop
                    },
                }
            };
            match next {
                Next::Stop => return,
                Next::Spawn(job) => {
                    Self::spawn_reader(self, job);
                    return;
                },
                Next::Discard(permit) => drop(permit),
                Next::Expire(slot, permit) => {
                    // The pipe closed under the queue lock; release the permit outside it.
                    drop(permit);
                    slot.fail_internal(timed_out_error());
                },
            }
        }
    }

    fn writer_finished(self: &Arc<Self>) {
        loop {
            if self.shutting_down.load(Ordering::Acquire) {
                let jobs = {
                    let mut writes = self.writes.lock().unwrap();
                    writes.active = 0;
                    writes.reserved = 0;
                    std::mem::take(&mut writes.jobs)
                };
                drop(jobs);
                return;
            }
            let next = {
                let mut writes = self.writes.lock().unwrap();
                match writes.jobs.pop_front() {
                    Some(job) => match write_job_status(&job) {
                        WriteStatus::Runnable => WriteNext::Spawn(job),
                        WriteStatus::Discard => WriteNext::Discard(job),
                        WriteStatus::Expired => WriteNext::Expire(job),
                    },
                    None => {
                        writes.active = writes.active.saturating_sub(1);
                        WriteNext::Stop
                    },
                }
            };
            match next {
                WriteNext::Stop => return,
                WriteNext::Spawn(job) => {
                    Self::spawn_writer(self, job);
                    return;
                },
                WriteNext::Expire(job) => {
                    if let Some(serving) = job.serving.upgrade() {
                        serving.record_error(timed_out_error());
                    }
                    drop(job);
                },
                WriteNext::Discard(job) => drop(job),
            }
        }
    }
}

fn run_read_job(job: ReadJob) {
    let result = read_job_content(&job);
    if let Some(slot) = job.slot.upgrade() {
        slot.set_result(result);
    }
}

fn read_job_content(job: &ReadJob) -> Result<ClipboardContent, ClipboardError> {
    let cancelled = || match job.slot.upgrade() {
        // Internal invalidation sets `finished`; stop the I/O and release the pipe promptly.
        Some(slot) => slot.is_caller_cancelled() || slot.is_finished(),
        None => true,
    };
    let bytes = read_pipe_raw(&job.fd, job.deadline, job.max_bytes, &cancelled)?;
    // The absolute deadline covers queue wait, pipe I/O and conversion. Check before conversion
    // and again inside the bounded conversion loop.
    if cancelled() {
        return Err(ClipboardError::Interrupted);
    }
    if Instant::now() >= job.deadline {
        return Err(ClipboardError::TimedOut);
    }
    match job.kind {
        ReadKind::Text => Ok(ClipboardContent::Text(convert_text(
            bytes,
            job.max_bytes,
            job.deadline,
            &cancelled,
        )?)),
        ReadKind::ImagePng => {
            let permit =
                job.image_permit.as_ref().expect("an image read holds its budget permit").clone();
            Ok(ClipboardContent::ImagePng(PngBytes::from_vec_with_permit(bytes, permit)))
        },
    }
}

fn run_write_job(job: WriteJob) {
    let result = write_pipe_bytes(&job.fd, job.payload.as_bytes(), job.deadline, || {
        match job.serving.upgrade() {
            Some(state) => state.is_cancelled(),
            None => true,
        }
    });
    if let Err(error) = result {
        if let Some(serving) = job.serving.upgrade() {
            if !serving.is_cancelled() && error.kind() != ErrorKind::BrokenPipe {
                serving.record_error(error);
            }
        }
    }
}

fn set_nonblocking(fd: &OwnedFd) -> io::Result<()> {
    let flags = fcntl_getfl(fd).map_err(io::Error::from)?;
    fcntl_setfl(fd, flags | OFlags::NONBLOCK).map_err(io::Error::from)
}

fn poll_slice(deadline: Instant, events: PollFlags, fd: &OwnedFd) -> io::Result<Option<Instant>> {
    let now = Instant::now();
    if now >= deadline {
        return Ok(None);
    }
    let remaining = deadline.saturating_duration_since(now);
    let slice = remaining.min(WORKER_POLL_SLICE);
    let timeout_ms = slice.as_millis().max(1).min(i32::MAX as u128) as i32;
    let mut poll_fd = PollFd::new(fd, events);
    match poll(std::slice::from_mut(&mut poll_fd), timeout_ms) {
        Ok(0) => Ok(Some(Instant::now())),
        Ok(_) => {
            let ready = poll_fd.revents();
            if ready.intersects(PollFlags::ERR | PollFlags::NVAL) {
                Err(io::Error::new(ErrorKind::Other, "the clipboard pipe reported an error"))
            } else {
                Ok(Some(Instant::now()))
            }
        },
        Err(error) if error.kind() == ErrorKind::Interrupted => Ok(Some(Instant::now())),
        Err(error) => Err(error.into()),
    }
}

fn read_pipe_raw(
    fd: &OwnedFd,
    deadline: Instant,
    max_bytes: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>, ClipboardError> {
    if cancelled() {
        return Err(ClipboardError::Interrupted);
    }
    set_nonblocking(fd).map_err(ClipboardError::from_io)?;

    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if cancelled() {
            return Err(ClipboardError::Interrupted);
        }
        if Instant::now() >= deadline {
            return Err(ClipboardError::TimedOut);
        }
        match rustix::io::read(fd, &mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(read) => {
                if bytes.len().saturating_add(read) > max_bytes {
                    return Err(ClipboardError::TooLarge);
                }
                bytes.extend_from_slice(&chunk[..read]);
            },
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if poll_slice(deadline, PollFlags::IN, fd)
                    .map_err(ClipboardError::from_io)?
                    .is_none()
                {
                    return Err(ClipboardError::TimedOut);
                }
            },
            Err(error) => return Err(ClipboardError::from_io(error.into())),
        }
    }
}

/// Test-facing text read. Production workers use [`read_pipe_raw`] and [`convert_text`] so the
/// same absolute deadline covers conversion.
#[cfg(test)]
fn read_pipe(
    fd: &OwnedFd,
    deadline: Instant,
    max_bytes: usize,
    cancelled: impl Fn() -> bool,
) -> io::Result<Arc<str>> {
    let bytes = read_pipe_raw(fd, deadline, max_bytes, &cancelled)
        .map_err(ClipboardError::into_io_error)?;
    convert_text(bytes, max_bytes, deadline, &cancelled).map_err(ClipboardError::into_io_error)
}

#[cfg(test)]
fn write_pipe(
    fd: &OwnedFd,
    text: &str,
    deadline: Instant,
    cancelled: impl Fn() -> bool,
) -> io::Result<()> {
    write_pipe_bytes(fd, text.as_bytes(), deadline, cancelled)
}

fn write_pipe_bytes(
    fd: &OwnedFd,
    bytes: &[u8],
    deadline: Instant,
    cancelled: impl Fn() -> bool,
) -> io::Result<()> {
    if cancelled() {
        return Err(io::Error::new(ErrorKind::Interrupted, "the source was cancelled"));
    }
    set_nonblocking(fd)?;

    let mut offset = 0;
    while offset < bytes.len() {
        if cancelled() {
            return Err(io::Error::new(ErrorKind::Interrupted, "the source was cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(ErrorKind::TimedOut, "the clipboard reader was too slow"));
        }
        match rustix::io::write(fd, &bytes[offset..]) {
            Ok(0) => return Err(io::Error::new(ErrorKind::WriteZero, "the clipboard pipe closed")),
            Ok(written) => offset += written,
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if poll_slice(deadline, PollFlags::OUT, fd)?.is_none() {
                    return Err(io::Error::new(
                        ErrorKind::TimedOut,
                        "the clipboard reader was too slow",
                    ));
                }
            },
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn push_text_char(
    output: &mut String,
    character: char,
    max_bytes: usize,
) -> Result<(), ClipboardError> {
    if output.len().saturating_add(character.len_utf8()) > max_bytes {
        return Err(ClipboardError::TooLarge);
    }
    output.push(character);
    Ok(())
}

/// Characters between cancellation/deadline checks inside one normalization pass.
///
/// A `char` is at most four UTF-8 bytes, so this bounds each unchecked run to 16 KiB even when
/// the whole payload is valid UTF-8 and `from_utf8` returns it in one piece.
const TEXT_CONVERSION_CHECK_CHARS: usize = 4096;

fn append_normalized_text(
    output: &mut String,
    text: &str,
    max_bytes: usize,
    check: &impl Fn() -> Result<(), ClipboardError>,
) -> Result<(), ClipboardError> {
    let mut characters = text.chars().peekable();
    let mut since_check = 0usize;
    while let Some(character) = characters.next() {
        since_check += 1;
        if since_check >= TEXT_CONVERSION_CHECK_CHARS {
            since_check = 0;
            check()?;
        }
        let character = if character == '\r' {
            // `text/*` types use CRLF line endings, but applications expect LF.
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
            '\n'
        } else {
            character
        };
        push_text_char(output, character, max_bytes)?;
    }
    Ok(())
}

/// Converts a pipe payload to UTF-8 with a hard output-byte cap and deadline.
///
/// The cap is checked while replacement/CRLF normalization builds the output, not after an
/// unbounded lossy conversion. Invalid UTF-8 uses one U+FFFD per invalid sequence, matching
/// `String::from_utf8_lossy`, but stops with [`ClipboardError::TooLarge`] before exceeding
/// `max_bytes`. Cancellation and the absolute deadline are checked before conversion, after it
/// and during bounded chunks.
fn convert_text(
    bytes: Vec<u8>,
    max_bytes: usize,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<str>, ClipboardError> {
    convert_text_with_clock(bytes, max_bytes, deadline, cancelled, &Instant::now)
}

fn convert_text_with_clock(
    bytes: Vec<u8>,
    max_bytes: usize,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
    now: &impl Fn() -> Instant,
) -> Result<Arc<str>, ClipboardError> {
    let check = || -> Result<(), ClipboardError> {
        if cancelled() {
            Err(ClipboardError::Interrupted)
        } else if now() >= deadline {
            Err(ClipboardError::TimedOut)
        } else {
            Ok(())
        }
    };
    check()?;
    let mut output = String::with_capacity(bytes.len().min(max_bytes));
    let mut rest = bytes.as_slice();
    let mut chunks = 0usize;
    loop {
        chunks += 1;
        if chunks % 64 == 0 {
            check()?;
        }
        match std::str::from_utf8(rest) {
            Ok(text) => {
                append_normalized_text(&mut output, text, max_bytes, &check)?;
                break;
            },
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                let valid = std::str::from_utf8(&rest[..valid_up_to])
                    .expect("the UTF-8 error reported a valid prefix");
                append_normalized_text(&mut output, valid, max_bytes, &check)?;
                let invalid = error.error_len().unwrap_or(rest.len() - valid_up_to).max(1);
                push_text_char(&mut output, '\u{fffd}', max_bytes)?;
                rest = &rest[valid_up_to.saturating_add(invalid)..];
                if rest.is_empty() {
                    break;
                }
            },
        }
    }
    check()?;
    Ok(Arc::from(output))
}

/// Test-facing conversion without a caller deadline.
#[cfg(test)]
fn text_from_bytes(bytes: Vec<u8>, max_bytes: usize) -> io::Result<Arc<str>> {
    let deadline =
        Instant::now().checked_add(Duration::from_secs(60 * 60)).unwrap_or_else(Instant::now);
    convert_text(bytes, max_bytes, deadline, &|| false).map_err(ClipboardError::into_io_error)
}

impl PrimarySelectionDeviceHandler for WinitState {
    fn selection(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        primary_selection_device: &ZwpPrimarySelectionDeviceV1,
    ) {
        let data = primary_selection_device.data::<PrimarySelectionDeviceData>().unwrap();
        let seat = data.seat().id();
        let offer = data.selection_offer();
        // The device dispatch captured this raw offer from the same event before forwarding it to
        // SCTK. Storing it here pairs it with SCTK's authoritative wrapper under one lock.
        let raw = self
            .pending_primary_offer
            .take()
            .map(|raw| Arc::new(RawPrimaryOffer(raw)) as Arc<dyn PrimaryOfferProxy>);
        self.clipboard.primary_selection_changed(&seat, offer, raw);
    }
}

impl PrimarySelectionSourceHandler for WinitState {
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
        mime: String,
        write_pipe: WritePipe,
    ) {
        self.clipboard.send_primary(source, &mime, write_pipe);
    }

    fn cancelled(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
    ) {
        self.clipboard.primary_source_cancelled(source);
    }
}

impl Dispatch<ZwpPrimarySelectionDeviceV1, PrimarySelectionDeviceData, WinitState> for WinitState {
    fn event(
        state: &mut WinitState,
        proxy: &ZwpPrimarySelectionDeviceV1,
        event: PrimarySelectionDeviceEvent,
        data: &PrimarySelectionDeviceData,
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        // SCTK 0.19.2 keeps the active raw offer private and its `receive_to_fd` discards the
        // request result. Capture the raw offer from this event, then forward the event unchanged
        // so SCTK remains the owner of offer lifetime, MIME data and source dispatch.
        if let PrimarySelectionDeviceEvent::Selection { id } = &event {
            state.pending_primary_offer = id.clone();
        }
        <PrimarySelectionManagerState as Dispatch<
            ZwpPrimarySelectionDeviceV1,
            PrimarySelectionDeviceData,
            WinitState,
        >>::event(state, proxy, event, data, conn, qhandle);
    }

    fn event_created_child(opcode: u16, qhandle: &QueueHandle<Self>) -> Arc<dyn ObjectData> {
        // Delegate child-offer creation to SCTK's device dispatch so offer user data stays
        // exactly as the rest of the toolkit expects.
        <PrimarySelectionManagerState as Dispatch<
            ZwpPrimarySelectionDeviceV1,
            PrimarySelectionDeviceData,
            WinitState,
        >>::event_created_child(opcode, qhandle)
    }
}

// The device dispatch above is ours; the other three PRIMARY interfaces keep SCTK's dispatch.
delegate_dispatch!(WinitState: [ZwpPrimarySelectionDeviceManagerV1: GlobalData] => PrimarySelectionManagerState);
delegate_dispatch!(WinitState: [ZwpPrimarySelectionOfferV1: PrimarySelectionOfferData] => PrimarySelectionManagerState);
delegate_dispatch!(WinitState: [ZwpPrimarySelectionSourceV1: GlobalData] => PrimarySelectionManagerState);

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use rustix::pipe::{pipe_with, PipeFlags};

    use super::*;

    fn wid(raw: u64) -> WindowId {
        WindowId::from(raw)
    }

    /// A test window with one focused seat and a fresh keyboard trigger.
    fn focused_inner(target: WindowId) -> (Inner, SelectionClaim) {
        let mut inner = Inner::default();
        inner.add_window(target);
        inner.add_test_seat();
        inner.keyboard_focus_enter(&ObjectId::null(), 1, target);
        inner.keyboard_key(&ObjectId::null(), 2, target);
        let trigger = inner.input_origin(target).unwrap();
        let claim = SelectionClaim { target_window: target, caller_generation: 1, trigger };
        (inner, claim)
    }

    fn test_png(budget: &Arc<ImageBudget>, bytes: &[u8]) -> PngBytes {
        let permit = budget.try_acquire(bytes.len().saturating_mul(2).max(1)).unwrap();
        PngBytes::from_vec_with_permit(bytes.to_vec(), permit)
    }

    fn test_source_charge(bytes: usize) -> Arc<SourceBytesCharge> {
        Arc::new(SourceBytesCharge::new(
            Arc::new(SourceBytesBudget::new(MAX_TOTAL_OWNED_BYTES)),
            bytes,
        ))
    }

    fn install_reserved_detached(
        inner: &mut Inner,
        reservation: ImageReservationInner,
        png: PngBytes,
        token: u64,
        now: Instant,
    ) -> Result<InstalledClaim, ClipboardError> {
        let marker = source_marker(&inner.marker_identity, token);
        let mut batch = TransitionBatch::default();
        let result = inner.admit_reserved_image_claim(
            ReservedImageInstall {
                token,
                marker,
                png,
                target_window: reservation.target_window,
                protocol: None,
            },
            reservation,
            now,
            &mut batch,
        );
        batch.complete();
        result
    }

    /// Issues a claim token for the current active seat through the production routing path.
    fn active_claim(inner: &mut Inner, target: WindowId) -> SelectionClaim {
        let trigger = inner.input_origin(target).unwrap();
        SelectionClaim { target_window: target, caller_generation: 1, trigger }
    }

    /// Exercises the production claim admission with a detached protocol handle (`None`).
    fn install_detached(
        inner: &mut Inner,
        kind: SelectionKind,
        claim: &SelectionClaim,
        text: &str,
        token: u64,
    ) -> io::Result<()> {
        install_detached_at(inner, kind, claim, text, token, Instant::now())
    }

    /// Claim admission with an explicit clock, so pending-grace sequences are deterministic.
    fn install_detached_at(
        inner: &mut Inner,
        kind: SelectionKind,
        claim: &SelectionClaim,
        text: &str,
        token: u64,
        now: Instant,
    ) -> io::Result<()> {
        let mut batch = TransitionBatch::default();
        let marker = source_marker(&inner.marker_identity, token);
        let install = ClaimInstall {
            kind,
            token,
            marker,
            payload: OwnedPayload::Text(Arc::from(text)),
            target_window: claim.target_window,
            caller_generation: claim.caller_generation,
            protocol: None,
        };
        let result = inner.admit_claim(install, claim, now, &mut batch);
        // `InstalledClaim::previous` owns the replaced source; dropping it releases that source.
        let result = match result {
            Ok(installed) => {
                drop(installed.previous);
                Ok(())
            },
            Err(error) => Err(error),
        };
        batch.complete();
        result
    }

    /// Drives one production offer observation and completes its invalidations.
    fn observe(
        inner: &mut Inner,
        kind: SelectionKind,
        key: Option<OfferKey>,
        mime_types: &[&str],
        now: Instant,
    ) {
        observe_at(inner, 0, kind, key, mime_types, now);
    }

    fn observe_at(
        inner: &mut Inner,
        position: usize,
        kind: SelectionKind,
        key: Option<OfferKey>,
        mime_types: &[&str],
        now: Instant,
    ) {
        let mut batch = TransitionBatch::default();
        inner.observe_selection(
            position,
            kind,
            ObservedOffer {
                key,
                mime_types: mime_types.iter().map(|mime| (*mime).to_owned()).collect(),
            },
            now,
            &mut batch,
        );
        batch.complete();
    }

    fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if condition() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn focus_and_input_are_per_seat_and_epoch_checked() {
        let target = wid(11);
        let other = wid(12);
        let now = Instant::now();
        let mut seat = SeatInputState::default();

        seat.enter(7, target, now);
        assert!(seat.current_origin(1, target, now + Duration::from_millis(1)).is_none());
        seat.key(8, target, now + Duration::from_millis(2));
        let keyboard = seat.current_origin(1, target, now + Duration::from_millis(3)).unwrap();
        assert!(seat.validates(
            &keyboard.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(3)
        ));

        seat.pointer(9, target, true, now + Duration::from_millis(4));
        let pointer = seat.current_origin(1, target, now + Duration::from_millis(5)).unwrap();
        assert_eq!(pointer.inner.origin, InputOrigin::PointerPress);
        assert!(!seat.validates(
            &keyboard.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(5)
        ));
        assert!(seat.validates(
            &pointer.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(5)
        ));

        assert!(seat.current_origin(1, other, now + Duration::from_millis(5)).is_none());
        assert!(seat.consume(&pointer.inner, SelectionKind::Regular));
        assert!(!seat.validates(
            &pointer.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(5)
        ));
        // The same input remains available to the other selection family.
        assert!(seat.validates(
            &pointer.inner,
            target,
            SelectionKind::Primary,
            now + Duration::from_millis(5)
        ));
        assert!(seat.consume(&pointer.inner, SelectionKind::Primary));
        assert!(!seat.validates(
            &pointer.inner,
            target,
            SelectionKind::Primary,
            now + Duration::from_millis(5)
        ));
        assert!(!seat.consume(&pointer.inner, SelectionKind::Regular));
        assert!(seat.current_origin(1, target, now + Duration::from_millis(5)).is_none());

        seat.leave();
        assert!(seat.current_origin(1, target, now + Duration::from_millis(6)).is_none());
        assert!(!seat.validates(
            &pointer.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(6)
        ));

        let mut replacement = SeatInputState::default();
        replacement.enter(10, target, now + Duration::from_millis(7));
        replacement.key(11, target, now + Duration::from_millis(8));
        let replacement_token =
            replacement.current_origin(2, target, now + Duration::from_millis(9)).unwrap();
        assert!(!seat.validates(
            &replacement_token.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(9)
        ));
        assert!(replacement.validates(
            &replacement_token.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(9)
        ));
    }

    #[test]
    fn claims_follow_the_active_seat_for_both_selection_families() {
        let target = wid(60);
        let mut inner = Inner::default();
        inner.add_window(target);
        let first = inner.add_test_seat();
        let second = inner.add_test_seat();
        assert_ne!(first, second);

        // Both seats focus the same window; the first was inserted first.
        inner.keyboard_focus_enter_at(0, 1, target);
        inner.keyboard_key_at(0, 2, target);
        let first_claim = active_claim(&mut inner, target);
        assert_eq!(first_claim.trigger.inner.seat_token, first);
        install_detached(&mut inner, SelectionKind::Regular, &first_claim, "first regular", 1)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &first_claim, "first primary", 2)
            .unwrap();

        // The first seat still has a fresh key, but a focus transfer to the second seat makes the
        // latter active. With no input yet on the second seat, routing fails instead of using the
        // first seat's retained serial.
        inner.keyboard_key_at(0, 7, target);
        inner.keyboard_focus_enter_at(1, 3, target);
        assert_eq!(inner.input_origin(target).unwrap_err().kind(), ErrorKind::InvalidInput);
        inner.pointer_input_at(1, 4, target, true);
        let second_claim = active_claim(&mut inner, target);
        assert_eq!(second_claim.trigger.inner.seat_token, second);
        assert_eq!(second_claim.trigger.inner.origin, InputOrigin::PointerPress);
        install_detached(&mut inner, SelectionKind::Regular, &second_claim, "second regular", 3)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &second_claim, "second primary", 4)
            .unwrap();

        assert_eq!(
            inner.seats[0].regular.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "first regular"
        );
        assert_eq!(
            inner.seats[0].primary.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "first primary"
        );
        assert_eq!(
            inner.seats[1].regular.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "second regular"
        );
        assert_eq!(
            inner.seats[1].primary.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "second primary"
        );

        // Returning input to the first seat routes the next claim there and leaves the second
        // seat's selection untouched.
        inner.keyboard_focus_enter_at(0, 5, target);
        inner.keyboard_key_at(0, 6, target);
        let first_again = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Regular, &first_again, "first again", 5)
            .unwrap();
        install_detached(
            &mut inner,
            SelectionKind::Primary,
            &first_again,
            "first primary again",
            6,
        )
        .unwrap();
        assert_eq!(
            inner.seats[0].regular.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "first again"
        );
        assert_eq!(
            inner.seats[0].primary.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "first primary again"
        );
        assert_eq!(
            inner.seats[1].regular.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "second regular"
        );
        assert_eq!(
            inner.seats[1].primary.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "second primary"
        );
    }

    #[test]
    fn stale_seat_token_is_rejected_after_another_seat_receives_input() {
        let target = wid(61);
        let mut inner = Inner::default();
        inner.add_window(target);
        let first = inner.add_test_seat();
        let second = inner.add_test_seat();
        inner.keyboard_focus_enter_at(0, 1, target);
        inner.keyboard_focus_enter_at(1, 2, target);

        inner.keyboard_key_at(0, 3, target);
        let stale = active_claim(&mut inner, target);
        assert_eq!(stale.trigger.inner.seat_token, first);

        // A later input turn on the other focused seat invalidates the retained token's route.
        inner.keyboard_key_at(1, 4, target);
        assert_eq!(inner.input_origin(target).unwrap().inner.seat_token, second);
        for (kind, token) in [(SelectionKind::Regular, 1), (SelectionKind::Primary, 2)] {
            let error = install_detached(&mut inner, kind, &stale, "stale", token).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidInput);
        }
        assert!(inner.seats[0].regular.owned.is_none());
        assert!(inner.seats[0].primary.owned.is_none());
        assert!(inner.seats[1].regular.owned.is_none());
        assert!(inner.seats[1].primary.owned.is_none());
    }

    #[test]
    fn reads_route_to_the_active_seat_for_both_selection_families() {
        let target = wid(62);
        let mut inner = Inner::default();
        inner.add_window(target);
        let first = inner.add_test_seat();
        let second = inner.add_test_seat();
        inner.keyboard_focus_enter_at(0, 1, target);
        inner.keyboard_focus_enter_at(1, 2, target);

        inner.keyboard_key_at(0, 3, target);
        let first_claim = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Regular, &first_claim, "first regular", 1)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &first_claim, "first primary", 2)
            .unwrap();
        inner.keyboard_key_at(1, 4, target);
        let second_claim = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Regular, &second_claim, "second regular", 3)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &second_claim, "second primary", 4)
            .unwrap();

        let regular = inner.read_route(target, SelectionKind::Regular).unwrap();
        assert_eq!(regular.frozen.seat_token, second);
        assert_eq!(
            regular.read,
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("second regular")))
        );
        let primary = inner.read_route(target, SelectionKind::Primary).unwrap();
        assert_eq!(primary.frozen.seat_token, second);
        assert_eq!(
            primary.read,
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("second primary")))
        );

        // The later input-triggering seat wins even though the first seat was inserted first.
        inner.keyboard_key_at(0, 5, target);
        let regular = inner.read_route(target, SelectionKind::Regular).unwrap();
        assert_eq!(regular.frozen.seat_token, first);
        assert_eq!(
            regular.read,
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("first regular")))
        );
        let primary = inner.read_route(target, SelectionKind::Primary).unwrap();
        assert_eq!(primary.frozen.seat_token, first);
        assert_eq!(
            primary.read,
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("first primary")))
        );
    }

    #[test]
    fn read_route_binds_the_active_seats_offer_snapshot_for_both_families() {
        let target = wid(66);
        let now = Instant::now();
        let mut inner = Inner::default();
        inner.add_window(target);
        inner.add_test_seat();
        inner.add_test_seat();
        inner.keyboard_focus_enter_at(0, 1, target);
        inner.keyboard_focus_enter_at(1, 2, target);

        let first_regular = inner.allocate_offer_key();
        let first_primary = inner.allocate_offer_key();
        observe_at(
            &mut inner,
            0,
            SelectionKind::Regular,
            Some(first_regular),
            &["text/plain"],
            now,
        );
        observe_at(
            &mut inner,
            0,
            SelectionKind::Primary,
            Some(first_primary),
            &["text/plain"],
            now,
        );
        let second_regular = inner.allocate_offer_key();
        let second_primary = inner.allocate_offer_key();
        observe_at(
            &mut inner,
            1,
            SelectionKind::Regular,
            Some(second_regular),
            &["text/plain"],
            now,
        );
        observe_at(
            &mut inner,
            1,
            SelectionKind::Primary,
            Some(second_primary),
            &["text/plain"],
            now,
        );

        // The second seat is active from its focus entry and already has its own offer snapshot,
        // even before a claimable key or pointer serial arrives.
        let regular = inner.read_route(target, SelectionKind::Regular).unwrap();
        assert_eq!(regular.frozen.seat_token, inner.seats[1].seat_token);
        assert_eq!(regular.frozen.expected_offer, Some(second_regular));
        inner.keyboard_key_at(1, 3, target);
        let regular = inner.read_route(target, SelectionKind::Regular).unwrap();
        assert_eq!(regular.frozen.seat_token, inner.seats[1].seat_token);
        assert_eq!(regular.read, OfferRead::Request("text/plain".to_owned()));
        assert_eq!(regular.frozen.expected_offer, Some(second_regular));
        let primary = inner.read_route(target, SelectionKind::Primary).unwrap();
        assert_eq!(primary.frozen.seat_token, inner.seats[1].seat_token);
        assert_eq!(primary.read, OfferRead::Request("text/plain".to_owned()));
        assert_eq!(primary.frozen.expected_offer, Some(second_primary));

        inner.keyboard_key_at(0, 4, target);
        let regular = inner.read_route(target, SelectionKind::Regular).unwrap();
        assert_eq!(regular.frozen.seat_token, inner.seats[0].seat_token);
        assert_eq!(regular.frozen.expected_offer, Some(first_regular));
        let primary = inner.read_route(target, SelectionKind::Primary).unwrap();
        assert_eq!(primary.frozen.seat_token, inner.seats[0].seat_token);
        assert_eq!(primary.frozen.expected_offer, Some(first_primary));
    }

    #[test]
    fn stale_active_focus_fails_instead_of_falling_back_to_another_focused_seat() {
        let target = wid(63);
        let mut inner = Inner::default();
        inner.add_window(target);
        let first = inner.add_test_seat();
        inner.add_test_seat();
        inner.keyboard_focus_enter_at(0, 1, target);
        inner.keyboard_focus_enter_at(1, 2, target);

        // Both seats own readable selections, so an insertion-order fallback would succeed.
        inner.keyboard_key_at(0, 3, target);
        let first_claim = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Regular, &first_claim, "first regular", 1)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &first_claim, "first primary", 2)
            .unwrap();
        inner.keyboard_key_at(1, 4, target);
        let second_claim = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Regular, &second_claim, "second regular", 3)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &second_claim, "second primary", 4)
            .unwrap();

        // The active seat leaves while the first seat still focuses the target.
        inner.keyboard_focus_leave_at(1);
        assert_eq!(
            inner.active_focused_seat(target).unwrap_err().io_kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(inner.input_origin(target).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert_eq!(
            inner.read_route(target, SelectionKind::Regular).unwrap_err().io_kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            inner.read_route(target, SelectionKind::Primary).unwrap_err().io_kind(),
            ErrorKind::InvalidInput
        );

        // Fresh input on the still-focused first seat makes it active again.
        inner.keyboard_key_at(0, 5, target);
        let claim = active_claim(&mut inner, target);
        assert_eq!(claim.trigger.inner.seat_token, first);
    }

    #[test]
    fn removed_and_readded_seat_requires_fresh_focus_input_and_claims_new_seat() {
        let target = wid(64);
        let mut inner = Inner::default();
        inner.add_window(target);
        inner.add_test_seat();
        let removed = inner.add_test_seat();
        inner.keyboard_focus_enter_at(0, 1, target);
        inner.keyboard_focus_enter_at(1, 2, target);
        inner.keyboard_key_at(0, 3, target);
        let first_claim = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Regular, &first_claim, "first regular", 1)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &first_claim, "first primary", 2)
            .unwrap();
        inner.keyboard_key_at(1, 4, target);
        let removed_claim = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Regular, &removed_claim, "removed regular", 3)
            .unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &removed_claim, "removed primary", 4)
            .unwrap();

        inner.remove_seat_at(1).complete();
        assert!(inner.active_seat.is_none());
        assert_eq!(inner.input_origin(target).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert_eq!(
            inner.read_route(target, SelectionKind::Regular).unwrap_err().io_kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            inner.read_route(target, SelectionKind::Primary).unwrap_err().io_kind(),
            ErrorKind::InvalidInput
        );
        assert!(inner.seats[0].input.focuses(target));

        let readded = inner.add_test_seat();
        assert_ne!(readded, removed);
        assert_eq!(inner.input_origin(target).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert_eq!(
            inner.read_route(target, SelectionKind::Regular).unwrap_err().io_kind(),
            ErrorKind::InvalidInput
        );

        inner.keyboard_focus_enter_at(1, 5, target);
        inner.keyboard_key_at(1, 6, target);
        let claim = active_claim(&mut inner, target);
        assert_eq!(claim.trigger.inner.seat_token, readded);
        install_detached(&mut inner, SelectionKind::Regular, &claim, "readded regular", 5).unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &claim, "readded primary", 6).unwrap();
        let regular = inner.read_route(target, SelectionKind::Regular).unwrap();
        assert_eq!(regular.frozen.seat_token, readded);
        assert_eq!(
            regular.read,
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("readded regular")))
        );
        let primary = inner.read_route(target, SelectionKind::Primary).unwrap();
        assert_eq!(primary.frozen.seat_token, readded);
        assert_eq!(
            primary.read,
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("readded primary")))
        );
    }

    #[test]
    fn single_seat_routing_and_reads_keep_their_existing_behavior() {
        let target = wid(65);
        let (mut inner, claim) = focused_inner(target);
        assert_eq!(claim.trigger.inner.seat_token, inner.seats[0].seat_token);
        install_detached(&mut inner, SelectionKind::Regular, &claim, "regular", 1).unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &claim, "primary", 2).unwrap();

        let regular = inner.read_route(target, SelectionKind::Regular).unwrap();
        assert_eq!(regular.frozen.seat_token, inner.seats[0].seat_token);
        assert_eq!(regular.read, OfferRead::SelfContent(OwnedPayload::Text(Arc::from("regular"))));
        let primary = inner.read_route(target, SelectionKind::Primary).unwrap();
        assert_eq!(primary.frozen.seat_token, inner.seats[0].seat_token);
        assert_eq!(primary.read, OfferRead::SelfContent(OwnedPayload::Text(Arc::from("primary"))));
    }

    #[test]
    fn pointer_serial_survives_click_to_focus_enter() {
        let target = wid(13);
        let other = wid(14);
        let now = Instant::now();
        let mut seat = SeatInputState::default();

        // Click-to-focus order: pointer button, then keyboard enter.
        seat.pointer(20, target, true, now);
        assert!(seat.current_origin(1, target, now).is_none());
        seat.enter(21, target, now + Duration::from_millis(1));
        let token = seat.current_origin(1, target, now + Duration::from_millis(2)).unwrap();
        assert_eq!(token.inner.origin, InputOrigin::PointerPress);
        assert_eq!(token.inner.serial, 20);
        assert!(seat.validates(
            &token.inner,
            target,
            SelectionKind::Regular,
            now + Duration::from_millis(2)
        ));

        // A pointer press on another window is not carried into this focus epoch.
        let mut seat = SeatInputState::default();
        seat.pointer(22, other, true, now);
        seat.enter(23, target, now + Duration::from_millis(1));
        assert!(seat.current_origin(1, target, now + Duration::from_millis(2)).is_none());
    }

    #[test]
    fn latest_origin_wins_for_pointer_release_and_keyboard() {
        let target = wid(15);
        let now = Instant::now();
        let mut seat = SeatInputState::default();
        seat.enter(1, target, now);
        seat.key(2, target, now + Duration::from_millis(1));
        seat.pointer(3, target, false, now + Duration::from_millis(2));
        let token = seat.current_origin(1, target, now + Duration::from_millis(3)).unwrap();
        assert_eq!(token.inner.origin, InputOrigin::PointerRelease);
        assert_eq!(token.inner.serial, 3);

        seat.key(4, target, now + Duration::from_millis(4));
        let token = seat.current_origin(1, target, now + Duration::from_millis(5)).unwrap();
        assert_eq!(token.inner.origin, InputOrigin::KeyboardKey);
        assert_eq!(token.inner.serial, 4);
    }

    #[test]
    fn old_serials_expire_and_focus_serial_is_not_a_claim() {
        let target = wid(21);
        let now = Instant::now();
        let mut seat = SeatInputState::default();
        seat.enter(1, target, now);
        assert!(seat.current_origin(1, target, now).is_none());
        seat.key(2, target, now);
        assert!(seat
            .current_origin(1, target, now + INPUT_ORIGIN_MAX_AGE + Duration::from_millis(1))
            .is_none());
    }

    #[test]
    fn release_claim_token_is_single_use_per_family() {
        let target = wid(30);
        let (mut inner, claim) = focused_inner(target);

        install_detached(&mut inner, SelectionKind::Regular, &claim, "regular", 1).unwrap();
        // A cloned claim is the same input turn; the per-family consumption still rejects it.
        let replayed_claim = claim.clone();
        let replay =
            install_detached(&mut inner, SelectionKind::Regular, &replayed_claim, "replay", 2)
                .unwrap_err();
        assert_eq!(replay.kind(), ErrorKind::InvalidInput);
        assert_eq!(
            inner.seats[0].regular.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "regular",
            "a replayed token must not replace the first source"
        );

        // The same input may still claim PRIMARY, once.
        install_detached(&mut inner, SelectionKind::Primary, &claim, "primary", 3).unwrap();
        let replay =
            install_detached(&mut inner, SelectionKind::Primary, &claim, "again", 4).unwrap_err();
        assert_eq!(replay.kind(), ErrorKind::InvalidInput);
        assert_eq!(
            inner.seats[0].primary.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "primary"
        );
    }

    #[test]
    fn same_input_can_claim_regular_and_primary_without_replaying_the_token() {
        let target = wid(31);
        let (mut inner, claim) = focused_inner(target);
        install_detached(&mut inner, SelectionKind::Regular, &claim, "regular", 1).unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &claim, "primary", 2).unwrap();
        assert!(inner.seats[0].regular.owned.is_some());
        assert!(inner.seats[0].primary.owned.is_some());
    }

    #[test]
    fn claim_pending_prefers_owned_text_over_a_preexisting_offer() {
        let target = wid(32);
        let (mut inner, claim) = focused_inner(target);
        let foreign = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(foreign),
            &["text/plain;charset=utf-8"],
            Instant::now(),
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::Request("text/plain;charset=utf-8".to_owned())
        );

        install_detached(&mut inner, SelectionKind::Regular, &claim, "claimed", 1).unwrap();
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("claimed")))
        );
    }

    #[test]
    fn stale_offer_then_null_then_reconcile_does_not_keep_stale_text() {
        let target = wid(33);
        let (mut inner, claim) = focused_inner(target);
        let now = Instant::now();
        let predecessor = inner.allocate_offer_key();
        observe(&mut inner, SelectionKind::Regular, Some(predecessor), &["text/plain"], now);
        install_detached_at(&mut inner, SelectionKind::Regular, &claim, "claimed", 1, now).unwrap();

        // The selection event that was already in flight when the claim was sent.
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(predecessor),
            &["text/plain"],
            now + Duration::from_millis(1),
        );
        assert!(inner.seats[0].regular.owned.is_some());
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("claimed")))
        );

        // An authoritative empty selection ends the owned source.
        observe(&mut inner, SelectionKind::Regular, None, &[], now + Duration::from_millis(2));
        assert!(inner.seats[0].regular.owned.is_none());
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap_err()
                .io_kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn pending_claim_cannot_survive_an_authoritative_empty_selection() {
        let target = wid(34);
        let (mut inner, claim) = focused_inner(target);
        let now = Instant::now();
        install_detached_at(&mut inner, SelectionKind::Regular, &claim, "claimed", 1, now).unwrap();

        // Absence matching the pre-claim state is tolerated inside the grace period.
        observe(&mut inner, SelectionKind::Regular, None, &[], now);
        assert!(inner.seats[0].regular.owned.is_some());

        // After the grace period the same absence is authoritative.
        observe(
            &mut inner,
            SelectionKind::Regular,
            None,
            &[],
            now + CLAIM_ECHO_GRACE + Duration::from_millis(1),
        );
        assert!(inner.seats[0].regular.owned.is_none());
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap_err()
                .io_kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn selection_change_cancels_inflight_read_even_when_the_new_owner_is_us() {
        let target = wid(35);
        let (mut inner, claim) = focused_inner(target);
        install_detached(&mut inner, SelectionKind::Regular, &claim, "claimed", 1).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = inner.transfers.create(
            Some(target),
            inner.seats[0].seat_token,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        );

        let echo = inner.allocate_offer_key();
        let echo_marker = source_marker(&inner.marker_identity, 1);
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(echo),
            &["text/plain", echo_marker.as_str()],
            Instant::now(),
        );

        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted transfer, got {other:?}"),
        }
    }

    #[test]
    fn claim_cancels_a_pending_read_from_the_previous_owner_before_the_echo() {
        let target = wid(36);
        let (mut inner, claim) = focused_inner(target);
        let foreign = inner.allocate_offer_key();
        observe(&mut inner, SelectionKind::Regular, Some(foreign), &["text/plain"], Instant::now());
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = inner.transfers.create(
            Some(target),
            inner.seats[0].seat_token,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        );

        install_detached(&mut inner, SelectionKind::Regular, &claim, "claimed", 1).unwrap();

        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted transfer, got {other:?}"),
        }
    }

    #[test]
    fn internal_selection_cancel_yields_interrupted_and_wakes_once() {
        let target = wid(37);
        let (mut inner, _claim) = focused_inner(target);
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = inner.transfers.create(
            Some(target),
            inner.seats[0].seat_token,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        );

        let first = inner.allocate_offer_key();
        observe(&mut inner, SelectionKind::Regular, Some(first), &["text/plain"], Instant::now());
        let second = inner.allocate_offer_key();
        observe(&mut inner, SelectionKind::Regular, Some(second), &["text/plain"], Instant::now());

        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted transfer, got {other:?}"),
        }
        // A late worker result is ignored after the internal completion.
        slot.set_result(Ok(ClipboardContent::Text(Arc::from("late"))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(TransferHandle { slot: slot.clone() }.try_take().is_none());
    }

    #[test]
    fn registry_invalidation_is_scoped_to_seat_and_family() {
        let mut registry = TransferRegistry::default();
        let regular = registry.create(Some(wid(48)), 7, SelectionKind::Regular, None);
        let primary = registry.create(Some(wid(48)), 7, SelectionKind::Primary, None);
        let mut batch = TransitionBatch::default();
        registry.interrupt_seat(7, SelectionKind::Regular, &mut batch.interrupted);
        batch.complete();

        match (TransferHandle { slot: regular.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected the regular transfer to be interrupted, got {other:?}"),
        }
        let primary_handle = TransferHandle { slot: primary.clone() };
        assert!(primary_handle.try_take().is_none());
        assert!(!primary.is_finished());
        primary.set_result(Ok(ClipboardContent::Text(Arc::from("primary"))));
        assert_eq!(&*primary_handle.try_take().unwrap().unwrap(), "primary");
    }

    #[test]
    fn explicit_handle_cancel_suppresses_result_and_wake() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(TransferSlot::new(
            2,
            Some(wid(38)),
            2,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        ));
        let handle = TransferHandle { slot: slot.clone() };
        handle.cancel();
        slot.set_result(Ok(ClipboardContent::Text(Arc::from("late"))));
        slot.fail_internal(ClipboardError::Interrupted);
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
        assert!(handle.try_take().is_none());
    }

    #[test]
    fn internal_cancel_does_not_invoke_the_waker_under_the_clipboard_lock() {
        let target = wid(39);
        let inner = Arc::new(Mutex::new(Inner::default()));
        {
            let mut guard = inner.lock().unwrap();
            guard.add_window(target);
            guard.add_test_seat();
            guard.keyboard_focus_enter(&ObjectId::null(), 1, target);
        }
        let contended = Arc::new(AtomicBool::new(false));
        let probe = inner.clone();
        let waker: TransferWaker = Arc::new({
            let contended = contended.clone();
            move || {
                if probe.try_lock().is_err() {
                    contended.store(true, Ordering::SeqCst);
                }
            }
        });
        let slot = {
            let mut guard = inner.lock().unwrap();
            let seat_token = guard.seats[0].seat_token;
            guard.transfers.create(Some(target), seat_token, SelectionKind::Regular, Some(waker))
        };
        let key = inner.lock().unwrap().allocate_offer_key();

        run_transition(&inner, |guard| {
            let mut batch = TransitionBatch::default();
            guard.observe_selection(
                0,
                SelectionKind::Regular,
                ObservedOffer::present(key, vec!["text/plain".to_owned()]),
                Instant::now(),
                &mut batch,
            );
            ((), batch)
        });

        assert!(!contended.load(Ordering::SeqCst), "the waker ran under the clipboard lock");
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted transfer, got {other:?}"),
        }
    }

    #[test]
    fn internal_cancel_of_a_window_read_wakes_once_with_interrupted() {
        let target = wid(40);
        let (mut inner, _claim) = focused_inner(target);
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = inner.transfers.create(
            Some(target),
            inner.seats[0].seat_token,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        );
        let batch = inner.remove_window(target);
        batch.complete();
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted transfer, got {other:?}"),
        }
    }

    #[test]
    fn markers_are_unique_per_owned_source_and_leak_no_host_identity() {
        let identity = ClipboardIdentity::new();
        let first = source_marker(&identity, 1);
        let second = source_marker(&identity, 2);
        assert_ne!(first, second);
        let rest = first.strip_prefix(MARKER_PREFIX).expect("marker prefix");
        let (owner, token) = rest.rsplit_once('-').expect("owner-token marker");
        let (process, clipboard) = owner.split_once('-').expect("process-clipboard marker");
        assert_eq!(process.len(), 16);
        assert_eq!(clipboard.len(), 16);
        assert_eq!(token.len(), 16);
        assert!(process.chars().all(|character| character.is_ascii_hexdigit()));
        assert!(clipboard.chars().all(|character| character.is_ascii_hexdigit()));
        assert!(token.chars().all(|character| character.is_ascii_hexdigit()));
        assert!(!first.contains("pid"));
        assert!(!first.contains("display"));
    }

    #[test]
    fn another_process_winit_marker_is_a_foreign_text_offer() {
        let target = wid(47);
        let (mut inner, _claim) = focused_inner(target);
        let own = source_marker(&inner.marker_identity, 1);
        let rest = own.strip_prefix(MARKER_PREFIX).expect("marker prefix");
        let mut foreign = String::from(MARKER_PREFIX);
        let mut characters = rest.chars();
        if let Some(first) = characters.next() {
            foreign.push(if first == '0' { '1' } else { '0' });
        }
        foreign.extend(characters);
        assert!(!inner.marker_identity.owns(&foreign));

        let key = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(key),
            &["text/plain", foreign.as_str()],
            Instant::now(),
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::Request("text/plain".to_owned())
        );
    }

    #[test]
    fn same_process_other_clipboard_marker_is_a_foreign_text_offer() {
        let target = wid(48);
        let (mut inner, _claim) = focused_inner(target);
        let other_clipboard = ClipboardIdentity::new();
        assert_ne!(inner.marker_identity, other_clipboard);
        let marker = source_marker(&other_clipboard, 1);
        assert!(!inner.marker_identity.owns(&marker));

        let key = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(key),
            &["text/plain", marker.as_str()],
            Instant::now(),
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::Request("text/plain".to_owned())
        );
    }

    #[test]
    fn stale_offer_from_an_earlier_claim_is_not_treated_as_the_current_self_text() {
        let target = wid(41);
        let (mut inner, first_claim) = focused_inner(target);
        install_detached(&mut inner, SelectionKind::Regular, &first_claim, "first", 1).unwrap();
        let first_marker = source_marker(&inner.marker_identity, 1);

        // A second input turn claims again, replacing the first source.
        inner.keyboard_key(&ObjectId::null(), 9, target);
        let second_claim = SelectionClaim {
            target_window: target,
            caller_generation: 2,
            trigger: inner.input_origin(target).unwrap(),
        };
        install_detached(&mut inner, SelectionKind::Regular, &second_claim, "second", 2).unwrap();
        let second_marker = source_marker(&inner.marker_identity, 2);

        // The compositor can still announce the first source. It is stale, not the current
        // self text, and must not destroy the second source.
        let stale = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(stale),
            &["text/plain", first_marker.as_str()],
            Instant::now(),
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("second")))
        );

        let echo = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(echo),
            &["text/plain", second_marker.as_str()],
            Instant::now(),
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("second")))
        );
    }

    #[test]
    fn many_stale_tokens_do_not_evict_current_source_recognition() {
        let target = wid(49);
        let (mut inner, claim) = focused_inner(target);
        // Simulate a long-lived clipboard that has allocated far more sources than the old
        // 16-entry retirement deque could retain.
        for token in 1..80 {
            inner.mark_source_token(token);
        }
        install_detached(&mut inner, SelectionKind::Regular, &claim, "current", 80).unwrap();
        let old_marker = source_marker(&inner.marker_identity, 1);
        let current_marker = source_marker(&inner.marker_identity, 80);

        let stale = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(stale),
            &["text/plain", old_marker.as_str()],
            Instant::now(),
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("current"))),
            "an old own marker must not destroy the current source"
        );

        let echo = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(echo),
            &["text/plain", current_marker.as_str()],
            Instant::now(),
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("current")))
        );
    }

    #[test]
    fn closing_a_window_cancels_its_regular_and_primary_owned_sources() {
        let target = wid(42);
        let (mut inner, claim) = focused_inner(target);
        install_detached(&mut inner, SelectionKind::Regular, &claim, "regular", 1).unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &claim, "primary", 2).unwrap();
        let regular_serving = inner.seats[0].regular.owned.as_ref().unwrap().serving.clone();
        let primary_serving = inner.seats[0].primary.owned.as_ref().unwrap().serving.clone();

        let batch = inner.remove_window(target);
        batch.complete();

        assert!(inner.seats[0].regular.owned.is_none());
        assert!(inner.seats[0].primary.owned.is_none());
        assert!(regular_serving.is_cancelled());
        assert!(primary_serving.is_cancelled());
        assert!(!inner.windows.contains(&target));
    }

    #[test]
    fn closing_a_window_with_a_pending_claim_preserves_a_live_predecessor() {
        let target = wid(71);
        let (mut inner, claim) = focused_inner(target);
        let regular_predecessor = inner.allocate_offer_key();
        inner.seats[0].regular.offer = Some(OfferView {
            key: regular_predecessor,
            mime_types: vec!["text/plain".to_owned()],
            marker: None,
        });
        let primary_predecessor = inner.allocate_offer_key();
        inner.seats[0].primary.offer = Some(OfferView {
            key: primary_predecessor,
            mime_types: vec!["text/plain".to_owned()],
            marker: None,
        });

        install_detached(&mut inner, SelectionKind::Regular, &claim, "regular", 1).unwrap();
        install_detached(&mut inner, SelectionKind::Primary, &claim, "primary", 2).unwrap();
        assert!(matches!(
            inner.seats[0].regular.owned.as_ref().map(|owned| &owned.phase),
            Some(SourcePhase::Pending { predecessor: Some(key), .. }) if *key == regular_predecessor
        ));
        assert!(matches!(
            inner.seats[0].primary.owned.as_ref().map(|owned| &owned.phase),
            Some(SourcePhase::Pending { predecessor: Some(key), .. }) if *key == primary_predecessor
        ));

        let batch = inner.remove_window(target);
        batch.complete();

        // The not-yet-echoed claim is gone, but the compositor may still be serving the
        // predecessor. Keeping the logical offer readable is what prevents a repeated identical
        // protocol offer from being tombstoned forever.
        assert!(inner.seats[0].regular.owned.is_none());
        assert_eq!(
            inner.seats[0].regular.offer.as_ref().map(|offer| offer.key),
            Some(regular_predecessor)
        );
        assert!(inner.seats[0].suppressed_regular_offer.is_none());
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::Request("text/plain".to_owned())
        );

        assert!(inner.seats[0].primary.owned.is_none());
        assert_eq!(
            inner.seats[0].primary.offer.as_ref().map(|offer| offer.key),
            Some(primary_predecessor)
        );
        assert!(inner.seats[0].suppressed_primary_offer.is_none());
        assert_eq!(
            inner.seats[0]
                .primary
                .classify(SelectionKind::Primary, &inner.marker_identity)
                .unwrap(),
            OfferRead::Request("text/plain".to_owned())
        );
    }

    #[test]
    fn closed_window_source_is_not_served_to_a_later_offer_request() {
        let target = wid(43);
        let (mut inner, claim) = focused_inner(target);
        install_detached(&mut inner, SelectionKind::Regular, &claim, "regular", 1).unwrap();
        let marker = source_marker(&inner.marker_identity, 1);
        let batch = inner.remove_window(target);
        batch.complete();

        // A compositor may keep announcing the destroyed source's offer for one round trip.
        let key = inner.allocate_offer_key();
        inner.seats[0].regular.offer = Some(OfferView {
            key,
            mime_types: vec!["text/plain".to_owned(), marker.clone()],
            marker: Some(marker),
        });
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap_err()
                .io_kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn primary_global_is_independent_of_the_regular_data_device_manager() {
        assert!(require_selection_family(true, false, true).is_ok());
        assert!(require_selection_family(false, true, true).is_ok());
        assert_eq!(
            require_selection_family(false, false, true).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            require_selection_family(true, true, false).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
    }

    #[test]
    fn admission_prefers_queued_jobs_and_bounds_both_queues() {
        assert_eq!(
            admission_decision(0, 0, MAX_ACTIVE_READS, MAX_QUEUED_READS),
            AdmissionDecision::Run
        );
        // A queued job is not bypassed by a new request even when an active slot is free.
        assert_eq!(
            admission_decision(0, 1, MAX_ACTIVE_READS, MAX_QUEUED_READS),
            AdmissionDecision::Queue
        );
        assert_eq!(
            admission_decision(MAX_ACTIVE_READS, 0, MAX_ACTIVE_READS, MAX_QUEUED_READS),
            AdmissionDecision::Queue
        );
        assert_eq!(
            admission_decision(
                MAX_ACTIVE_READS,
                MAX_QUEUED_READS,
                MAX_ACTIVE_READS,
                MAX_QUEUED_READS
            ),
            AdmissionDecision::Full
        );

        assert!(admit_source_bytes(0, 1024, MAX_TOTAL_OWNED_BYTES).is_ok());
        assert!(admit_source_bytes(MAX_TOTAL_OWNED_BYTES - 10, 10, MAX_TOTAL_OWNED_BYTES).is_ok());
        assert_eq!(
            admit_source_bytes(0, MAX_SOURCE_BYTES + 1, MAX_TOTAL_OWNED_BYTES).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        assert_eq!(
            admit_source_bytes(MAX_TOTAL_OWNED_BYTES, 1, MAX_TOTAL_OWNED_BYTES).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
    }

    #[test]
    fn admission_rejects_an_overflowing_total_deadline() {
        let completion_waker: TransferWaker = Arc::new(|| {});
        let options = TransferOptions {
            target_window: wid(44),
            caller_generation: 1,
            total_deadline: Duration::MAX,
            max_bytes: 16,
            completion_waker,
        };
        assert_eq!(
            validate_transfer_options(&options).unwrap_err().io_kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn retained_writer_reference_keeps_a_replaced_source_charged() {
        let target = wid(209);
        let (mut inner, claim) = focused_inner(target);
        // A small budget makes the lifetime accounting exact without 16 MiB test payloads.
        inner.source_bytes = Arc::new(SourceBytesBudget::new(12));
        let now = Instant::now();
        install_detached_at(&mut inner, SelectionKind::Regular, &claim, "aaaa", 40, now).unwrap();
        assert_eq!(inner.source_bytes.used(), 4);

        // A blocked writer holds the payload and its shared lifetime charge. This clone must not
        // double-count the four bytes while the source is still installed.
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let writer = {
            let owned = inner.seats[0].regular.owned.as_ref().unwrap();
            assert_eq!(Arc::strong_count(&owned.source_bytes), 1);
            WriteJob {
                serving: Arc::downgrade(&owned.serving),
                fd: write,
                payload: owned.payload.clone(),
                _source_bytes: owned.source_bytes.clone(),
                deadline: now + Duration::from_secs(30),
            }
        };
        assert_eq!(
            Arc::strong_count(&inner.seats[0].regular.owned.as_ref().unwrap().source_bytes),
            2
        );
        assert_eq!(inner.source_bytes.used(), 4, "a shared clone is charged only once");

        // Replacing the source admits both allocations while the writer still retains the old one.
        inner.keyboard_key_at(0, 5, target);
        let second = active_claim(&mut inner, target);
        install_detached_at(
            &mut inner,
            SelectionKind::Regular,
            &second,
            "bbbb",
            41,
            now + Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(inner.source_bytes.used(), 8, "the replaced payload stays charged");

        // An eight-byte replacement needs the installed four-byte source, the writer's four-byte
        // charge, and eight new bytes at once: 16 > 12. A replacement discount would wrongly
        // admit it by pretending the installed source's bytes were already released.
        inner.keyboard_key_at(0, 6, target);
        let third = active_claim(&mut inner, target);
        let error = install_detached_at(
            &mut inner,
            SelectionKind::Regular,
            &third,
            "cccccccc",
            42,
            now + Duration::from_millis(2),
        )
        .expect_err("the retained old payload must keep the replacement over budget");
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert_eq!(inner.source_bytes.used(), 8);

        // Cancelling/releasing the blocked writer releases the replaced payload's charge, so the
        // same replacement now fits.
        drop(writer);
        drop(read);
        assert_eq!(inner.source_bytes.used(), 4, "the writer releases the replaced charge");
        install_detached_at(
            &mut inner,
            SelectionKind::Regular,
            &third,
            "cccccccc",
            43,
            now + Duration::from_millis(3),
        )
        .unwrap();
        assert_eq!(inner.source_bytes.used(), 8);
    }

    #[test]
    fn held_previous_source_blocks_a_replacement_discount() {
        let target = wid(211);
        let (mut inner, claim) = focused_inner(target);
        inner.source_bytes = Arc::new(SourceBytesBudget::new(8));
        let now = Instant::now();
        install_detached_at(&mut inner, SelectionKind::Regular, &claim, "aaaa", 50, now).unwrap();
        assert_eq!(inner.source_bytes.used(), 4);

        // Install a replacement but retain its predecessor as the production caller does until
        // the protocol flush succeeds. The held source must remain charged.
        inner.keyboard_key_at(0, 5, target);
        let second = active_claim(&mut inner, target);
        let mut second_batch = TransitionBatch::default();
        let second_install = ClaimInstall {
            kind: SelectionKind::Regular,
            token: 51,
            marker: source_marker(&inner.marker_identity, 51),
            payload: OwnedPayload::Text(Arc::from("bbbb")),
            target_window: target,
            caller_generation: second.caller_generation,
            protocol: None,
        };
        let installed = inner
            .admit_claim(second_install, &second, now + Duration::from_millis(1), &mut second_batch)
            .unwrap();
        second_batch.complete();
        let held_previous = installed.previous.expect("the replacement must retain the old source");
        assert_eq!(inner.source_bytes.used(), 8);
        assert_eq!(Arc::strong_count(&held_previous.source_bytes), 1);

        // A newer regular attempt that would fit only by discounting the installed source must be
        // refused while the held predecessor is charged.
        inner.keyboard_key_at(0, 6, target);
        let reservation_claim = active_claim(&mut inner, target);
        let reservation = inner
            .reserve_image_claim_at(&reservation_claim, now + Duration::from_millis(2))
            .unwrap();

        inner.keyboard_key_at(0, 7, target);
        let third = active_claim(&mut inner, target);
        let mut third_batch = TransitionBatch::default();
        let third_install = ClaimInstall {
            kind: SelectionKind::Regular,
            token: 52,
            marker: source_marker(&inner.marker_identity, 52),
            payload: OwnedPayload::Text(Arc::from("cccc")),
            target_window: target,
            caller_generation: third.caller_generation,
            protocol: None,
        };
        let error = match inner.admit_claim(
            third_install,
            &third,
            now + Duration::from_millis(3),
            &mut third_batch,
        ) {
            Ok(_) => panic!("a held previous source must keep the replacement over budget"),
            Err(error) => error,
        };
        third_batch.complete();
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert_eq!(inner.source_bytes.used(), 8, "the held previous source is never discounted");

        // Refusal preserves the current source, and the newer regular attempt still invalidates
        // the older image reservation.
        {
            let current =
                inner.seats[0].regular.owned.as_ref().expect("the refused claim keeps its source");
            assert_eq!(current.token, 51);
            assert_eq!(current.payload, OwnedPayload::Text(Arc::from("bbbb")));
        }
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nheld previous");
        let error = match install_reserved_detached(
            &mut inner,
            reservation,
            png,
            53,
            now + Duration::from_millis(4),
        ) {
            Ok(_) => panic!("the newer regular attempt must invalidate the reservation"),
            Err(error) => error,
        };
        assert!(matches!(error, ClipboardError::Interrupted));
        assert_eq!(budget.used(), 0);

        drop(held_previous);
        assert_eq!(
            inner.source_bytes.used(),
            4,
            "dropping the held source releases only its charge"
        );
    }

    #[test]
    fn write_job_debug_redacts_selection_text() {
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let job = WriteJob {
            serving: Weak::<SourceServeState>::new(),
            fd: write,
            payload: OwnedPayload::Text(Arc::from("private clipboard payload")),
            _source_bytes: test_source_charge(0),
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let debug = format!("{job:?}");
        assert!(!debug.contains("private clipboard payload"));
        assert!(debug.contains("payload_bytes"));
        drop(read);
    }

    fn saturate_reads(workers: &Workers) -> (Vec<Arc<TransferSlot>>, Vec<OwnedFd>) {
        let mut slots = Vec::new();
        let mut writers = Vec::new();
        for request in 0..MAX_ACTIVE_READS {
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            let slot = Arc::new(TransferSlot::new(
                request as u64,
                Some(wid(45)),
                9,
                SelectionKind::Regular,
                None,
            ));
            let reservation = workers.reserve_read().unwrap();
            assert_eq!(reservation, Reservation::Active);
            workers
                .submit_reserved_read(reservation, ReadJob {
                    slot: Arc::downgrade(&slot),
                    fd: read,
                    deadline: Instant::now() + Duration::from_secs(30),
                    max_bytes: 16,
                    kind: ReadKind::Text,
                    image_permit: None,
                })
                .unwrap();
            slots.push(slot);
            writers.push(write);
        }
        (slots, writers)
    }

    fn saturate_writes(workers: &Workers, serving: &Arc<SourceServeState>) -> Vec<OwnedFd> {
        let mut readers = Vec::new();
        let text: Arc<str> = Arc::from("x".repeat(1024 * 1024));
        for _ in 0..MAX_ACTIVE_WRITES {
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            let reservation = workers.reserve_write().unwrap();
            assert_eq!(reservation, Reservation::Active);
            workers
                .submit_reserved_write(reservation, WriteJob {
                    serving: Arc::downgrade(serving),
                    fd: write,
                    payload: OwnedPayload::Text(text.clone()),
                    _source_bytes: test_source_charge(0),
                    deadline: Instant::now() + Duration::from_secs(30),
                })
                .unwrap();
            readers.push(read);
        }
        readers
    }

    #[test]
    fn worker_queues_have_hard_active_and_queued_bounds_and_admit_real_pipes() {
        let workers = Workers::default();
        let mut writers = Vec::new();
        let mut slots = Vec::new();
        for request in 0..(MAX_ACTIVE_READS + MAX_QUEUED_READS) {
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            let slot = Arc::new(TransferSlot::new(
                request as u64,
                Some(wid(45)),
                4,
                SelectionKind::Regular,
                None,
            ));
            let job = ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            };
            let reservation = workers.reserve_read().unwrap();
            workers.submit_reserved_read(reservation, job).unwrap();
            writers.push(write);
            slots.push(slot);
        }
        {
            let reads = workers.inner.reads.lock().unwrap();
            assert_eq!(reads.active, MAX_ACTIVE_READS);
            assert_eq!(reads.jobs.len(), MAX_QUEUED_READS);
            assert_eq!(reads.reserved, 0);
        }

        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let slot = Arc::new(TransferSlot::new(99, Some(wid(45)), 4, SelectionKind::Regular, None));
        let error = workers
            .reserve_read()
            .and_then(|reservation| {
                workers.submit_reserved_read(reservation, ReadJob {
                    slot: Arc::downgrade(&slot),
                    fd: read,
                    deadline: Instant::now() + Duration::from_secs(30),
                    max_bytes: 16,
                    kind: ReadKind::Text,
                    image_permit: None,
                })
            })
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::WouldBlock);

        for slot in &slots {
            slot.cancel();
        }
        slot.cancel();
        drop(writers);
        drop(write);
        workers.shutdown();
    }

    #[test]
    fn write_queue_has_hard_active_and_queued_bounds() {
        let workers = Workers::default();
        let serving = Arc::new(SourceServeState::default());
        let mut readers = Vec::new();
        let text: Arc<str> = Arc::from("x".repeat(1024 * 1024));
        for _ in 0..(MAX_ACTIVE_WRITES + MAX_QUEUED_WRITES) {
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            readers.push(read);
            let job = WriteJob {
                serving: Arc::downgrade(&serving),
                fd: write,
                payload: OwnedPayload::Text(text.clone()),
                _source_bytes: test_source_charge(0),
                deadline: Instant::now() + Duration::from_secs(30),
            };
            let reservation = workers.reserve_write().unwrap();
            workers.submit_reserved_write(reservation, job).unwrap();
        }
        {
            let writes = workers.inner.writes.lock().unwrap();
            assert_eq!(writes.active, MAX_ACTIVE_WRITES);
            assert_eq!(writes.jobs.len(), MAX_QUEUED_WRITES);
            assert_eq!(writes.reserved, 0);
        }
        let error = workers.reserve_write().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::WouldBlock);

        serving.cancel();
        workers.shutdown();
        drop(readers);
    }

    #[test]
    fn read_admission_cannot_observe_a_drained_queue_during_prune() {
        let workers = Workers::default();
        // No real coordinator: these tests drive prune at an exact interleaving.
        *workers.inner.coordinator.lock().unwrap() = true;
        let mut slots = Vec::new();
        let mut writers = Vec::new();
        // Hold every active permit without starting workers, then fill the queue with real jobs.
        workers.inner.reads.lock().unwrap().active = MAX_ACTIVE_READS;
        for request in 0..MAX_QUEUED_READS {
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            let slot = Arc::new(TransferSlot::new(
                request as u64,
                Some(wid(60)),
                9,
                SelectionKind::Regular,
                None,
            ));
            let reservation = workers.reserve_read().unwrap();
            assert_eq!(reservation, Reservation::Queued);
            workers
                .submit_reserved_read(reservation, ReadJob {
                    slot: Arc::downgrade(&slot),
                    fd: read,
                    deadline: Instant::now() + Duration::from_secs(30),
                    max_bytes: 16,
                    kind: ReadKind::Text,
                    image_permit: None,
                })
                .unwrap();
            slots.push(slot);
            writers.push(write);
        }

        // The probe runs after the queue has been partitioned and before the survivors are put
        // back, while the queue lock is still held. Admission must not run during that window.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        *workers.inner.read_prune_probe.lock().unwrap() = Some(PruneProbe(Box::new(move || {
            entered_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        })));
        let prune_workers = Workers { inner: Arc::clone(&workers.inner) };
        let prune = std::thread::spawn(move || prune_workers.inner.prune_queued());
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("prune should reach its probe while holding the read queue lock");

        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let admit_workers = Workers { inner: Arc::clone(&workers.inner) };
        let admit = std::thread::spawn(move || {
            attempt_tx.send(admit_workers.reserve_read()).unwrap();
        });
        // A buggy prune that released the lock with an empty queue would admit here; the fixed
        // implementation blocks until the queue is whole again.
        assert!(
            attempt_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "admission advanced while prune held an empty read queue"
        );
        resume_tx.send(()).unwrap();
        prune.join().unwrap();
        assert_eq!(
            attempt_rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        admit.join().unwrap();

        let reads = workers.inner.reads.lock().unwrap();
        assert_eq!(reads.active, MAX_ACTIVE_READS);
        assert_eq!(reads.jobs.len(), MAX_QUEUED_READS);
        assert_eq!(reads.reserved, 0);
        drop(reads);
        for slot in &slots {
            slot.cancel();
        }
        drop(writers);
        workers.shutdown();
    }

    #[test]
    fn write_admission_cannot_observe_a_drained_queue_during_prune() {
        let workers = Workers::default();
        // No real coordinator: these tests drive prune at an exact interleaving.
        *workers.inner.coordinator.lock().unwrap() = true;
        let serving = Arc::new(SourceServeState::default());
        let mut readers = Vec::new();
        workers.inner.writes.lock().unwrap().active = MAX_ACTIVE_WRITES;
        for _ in 0..MAX_QUEUED_WRITES {
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            let reservation = workers.reserve_write().unwrap();
            assert_eq!(reservation, Reservation::Queued);
            workers
                .submit_reserved_write(reservation, WriteJob {
                    serving: Arc::downgrade(&serving),
                    fd: write,
                    payload: OwnedPayload::Text(Arc::from("queued")),
                    _source_bytes: test_source_charge(0),
                    deadline: Instant::now() + Duration::from_secs(30),
                })
                .unwrap();
            readers.push(read);
        }

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        *workers.inner.write_prune_probe.lock().unwrap() = Some(PruneProbe(Box::new(move || {
            entered_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        })));
        let prune_workers = Workers { inner: Arc::clone(&workers.inner) };
        let prune = std::thread::spawn(move || prune_workers.inner.prune_queued());
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("prune should reach its probe while holding the write queue lock");

        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let admit_workers = Workers { inner: Arc::clone(&workers.inner) };
        let admit = std::thread::spawn(move || {
            attempt_tx.send(admit_workers.reserve_write()).unwrap();
        });
        assert!(
            attempt_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "admission advanced while prune held an empty write queue"
        );
        resume_tx.send(()).unwrap();
        prune.join().unwrap();
        assert_eq!(
            attempt_rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        admit.join().unwrap();

        let writes = workers.inner.writes.lock().unwrap();
        assert_eq!(writes.active, MAX_ACTIVE_WRITES);
        assert_eq!(writes.jobs.len(), MAX_QUEUED_WRITES);
        assert_eq!(writes.reserved, 0);
        drop(writes);
        serving.cancel();
        workers.shutdown();
        drop(readers);
    }

    #[test]
    fn expired_queued_read_wakes_after_prune_releases_the_queue_lock() {
        let workers = Workers::default();
        // No real coordinator: these tests drive prune at an exact interleaving.
        *workers.inner.coordinator.lock().unwrap() = true;
        workers.inner.reads.lock().unwrap().active = MAX_ACTIVE_READS;
        let wakes = Arc::new(AtomicUsize::new(0));
        let queue_locked = Arc::new(AtomicBool::new(false));
        let wakes_in_waker = Arc::clone(&wakes);
        let queue_locked_in_waker = Arc::clone(&queue_locked);
        let workers_in_waker = Workers { inner: Arc::clone(&workers.inner) };
        let waker: TransferWaker = Arc::new(move || {
            wakes_in_waker.fetch_add(1, Ordering::SeqCst);
            match workers_in_waker.inner.reads.try_lock() {
                Ok(guard) => drop(guard),
                Err(_) => queue_locked_in_waker.store(true, Ordering::SeqCst),
            }
        });
        let slot =
            Arc::new(TransferSlot::new(301, Some(wid(61)), 9, SelectionKind::Regular, Some(waker)));
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let reservation = workers.reserve_read().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_millis(5),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(15));

        workers.inner.prune_queued();

        assert!(!queue_locked.load(Ordering::SeqCst), "completion waker ran under the queue lock");
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::TimedOut),
            other => panic!("expected an expired queued read, got {other:?}"),
        }
        let reads = workers.inner.reads.lock().unwrap();
        assert!(reads.jobs.is_empty());
        assert_eq!(reads.active, MAX_ACTIVE_READS);
        assert_eq!(reads.reserved, 0);
        drop(reads);
        slot.cancel();
        drop(write);
        workers.shutdown();
    }

    #[test]
    fn expired_queued_write_is_reported_after_prune() {
        let workers = Workers::default();
        // No real coordinator: this test drives prune at an exact point.
        *workers.inner.coordinator.lock().unwrap() = true;
        workers.inner.writes.lock().unwrap().active = MAX_ACTIVE_WRITES;
        let serving = Arc::new(SourceServeState::default());
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let reservation = workers.reserve_write().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_write(reservation, WriteJob {
                serving: Arc::downgrade(&serving),
                fd: write,
                payload: OwnedPayload::Text(Arc::from("queued")),
                _source_bytes: test_source_charge(0),
                deadline: Instant::now() + Duration::from_millis(5),
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(15));

        workers.inner.prune_queued();

        assert!(
            serving.error().is_some_and(|error| error.contains("deadline")),
            "an expired queued write must record its deadline error"
        );
        let writes = workers.inner.writes.lock().unwrap();
        assert!(writes.jobs.is_empty());
        assert_eq!(writes.active, MAX_ACTIVE_WRITES);
        assert_eq!(writes.reserved, 0);
        drop(writes);
        serving.cancel();
        drop(read);
        workers.shutdown();
    }

    #[test]
    fn cancelling_an_active_read_ends_the_worker_permit() {
        let workers = Workers::default();
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let slot = Arc::new(TransferSlot::new(7, Some(wid(46)), 5, SelectionKind::Regular, None));
        let job = ReadJob {
            slot: Arc::downgrade(&slot),
            fd: read,
            deadline: Instant::now() + Duration::from_secs(2),
            max_bytes: 16,
            kind: ReadKind::Text,
            image_permit: None,
        };
        let reservation = workers.reserve_read().unwrap();
        workers.submit_reserved_read(reservation, job).unwrap();
        slot.cancel();

        // The reader notices cancellation after at most one poll slice.
        assert!(wait_until(Duration::from_secs(1), || {
            workers.inner.reads.lock().unwrap().active == 0
        }));
        assert!(TransferHandle { slot: slot.clone() }.try_take().is_none());
        drop(write);
        workers.shutdown();
    }

    #[test]
    fn queued_read_cancellation_releases_fd_before_an_active_slot_frees() {
        let workers = Workers::default();
        let (slots, writers) = saturate_reads(&workers);
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let queued_slot =
            Arc::new(TransferSlot::new(99, Some(wid(45)), 9, SelectionKind::Regular, None));
        let reservation = workers.reserve_read().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&queued_slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            })
            .unwrap();

        queued_slot.cancel();
        assert!(wait_until(Duration::from_secs(2), || {
            workers.inner.reads.lock().unwrap().jobs.is_empty()
        }));
        assert!(TransferHandle { slot: queued_slot.clone() }.try_take().is_none());
        // The queued read end is closed even though every active reader still holds its permit.
        assert_eq!(rustix::io::write(&write, b"x").unwrap_err(), rustix::io::Errno::PIPE);
        assert_eq!(workers.inner.reads.lock().unwrap().active, MAX_ACTIVE_READS);

        for slot in &slots {
            slot.cancel();
        }
        drop(writers);
        workers.shutdown();
    }

    #[test]
    fn queued_read_internal_invalidation_wakes_once_and_releases_fd() {
        let workers = Workers::default();
        let (slots, writers) = saturate_reads(&workers);
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let queued_slot = Arc::new(TransferSlot::new(
            101,
            Some(wid(45)),
            9,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        ));
        let reservation = workers.reserve_read().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&queued_slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            })
            .unwrap();

        queued_slot.fail_internal(ClipboardError::Interrupted);
        assert!(wait_until(Duration::from_secs(2), || {
            workers.inner.reads.lock().unwrap().jobs.is_empty()
        }));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(rustix::io::write(&write, b"x").unwrap_err(), rustix::io::Errno::PIPE);
        match (TransferHandle { slot: queued_slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted queued read, got {other:?}"),
        }

        for slot in &slots {
            slot.cancel();
        }
        drop(writers);
        workers.shutdown();
    }

    #[test]
    fn queued_read_deadline_completes_with_timed_out_without_starting_the_reader() {
        let workers = Workers::default();
        let (slots, writers) = saturate_reads(&workers);
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        // Data is already present, so a reader that started would succeed instead of timing out.
        // The queued job is never promoted while every active reader holds its permit.
        rustix::io::write(&write, b"data").unwrap();
        drop(write);
        let queued_slot =
            Arc::new(TransferSlot::new(100, Some(wid(45)), 9, SelectionKind::Regular, None));
        let reservation = workers.reserve_read().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&queued_slot),
                fd: read,
                deadline: Instant::now() + Duration::from_millis(80),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            })
            .unwrap();

        assert!(wait_until(Duration::from_secs(2), || queued_slot.is_finished()));
        match (TransferHandle { slot: queued_slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::TimedOut),
            other => panic!("expected a timed-out queued read, got {other:?}"),
        }

        for slot in &slots {
            slot.cancel();
        }
        drop(writers);
        workers.shutdown();
    }

    #[test]
    fn queued_write_cancellation_releases_pipe_and_text() {
        let workers = Workers::default();
        let serving = Arc::new(SourceServeState::default());
        let readers = saturate_writes(&workers, &serving);
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let queued_serving = Arc::new(SourceServeState::default());
        let source_budget = Arc::new(SourceBytesBudget::new(1024));
        let reservation = workers.reserve_write().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_write(reservation, WriteJob {
                serving: Arc::downgrade(&queued_serving),
                fd: write,
                payload: OwnedPayload::Text(Arc::from("queued")),
                _source_bytes: Arc::new(SourceBytesCharge::new(Arc::clone(&source_budget), 64)),
                deadline: Instant::now() + Duration::from_secs(30),
            })
            .unwrap();
        assert_eq!(source_budget.used(), 64);

        queued_serving.cancel();
        assert!(wait_until(Duration::from_secs(2), || {
            workers.inner.writes.lock().unwrap().jobs.is_empty()
        }));
        assert_eq!(source_budget.used(), 0, "the discarded job releases its source charge");
        let mut buffer = [0u8; 8];
        assert_eq!(rustix::io::read(&read, &mut buffer).unwrap(), 0);

        serving.cancel();
        drop(readers);
        workers.shutdown();
    }

    #[test]
    fn promote_write_discards_a_cancelled_job_and_releases_its_source_charge() {
        let workers = Workers::default();
        let serving = Arc::new(SourceServeState::default());
        serving.cancel();
        let source_budget = Arc::new(SourceBytesBudget::new(1024));
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        {
            let mut writes = workers.inner.writes.lock().unwrap();
            writes.active = 0;
            writes.jobs.push_back(WriteJob {
                serving: Arc::downgrade(&serving),
                fd: write,
                payload: OwnedPayload::Text(Arc::from("discarded")),
                _source_bytes: Arc::new(SourceBytesCharge::new(Arc::clone(&source_budget), 64)),
                deadline: Instant::now() + Duration::from_secs(30),
            });
        }
        assert_eq!(source_budget.used(), 64);

        Workers { inner: Arc::clone(&workers.inner) }.promote_write();
        assert_eq!(source_budget.used(), 0, "the discarded job releases its source charge");
        let mut buffer = [0u8; 8];
        assert_eq!(rustix::io::read(&read, &mut buffer).unwrap(), 0);
        workers.shutdown();
    }

    #[test]
    fn queued_write_deadline_closes_pipe_without_starting_the_writer() {
        let workers = Workers::default();
        let serving = Arc::new(SourceServeState::default());
        let readers = saturate_writes(&workers, &serving);
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let queued_serving = Arc::new(SourceServeState::default());
        let reservation = workers.reserve_write().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_write(reservation, WriteJob {
                serving: Arc::downgrade(&queued_serving),
                fd: write,
                // Smaller than the pipe capacity: a writer that started would deliver it.
                payload: OwnedPayload::Text(Arc::from("queued")),
                _source_bytes: test_source_charge(0),
                deadline: Instant::now() + Duration::from_millis(80),
            })
            .unwrap();

        assert!(wait_until(Duration::from_secs(2), || queued_serving.error().is_some()));
        assert!(queued_serving.error().unwrap().contains("deadline"));
        let mut buffer = [0u8; 8];
        assert_eq!(rustix::io::read(&read, &mut buffer).unwrap(), 0);

        serving.cancel();
        drop(readers);
        workers.shutdown();
    }

    fn pipe_reader(
        payload: usize,
        chunk: usize,
        delay: Duration,
    ) -> (OwnedFd, std::thread::JoinHandle<()>) {
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let writer = std::thread::spawn(move || {
            let bytes = vec![b'x'; payload];
            for part in bytes.chunks(chunk.max(1)) {
                if rustix::io::write(&write, part).is_err() {
                    return;
                }
                std::thread::sleep(delay);
            }
        });
        (read, writer)
    }

    #[test]
    fn peer_death_after_successful_receive_is_accepted_as_empty_eof_ambiguity() {
        // The peer accepted the receive request and then closed the write end without sending
        // anything. A pipe EOF is indistinguishable from a genuinely empty selection with this
        // protocol shape, so `Ok("")` is the accepted ambiguity, not evidence that the peer was
        // alive after the request was queued.
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        drop(write);
        let text = read_pipe(&read, Instant::now() + Duration::from_secs(1), 16, || false).unwrap();
        assert_eq!(&*text, "");
    }

    #[test]
    fn pipe_empty_is_success_not_absence() {
        let (read, writer) = pipe_reader(0, 1, Duration::ZERO);
        let cancelled = AtomicBool::new(false);
        let text = read_pipe(&read, Instant::now() + Duration::from_secs(1), 16, || {
            cancelled.load(Ordering::Acquire)
        })
        .unwrap();
        assert_eq!(&*text, "");
        writer.join().unwrap();
    }

    #[test]
    fn pipe_trickle_obeys_one_total_deadline() {
        let (read, writer) = pipe_reader(20, 1, Duration::from_millis(40));
        let cancelled = AtomicBool::new(false);
        let started = Instant::now();
        let error = read_pipe(&read, started + Duration::from_millis(180), 1024, || {
            cancelled.load(Ordering::Acquire)
        })
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = writer.join();
    }

    #[test]
    fn serving_write_has_a_total_deadline_and_is_cancellable() {
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let text = "x".repeat(1024 * 1024);
        let cancelled = AtomicBool::new(false);
        let started = Instant::now();
        let error = write_pipe(&write, &text, started + Duration::from_millis(120), || {
            cancelled.load(Ordering::Acquire)
        })
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(read);

        let (_read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let cancelled = AtomicBool::new(true);
        let error = write_pipe(&write, "text", Instant::now() + Duration::from_secs(1), || {
            cancelled.load(Ordering::Acquire)
        })
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
    }

    #[test]
    fn pipe_oversized_and_cancelled_paths_close_cleanly() {
        let (read, writer) = pipe_reader(64, 64, Duration::ZERO);
        let cancelled = AtomicBool::new(false);
        let error = read_pipe(&read, Instant::now() + Duration::from_secs(1), 16, || {
            cancelled.load(Ordering::Acquire)
        })
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        let _ = writer.join();

        let (read, writer) = pipe_reader(4, 1, Duration::from_millis(100));
        let cancelled = AtomicBool::new(true);
        let error = read_pipe(&read, Instant::now() + Duration::from_secs(1), 16, || {
            cancelled.load(Ordering::Acquire)
        })
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
        let _ = writer.join();
    }
    #[test]
    fn flush_disposition_treats_would_block_as_queued_and_fatal_as_error() {
        assert!(flush_queued(Ok(())).is_ok());
        let would_block = WaylandError::Io(io::Error::from(ErrorKind::WouldBlock));
        assert!(flush_queued(Err(would_block)).is_ok());
        let fatal = WaylandError::Io(io::Error::new(ErrorKind::BrokenPipe, "closed"));
        let error = flush_queued(Err(fatal)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Other);
    }

    #[test]
    fn typed_disconnect_mapping_preserves_kind_without_changing_legacy_flush() {
        let broken = WaylandError::Io(io::Error::new(ErrorKind::BrokenPipe, "closed"));
        assert!(matches!(flush_queued_typed(Err(broken)), Err(ClipboardError::Disconnected)));
        let not_connected = WaylandError::Io(io::Error::from(ErrorKind::NotConnected));
        assert!(matches!(
            flush_queued_typed(Err(not_connected)),
            Err(ClipboardError::Disconnected)
        ));
        let reset = WaylandError::Io(io::Error::new(ErrorKind::ConnectionReset, "reset"));
        match flush_queued_typed(Err(reset)) {
            Err(ClipboardError::Backend(error)) => {
                assert_eq!(error.kind(), ErrorKind::ConnectionReset)
            },
            other => panic!("expected the original backend kind, got {other:?}"),
        }

        // The legacy compatibility path still deliberately reports a fatal flush as Other.
        let fatal = WaylandError::Io(io::Error::new(ErrorKind::BrokenPipe, "closed"));
        assert_eq!(flush_queued(Err(fatal)).unwrap_err().kind(), ErrorKind::Other);
    }

    #[test]
    fn fatal_claim_rollback_restores_the_previous_source() {
        let target = wid(50);
        let (mut inner, first_claim) = focused_inner(target);
        install_detached(&mut inner, SelectionKind::Regular, &first_claim, "first", 1).unwrap();

        inner.keyboard_key(&ObjectId::null(), 2, target);
        let second_claim = SelectionClaim {
            target_window: target,
            caller_generation: 2,
            trigger: inner.input_origin(target).unwrap(),
        };
        let marker = source_marker(&inner.marker_identity, 2);
        let mut batch = TransitionBatch::default();
        let installed = inner
            .admit_claim(
                ClaimInstall {
                    kind: SelectionKind::Regular,
                    token: 2,
                    marker,
                    payload: OwnedPayload::Text(Arc::from("second")),
                    target_window: target,
                    caller_generation: 2,
                    protocol: None,
                },
                &second_claim,
                Instant::now(),
                &mut batch,
            )
            .unwrap();
        batch.complete();

        let unrestored =
            inner.rollback_claim(SelectionKind::Regular, installed.token, installed.previous);
        assert!(unrestored.is_none());
        let owned = inner.seats[0].regular.owned.as_ref().unwrap();
        assert_eq!(owned.payload.as_text().unwrap().as_ref(), "first");
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::SelfContent(OwnedPayload::Text(Arc::from("first")))
        );
    }

    #[test]
    fn source_cancellation_clears_predecessor_offer_and_interrupts_reads() {
        let target = wid(51);
        let (mut inner, claim) = focused_inner(target);
        let predecessor = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Regular,
            Some(predecessor),
            &["text/plain"],
            Instant::now(),
        );
        install_detached(&mut inner, SelectionKind::Regular, &claim, "ours", 1).unwrap();

        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = inner.transfers.create(
            Some(target),
            inner.seats[0].seat_token,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        );
        let mut batch = TransitionBatch::default();
        inner.cancel_owned_source_at(0, SelectionKind::Regular, &mut batch);
        batch.complete();

        assert!(inner.seats[0].regular.owned.is_none());
        assert!(inner.seats[0].regular.offer.is_none());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        match (TransferHandle { slot }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted transfer, got {other:?}"),
        }
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap_err()
                .io_kind(),
            ErrorKind::NotFound
        );

        // A later authoritative selection is still readable.
        let next = inner.allocate_offer_key();
        observe(&mut inner, SelectionKind::Regular, Some(next), &["text/plain"], Instant::now());
        assert_eq!(
            inner.seats[0]
                .regular
                .classify(SelectionKind::Regular, &inner.marker_identity)
                .unwrap(),
            OfferRead::Request("text/plain".to_owned())
        );
    }

    #[test]
    fn primary_source_cancellation_clears_offer_and_reads() {
        let target = wid(52);
        let (mut inner, claim) = focused_inner(target);
        let predecessor = inner.allocate_offer_key();
        observe(
            &mut inner,
            SelectionKind::Primary,
            Some(predecessor),
            &["text/plain;charset=utf-8"],
            Instant::now(),
        );
        install_detached(&mut inner, SelectionKind::Primary, &claim, "primary", 1).unwrap();
        let slot = inner.transfers.create(
            Some(target),
            inner.seats[0].seat_token,
            SelectionKind::Primary,
            None,
        );
        let mut batch = TransitionBatch::default();
        inner.cancel_owned_source_at(0, SelectionKind::Primary, &mut batch);
        batch.complete();

        assert!(inner.seats[0].primary.owned.is_none());
        assert!(inner.seats[0].primary.offer.is_none());
        match (TransferHandle { slot }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Interrupted),
            other => panic!("expected an interrupted PRIMARY transfer, got {other:?}"),
        }
        assert_eq!(
            inner.seats[0]
                .primary
                .classify(SelectionKind::Primary, &inner.marker_identity)
                .unwrap_err()
                .io_kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn keyboard_focus_loss_clears_legacy_serial_and_rejects_store() {
        let target = wid(53);
        let (mut inner, _claim) = focused_inner(target);
        assert!(inner.seats[0].serial.is_some());
        inner.keyboard_focus_leave(&ObjectId::null());
        assert!(inner.seats[0].serial.is_none());
        assert!(inner.input_origin(target).is_err());

        let marker = source_marker(&inner.marker_identity, 9);
        let mut batch = TransitionBatch::default();
        let error = inner
            .install_legacy_source(
                SelectionKind::Regular,
                Arc::from("stale"),
                9,
                marker,
                None,
                &mut batch,
            )
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Other);
    }

    #[test]
    fn self_paste_and_output_conversion_obey_the_byte_cap() {
        assert_eq!(
            checked_self_text(Arc::from("12345"), 4).unwrap_err().io_kind(),
            ErrorKind::InvalidData
        );
        assert_eq!(&*checked_self_text(Arc::from("1234"), 4).unwrap(), "1234");

        let invalid = text_from_bytes(vec![0xff; 16], 4).unwrap_err();
        assert_eq!(invalid.kind(), ErrorKind::InvalidData);
        let normalized = text_from_bytes(b"a\r\nb\rc".to_vec(), 8).unwrap();
        assert_eq!(&*normalized, "a\nb\nc");
    }

    #[test]
    fn transfer_options_reject_oversized_limits() {
        let completion_waker: TransferWaker = Arc::new(|| {});
        for (deadline, max_bytes) in [
            (MAX_TOTAL_DEADLINE + Duration::from_millis(1), 16),
            (Duration::from_secs(1), MAX_TRANSFER_BYTES + 1),
            (Duration::from_secs(1), 0),
        ] {
            let options = TransferOptions {
                target_window: wid(54),
                caller_generation: 1,
                total_deadline: deadline,
                max_bytes,
                completion_waker: completion_waker.clone(),
            };
            assert_eq!(
                validate_transfer_options(&options).unwrap_err().io_kind(),
                ErrorKind::InvalidInput
            );
        }
    }

    /// A deterministic stand-in for the raw PRIMARY offer. `is_alive` models SCTK's destruction
    /// of the proxy; `request_receive` counts protocol requests and can deliver a payload.
    #[derive(Debug)]
    struct FakePrimaryOffer {
        alive: AtomicBool,
        requests: AtomicUsize,
        payload: &'static [u8],
    }

    impl FakePrimaryOffer {
        fn new(alive: bool, payload: &'static [u8]) -> Arc<Self> {
            Arc::new(Self { alive: AtomicBool::new(alive), requests: AtomicUsize::new(0), payload })
        }

        fn destroy(&self) {
            self.alive.store(false, Ordering::SeqCst);
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }
    }

    impl PrimaryOfferProxy for FakePrimaryOffer {
        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }

        fn request_receive(&self, _mime: String, fd: BorrowedFd<'_>) -> io::Result<()> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            if !self.payload.is_empty() {
                rustix::io::write(fd, self.payload).map_err(io::Error::from)?;
            }
            Ok(())
        }
    }

    fn proxy(offer: &Arc<FakePrimaryOffer>) -> Arc<dyn PrimaryOfferProxy> {
        offer.clone() as Arc<dyn PrimaryOfferProxy>
    }

    #[test]
    fn primary_checked_receive_distinguishes_dead_raw_offer_from_valid_empty() {
        let dead = FakePrimaryOffer::new(false, b"");
        let error = receive_primary_offer(dead.as_ref(), "text/plain".into()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(dead.requests(), 0);

        let empty = FakePrimaryOffer::new(true, b"");
        let read = receive_primary_offer(empty.as_ref(), "text/plain".into()).unwrap();
        assert_eq!(empty.requests(), 1);
        let text = read_pipe(&read, Instant::now() + Duration::from_secs(1), 16, || false).unwrap();
        assert_eq!(&*text, "");
    }

    #[test]
    fn replaced_primary_offer_uses_only_the_current_raw_proxy() {
        let mut inner = Inner::default();
        inner.add_test_seat();
        let stale = FakePrimaryOffer::new(true, b"stale");
        inner.seats[0].update_primary_raw_offer(
            true,
            true,
            PrimaryRawOfferUpdate::Replace(Some(proxy(&stale))),
        );

        // A new selection event destroys the predecessor before SCTK reports the replacement.
        stale.destroy();
        let current = FakePrimaryOffer::new(true, b"");
        inner.seats[0].update_primary_raw_offer(
            true,
            true,
            PrimaryRawOfferUpdate::Replace(Some(proxy(&current))),
        );

        let snapshot = inner.seats[0].primary_receive_offer().expect("paired raw offer");
        assert!(Arc::ptr_eq(&snapshot, &proxy(&current)));

        let error = receive_primary_offer(stale.as_ref(), "text/plain".into()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(stale.requests(), 0);

        let read = receive_primary_offer(snapshot.as_ref(), "text/plain".into()).unwrap();
        assert_eq!(current.requests(), 1);
        let text = read_pipe(&read, Instant::now() + Duration::from_secs(1), 16, || false).unwrap();
        assert_eq!(&*text, "");
    }

    #[test]
    fn primary_snapshot_without_a_raw_offer_is_refused() {
        let error =
            receive_selection_offer(SelectionOfferKind::Primary { raw: None }, "text/plain".into())
                .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
    }

    #[test]
    fn destroyed_primary_raw_offer_sends_no_protocol_request() {
        let offer = FakePrimaryOffer::new(true, b"text");
        offer.destroy();
        let error = receive_primary_offer(offer.as_ref(), "text/plain".into()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert_eq!(offer.requests(), 0);
    }

    #[test]
    fn primary_raw_offer_side_table_is_cleared_with_the_selection() {
        let mut inner = Inner::default();
        inner.add_test_seat();
        let offer = FakePrimaryOffer::new(true, b"");
        inner.seats[0].update_primary_raw_offer(
            true,
            true,
            PrimaryRawOfferUpdate::Replace(Some(proxy(&offer))),
        );
        assert!(Arc::ptr_eq(&inner.seats[0].primary_receive_offer().unwrap(), &proxy(&offer)));

        // Reconciliation keeps the pair only for an unchanged, present wrapper.
        inner.seats[0].update_primary_raw_offer(true, true, PrimaryRawOfferUpdate::Reconcile);
        assert!(inner.seats[0].primary_receive_offer().is_some());
        inner.seats[0].update_primary_raw_offer(false, true, PrimaryRawOfferUpdate::Reconcile);
        assert!(inner.seats[0].primary_receive_offer().is_none());

        // A NULL selection event supplies Replace(None) and clears the side table.
        inner.seats[0].update_primary_raw_offer(false, false, PrimaryRawOfferUpdate::Replace(None));
        assert!(inner.seats[0].primary_receive_offer().is_none());
    }

    #[test]
    fn checked_receive_never_turns_a_dead_offer_into_empty_success() {
        let called = AtomicBool::new(false);
        let error = checked_receive(
            || false,
            |_| {
                called.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);
        assert!(!called.load(Ordering::SeqCst));

        let error = checked_receive(
            || true,
            |_| Err(transfer_interrupted("the selection offer is no longer valid")),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Interrupted);

        let read = checked_receive(
            || true,
            |fd| {
                rustix::io::write(fd, b"ok").map_err(io::Error::from)?;
                Ok(())
            },
        )
        .unwrap();
        let mut buffer = [0u8; 8];
        assert_eq!(rustix::io::read(&read, &mut buffer).unwrap(), 2);
        assert_eq!(&buffer[..2], b"ok");
    }

    #[test]
    fn coordinator_spawn_failure_fails_a_queued_read_submission() {
        let workers = Workers::default();
        workers.inner.coordinator_spawn_failure.store(true, Ordering::Release);
        // Simulate every active read permit being held without starting real worker threads.
        workers.inner.reads.lock().unwrap().active = MAX_ACTIVE_READS;
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let slot = Arc::new(TransferSlot::new(101, Some(wid(56)), 9, SelectionKind::Regular, None));
        let reservation = workers.reserve_read().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        let error = workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            })
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Other);
        assert!(error.to_string().contains("coordinator failed to start"));
        assert!(slot.is_finished());
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Other),
            other => panic!("expected a deterministic coordinator failure, got {other:?}"),
        }
        assert!(workers.inner.reads.lock().unwrap().jobs.is_empty());
        // The failed queued job dropped its read end, so the unused writer sees a closed pipe.
        assert_eq!(rustix::io::write(&write, b"x").unwrap_err(), rustix::io::Errno::PIPE);
        workers.shutdown();
    }

    #[test]
    fn coordinator_spawn_failure_drains_a_no_active_worker_queue() {
        let workers = Workers::default();
        workers.inner.coordinator_spawn_failure.store(true, Ordering::Release);
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let slot = Arc::new(TransferSlot::new(102, Some(wid(57)), 9, SelectionKind::Regular, None));
        let serving = Arc::new(SourceServeState::default());
        {
            let mut reads = workers.inner.reads.lock().unwrap();
            // No active worker exists to promote these jobs; only the coordinator can finish
            // them. Its spawn failure must still complete both queues deterministically.
            reads.active = 0;
            reads.jobs.push_back(ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            });
        }
        {
            let mut writes = workers.inner.writes.lock().unwrap();
            writes.active = 0;
            writes.jobs.push_back(WriteJob {
                serving: Arc::downgrade(&serving),
                fd: write,
                payload: OwnedPayload::Text(Arc::from("queued")),
                _source_bytes: test_source_charge(0),
                deadline: Instant::now() + Duration::from_secs(30),
            });
        }

        let error = workers
            .inner
            .start_coordinator(|_| Err(io::Error::new(ErrorKind::Other, "forced")))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Other);
        assert!(error.to_string().contains("coordinator failed to start"));
        assert!(slot.is_finished());
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Other),
            other => panic!("expected the queued read to fail, got {other:?}"),
        }
        assert!(serving.error().is_some_and(|error| error.contains("coordinator failed to start")));
        assert!(workers.inner.reads.lock().unwrap().jobs.is_empty());
        assert!(workers.inner.writes.lock().unwrap().jobs.is_empty());
        assert_eq!(workers.inner.reads.lock().unwrap().active, 0);
        assert_eq!(workers.inner.writes.lock().unwrap().active, 0);
        workers.shutdown();
    }

    #[test]
    fn blocking_transfer_wait_is_bounded_by_the_admission_deadline() {
        let slot = Arc::new(TransferSlot::new(103, Some(wid(58)), 9, SelectionKind::Regular, None));
        slot.set_deadline(Instant::now() + Duration::from_millis(50));
        let started = Instant::now();
        let error = slot.wait().unwrap_err();
        assert_eq!(error.io_kind(), ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn coordinator_promotes_reads_requeued_after_workers_finish_during_prune() {
        let workers = Workers::default();
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        rustix::io::write(&write, b"data").unwrap();
        drop(write);
        let slot = Arc::new(TransferSlot::new(200, Some(wid(55)), 9, SelectionKind::Regular, None));
        {
            let mut reads = workers.inner.reads.lock().unwrap();
            // This is the state left when every active reader finished while the coordinator
            // held no queue lock: runnable work is back in the queue with no promoter.
            reads.active = 0;
            reads.jobs.push_back(ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(1),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            });
        }
        workers.inner.prune_queued();
        assert!(wait_until(Duration::from_secs(1), || slot.is_finished()));
        match (TransferHandle { slot }).try_take() {
            Some(Ok(text)) => assert_eq!(&*text, "data"),
            other => panic!("expected the promoted read to complete, got {other:?}"),
        }
        workers.shutdown();
    }

    fn set_regular_offer(inner: &mut Inner, mime_types: &[&str]) -> OfferKey {
        let key = inner.allocate_offer_key();
        inner.seats[0].regular.offer = Some(OfferView {
            key,
            mime_types: mime_types.iter().map(|mime| (*mime).to_owned()).collect(),
            marker: None,
        });
        key
    }

    fn route_slot(
        inner: &Arc<Mutex<Inner>>,
        target: WindowId,
        wakes: Arc<AtomicUsize>,
    ) -> Arc<TransferSlot> {
        route_slot_with_generation(inner, target, wakes, 0)
    }

    fn route_slot_with_generation(
        inner: &Arc<Mutex<Inner>>,
        target: WindowId,
        wakes: Arc<AtomicUsize>,
        caller_generation: u64,
    ) -> Arc<TransferSlot> {
        let mut guard = inner.lock().unwrap();
        let route = guard
            .read_route_with(target, SelectionKind::Regular, ReadPreference::TextThenImage)
            .unwrap();
        let slot = guard.transfers.create_with_route(
            caller_generation,
            Some(target),
            route.frozen.clone(),
            Arc::downgrade(inner),
            Some(Arc::new(move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            })),
        );
        slot.set_deadline(Instant::now() + Duration::from_secs(2));
        slot
    }

    #[test]
    fn clipboard_error_legacy_mapping_covers_typed_admission_and_backend_kinds() {
        for (error, kind) in [
            (ClipboardError::Absent, ErrorKind::NotFound),
            (ClipboardError::NotOffered, ErrorKind::InvalidData),
            (ClipboardError::StaleSelfSource, ErrorKind::InvalidData),
            (ClipboardError::Interrupted, ErrorKind::Interrupted),
            (ClipboardError::TimedOut, ErrorKind::TimedOut),
            (ClipboardError::Unsupported, ErrorKind::Unsupported),
            (ClipboardError::Busy, ErrorKind::WouldBlock),
            (ClipboardError::TooLarge, ErrorKind::InvalidData),
            (ClipboardError::InvalidInput, ErrorKind::InvalidInput),
            (ClipboardError::Disconnected, ErrorKind::BrokenPipe),
        ] {
            assert_eq!(error.io_kind(), kind);
            assert_eq!(error.into_io_error().kind(), kind);
        }

        let backend = ClipboardError::from_io(io::Error::new(ErrorKind::ConnectionReset, "reset"));
        assert_eq!(backend.io_kind(), ErrorKind::ConnectionReset);
        assert_eq!(backend.into_io_error().kind(), ErrorKind::ConnectionReset);
        assert_eq!(
            ClipboardError::from_io(io::Error::from(ErrorKind::WouldBlock)).io_kind(),
            ErrorKind::WouldBlock,
            "the source/worker budget keeps its legacy WouldBlock mapping"
        );
    }

    #[test]
    fn combined_regular_classification_prefers_text_then_exact_png_from_one_offer() {
        let target = wid(70);
        let (mut inner, _) = focused_inner(target);
        let identity = inner.marker_identity.clone();

        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextThenImage)
                .unwrap_err()
                .io_kind(),
            ErrorKind::NotFound,
            "an absent offer is Absent"
        );

        set_regular_offer(&mut inner, &[IMAGE_PNG_MIME, "text/plain"]);
        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextThenImage)
                .unwrap(),
            OfferRead::Request("text/plain".to_owned())
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextOnly)
                .unwrap(),
            OfferRead::Request("text/plain".to_owned())
        );

        set_regular_offer(&mut inner, &[IMAGE_PNG_MIME]);
        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextThenImage)
                .unwrap(),
            OfferRead::RequestImage(IMAGE_PNG_MIME.to_owned())
        );
        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextOnly)
                .unwrap_err()
                .io_kind(),
            ErrorKind::InvalidData,
            "a text-only read does not accept an image offer"
        );

        set_regular_offer(&mut inner, &["application/pdf"]);
        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextThenImage)
                .unwrap_err()
                .io_kind(),
            ErrorKind::InvalidData,
            "an offer with neither text nor exact PNG is NotOffered"
        );

        // A foreign winit marker does not block an image read.
        set_regular_offer(&mut inner, &[IMAGE_PNG_MIME]);
        let foreign = ClipboardIdentity::new();
        inner.seats[0].regular.offer.as_mut().unwrap().marker = Some(source_marker(&foreign, 1));
        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextThenImage)
                .unwrap(),
            OfferRead::RequestImage(IMAGE_PNG_MIME.to_owned())
        );

        // A stale marker from this clipboard fails before image preference is considered.
        inner.last_source_token = 5;
        let stale = source_marker(&identity, 4);
        set_regular_offer(&mut inner, &[IMAGE_PNG_MIME]);
        inner.seats[0].regular.offer.as_mut().unwrap().marker = Some(stale);
        assert_eq!(
            inner.seats[0]
                .regular
                .classify_with(SelectionKind::Regular, &identity, ReadPreference::TextThenImage)
                .unwrap_err()
                .io_kind(),
            ErrorKind::InvalidData
        );
        assert_eq!(
            inner
                .read_route_with(target, SelectionKind::Regular, ReadPreference::TextThenImage)
                .unwrap_err()
                .io_kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn image_only_route_freezes_the_exact_offer_and_focus_epoch() {
        let target = wid(71);
        let (mut inner, _) = focused_inner(target);
        let key = set_regular_offer(&mut inner, &[IMAGE_PNG_MIME]);
        let epoch = inner.seats[0].input.epoch;
        let route = inner
            .read_route_with(target, SelectionKind::Regular, ReadPreference::TextThenImage)
            .unwrap();
        assert_eq!(route.read, OfferRead::RequestImage(IMAGE_PNG_MIME.to_owned()));
        assert_eq!(route.frozen.expected_offer, Some(key));
        assert_eq!(route.frozen.expected_source, None);
        assert_eq!(route.frozen.focus_epoch, epoch);
        assert!(route.frozen.check_focus);
    }

    #[test]
    fn completed_result_is_revoked_by_focus_loss_before_take_without_a_second_wake() {
        let target = wid(72);
        let (inner, _) = focused_inner(target);
        let inner = Arc::new(Mutex::new(inner));
        {
            let mut guard = inner.lock().unwrap();
            set_regular_offer(&mut guard, &["text/plain"]);
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = route_slot(&inner, target, wakes.clone());
        slot.set_result(Ok(ClipboardContent::Text(Arc::from("late"))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "completion wakes exactly once");

        // Do not complete the transition's batch: try_take itself must revalidate the frozen route.
        let batch = inner.lock().unwrap().keyboard_focus_leave_at(0);
        let handle = RegularTransferHandle { slot: slot.clone() };
        match handle.try_take() {
            Some(Err(error)) => assert_eq!(error.io_kind(), ErrorKind::Interrupted),
            other => panic!("expected focus loss to revoke the result, got {other:?}"),
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        batch.complete();
        assert!(handle.try_take().is_none(), "the result is consumed once");
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "no cancellation wake");
    }

    #[test]
    fn completed_but_unconsumed_result_is_replaced_by_owner_change() {
        let target = wid(73);
        let (inner, _) = focused_inner(target);
        let inner = Arc::new(Mutex::new(inner));
        {
            let mut guard = inner.lock().unwrap();
            set_regular_offer(&mut guard, &["text/plain"]);
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = route_slot(&inner, target, wakes.clone());
        slot.set_result(Ok(ClipboardContent::Text(Arc::from("late"))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);

        {
            let mut guard = inner.lock().unwrap();
            let next = guard.allocate_offer_key();
            observe(
                &mut guard,
                SelectionKind::Regular,
                Some(next),
                &["text/plain"],
                Instant::now(),
            );
        }

        let handle = RegularTransferHandle { slot: slot.clone() };
        match handle.try_take() {
            Some(Err(error)) => assert_eq!(error.io_kind(), ErrorKind::Interrupted),
            other => {
                panic!("expected the owner change to revoke the completed result, got {other:?}")
            },
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "the stored replacement does not wake twice");
        assert!(handle.try_take().is_none());
    }

    #[test]
    fn completed_result_is_revoked_by_window_close_before_take() {
        let target = wid(74);
        let (inner, _) = focused_inner(target);
        let inner = Arc::new(Mutex::new(inner));
        {
            let mut guard = inner.lock().unwrap();
            set_regular_offer(&mut guard, &["text/plain"]);
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = route_slot(&inner, target, wakes.clone());
        slot.set_result(Ok(ClipboardContent::Text(Arc::from("late"))));

        let batch = inner.lock().unwrap().remove_window(target);
        batch.complete();
        let handle = RegularTransferHandle { slot };
        match handle.try_take() {
            Some(Err(error)) => assert_eq!(error.io_kind(), ErrorKind::Interrupted),
            other => panic!("expected window close to revoke the result, got {other:?}"),
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn image_budget_caps_full_cap_reads_and_releases_shared_permits() {
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let full = MAX_TRANSFER_BYTES * 2;
        let first = budget.try_acquire(full).unwrap();
        let second = budget.try_acquire(full).unwrap();
        let third = budget.try_acquire(full).unwrap();
        let fourth = budget.try_acquire(full).unwrap();
        assert_eq!(budget.used(), IMAGE_BUDGET_BYTES);
        assert!(budget.try_acquire(1).is_none(), "queue slots cannot bypass the byte budget");

        // A clone shares one reservation; only the last clone releases it.
        let shared = first.clone();
        drop(first);
        assert_eq!(budget.used(), IMAGE_BUDGET_BYTES);
        drop(shared);
        assert_eq!(budget.used(), IMAGE_BUDGET_BYTES - full);
        drop(second);
        drop(third);
        drop(fourth);
        assert_eq!(budget.used(), 0);
        let exhausted = budget.try_acquire(IMAGE_BUDGET_BYTES).unwrap();
        assert_eq!(budget.used(), IMAGE_BUDGET_BYTES);
        drop(exhausted);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn png_bytes_are_immutable_and_keep_their_permit_with_every_clone() {
        assert!(PngBytes::admit(vec![0; MAX_TRANSFER_BYTES + 1]).is_err());
        let png = PngBytes::admit(vec![0x89, b'P', b'N', b'G']).unwrap();
        assert_eq!(png.as_bytes(), [0x89u8, b'P', b'N', b'G'].as_slice());
        assert_eq!(png.len(), 4);
        assert!(!png.is_empty());
        let clone = png.clone();
        drop(png);
        assert_eq!(clone.as_bytes(), [0x89u8, b'P', b'N', b'G'].as_slice());
    }

    #[test]
    fn png_admission_accounts_for_spare_capacity() {
        let mut bytes = Vec::with_capacity(4096);
        bytes.push(0x89);
        let capacity = bytes.capacity();
        let png = PngBytes::admit(bytes).unwrap();
        assert_eq!(png.inner._permit._inner.bytes, capacity * 2);
        assert_eq!(png.len(), 1);
        let mut oversized = Vec::with_capacity(MAX_TRANSFER_BYTES + 1);
        oversized.push(0x89);
        assert!(matches!(PngBytes::admit(oversized), Err(ClipboardError::TooLarge)));
    }

    #[test]
    fn queued_image_read_discard_releases_its_budget_permit() {
        let workers = Workers::default();
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let (slots, writers) = saturate_reads(&workers);
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let queued_slot =
            Arc::new(TransferSlot::new(120, Some(wid(75)), 9, SelectionKind::Regular, None));
        let permit = budget.try_acquire(MAX_TRANSFER_BYTES * 2).unwrap();
        assert_eq!(budget.used(), MAX_TRANSFER_BYTES * 2);
        let reservation = workers.reserve_read().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&queued_slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: MAX_TRANSFER_BYTES,
                kind: ReadKind::ImagePng,
                image_permit: Some(permit),
            })
            .unwrap();
        assert_eq!(
            workers.inner.reads.lock().unwrap().jobs.len(),
            1,
            "the queued job holds its permit"
        );
        assert_eq!(budget.used(), MAX_TRANSFER_BYTES * 2);

        queued_slot.cancel();
        assert!(wait_until(Duration::from_secs(2), || {
            workers.inner.reads.lock().unwrap().jobs.is_empty()
        }));
        assert_eq!(budget.used(), 0, "discarding the queued job releases the permit");
        for slot in &slots {
            slot.cancel();
        }
        drop(writers);
        drop(write);
        workers.shutdown();
    }

    #[test]
    fn cancel_after_completion_keeps_one_wake_and_discards_the_result() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(TransferSlot::new(
            121,
            Some(wid(76)),
            9,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        ));
        let handle = RegularTransferHandle { slot: slot.clone() };
        slot.set_result(Ok(ClipboardContent::Text(Arc::from("late"))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        handle.cancel();
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "cancel never wakes");
        assert!(handle.try_take().is_none());
    }

    #[test]
    fn text_conversion_checks_the_absolute_deadline_before_and_after() {
        let start = Instant::now();
        let expired = start - Duration::from_millis(1);
        assert_eq!(
            convert_text(b"hello".to_vec(), 16, expired, &|| false).unwrap_err().io_kind(),
            ErrorKind::TimedOut,
            "the deadline is checked before conversion"
        );

        let calls = AtomicUsize::new(0);
        let clock = || {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                start
            } else {
                start + Duration::from_secs(2)
            }
        };
        assert_eq!(
            convert_text_with_clock(
                b"hello".to_vec(),
                16,
                start + Duration::from_secs(1),
                &|| false,
                &clock,
            )
            .unwrap_err()
            .io_kind(),
            ErrorKind::TimedOut,
            "the deadline is checked again after conversion"
        );

        assert_eq!(
            convert_text(b"hello".to_vec(), 16, start + Duration::from_secs(1), &|| true)
                .unwrap_err()
                .io_kind(),
            ErrorKind::Interrupted
        );
    }

    #[test]
    fn coordinator_promotes_writes_requeued_after_workers_finish_during_prune() {
        let workers = Workers::default();
        let serving = Arc::new(SourceServeState::default());
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        {
            let mut writes = workers.inner.writes.lock().unwrap();
            writes.active = 0;
            writes.jobs.push_back(WriteJob {
                serving: Arc::downgrade(&serving),
                fd: write,
                payload: OwnedPayload::Text(Arc::from("data")),
                _source_bytes: test_source_charge(0),
                deadline: Instant::now() + Duration::from_secs(1),
            });
        }
        workers.inner.prune_queued();
        set_nonblocking(&read).unwrap();
        let mut buffer = [0u8; 8];
        assert!(wait_until(Duration::from_secs(1), || {
            rustix::io::read(&read, &mut buffer).is_ok()
        }));
        assert_eq!(&buffer[..4], b"data");
        workers.shutdown();
    }

    #[test]
    fn regular_handle_retains_the_caller_generation_without_validating_it() {
        let target = wid(90);
        let (inner, _) = focused_inner(target);
        let inner = Arc::new(Mutex::new(inner));
        {
            let mut guard = inner.lock().unwrap();
            set_regular_offer(&mut guard, &["text/plain"]);
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = route_slot_with_generation(&inner, target, wakes, 77);
        let handle = RegularTransferHandle { slot };
        assert_eq!(handle.caller_generation(), 77);
        assert!(handle.try_take().is_none());
    }

    #[test]
    fn admission_deadline_helper_recognizes_expiry() {
        let now = Instant::now();
        assert!(transfer_deadline_passed(now - Duration::from_millis(1)));
        assert!(!transfer_deadline_passed(now + Duration::from_secs(1)));
    }

    #[test]
    fn valid_utf8_conversion_checks_the_deadline_in_bounded_chunks() {
        let start = Instant::now();
        let calls = AtomicUsize::new(0);
        let clock = || {
            // First call is the pre-conversion check; every later call is expired. Without a
            // mid-chunk check the 8192-byte payload would reach TooLarge at the 4097th byte.
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                start
            } else {
                start + Duration::from_secs(2)
            }
        };
        let error = convert_text_with_clock(
            vec![b'x'; 8192],
            4096,
            start + Duration::from_secs(1),
            &|| false,
            &clock,
        )
        .unwrap_err();
        assert_eq!(error.io_kind(), ErrorKind::TimedOut);
    }

    #[test]
    fn image_worker_pipe_read_returns_exact_png_once_and_releases_its_permit() {
        let workers = Workers::default();
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let payload = b"\x89PNG\r\n\x1a\nimage".to_vec();
        rustix::io::write(&write, &payload).unwrap();
        drop(write);

        let slot = Arc::new(TransferSlot::new(150, Some(wid(91)), 9, SelectionKind::Regular, None));
        let permit = budget.try_acquire(MAX_TRANSFER_BYTES * 2).unwrap();
        assert_eq!(budget.used(), MAX_TRANSFER_BYTES * 2);
        let reservation = workers.reserve_read().unwrap();
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(2),
                max_bytes: MAX_TRANSFER_BYTES,
                kind: ReadKind::ImagePng,
                image_permit: Some(permit),
            })
            .unwrap();
        assert!(wait_until(Duration::from_secs(2), || slot.is_finished()));
        assert_eq!(
            budget.used(),
            MAX_TRANSFER_BYTES * 2,
            "the completed image result keeps its permit"
        );

        let handle = RegularTransferHandle { slot: slot.clone() };
        match handle.try_take() {
            Some(Ok(ClipboardContent::ImagePng(png))) => {
                assert_eq!(png.as_bytes(), payload.as_slice());
                assert_eq!(png.len(), payload.len());
            },
            other => panic!("expected one image worker result, got {other:?}"),
        }
        assert_eq!(budget.used(), 0, "consuming the result releases the permit");
        assert!(handle.try_take().is_none(), "the image result is consumed exactly once");
        workers.shutdown();
    }

    #[test]
    fn image_worker_read_over_max_bytes_returns_too_large_and_releases_its_permit() {
        let workers = Workers::default();
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        rustix::io::write(&write, b"abcdefgh").unwrap();
        drop(write);

        let slot = Arc::new(TransferSlot::new(151, Some(wid(92)), 9, SelectionKind::Regular, None));
        let permit = budget.try_acquire(MAX_TRANSFER_BYTES * 2).unwrap();
        let reservation = workers.reserve_read().unwrap();
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(2),
                max_bytes: 4,
                kind: ReadKind::ImagePng,
                image_permit: Some(permit),
            })
            .unwrap();
        assert!(wait_until(Duration::from_secs(2), || slot.is_finished()));
        let handle = RegularTransferHandle { slot };
        match handle.try_take() {
            Some(Err(error)) => assert_eq!(error.io_kind(), ErrorKind::InvalidData),
            other => panic!("expected a TooLarge image read, got {other:?}"),
        }
        assert_eq!(budget.used(), 0, "a failed image read releases its permit");
        workers.shutdown();
    }

    #[test]
    fn image_worker_read_deadline_releases_its_permit() {
        let workers = Workers::default();
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();

        let slot = Arc::new(TransferSlot::new(152, Some(wid(93)), 9, SelectionKind::Regular, None));
        let permit = budget.try_acquire(MAX_TRANSFER_BYTES * 2).unwrap();
        let reservation = workers.reserve_read().unwrap();
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_millis(30),
                max_bytes: MAX_TRANSFER_BYTES,
                kind: ReadKind::ImagePng,
                image_permit: Some(permit),
            })
            .unwrap();
        assert!(wait_until(Duration::from_secs(2), || slot.is_finished()));
        let handle = RegularTransferHandle { slot };
        match handle.try_take() {
            Some(Err(error)) => assert_eq!(error.io_kind(), ErrorKind::TimedOut),
            other => panic!("expected an expired image read, got {other:?}"),
        }
        assert_eq!(budget.used(), 0, "an expired image read releases its permit");
        drop(write);
        workers.shutdown();
    }

    #[test]
    fn cancelled_active_image_read_releases_its_permit() {
        let workers = Workers::default();
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();

        let slot = Arc::new(TransferSlot::new(153, Some(wid(94)), 9, SelectionKind::Regular, None));
        let permit = budget.try_acquire(MAX_TRANSFER_BYTES * 2).unwrap();
        let reservation = workers.reserve_read().unwrap();
        workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: MAX_TRANSFER_BYTES,
                kind: ReadKind::ImagePng,
                image_permit: Some(permit),
            })
            .unwrap();
        slot.cancel();
        assert!(wait_until(Duration::from_secs(2), || budget.used() == 0));
        let handle = RegularTransferHandle { slot };
        assert!(handle.try_take().is_none(), "a cancelled read yields no result");
        drop(write);
        workers.shutdown();
    }

    #[test]
    fn completed_unconsumed_image_result_is_revoked_by_owner_change_and_releases_permit() {
        let target = wid(95);
        let (inner, _) = focused_inner(target);
        let inner = Arc::new(Mutex::new(inner));
        {
            let mut guard = inner.lock().unwrap();
            set_regular_offer(&mut guard, &[IMAGE_PNG_MIME]);
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = route_slot(&inner, target, wakes.clone());
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let permit = budget.try_acquire(MAX_TRANSFER_BYTES * 2).unwrap();
        slot.set_result(Ok(ClipboardContent::ImagePng(PngBytes::from_vec_with_permit(
            vec![0x89, b'P', b'N', b'G'],
            permit,
        ))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(budget.used(), MAX_TRANSFER_BYTES * 2);

        {
            let mut guard = inner.lock().unwrap();
            let next = guard.allocate_offer_key();
            observe(
                &mut guard,
                SelectionKind::Regular,
                Some(next),
                &[IMAGE_PNG_MIME],
                Instant::now(),
            );
        }

        let handle = RegularTransferHandle { slot };
        match handle.try_take() {
            Some(Err(error)) => assert_eq!(error.io_kind(), ErrorKind::Interrupted),
            other => panic!("expected the owner change to revoke the image result, got {other:?}"),
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "the replacement does not wake twice");
        assert_eq!(budget.used(), 0, "the revoked image result releases its permit");
        assert!(handle.try_take().is_none());
    }

    #[test]
    fn active_seat_move_and_return_revokes_a_completed_unconsumed_read() {
        let target = wid(96);
        let mut inner = Inner::default();
        inner.add_window(target);
        let first = inner.add_test_seat();
        let second = inner.add_test_seat();
        inner.keyboard_focus_enter_at(0, 1, target);
        let inner = Arc::new(Mutex::new(inner));
        {
            let mut guard = inner.lock().unwrap();
            set_regular_offer(&mut guard, &["text/plain"]);
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = route_slot(&inner, target, wakes.clone());
        slot.set_result(Ok(ClipboardContent::Text(Arc::from("late"))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);

        {
            let mut guard = inner.lock().unwrap();
            assert_eq!(guard.active_seat, Some(first));
            // Both seats focus the same window. Input on the second seat makes it active, then
            // input returns to the first without a new focus entry, so its focus epoch is
            // unchanged and only the active-seat epoch proves the A-B-A movement.
            guard.keyboard_focus_enter_at(1, 2, target);
            assert_eq!(guard.active_seat, Some(second));
            guard.keyboard_key_at(0, 3, target);
            assert_eq!(guard.active_seat, Some(first));
        }

        let handle = RegularTransferHandle { slot };
        match handle.try_take() {
            Some(Err(error)) => assert_eq!(error.io_kind(), ErrorKind::Interrupted),
            other => panic!("expected A-B-A seat movement to revoke the result, got {other:?}"),
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(handle.try_take().is_none());
    }

    #[test]
    fn admission_failure_may_wake_once_before_begin_returns_err() {
        let workers = Workers::default();
        workers.inner.coordinator_spawn_failure.store(true, Ordering::Release);
        workers.inner.reads.lock().unwrap().active = MAX_ACTIVE_READS;
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(TransferSlot::new(
            154,
            Some(wid(97)),
            9,
            SelectionKind::Regular,
            Some({
                let wakes = wakes.clone();
                Arc::new(move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                })
            }),
        ));
        let reservation = workers.reserve_read().unwrap();
        assert_eq!(reservation, Reservation::Queued);
        let error = workers
            .submit_reserved_read(reservation, ReadJob {
                slot: Arc::downgrade(&slot),
                fd: read,
                deadline: Instant::now() + Duration::from_secs(30),
                max_bytes: 16,
                kind: ReadKind::Text,
                image_permit: None,
            })
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            1,
            "the documented admission-failure wake fired before the Err was observed"
        );
        // The caller must treat the returned error as authoritative, not a missing wake.
        match (TransferHandle { slot: slot.clone() }).try_take() {
            Some(Err(error)) => assert_eq!(error.kind(), ErrorKind::Other),
            other => panic!("expected the queued read to fail, got {other:?}"),
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "try_take never wakes again");
        drop(write);
        workers.shutdown();
    }

    #[test]
    fn image_reservation_keeps_the_original_serial_after_pointer_release() {
        let target = wid(200);
        let (mut inner, claim) = focused_inner(target);
        let original = claim.trigger.inner.serial;
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();
        assert_eq!(reservation.serial, original);

        // A pointer release is later unrelated input. It replaces the latest serial but must not
        // rewrite or invalidate the reservation's captured serial.
        inner.pointer_input_at(0, original + 1, target, false);
        let latest = inner.seats[0].input.latest(target, now).unwrap();
        assert_eq!(latest.serial, original + 1);
        assert_ne!(latest.serial, reservation.serial);

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\noriginal");
        let installed = install_reserved_detached(
            &mut inner,
            reservation,
            png,
            20,
            now + Duration::from_millis(1),
        )
        .unwrap();
        let owned = inner.seats[0].regular.owned.as_ref().unwrap();
        assert!(matches!(&owned.payload, OwnedPayload::Png(_)));
        assert_eq!(owned.selection_serial, original, "later input never substitutes its serial");
        drop(installed.previous);
    }

    #[test]
    fn newer_failed_regular_text_attempt_supersedes_an_image_reservation() {
        let target = wid(201);
        let (mut inner, claim) = focused_inner(target);
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();

        // Reusing the already-consumed trigger makes the newer regular text attempt fail. The
        // ownership epoch still advances before validation, so the old reservation cannot win.
        let error = install_detached(&mut inner, SelectionKind::Regular, &claim, "new", 21);
        assert_eq!(
            error.expect_err("the stale claim must be rejected").kind(),
            ErrorKind::InvalidInput
        );

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nold");
        let error = install_reserved_detached(&mut inner, reservation, png, 22, now)
            .err()
            .expect("the stale claim must be rejected");
        assert!(matches!(error, ClipboardError::Interrupted));
        assert_eq!(budget.used(), 0, "the rejected encode releases its permit");
    }

    #[test]
    fn newer_failed_image_reservation_attempt_supersedes_an_older_one() {
        let target = wid(208);
        let (mut inner, claim) = focused_inner(target);
        let now = Instant::now();
        let older = inner.reserve_image_claim_at(&claim, now).unwrap();

        // The trigger is already consumed, so the newer reservation attempt fails after advancing
        // the ownership epoch. Its failure must not let the older encode install later.
        assert!(inner.reserve_image_claim_at(&claim, now).is_err());

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nolder");
        let error = install_reserved_detached(&mut inner, older, png, 32, now)
            .err()
            .expect("the stale claim must be rejected");
        assert!(matches!(error, ClipboardError::Interrupted));
    }

    #[test]
    fn primary_claim_does_not_invalidate_a_regular_image_reservation() {
        let target = wid(202);
        let (mut inner, claim) = focused_inner(target);
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();

        // New input supplies a fresh PRIMARY trigger. It advances no regular ownership epoch.
        inner.keyboard_key_at(0, 3, target);
        let primary_claim = active_claim(&mut inner, target);
        install_detached(&mut inner, SelectionKind::Primary, &primary_claim, "primary", 23)
            .unwrap();

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nregular");
        let installed = install_reserved_detached(
            &mut inner,
            reservation,
            png,
            24,
            now + Duration::from_millis(1),
        )
        .unwrap();
        assert!(matches!(
            &inner.seats[0].regular.owned.as_ref().unwrap().payload,
            OwnedPayload::Png(_)
        ));
        assert_eq!(
            inner.seats[0].primary.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "primary"
        );
        drop(installed.previous);
    }

    #[test]
    fn invalidate_image_reservations_bumps_the_epoch_without_clearing_the_source() {
        let target = wid(203);
        let (mut inner, first_claim) = focused_inner(target);
        install_detached(&mut inner, SelectionKind::Regular, &first_claim, "installed", 25)
            .unwrap();
        inner.keyboard_key_at(0, 4, target);
        let claim = active_claim(&mut inner, target);
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();
        assert_eq!(inner.image_reservations.len(), 1);

        inner.invalidate_image_reservations_at(target).unwrap();
        assert!(inner.image_reservations.is_empty());
        assert_eq!(
            inner.seats[0].regular.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "installed",
            "invalidation never clears the installed source"
        );

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nnew");
        let error = install_reserved_detached(&mut inner, reservation, png, 26, now)
            .err()
            .expect("the stale claim must be rejected");
        assert!(matches!(error, ClipboardError::Interrupted));
        assert_eq!(
            inner.seats[0].regular.owned.as_ref().unwrap().payload.as_text().unwrap().as_ref(),
            "installed"
        );
    }

    #[test]
    fn focus_window_and_expiry_invalidate_image_reservations() {
        // Focus loss.
        let target = wid(204);
        let (mut inner, claim) = focused_inner(target);
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();
        let batch = inner.keyboard_focus_leave_at(0);
        batch.complete();
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nfocus");
        assert!(matches!(
            install_reserved_detached(&mut inner, reservation, png, 27, now)
                .err()
                .expect("the stale claim must be rejected"),
            ClipboardError::Interrupted
        ));

        // Window close.
        let (mut inner, claim) = focused_inner(target);
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();
        let batch = inner.remove_window(target);
        batch.complete();
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nwindow");
        assert!(matches!(
            install_reserved_detached(&mut inner, reservation, png, 28, now)
                .err()
                .expect("the stale claim must be rejected"),
            ClipboardError::Interrupted
        ));

        // The reservation lifetime is bounded by the triggering input's five-second freshness.
        let (mut inner, claim) = focused_inner(target);
        let trigger_at = claim.trigger.inner.at;
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nexpiry");
        assert!(matches!(
            install_reserved_detached(
                &mut inner,
                reservation,
                png,
                29,
                trigger_at + INPUT_ORIGIN_MAX_AGE + Duration::from_millis(1),
            )
            .err()
            .expect("the stale claim must be rejected"),
            ClipboardError::TimedOut
        ));
    }

    #[test]
    fn reservation_expiry_is_bounded_by_the_original_trigger() {
        let target = wid(208);
        let (mut inner, claim) = focused_inner(target);
        let trigger_at = claim.trigger.inner.at;
        // The token is still fresh when reserved, but the reservation must expire exactly five
        // seconds after the original input, not five seconds after this call.
        let reserved_at = trigger_at + Duration::from_secs(4);
        let reservation = inner.reserve_image_claim_at(&claim, reserved_at).unwrap();
        assert_eq!(reservation.deadline, trigger_at + INPUT_ORIGIN_MAX_AGE);

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nold trigger");
        let error = install_reserved_detached(
            &mut inner,
            reservation,
            png,
            43,
            trigger_at + INPUT_ORIGIN_MAX_AGE + Duration::from_millis(1),
        )
        .err()
        .expect("the reservation must not outlive the triggering input freshness");
        assert!(matches!(error, ClipboardError::TimedOut));
        assert_eq!(budget.used(), 0, "a rejected claim releases its image permit");
    }

    #[test]
    fn oversized_legacy_text_store_supersedes_an_image_reservation() {
        let target = wid(210);
        let (mut inner, claim) = focused_inner(target);
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();

        // The compatibility store reaches the ownership transition before its size admission.
        // Its failure is therefore a newer regular attempt and must still invalidate the old
        // image reservation.
        let oversized: Arc<str> = Arc::from("x".repeat(MAX_SOURCE_BYTES + 1));
        let mut batch = TransitionBatch::default();
        let marker = source_marker(&inner.marker_identity, 44);
        let error = inner
            .install_legacy_source(SelectionKind::Regular, oversized, 44, marker, None, &mut batch)
            .expect_err("the oversized compatibility store must fail");
        batch.complete();
        assert_eq!(error.kind(), ErrorKind::WouldBlock);

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nsuperseded");
        let error = install_reserved_detached(&mut inner, reservation, png, 45, now)
            .err()
            .expect("the older image reservation must be superseded");
        assert!(matches!(error, ClipboardError::Interrupted));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn active_seat_a_b_a_invalidates_an_image_reservation() {
        let target = wid(205);
        let mut inner = Inner::default();
        inner.add_window(target);
        let first = inner.add_test_seat();
        let second = inner.add_test_seat();
        inner.keyboard_focus_enter_at(0, 1, target);
        inner.keyboard_key_at(0, 2, target);
        let claim = active_claim(&mut inner, target);
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();

        inner.keyboard_focus_enter_at(1, 3, target);
        inner.keyboard_key_at(0, 4, target);
        assert_eq!(inner.active_seat, Some(first));
        assert_ne!(inner.active_seat, Some(second));

        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let png = test_png(&budget, b"\x89PNG\r\n\x1a\nseat");
        assert!(matches!(
            install_reserved_detached(&mut inner, reservation, png, 30, now)
                .err()
                .expect("the stale claim must be rejected"),
            ClipboardError::Interrupted
        ));
    }

    #[test]
    fn dropping_a_reservation_removes_pending_accounting() {
        let target = wid(206);
        let (inner, claim) = focused_inner(target);
        let inner = Arc::new(Mutex::new(inner));
        let reservation = {
            let mut guard = inner.lock().unwrap();
            let mut route = guard.reserve_image_claim_at(&claim, Instant::now()).unwrap();
            route.validation = Arc::downgrade(&inner);
            ImageClaimReservation { inner: Some(route) }
        };
        assert_eq!(reservation.serial(), claim.trigger.inner.serial);
        assert_eq!(inner.lock().unwrap().image_reservations.len(), 1);
        drop(reservation);
        assert!(inner.lock().unwrap().image_reservations.is_empty());
    }

    #[test]
    fn self_png_source_returns_exact_bytes_and_owner_change_revokes_it() {
        let target = wid(207);
        let (mut inner, claim) = focused_inner(target);
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let payload = b"\x89PNG\r\n\x1a\nself".to_vec();
        let now = Instant::now();
        let reservation = inner.reserve_image_claim_at(&claim, now).unwrap();
        let png = test_png(&budget, &payload);
        let installed = install_reserved_detached(&mut inner, reservation, png, 31, now).unwrap();
        drop(installed.previous);
        let source_permit_usage = budget.used();

        let route = inner
            .read_route_with(target, SelectionKind::Regular, ReadPreference::TextThenImage)
            .unwrap();
        match route.read {
            OfferRead::SelfContent(OwnedPayload::Png(png)) => {
                assert_eq!(png.as_bytes(), payload.as_slice());
            },
            other => panic!("expected a self PNG source, got {other:?}"),
        }
        assert!(
            matches!(
                inner.seats[0]
                    .regular
                    .classify_with(
                        SelectionKind::Regular,
                        &inner.marker_identity,
                        ReadPreference::TextOnly,
                    )
                    .unwrap_err(),
                ClipboardError::NotOffered
            ),
            "a text-only read does not accept an owned PNG source"
        );
        assert!(matches!(
            checked_self_content(
                inner.seats[0].regular.owned.as_ref().unwrap().payload.clone(),
                2,
            )
            .unwrap_err(),
            ClipboardError::TooLarge
        ));

        let inner = Arc::new(Mutex::new(inner));
        let wakes = Arc::new(AtomicUsize::new(0));
        let slot = route_slot(&inner, target, wakes.clone());
        let result_png = test_png(&budget, &payload);
        slot.set_result(Ok(ClipboardContent::ImagePng(result_png)));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);

        // Another owner replaces the self source. Both the source and the stored-but-unconsumed
        // result release their shared-permit references.
        {
            let mut guard = inner.lock().unwrap();
            let next = guard.allocate_offer_key();
            observe(
                &mut guard,
                SelectionKind::Regular,
                Some(next),
                &[IMAGE_PNG_MIME],
                Instant::now(),
            );
        }
        let handle = RegularTransferHandle { slot };
        match handle.try_take() {
            Some(Err(error)) => assert!(matches!(error, ClipboardError::Interrupted)),
            other => panic!("expected the owner change to revoke self PNG, got {other:?}"),
        }
        assert!(source_permit_usage > 0);
        assert_eq!(budget.used(), 0, "source and revoked result release their permit");
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn png_writer_acquires_budget_before_growth_and_enforces_its_cap() {
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let mut writer = PngBytesWriter::with_budget(&budget, 8).unwrap();
        assert_eq!(budget.used(), 16, "permit is acquired before the output vector grows");
        assert!(writer.is_empty());
        writer.write_all(&[1u8; 8]).unwrap();
        assert_eq!(writer.len(), 8);
        let error = writer.write(&[9]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(writer.len(), 8, "an over-cap write never extends the buffer");
        let png = writer.finish().unwrap();
        assert_eq!(png.as_bytes(), &[1u8; 8]);
        assert_eq!(budget.used(), 16, "the finished PngBytes keeps the one permit");
        drop(png);
        assert_eq!(budget.used(), 0);

        assert!(matches!(
            PngBytesWriter::with_budget(&budget, 0),
            Err(ClipboardError::InvalidInput)
        ));
        assert!(matches!(
            PngBytesWriter::with_budget(&budget, MAX_TRANSFER_BYTES + 1),
            Err(ClipboardError::InvalidInput)
        ));
    }

    #[test]
    fn image_source_writer_serves_exact_bytes_and_cancellation_releases_the_permit() {
        let budget = Arc::new(ImageBudget::new(IMAGE_BUDGET_BYTES));
        let payload = b"\x89PNG\r\n\x1a\nserved".to_vec();
        let png = test_png(&budget, &payload);
        let serving = Arc::new(SourceServeState::default());
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        run_write_job(WriteJob {
            serving: Arc::downgrade(&serving),
            fd: write,
            payload: OwnedPayload::Png(png),
            _source_bytes: test_source_charge(0),
            deadline: Instant::now() + Duration::from_secs(2),
        });
        let mut actual = Vec::new();
        let mut chunk = [0u8; 64];
        loop {
            match rustix::io::read(&read, &mut chunk) {
                Ok(0) => break,
                Ok(read_bytes) => actual.extend_from_slice(&chunk[..read_bytes]),
                Err(error) if error.kind() == ErrorKind::Interrupted => {},
                Err(error) => panic!("reading the served PNG failed: {error}"),
            }
        }
        assert_eq!(actual, payload);
        assert_eq!(budget.used(), 0, "the writer releases its payload reference");

        let png = test_png(&budget, &payload);
        let serving = Arc::new(SourceServeState::default());
        serving.cancel();
        let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        run_write_job(WriteJob {
            serving: Arc::downgrade(&serving),
            fd: write,
            payload: OwnedPayload::Png(png),
            _source_bytes: test_source_charge(0),
            deadline: Instant::now() + Duration::from_secs(2),
        });
        assert_eq!(budget.used(), 0, "a cancelled source writer stops and releases its permit");
        drop(read);
    }
}
