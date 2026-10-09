//! Finder "Open Documents" support for the macOS app.
//!
//! Finder (double click, Open With, a file dropped on the Dock icon) asks an app to open files by
//! sending it an `odoc` Apple Event, not by passing paths on the command line. winit's application
//! delegate doesn't answer that event, so AppKit's default handler tells the user "WordCraft
//! cannot open files in the Word Document format". This crate installs an `odoc` handler that
//! queues the paths; the app drains the queue every frame and opens them.
//!
//! AppKit installs its default Apple Event handlers inside `-[NSApplication finishLaunching]`, and
//! the `odoc` event for the files that launched the app arrives right after
//! `NSApplicationWillFinishLaunchingNotification`. So [`install`] registers for that notification
//! before the event loop starts, and the handler goes in when it fires: late enough not to be
//! replaced by AppKit's, early enough to catch the files the app was launched with.
//!
//! Talking to AppKit takes Objective-C messages, which Rust can only send as `unsafe` calls. This
//! is the one crate allowed `unsafe` (the workspace forbids it); it is confined to the `imp`
//! module, compiled on macOS only, and every call is a documented Foundation API. On other
//! platforms the public functions are no-ops.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::{Mutex, OnceLock};

/// Paths Finder asked us to open, not yet taken by the app.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Wakes the UI when a file arrives while it is idle.
static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Start listening for Finder's open requests. Call once, before the event loop runs.
pub fn install() {
    #[cfg(target_os = "macos")]
    imp::install();
}

/// Called whenever new paths are queued, so an idle UI wakes up and opens them.
pub fn set_waker(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKER.set(Box::new(wake));
}

/// Paths Finder asked us to open since the last call, in the order they arrived.
pub fn take_pending() -> Vec<String> {
    PENDING.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Queue paths for [`take_pending`] and wake the UI.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn push(paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    if let Ok(mut q) = PENDING.lock() {
        q.extend(paths);
    }
    if let Some(wake) = WAKER.get() {
        wake();
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{AllocAnyThread, class, define_class, msg_send, sel};
    use objc2_foundation::{NSObjectProtocol, NSString};

    /// `kCoreEventClass` ('aevt').
    const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    /// `kAEOpenDocuments` ('odoc').
    const K_AE_OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    /// `keyDirectObject` ('----').
    const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "WordCraftOpenDocumentsHandler"]
        struct OpenHandler;

        unsafe impl NSObjectProtocol for OpenHandler {}

        impl OpenHandler {
            #[unsafe(method(applicationWillFinishLaunching:))]
            fn will_finish_launching(&self, _notification: &AnyObject) {
                // SAFETY: -[NSAppleEventManager setEventHandler:andSelector:forEventClass:andEventID:]
                // with a handler that lives for the whole process (leaked in `install`) and
                // implements the selector registered.
                unsafe {
                    let manager: *mut AnyObject = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
                    let Some(manager) = manager.as_ref() else { return };
                    let _: () = msg_send![
                        manager,
                        setEventHandler: self,
                        andSelector: sel!(handleOpenDocuments:withReplyEvent:),
                        forEventClass: K_CORE_EVENT_CLASS,
                        andEventID: K_AE_OPEN_DOCUMENTS
                    ];
                }
            }

            #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
            fn handle_open_documents(&self, event: &AnyObject, _reply: &AnyObject) {
                super::push(paths_from_event(event));
            }
        }
    );

    /// File paths in an `odoc` event's direct object: a list of file references, or one.
    fn paths_from_event(event: &AnyObject) -> Vec<String> {
        let mut out = Vec::new();
        // SAFETY: NSAppleEventDescriptor accessors; every returned object is checked for nil.
        unsafe {
            let list: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT];
            let Some(list) = list.as_ref() else { return out };
            let n: isize = msg_send![list, numberOfItems];
            if n <= 0 {
                out.extend(path_of(list));
            } else {
                // Apple Event lists are 1-based; a few thousand files is already absurd.
                for i in 1..=n.min(4096) {
                    let item: *mut AnyObject = msg_send![list, descriptorAtIndex: i];
                    if let Some(item) = item.as_ref() {
                        out.extend(path_of(item));
                    }
                }
            }
        }
        out
    }

    /// Path of one file descriptor (alias, bookmark or file URL), via `-fileURLValue` (10.11+).
    fn path_of(desc: &AnyObject) -> Option<String> {
        // SAFETY: -[NSAppleEventDescriptor fileURLValue] and -[NSURL path]; nil-checked.
        unsafe {
            let url: *mut AnyObject = msg_send![desc, fileURLValue];
            let url = url.as_ref()?;
            let path: *mut NSString = msg_send![url, path];
            path.as_ref().map(|p| p.to_string())
        }
    }

    pub(super) fn install() {
        // SAFETY: -init on a freshly allocated NSObject subclass with no ivars.
        let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::alloc(), init] };
        let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
        // SAFETY: -[NSNotificationCenter addObserver:selector:name:object:]; the observer
        // implements the selector and is leaked below, so it outlives the registration.
        unsafe {
            let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
            if let Some(center) = center.as_ref() {
                let nil: *const AnyObject = std::ptr::null();
                let _: () = msg_send![
                    center,
                    addObserver: &*handler,
                    selector: sel!(applicationWillFinishLaunching:),
                    name: &*name,
                    object: nil
                ];
            }
        }
        // Neither the notification center nor the Apple Event manager retains its target.
        std::mem::forget(handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_drains_in_order_and_wakes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static WOKEN: AtomicUsize = AtomicUsize::new(0);
        set_waker(|| {
            WOKEN.fetch_add(1, Ordering::SeqCst);
        });
        push(vec![]);
        assert_eq!(WOKEN.load(Ordering::SeqCst), 0, "nothing queued, nothing to wake for");
        push(vec!["/a.docx".into(), "/b.rtf".into()]);
        push(vec!["/c.odt".into()]);
        assert_eq!(take_pending(), ["/a.docx", "/b.rtf", "/c.odt"]);
        assert!(take_pending().is_empty());
        assert_eq!(WOKEN.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn install_is_harmless_off_the_main_app() {
        // Off macOS this is a no-op; on macOS it only registers a notification observer.
        install();
    }
}
