//! Shared title-bar policy and hit-test geometry.
//!
//! The policy is common to every platform, while the actual controls remain native on Windows
//! and macOS and custom on frameless Linux windows.


use crate::WordApp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    Native,
    Integrated,
    Frameless,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Config {
    pub mode: Mode,
    pub height: f32,
    pub left_reserved: f32,
    pub right_reserved: f32,
    pub custom_controls: bool,
}


pub(crate) const BACK_WIDTH: f32 = 52.0;

pub(crate) fn config(app: &WordApp) -> Config {
    if app.frameless {
        Config {
            mode: Mode::Frameless,
            height: crate::caption_buttons::BAND_HEIGHT,
            left_reserved: BACK_WIDTH,
            right_reserved: crate::caption_buttons::BUTTONS_WIDTH,
            custom_controls: true,
        }
    } else if app.integrated_titlebar {
        Config {
            mode: Mode::Integrated,
            height: app.mac_titlebar.height,
            left_reserved: app.mac_titlebar.content_left,
            right_reserved: 0.0,
            custom_controls: false,
        }
    } else {
        Config { mode: Mode::Native, height: 0.0, left_reserved: 0.0, right_reserved: 0.0, custom_controls: false }
    }
}

