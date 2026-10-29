# Craft KDE accessor on the current Storytold winit fork

Source: https://github.com/storytold/winit
Pinned commit: `a517b3846ee8457d84ee4975110bfda6797c44dc`.
Package version: 0.30.13. Code license: Apache-2.0; upstream LICENSE and
attribution notices are retained. This is third-party source, not Craft-authored
windowing code. Unlike the previous crates.io snapshot, this base retains the
clipboard and IME APIs used by the current pinned Storytold egui fork.

The only local Rust source change adds the safe `WindowExtWayland::wayland_surface`
accessor in `src/platform/wayland.rs`. It clones the existing WlSurface proxy;
there is no raw-pointer reconstruction, unsafe code addition, new connection,
or replacement of upstream clipboard/input handling. The application must retain
the native window until the auxiliary association is dropped. It must not destroy
or repurpose the borrowed surface or replace its event handling.

Packaging: omit the four unused `docs/res/keyboard_*.svg` diagrams, which are
CC-BY-SA-4.0 derivatives of Tomiĉo's illustration modified by John Nunley, not
Apache-2.0 assets. Keep `docs/res/ATTRIBUTION.md` as provenance. Rust documentation
references their upstream URLs; these local diagrams are not needed to build.
The Git metadata, lockfile, and build outputs are not part of this snapshot.

This snapshot is being validated for the 2026-10-29 KDE reconciliation worktrees.
It does not imply successful application CI or multi-window runtime acceptance.
