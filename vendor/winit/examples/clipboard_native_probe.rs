//! Opt-in native protocol probe; driven by a private compositor, never a user's clipboard.
#[cfg(all(unix, not(target_os = "macos"), feature = "wayland"))]
mod probe {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use winit::application::ApplicationHandler;
    use winit::event::{ElementState, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
    use winit::keyboard::Key;
    use winit::platform::wayland::{
        ActiveEventLoopExtWayland, EventLoopBuilderExtWayland, SelectionClaim, TransferHandle,
        TransferOptions,
    };
    use winit::window::{Window, WindowId};

    #[derive(Debug)]
    pub struct Ready;
    struct Pending {
        id: u64,
        family: &'static str,
        started: Instant,
        handle: TransferHandle,
    }
    pub struct App {
        window: Option<Window>,
        proxy: EventLoopProxy<Ready>,
        pending: Vec<Pending>,
        sequence: u64,
        started: Instant,
        frames: usize,
    }

    fn fingerprint(s: &str) -> u64 {
        s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
    }
    impl App {
        fn read(&mut self, event_loop: &ActiveEventLoop, primary: bool, cap: usize) {
            let Some(window) = self.window.as_ref() else {
                return;
            };
            let Some(clipboard) = event_loop.clipboard() else {
                println!("NO_WAYLAND");
                return;
            };
            self.sequence += 1;
            let id = self.sequence;
            let family = if primary { "primary" } else { "regular" };
            let proxy = self.proxy.clone();
            let options = TransferOptions::new(
                window.id(),
                id,
                Arc::new(move || {
                    let _ = proxy.send_event(Ready);
                }),
            )
            .with_limits(Duration::from_millis(800), cap);
            let started = Instant::now();
            let result = if primary {
                clipboard.begin_load_primary_text(options)
            } else {
                clipboard.begin_load_text(options)
            };
            match result {
                Ok(handle) => {
                    println!("ADMITTED {id} {family} us={}", started.elapsed().as_micros());
                    self.pending.push(Pending { id, family, started, handle });
                },
                Err(error) => println!("ADMISSION_ERROR {id} {family} {:?}", error.kind()),
            }
            self.drain();
        }
        fn drain(&mut self) {
            self.pending.retain_mut(|pending| {
                let Some(result) = pending.handle.try_take() else {
                    return true;
                };
                match result {
                    Ok(text) => println!(
                        "COMPLETE {} {} bytes={} hash={:016x} us={}",
                        pending.id,
                        pending.family,
                        text.len(),
                        fingerprint(&text),
                        pending.started.elapsed().as_micros()
                    ),
                    Err(error) => println!(
                        "TRANSFER_ERROR {} {} {:?}",
                        pending.id,
                        pending.family,
                        error.kind()
                    ),
                }
                assert!(pending.handle.try_take().is_none(), "completion delivered twice");
                false
            });
        }
        fn claim(&mut self, event_loop: &ActiveEventLoop) {
            let Some(window) = self.window.as_ref() else {
                return;
            };
            let Some(clipboard) = event_loop.clipboard() else {
                return;
            };
            let origin = match clipboard.input_origin(window.id()) {
                Ok(origin) => origin,
                Err(error) => {
                    println!("ORIGIN_ERROR {:?}", error.kind());
                    return;
                },
            };
            let claim = SelectionClaim {
                target_window: window.id(),
                caller_generation: self.sequence,
                trigger: origin,
            };
            let text: Arc<str> = Arc::from("owned native 🦀\n");
            for (family, result) in [
                ("regular", clipboard.claim_text(text.clone(), claim.clone())),
                ("primary", clipboard.claim_primary_text(text.clone(), claim.clone())),
            ] {
                match result {
                    Ok(()) => println!("CLAIMED {family}"),
                    Err(error) => println!("CLAIM_ERROR {family} {:?}", error.kind()),
                }
            }
            assert_eq!(
                clipboard.claim_text(text, claim).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
            println!("DUPLICATE_CLAIM_REJECTED");
            self.read(event_loop, false, 16 * 1024 * 1024);
            self.read(event_loop, true, 16 * 1024 * 1024);
        }
    }
    impl ApplicationHandler<Ready> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none() {
                self.window = Some(
                    event_loop
                        .create_window(
                            Window::default_attributes()
                                .with_title("Craft clipboard native probe")
                                .with_inner_size(winit::dpi::LogicalSize::new(640.0, 360.0)),
                        )
                        .expect("create probe window"),
                );
                self.window.as_ref().unwrap().request_redraw();
            }
        }
        fn user_event(&mut self, _: &ActiveEventLoop, _: Ready) {
            println!("WAKE");
            self.drain();
        }
        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::RedrawRequested => {
                    if let Some(window) = self.window.as_ref() {
                        window.pre_present_notify();
                        crate::fill::fill_window(window);
                        self.frames += 1;
                        println!("FRAME {}", self.frames);
                    }
                },
                WindowEvent::Focused(focused) => println!("FOCUSED {focused}"),
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed =>
                {
                    if let Key::Character(key) = event.logical_key {
                        println!("KEY {key}");
                        match key.as_str() {
                            "r" => self.read(event_loop, false, 16 * 1024 * 1024),
                            "p" => self.read(event_loop, true, 16 * 1024 * 1024),
                            "b" => self.read(event_loop, false, 1024),
                            "c" => self.claim(event_loop),
                            "q" => event_loop.exit(),
                            _ => {},
                        }
                    }
                },
                WindowEvent::CloseRequested => event_loop.exit(),
                _ => {},
            }
        }
        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if self.started.elapsed() > Duration::from_secs(60) {
                println!("PROBE_TIMEOUT");
                event_loop.exit();
            }
            event_loop
                .set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1)));
        }
        fn exiting(&mut self, _: &ActiveEventLoop) {
            if let Some(window) = self.window.as_ref() {
                crate::fill::cleanup_window(window);
            }
            println!("EXIT pending={}", self.pending.len());
        }
    }
    pub fn run() {
        let event_loop = EventLoop::<Ready>::with_user_event()
            .with_wayland()
            .build()
            .expect("Wayland event loop");
        let mut app = App {
            window: None,
            proxy: event_loop.create_proxy(),
            pending: Vec::new(),
            sequence: 0,
            started: Instant::now(),
            frames: 0,
        };
        event_loop.run_app(&mut app).expect("native probe event loop");
    }
}
#[cfg(all(unix, not(target_os = "macos"), feature = "wayland"))]
#[path = "util/fill.rs"]
mod fill;
fn main() {
    #[cfg(all(unix, not(target_os = "macos"), feature = "wayland"))]
    probe::run();
}
