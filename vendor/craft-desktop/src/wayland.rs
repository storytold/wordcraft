//! KDE AppMenu metadata on an existing Wayland surface and connection.
//!
//! The host retains its window and calls `poll` on the UI thread. Winit owns
//! socket reads and the surface's event queue. This module dispatches only its
//! own registry/appmenu events, using a waker instead of blocking roundtrips.

use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use wayland_client::protocol::{wl_display, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_plasma::appmenu::client::{
    org_kde_kwin_appmenu as appmenu, org_kde_kwin_appmenu_manager as manager,
};

type Manager = manager::OrgKdeKwinAppmenuManager;
type AppMenu = appmenu::OrgKdeKwinAppmenu;

struct WakeUi(Arc<dyn Fn() + Send + Sync>);

impl Wake for WakeUi {
    fn wake(self: Arc<Self>) {
        (self.0)();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        (self.0)();
    }
}

/// Nonblocking, UI-thread companion to the D-Bus exporter.
///
/// The caller must drop this before releasing the associated window. A surface
/// clone does not transfer ownership, and we never destroy or reassign it.
pub struct Host {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    _registry: wl_registry::WlRegistry,
    waker: Waker,
}

struct State {
    surface: wl_surface::WlSurface,
    manager: Option<(u32, Manager)>,
    appmenu: Option<AppMenu>,
    address: Option<(String, String)>,
    published: Option<(String, String)>,
    error: Option<String>,
}

impl Host {
    pub fn new(
        surface: wl_surface::WlSurface,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        if !surface.is_alive() {
            return Err("Wayland appmenu: surface is no longer alive".into());
        }
        let backend = surface
            .backend()
            .upgrade()
            .ok_or("Wayland appmenu: connection is no longer alive")?;
        let connection = Connection::from_backend(backend);
        let queue = connection.new_event_queue::<State>();
        let registry = connection
            .display()
            .send_constructor(
                wl_display::Request::GetRegistry {},
                queue.handle().make_data::<wl_registry::WlRegistry, ()>(()),
            )
            .map_err(|e| format!("Wayland appmenu registry: {e}"))?;
        let mut host = Self {
            connection,
            queue,
            state: State {
                surface,
                manager: None,
                appmenu: None,
                address: None,
                published: None,
                error: None,
            },
            _registry: registry,
            waker: Waker::from(Arc::new(WakeUi(repaint))),
        };
        // Register the waker before returning. Subsequent registry events are
        // routed by winit's read and schedule a frame even for an idle window.
        host.poll()?;
        Ok(host)
    }

    /// Advertise only an address whose object is already exported on D-Bus.
    /// If this compositor has no appmenu manager, keep the address for a later
    /// advertisement without changing any window or desktop settings.
    pub fn set_address(&mut self, service: &str, path: &str) -> Result<(), String> {
        validate_address(service, path)?;
        self.state.address = Some((service.to_owned(), path.to_owned()));
        self.state.synchronize(&self.queue.handle())?;
        self.flush()
    }

    /// Remove stale metadata when the exporter loses its D-Bus connection.
    pub fn clear_address(&mut self) -> Result<(), String> {
        self.state.address = None;
        self.state.release_menu();
        self.flush()
    }

    /// Dispatch only events queued for this adapter; never read the socket or
    /// wait for the compositor. The registered callback requests the next poll.
    pub fn poll(&mut self) -> Result<(), String> {
        if !self.state.surface.is_alive() {
            return Err("Wayland appmenu: surface was destroyed".into());
        }
        let mut context = Context::from_waker(&self.waker);
        if let Poll::Ready(result) = self
            .queue
            .poll_dispatch_pending(&mut context, &mut self.state)
        {
            return match result {
                Err(error) => Err(format!("Wayland appmenu dispatch: {error}")),
                Ok(never) => match never {},
            };
        }
        if let Some(error) = self.state.error.take() {
            return Err(error);
        }
        self.state.synchronize(&self.queue.handle())?;
        self.flush()
    }

    fn flush(&self) -> Result<(), String> {
        match self.connection.flush() {
            Ok(()) => Ok(()),
            // Winit will retry flushing the same connection. Backpressure is
            // not a disconnected compositor and must not disable the menu.
            Err(wayland_client::backend::WaylandError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                Ok(())
            }
            Err(error) => Err(format!("Wayland appmenu flush: {error}")),
        }
    }
}

fn validate_address(service: &str, path: &str) -> Result<(), String> {
    // Wayland appmenu strings are Latin-1 D-Bus names/paths. The D-Bus syntax
    // validators also reject invalid values before they reach the compositor.
    if !service.is_ascii() || !path.is_ascii() {
        return Err("Wayland appmenu address must be ASCII".into());
    }
    zbus::names::BusName::try_from(service).map_err(|e| format!("Wayland appmenu service: {e}"))?;
    zbus::zvariant::ObjectPath::try_from(path).map_err(|e| format!("Wayland appmenu path: {e}"))?;
    Ok(())
}

impl State {
    fn synchronize(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        let (Some((_, manager)), Some(address)) = (&self.manager, &self.address) else {
            return Ok(());
        };
        if !self.surface.is_alive() || !manager.is_alive() {
            return Err("Wayland appmenu: surface or manager was destroyed".into());
        }
        if self.appmenu.is_none() {
            self.appmenu = Some(
                manager
                    .send_constructor(
                        manager::Request::Create {
                            surface: self.surface.clone(),
                        },
                        qh.make_data::<AppMenu, ()>(()),
                    )
                    .map_err(|e| format!("Wayland appmenu create: {e}"))?,
            );
        }
        if self.published.as_ref() != Some(address)
            && let Some(appmenu) = &self.appmenu
        {
            appmenu
                .send_request(appmenu::Request::SetAddress {
                    service_name: address.0.clone(),
                    object_path: address.1.clone(),
                })
                .map_err(|e| format!("Wayland appmenu set_address: {e}"))?;
            self.published = Some(address.clone());
            log::debug!(
                "Wayland appmenu address associated: {} {}",
                address.0,
                address.1
            );
        }
        Ok(())
    }

    fn release_menu(&mut self) {
        if let Some(appmenu) = self.appmenu.take()
            && appmenu.is_alive()
        {
            let _ = appmenu.send_request(appmenu::Request::Release {});
        }
        self.published = None;
    }

    fn release_manager(&mut self) {
        self.release_menu();
        if let Some((_, manager)) = self.manager.take()
            && manager.is_alive()
            && manager.version() >= 2
        {
            let _ = manager.send_request(manager::Request::Release {});
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.state.release_manager();
        let _ = self.flush();
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == Manager::interface().name
                && version > 0
                && state.manager.is_none() =>
            {
                let manager = registry.send_constructor::<Manager>(
                    wl_registry::Request::Bind {
                        name,
                        id: (Manager::interface(), version.min(2)),
                    },
                    qh.make_data::<Manager, ()>(()),
                );
                match manager {
                    Ok(manager) => state.manager = Some((name, manager)),
                    Err(error) => state.error = Some(format!("Wayland appmenu bind: {error}")),
                }
            }
            wl_registry::Event::GlobalRemove { name }
                if state.manager.as_ref().is_some_and(|(id, _)| *id == name) =>
            {
                state.release_manager();
            }
            _ => {}
        }
    }
}

// These protocols carry requests only; they never send events to the client.
impl Dispatch<Manager, ()> for State {
    fn event(
        _: &mut Self,
        _: &Manager,
        _: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<AppMenu, ()> for State {
    fn event(
        _: &mut Self,
        _: &AppMenu,
        _: appmenu::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wayland_client::protocol::wl_compositor;

    #[test]
    fn accepts_well_known_and_unique_dbus_addresses() {
        assert!(validate_address("ai.storyteller.wordcraft.AppMenu.p42", "/MenuBar").is_ok());
        assert!(validate_address(":1.42", "/MenuBar").is_ok());
        assert!(validate_address("not a bus name", "/MenuBar").is_err());
        assert!(validate_address(":1.42", "relative/path").is_err());
        assert!(validate_address(":1.42", "/Menu bar").is_err());
        assert!(validate_address(":1.42", "/Меню").is_err());
    }

    #[test]
    fn queue_waker_calls_the_host_without_an_executor() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let waker = Waker::from(Arc::new(WakeUi(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }))));
        waker.wake_by_ref();
        waker.wake();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    // A deliberately small wire peer, not a compositor implementation. It
    // advertises only the two interfaces these tests need, and records exact
    // object IDs/requests. Every read has a deadline; no real desktop is used.
    struct Peer(UnixStream);

    impl Peer {
        fn read(&mut self) -> (u32, u16, Vec<u8>) {
            let mut header = [0; 8];
            self.0.read_exact(&mut header).unwrap();
            let id = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let size_opcode = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let size = usize::try_from(size_opcode >> 16).unwrap();
            assert!((8..=4096).contains(&size));
            let mut payload = vec![0; size - 8];
            self.0.read_exact(&mut payload).unwrap();
            (id, (size_opcode & 0xffff) as u16, payload)
        }

        fn event(&mut self, object: u32, opcode: u16, payload: &[u8]) {
            let size_opcode = (u32::try_from(payload.len() + 8).unwrap() << 16) | u32::from(opcode);
            self.0.write_all(&object.to_ne_bytes()).unwrap();
            self.0.write_all(&size_opcode.to_ne_bytes()).unwrap();
            self.0.write_all(payload).unwrap();
        }

        fn global(&mut self, registry: u32, name: u32, interface: &str, version: u32) {
            let mut payload = name.to_ne_bytes().to_vec();
            wire_string(&mut payload, interface);
            payload.extend_from_slice(&version.to_ne_bytes());
            self.event(registry, 0, &payload);
        }

        fn empty(&mut self) {
            self.0.set_nonblocking(true).unwrap();
            let result = self.0.read(&mut [0]);
            self.0.set_nonblocking(false).unwrap();
            assert!(matches!(result, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
        }
    }

    fn wire_string(payload: &mut Vec<u8>, value: &str) {
        payload.extend_from_slice(&u32::try_from(value.len() + 1).unwrap().to_ne_bytes());
        payload.extend_from_slice(value.as_bytes());
        payload.push(0);
        while !payload.len().is_multiple_of(4) {
            payload.push(0);
        }
    }

    fn word(payload: &[u8], index: usize) -> u32 {
        u32::from_ne_bytes(payload[index * 4..index * 4 + 4].try_into().unwrap())
    }

    #[derive(Default)]
    struct SurfaceState {
        compositor: Option<wl_compositor::WlCompositor>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for SurfaceState {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name, interface, ..
            } = event
                && interface == "wl_compositor"
            {
                state.compositor = Some(registry.bind(name, 1, qh, ()));
            }
        }
    }

    impl Dispatch<wl_compositor::WlCompositor, ()> for SurfaceState {
        fn event(
            _: &mut Self,
            _: &wl_compositor::WlCompositor,
            _: wl_compositor::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_surface::WlSurface, ()> for SurfaceState {
        fn event(
            _: &mut Self,
            _: &wl_surface::WlSurface,
            _: wl_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    struct Fixture {
        connection: Connection,
        queue: EventQueue<SurfaceState>,
        state: SurfaceState,
        surface: wl_surface::WlSurface,
        peer: Peer,
    }

    impl Fixture {
        fn new() -> Self {
            let (client, server) = UnixStream::pair().unwrap();
            server
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            server
                .set_write_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            let mut peer = Peer(server);
            let connection = Connection::from_socket(client).unwrap();
            let mut queue = connection.new_event_queue::<SurfaceState>();
            let registry = connection.display().get_registry(&queue.handle(), ());
            connection.flush().unwrap();
            let (id, opcode, payload) = peer.read();
            assert_eq!((id, opcode), (1, 1)); // wl_display.get_registry
            assert_eq!(word(&payload, 0), registry.id().protocol_id());
            peer.global(registry.id().protocol_id(), 7, "wl_compositor", 1);
            connection.prepare_read().unwrap().read().unwrap();
            let mut state = SurfaceState::default();
            queue.dispatch_pending(&mut state).unwrap();
            connection.flush().unwrap();
            let (id, opcode, _) = peer.read();
            assert_eq!((id, opcode), (registry.id().protocol_id(), 0));
            let compositor = state.compositor.as_ref().unwrap();
            let surface = compositor.create_surface(&queue.handle(), ());
            connection.flush().unwrap();
            let (id, opcode, payload) = peer.read();
            assert_eq!((id, opcode), (compositor.id().protocol_id(), 0));
            assert_eq!(word(&payload, 0), surface.id().protocol_id());
            Self {
                connection,
                queue,
                state,
                surface,
                peer,
            }
        }

        // This stands in for winit: only the owner reads the socket and routes
        // events. The appmenu Host itself never performs a read.
        fn read_as_window_host(&mut self) {
            self.connection.prepare_read().unwrap().read().unwrap();
            self.queue.dispatch_pending(&mut self.state).unwrap();
        }
    }

    #[test]
    fn wire_association_uses_existing_surface_and_owns_only_its_metadata() {
        for version in [1, 2] {
            let mut fixture = Fixture::new();
            let wakes = Arc::new(AtomicUsize::new(0));
            let observed = wakes.clone();
            let mut host = Host::new(
                fixture.surface.clone(),
                Arc::new(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                }),
            )
            .unwrap();
            let (id, opcode, payload) = fixture.peer.read();
            assert_eq!((id, opcode), (1, 1));
            let registry_id = word(&payload, 0);

            host.set_address(":1.42", "/MenuBar").unwrap();
            host.poll().unwrap();
            fixture.peer.empty(); // No manager: address is cached, no wait/read.
            fixture
                .peer
                .global(registry_id, 11, Manager::interface().name, version);
            fixture.read_as_window_host();
            assert!(wakes.load(Ordering::Relaxed) > 0);
            host.poll().unwrap();

            let (id, opcode, payload) = fixture.peer.read();
            assert_eq!((id, opcode), (registry_id, 0));
            assert_eq!(word(&payload, 0), 11);
            let manager_id = word(&payload, payload.len() / 4 - 1);
            let (id, opcode, payload) = fixture.peer.read();
            assert_eq!((id, opcode), (manager_id, 0));
            let appmenu_id = word(&payload, 0);
            assert_eq!(word(&payload, 1), fixture.surface.id().protocol_id());
            let mut expected_address = Vec::new();
            wire_string(&mut expected_address, ":1.42");
            wire_string(&mut expected_address, "/MenuBar");
            assert_eq!(fixture.peer.read(), (appmenu_id, 0, expected_address));

            host.set_address(":1.42", "/MenuBar").unwrap();
            fixture.peer.empty(); // Unchanged addresses do not emit requests.
            host.set_address(":1.43", "/MenuBar").unwrap();
            let (id, opcode, _) = fixture.peer.read();
            assert_eq!((id, opcode), (appmenu_id, 0));
            host.clear_address().unwrap();
            assert_eq!(fixture.peer.read(), (appmenu_id, 1, vec![]));

            host.set_address(":1.44", "/MenuBar").unwrap();
            let (id, opcode, payload) = fixture.peer.read();
            assert_eq!((id, opcode), (manager_id, 0));
            let replacement_id = word(&payload, 0);
            let (id, opcode, _) = fixture.peer.read();
            assert_eq!((id, opcode), (replacement_id, 0));
            drop(host);
            assert_eq!(fixture.peer.read(), (replacement_id, 1, vec![]));
            if version == 2 {
                assert_eq!(fixture.peer.read(), (manager_id, 1, vec![]));
            }
            fixture.peer.empty(); // Never destroy the window's wl_surface.
            assert!(fixture.surface.is_alive());
        }
    }

    #[test]
    fn wire_global_removal_releases_metadata_and_reappearance_rebinds() {
        let mut fixture = Fixture::new();
        let mut host = Host::new(fixture.surface.clone(), Arc::new(|| {})).unwrap();
        let (_, _, payload) = fixture.peer.read();
        let registry_id = word(&payload, 0);
        host.set_address(":1.42", "/MenuBar").unwrap();
        fixture
            .peer
            .global(registry_id, 11, Manager::interface().name, 2);
        fixture.read_as_window_host();
        host.poll().unwrap();
        let (_, _, payload) = fixture.peer.read();
        let manager_id = word(&payload, payload.len() / 4 - 1);
        let (_, _, payload) = fixture.peer.read();
        let appmenu_id = word(&payload, 0);
        fixture.peer.read();

        fixture.peer.event(registry_id, 1, &11u32.to_ne_bytes());
        fixture.read_as_window_host();
        host.poll().unwrap();
        assert_eq!(fixture.peer.read(), (appmenu_id, 1, vec![]));
        assert_eq!(fixture.peer.read(), (manager_id, 1, vec![]));
        fixture.peer.empty();

        fixture
            .peer
            .global(registry_id, 12, Manager::interface().name, 2);
        fixture.read_as_window_host();
        host.poll().unwrap();
        let (_, _, payload) = fixture.peer.read();
        assert_eq!(word(&payload, 0), 12);
        let (_, _, payload) = fixture.peer.read();
        assert_eq!(word(&payload, 1), fixture.surface.id().protocol_id());
        let (_, opcode, _) = fixture.peer.read();
        assert_eq!(opcode, 0); // Cached address is republished after rebind.
    }
}
