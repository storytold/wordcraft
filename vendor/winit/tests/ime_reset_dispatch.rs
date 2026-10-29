//! Native AppKit driver for mixed immediate/deferred reset dispatch.
//! Run each scenario in a fresh process on the main thread.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::Cell;
    use std::process::Command;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use block2::{Block, RcBlock};
    use core_foundation::base::CFTypeRef;
    use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoopGetMain, CFRunLoopRef};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSNotFound, NSRange, NSString};
    use rwh_06::{HasWindowHandle, RawWindowHandle};
    use winit::application::ApplicationHandler;
    use winit::event::{Ime, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowExtMacOS};
    use winit::window::{Window, WindowId};

    extern "C" {
        fn CFRunLoopPerformBlock(run_loop: CFRunLoopRef, mode: CFTypeRef, block: &Block<dyn Fn()>);
    }

    fn later(action: impl FnOnce() + 'static) {
        let action = Cell::new(Some(action));
        let block = RcBlock::new(move || {
            if let Some(action) = action.take() {
                action();
            }
        });
        unsafe { CFRunLoopPerformBlock(CFRunLoopGetMain(), kCFRunLoopDefaultMode as _, &block) };
    }

    fn marked(view: &AnyObject, text: &str) {
        let text = NSString::from_str(text);
        let _: () = unsafe {
            objc2::msg_send![view, setMarkedText: &*text
                selectedRange: NSRange::new(0, 0)
                replacementRange: NSRange::new(NSNotFound as _, 0)]
        };
    }

    fn inserted(view: &AnyObject, text: &str, range: NSRange) {
        let text = NSString::from_str(text);
        let _: () = unsafe { objc2::msg_send![view, insertText: &*text replacementRange: range] };
    }

    struct App {
        scenario: String,
        proxy: EventLoopProxy<()>,
        window: Option<Window>,
        events: Vec<WindowEvent>,
        requested: bool,
        checked: Rc<Cell<bool>>,
    }

    impl ApplicationHandler<()> for App {
        fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
            self.checked.set(true);
            event_loop.exit();
        }

        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let window =
                event_loop.create_window(Window::default_attributes().with_visible(false)).unwrap();
            window.set_ime_allowed(true);
            let RawWindowHandle::AppKit(handle) = window.window_handle().unwrap().as_raw() else {
                panic!("AppKit view required");
            };
            // Retain the native view so the close scenario can also attempt a late callback.
            let view =
                unsafe { Retained::<AnyObject>::retain(handle.ns_view.as_ptr().cast()) }.unwrap();
            self.window = Some(window);
            let scenario = self.scenario.clone();
            let proxy = self.proxy.clone();
            // This block runs outside an application callback. The first native text callback
            // enters the actual ApplicationDelegate, requests reset, and returns to this same
            // driver before CF blocks can run. The second callback used to dispatch immediately.
            later(move || {
                if scenario == "attached" {
                    inserted(&view, "abc", NSRange::new(NSNotFound as _, 0));
                    inserted(&view, "é", NSRange::new(2, 1));
                } else if scenario == "empty" {
                    inserted(&view, "old", NSRange::new(NSNotFound as _, 0));
                } else if scenario == "terminal" {
                    marked(&view, "seed");
                    inserted(&view, "copied old", NSRange::new(NSNotFound as _, 0));
                } else {
                    marked(&view, "old");
                }
                marked(&view, "new");
                inserted(&view, "fresh", NSRange::new(NSNotFound as _, 0));
                if scenario == "attached" {
                    // After old replacement, reset starts a new document range at offset 3.
                    // "fresh" has five UTF-16 units, so its final character is at 7.
                    inserted(&view, "é", NSRange::new(7, 1));
                }
                later(move || proxy.send_event(()).unwrap());
            });
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            let trigger = match &event {
                WindowEvent::Ime(Ime::Preedit(text, _)) => {
                    text == "old" || self.scenario == "terminal" && text.is_empty()
                },
                WindowEvent::Ime(Ime::Commit(text)) => text == "old" || text == "é",
                _ => false,
            };
            if let WindowEvent::Ime(Ime::Commit(text)) = &event {
                if text == "é" {
                    let delete =
                        self.window.as_ref().unwrap().take_ime_delete_surrounding().unwrap();
                    assert_eq!((delete.before_bytes, delete.after_bytes), (1, 0));
                }
            }
            if trigger && !self.requested {
                self.requested = true;
                self.window.as_ref().unwrap().reset_ime_with_request(7);
                if self.scenario == "close" {
                    self.window.take();
                }
            }
            self.events.push(event);
        }
    }

    fn child(scenario: String) {
        let mut builder = EventLoop::builder();
        builder.with_activation_policy(ActivationPolicy::Accessory).with_default_menu(false);
        let event_loop = builder.build().unwrap();
        let checked = Rc::new(Cell::new(false));
        let mut app = App {
            scenario: scenario.clone(),
            proxy: event_loop.create_proxy(),
            window: None,
            events: Vec::new(),
            requested: false,
            checked: Rc::clone(&checked),
        };
        event_loop.run_app(&mut app).unwrap();
        assert!(checked.get() && app.requested);
        let completion = app
            .events
            .iter()
            .position(|event| matches!(event, WindowEvent::ImeResetCompleted { request_id: 7 }));
        if scenario == "close" {
            assert!(completion.is_none(), "closed view delivered reset completion");
        } else {
            let completion = completion.expect("completion was not delivered");
            let fresh = app.events.iter().position(|event| matches!(event, WindowEvent::Ime(Ime::Preedit(text, _)) if text == "new")).unwrap();
            assert!(
                completion < fresh,
                "fresh callback bypassed deferred completion: {:?}",
                app.events
            );
            if scenario == "terminal" {
                let old = app.events.iter().position(|event| matches!(event, WindowEvent::Ime(Ime::Commit(text)) if text == "copied old")).unwrap();
                assert!(
                    old < completion,
                    "copied old result followed completion: {:?}",
                    app.events
                );
            }
        }
    }

    pub fn run() {
        if let Some(scenario) = std::env::args().nth(1).filter(|s| !s.starts_with('-')) {
            child(scenario);
            return;
        }
        for scenario in ["marked", "empty", "attached", "terminal", "close"] {
            let mut child =
                Command::new(std::env::current_exe().unwrap()).arg(scenario).spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("native dispatch scenario timed out: {scenario}");
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            assert!(status.success(), "native dispatch scenario failed: {scenario}");
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}
