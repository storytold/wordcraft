//! Documents macOS asks WordCraft to open.
//!
//! macOS doesn't pass files on the command line: "Open With", double-clicking a document and
//! dropping one on the Dock icon send an "open documents" event to the app delegate, and winit's
//! default delegate ignores it (macOS then reports the format as unsupported). [`install`] handles
//! that Apple Event and queues the files; the app drains them with [`take`] every frame and is woken by the
//! function given to [`set_waker`].
//!
//! The only crate with `unsafe` (the Objective-C class and messages); everything else here is safe Rust.
//! On other platforms every function is a no-op.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;
use std::sync::Mutex;

type Waker = Box<dyn Fn() + Send>;

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static WAKER: Mutex<Option<Waker>> = Mutex::new(None);

/// Files macOS asked to open since the last call, in order.
pub fn take() -> Vec<PathBuf> {
    PENDING.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default()
}

/// Called (on the main thread) whenever new files arrive, e.g. to request a repaint.
pub fn set_waker(f: impl Fn() + Send + 'static) {
    if let Ok(mut w) = WAKER.lock() {
        *w = Some(Box::new(f));
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn push(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    log::info!("macOS asked to open {paths:?}");
    if let Ok(mut p) = PENDING.lock() {
        p.extend(paths);
    }
    if let Ok(w) = WAKER.lock()
        && let Some(w) = w.as_ref()
    {
        w();
    }
}

/// Starts listening for "open documents" requests. Call on the main thread before the event loop
/// runs, so the files a launch was asked to open arrive too.
#[cfg(target_os = "macos")]
pub fn install() {
    mac::install();
}

/// No-op off macOS.
#[cfg(not(target_os = "macos"))]
pub fn install() {}

/// winit owns the app delegate (and panics if it's replaced), so this uses AppKit's Apple Event
/// route instead: when the app is about to finish launching (AppKit has installed its default
/// handlers by then, and the launch's own "open documents" event is still queued), our handler
/// takes over the `aevt`/`odoc` event.
#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter};

    /// `kCoreEventClass`, `kAEOpenDocuments` and `keyDirectObject` (four-character codes).
    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class has no Drop impl.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "WordCraftOpenDocumentsHandler"]
        struct Handler;

        // SAFETY: NSObjectProtocol has no safety requirements.
        unsafe impl NSObjectProtocol for Handler {}

        impl Handler {
            /// NSApplicationWillFinishLaunchingNotification observer.
            #[unsafe(method(willFinishLaunching:))]
            fn will_finish_launching(&self, _note: &NSNotification) {
                let manager = NSAppleEventManager::sharedAppleEventManager();
                // SAFETY: `self` implements the selector with the signature the manager calls
                // (`- (void)handleOpenDocuments:(NSAppleEventDescriptor *)withReply:(NSAppleEventDescriptor *)`),
                // and lives for the whole app (thread-local below).
                let () = unsafe {
                    msg_send![&manager, setEventHandler: self, andSelector: sel!(handleOpenDocuments:withReply:),
                        forEventClass: CORE_EVENT_CLASS, andEventID: OPEN_DOCUMENTS]
                };
            }

            #[unsafe(method(handleOpenDocuments:withReply:))]
            fn handle_open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                // SAFETY: `paramDescriptorForKeyword:` takes an AEKeyword (u32) and returns a nullable descriptor.
                let files: Option<Retained<NSAppleEventDescriptor>> = unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
                let Some(files) = files else { return };
                // A list of file references, or a single one.
                let items: Vec<Retained<NSAppleEventDescriptor>> = match files.numberOfItems() {
                    0 => vec![files],
                    n => (1..=n).filter_map(|i| files.descriptorAtIndex(i)).collect(),
                };
                super::push(items.iter().filter_map(|d| d.fileURLValue()).filter_map(|u| u.to_file_path()).collect());
            }
        }
    );

    thread_local! {
        // Neither the notification centre nor the Apple Event manager retains the handler.
        static HANDLER: std::cell::RefCell<Option<Retained<Handler>>> = const { std::cell::RefCell::new(None) };
    }

    pub fn install() {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("macos-open: install() called off the main thread; Finder file opening is off");
            return;
        };
        let this = Handler::alloc(mtm).set_ivars(());
        // SAFETY: `init` is NSObject's designated initialiser and the class adds no ivars to set up.
        let handler: Retained<Handler> = unsafe { msg_send![super(this), init] };
        // SAFETY: the handler implements `willFinishLaunching:` taking the notification, and is kept
        // alive for the app's lifetime (below), so the centre never messages a freed observer.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &handler,
                sel!(willFinishLaunching:),
                Some(NSApplicationWillFinishLaunchingNotification),
                None,
            );
        }
        HANDLER.with(|h| *h.borrow_mut() = Some(handler));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn queued_files_are_taken_once_in_order() {
        super::push(vec!["/a.docx".into(), "/b.docx".into()]);
        super::push(vec![]);
        assert_eq!(super::take(), vec![std::path::PathBuf::from("/a.docx"), "/b.docx".into()]);
        assert!(super::take().is_empty());
    }
}
