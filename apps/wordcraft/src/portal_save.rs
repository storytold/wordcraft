//! Portal window parent identification for Linux/X11 (#6180, #7317).

#![allow(dead_code)]

/// The portal's parent window identifier for `window`: `x11:<xid>` on X11, empty elsewhere.
///
/// On Wayland, exporting an xdg-foreign handle requires `unsafe` code forbidden by workspace rules
/// (`#![forbid(unsafe_code)]`, Rust Rule 1), so Wayland dialogs remain unparented, matching
/// PhotoCraft and VectorCraft.
pub fn parent_id(window: Option<eframe::wgpu::rwh::RawWindowHandle>) -> String {
    use eframe::wgpu::rwh::RawWindowHandle;
    match window {
        Some(RawWindowHandle::Xlib(h)) => format!("x11:{:x}", h.window),
        Some(RawWindowHandle::Xcb(h)) => format!("x11:{:x}", h.window.get()),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_windows_are_named_to_the_portal() {
        use eframe::wgpu::rwh::{RawWindowHandle, XlibWindowHandle};
        assert_eq!(parent_id(Some(RawWindowHandle::Xlib(XlibWindowHandle::new(0x3a0000f)))), "x11:3a0000f");
        assert_eq!(parent_id(None), "");
    }
}
