//! The editor where it can't run: Linux, the browser, and tests.
//!
//! Same shape as the real thing, so the front end needs no `cfg` of its own: opening one says
//! why it can't be done, and a picture that can't be edited is still a picture.

/// The app's window, as the host hands it over. Nothing here opens anything in it.
pub struct Window<'a>(std::marker::PhantomData<&'a ()>);

impl<'a> Window<'a> {
    pub fn new<T>(_window: &'a T) -> Self {
        Self(std::marker::PhantomData)
    }
}

/// What the editor has to say.
#[derive(Clone, Debug)]
pub enum Event {
    /// draw.io answered `load`: the diagram is on screen.
    Ready,
    /// The edited diagram, as a PNG with its XML inside it.
    Exported(Vec<u8>),
    /// draw.io's own Save or Exit was pressed: the edit is over.
    Closed,
    /// Something went wrong; the text is for the user.
    Failed(String),
}

/// An editor that never opens.
#[derive(Default)]
pub struct Editor;

impl Editor {
    /// Why there is no editor here.
    pub const WHY: &'static str = "editing a diagram in place needs a native window (macOS or Windows)";

    pub fn open(_window: &Window<'_>, _png: &[u8]) -> Result<Self, String> {
        Err(Self::WHY.into())
    }
    pub fn poll(&mut self) -> Vec<Event> {
        Vec::new()
    }
    pub fn save(&mut self) {}
    pub fn set_bounds(&self, _x: f64, _y: f64, _w: f64, _h: f64) {}
    pub fn set_visible(&self, _visible: bool) {}
    pub fn focus(&self) {}
}
