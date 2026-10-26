# Vendored winit (Android input delta)

This is the `winit` that WordCraft's workspace already pinned:
`storytold/winit` rev `a517b3846ee8457d84ee4975110bfda6797c44dc` — the
egui-0.37-compatible fork eframe 0.37 builds against — unmodified except for
`src/platform_impl/android/mod.rs`, vendored because WordCraft's Android build
has no upstream route to the fix: the fork carries upstream's `//
TODO mouse events` in the Android backend (0.31 is still in beta), so a mouse
attached to the device (Bluetooth, USB or DeX) gets no hover, no scroll wheel
and no right or middle button; only touch works.

The workspace pins this copy with `[patch.crates-io]` in the root `Cargo.toml`;
nothing else in the tree may reference it directly.

## The delta (one file: `src/platform_impl/android/mod.rs`)

Upstream's Android backend translates every `MotionEvent` into a `Touch` event
and drops the rest, so a mouse attached to the device (Bluetooth, USB or DeX) has
no hover, no scroll wheel and no right or middle button; only touch works. The
patch adds one method, `EventLoop::dispatch_mouse_input`, and one call to it:

- pointers whose `Source` is `Mouse` or `Touchpad` are routed to the new method
  (touchscreen and touch-dragged stylus pointers keep the untouched `Touch` path);
- `Scroll` becomes `MouseWheel { delta: LineDelta(h, v) }` from the event's
  `Vscroll`/`Hscroll` axes;
- `ButtonPress`/`ButtonRelease` become `MouseInput` for the button in
  `action_button()` (right, middle, back, forward, and primary if a device sends
  it that way), preceded by a `CursorMoved`;
- a mouse's primary button arrives as an ordinary `Down`/`Up` (per Android's
  button-event rules), which become `MouseInput` for `MouseButton::Left` —
  while a non-primary bare `Down` is swallowed, so a right click does not also
  select as a left one;
- `Move`/`HoverMove` become `CursorMoved`, `HoverEnter`/`HoverExit` become
  `CursorEntered`/`CursorLeft`, so the cursor tracks the mouse without a press.

The delta (the routing, the method and its follow-up fix) is FilmCraft's, from
the same port series in `filmcraft`'s `third_party/winit` — the method differs
only in the base it documents.

Keyboard events are **not** touched here: upstream also never emits
`ModifiersChanged` on Android (it ignores the meta state Android carries on every
`KeyEvent`), but WordCraft fixes that on its own side, in `crates/ui-egui`, where
it is unit-testable — see `stamp_android_modifiers` there.

## Rebasing onto a newer winit

1. Take the new pinned revision as the base again (replace this directory,
   keeping this README), then drop the images it ships for its own docs and
   examples (`rm docs/res/*.svg examples/data/*.png`): they are never compiled
   or shipped, and WordCraft's `cargo xtask assets` gate attributes every asset
   in the repository file by file. Drop the checkout's `.git` and `.cargo-ok`
   too — this is a source snapshot, not a clone.
2. Re-apply the delta above; it is self-contained in the `MotionEvent` arm of
   `handle_input_event` plus the new method beside it. Search for
   `TODO mouse events` — as long as upstream still has it, the hunk applies in
   the same place.
3. If upstream has implemented Android mouse input, delete this directory, drop
   the `[patch.crates-io]` entry, and delete `stamp_android_modifiers` in
   `crates/ui-egui/src/lib.rs` only after checking that the new `winit` emits
   `ModifiersChanged` there too.

Licensed Apache-2.0, as upstream; `LICENSE` is upstream's, and every other file
is the pinned revision's own, untouched; the only source change is the one
file.
