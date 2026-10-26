//! AppKit termination routing must be tested on the process's main thread.
//! Each scenario has its own process because NSApplication and Winit are global.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod macos {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply};
    use objc2_foundation::MainThreadMarker;
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    use winit::window::{Window, WindowId};

    #[derive(Clone, Copy, Debug)]
    enum UserEvent {
        Retry,
        CheckStale,
    }

    struct App {
        scenario: String,
        window: Option<Window>,
        proxy: EventLoopProxy<UserEvent>,
        closes: usize,
    }

    fn application() -> objc2::rc::Retained<NSApplication> {
        NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"))
    }

    fn should_terminate() -> NSApplicationTerminateReply {
        let app = application();
        // This sends the actual AppKit delegate selector, not a mock decision function.
        unsafe {
            app.delegate().expect("Winit application delegate").applicationShouldTerminate(&app)
        }
    }

    impl ApplicationHandler<UserEvent> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.scenario == "no-window" {
                assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateNow);
                event_loop.exit();
                return;
            }
            self.window = Some(
                event_loop
                    .create_window(Window::default_attributes().with_visible(false))
                    .expect("window"),
            );
            if self.scenario == "default" {
                assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateNow);
                event_loop.exit();
                return;
            }
            if self.scenario == "native-terminate" {
                // terminate: must return to the ordinary run loop before CloseRequested runs.
                unsafe { application().terminate(None) };
                assert!(!event_loop.exiting());
                assert_eq!(self.closes, 0);
                println!("native terminate returned without exiting");
                return;
            }
            assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateCancel);
            assert!(!event_loop.exiting());
            assert_eq!(self.closes, 0, "close requests must be asynchronous");
            assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateCancel);
            if self.scenario == "stale-window" {
                self.window = None;
                self.proxy.send_event(UserEvent::CheckStale).expect("queue stale check");
            }
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            if event != WindowEvent::CloseRequested {
                return;
            }
            self.closes += 1;
            assert!(self.closes <= 2, "duplicate termination requests were not coalesced");
            if self.scenario == "native-terminate" {
                assert_eq!(self.closes, 1);
                event_loop.exit();
                return;
            }
            assert_eq!(self.scenario, "cancel-retry");
            if self.closes == 1 {
                // A request nested inside an active close handler must not re-enter it.
                assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateCancel);
                assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateCancel);
                assert!(!event_loop.exiting());
                self.proxy.send_event(UserEvent::Retry).expect("queue retry");
            } else {
                event_loop.exit();
                // An already approved event-loop exit must not be postponed again.
                assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateNow);
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
            match event {
                UserEvent::Retry => {
                    assert_eq!(self.closes, 1);
                    assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateCancel);
                },
                UserEvent::CheckStale => {
                    assert_eq!(self.closes, 0, "a closed window received a stale CloseRequested");
                    assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateNow);
                    event_loop.exit();
                },
            }
        }
    }

    fn child(scenario: String) {
        let mut builder = EventLoop::<UserEvent>::with_user_event();
        builder.with_activation_policy(ActivationPolicy::Accessory).with_default_menu(false);
        if scenario != "default" {
            builder.with_close_requested_on_terminate(true);
        }
        let event_loop = builder.build().expect("event loop");
        assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateNow);
        if scenario == "not-running" {
            println!("scenario passed: {scenario}");
            return;
        }
        let mut app = App {
            scenario: scenario.clone(),
            window: None,
            proxy: event_loop.create_proxy(),
            closes: 0,
        };
        event_loop.run_app(&mut app).expect("run app");
        let expected_closes = match scenario.as_str() {
            "cancel-retry" => 2,
            "native-terminate" => 1,
            _ => 0,
        };
        assert_eq!(app.closes, expected_closes);
        assert_eq!(should_terminate(), NSApplicationTerminateReply::NSTerminateNow);
        println!("scenario passed: {scenario}");
    }

    pub fn run() {
        if let Some(scenario) = std::env::args().nth(1).filter(|s| !s.starts_with('-')) {
            child(scenario);
            return;
        }
        for scenario in [
            "not-running",
            "no-window",
            "default",
            "cancel-retry",
            "stale-window",
            "native-terminate",
        ] {
            let mut child = Command::new(std::env::current_exe().expect("test executable"))
                .arg(scenario)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start scenario");
            let deadline = Instant::now() + Duration::from_secs(15);
            let status = loop {
                if let Some(status) = child.try_wait().expect("wait for scenario") {
                    break status;
                }
                if Instant::now() >= deadline {
                    child.kill().expect("stop hung scenario");
                    child.wait().expect("reap scenario");
                    panic!("scenario {scenario} did not wake or exit the event loop");
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            let mut stdout = String::new();
            let mut stderr = String::new();
            child
                .stdout
                .take()
                .expect("scenario stdout")
                .read_to_string(&mut stdout)
                .expect("read stdout");
            child
                .stderr
                .take()
                .expect("scenario stderr")
                .read_to_string(&mut stderr)
                .expect("read stderr");
            assert!(status.success(), "scenario {scenario}: {status}\n{stdout}\n{stderr}");
            // A bad NSTerminateNow may exit successfully without executing the assertions.
            assert!(
                stdout.contains(&format!("scenario passed: {scenario}")),
                "scenario {scenario} exited before completion:\n{stdout}\n{stderr}"
            );
            println!("test macos_termination::{scenario} ... ok");
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}
