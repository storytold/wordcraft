# craft-desktop (local integration snapshot)

Portable menu snapshots + opt-in Linux DBusMenu/window binding and portal appearance.
No app or egui dependency. The caller must revalidate queued actions on its UI thread
and retain its local menu. Enable `linux` in the desktop host, `portal` for appearance;
the default model-only feature set builds without Linux dependencies (including wasm).

Canonical source: workspace `shared/craft-desktop`; identical copies in each app's
`vendor/craft-desktop` keep independent checkouts and Flatpak builds self-contained.
The root winit patch must point to the adjacent `vendor/winit` snapshot. Patches in
this dependency's manifest do not propagate to downstream workspaces.

The transport derives from the pre-existing WordCraft DBusMenu implementation,
under the same MIT OR Apache-2.0 licence. Native Wayland uses a minimal additive safe
winit accessor; see `../winit/CRAFT_PATCH.md` for provenance and maintenance.

## Recovery and overload contract (2026-10-22)

- Every exporter recovery retires all previously advertised item IDs. The counter
  survives reconnection at the same service/path; it fails closed on exhaustion,
  rather than wrapping. A cached old ID cannot select a replacement action/document.
- The click queue is bounded to 32 entries. Accepted events remain FIFO. On overflow,
  `Event` returns a D-Bus error and `EventGroup` returns the rejected IDs; the failed
  request was not accepted. There is no blocking send under the tree read lock and no
  unbounded fallback queue. This intentionally rejects overload rather than promising
  lossless delivery for an arbitrarily stalled UI or unlimited desktop-session callers.
- Portal read/type failures preserve the affected field's last value independently.
  A successfully decoded scheme 0 still clears the preference. Validly typed but
  out-of-range accents clear the exported accent rather than retaining stale data.

Run `cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal`
from an app checkout. Run the three isolated wire tests separately under
`dbus-run-session -- cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal -- --ignored`.
They cover same-address D-Bus recovery with a cached ID, layout/activation, and partial
portal failures plus preference reset. Do not run the portal mock on your real session bus.

## Session-bus trust boundary

The exporter does not authenticate a physical panel click or restrict callers to
Plasma. Any session-bus peer admitted by the deployment's bus policy can request
currently exported actions. Event payloads do not supply arbitrary command
parameters: the ID resolves to the stored action, and the UI must still revalidate
its window/document, enabled state and modal guards before executing it. Those
checks protect application state; they are not sender authorization. Packaged
Flatpak incoming-call restrictions require separate acceptance testing and must
not be described as a verified panel-only boundary.
