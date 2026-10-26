# Experimental Haiku frontend

The experimental frontend lives in [`apps/wordcraft-haiku`](../apps/wordcraft-haiku/README.md).
It uses the existing WordApp and document engine with a CPU renderer and Haiku's
in-process Wayland server. Its separate Cargo workspace keeps experimental
crate overrides out of the standard desktop application.

## Dependency work

The manifest pins public fork revisions while these changes are under review:

| Component | Change |
| --- | --- |
| Software renderer | [Invalidate cached texture content](https://github.com/DGriffin91/egui_software_backend/pull/23) |
| Software renderer | [egui 0.36 and Haiku host hooks](https://github.com/DGriffin91/egui_software_backend/pull/24) |
| File dialogs | [Native Haiku command bridge](https://github.com/PolyMeilex/rfd/pull/342) |
| Event loop | [Haiku notification pipes and poller](https://github.com/Smithay/calloop/pull/256) |
| Wayland toolkit | [Haiku close-on-exec pipes](https://github.com/Smithay/client-toolkit/pull/544) |
| Wayland connection | [In-process system backend](https://github.com/Smithay/wayland-rs/pull/979) |
| Window handling | [Haiku platform integration](https://github.com/rust-windowing/winit/pull/4762) |

The Wayland, calloop and toolkit release backports match the versions used by
Winit 0.30. The Wayland development branch has a different connection API, so its
PR revision cannot replace the release backport directly.

The initial prototype passed blank-document keyboard input, native Open, Save As,
Cancel, DOCX content checks and closing from the native window tab. The submitted
integration changes dependency revisions and rendering-cache behavior. It remains
experimental pending a complete build and repeat native application tests.

See the frontend README for the build command and the unresolved teardown,
clipboard, printing and settings limitations.
