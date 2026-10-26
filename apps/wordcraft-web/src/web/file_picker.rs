//! Request-scoped browser input, including dismissal without a `cancel` event.

use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};

pub(super) struct Selection {
    input: web_sys::HtmlInputElement,
    ready: js_sys::Promise,
    on_event: Closure<dyn FnMut(web_sys::Event)>,
    listeners: Vec<(web_sys::EventTarget, &'static str)>,
}

impl Selection {
    pub(super) async fn files(self) -> Vec<web_sys::File> {
        let _ = wasm_bindgen_futures::JsFuture::from(self.ready.clone()).await;
        self.input
            .files()
            .map(|files| (0..files.length().min(super::MAX_PICKED_FILES as u32)).filter_map(|i| files.get(i)).collect())
            .unwrap_or_default()
    }
}

impl Drop for Selection {
    fn drop(&mut self) {
        for (target, event) in &self.listeners {
            let _ = target.remove_event_listener_with_callback_and_bool(event, self.on_event.as_ref().unchecked_ref(), true);
        }
        self.input.remove();
    }
}

/// Page interaction proves the native dialog has closed, without a timeout that can
/// mistake a slow selection for cancellation. Read selected files before removing input.
pub(super) fn pick(extensions: &[&str], many: bool) -> Result<Selection, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window.document().ok_or_else(|| JsValue::from_str("no document"))?;
    let body = document.body().ok_or_else(|| JsValue::from_str("no document body"))?;
    let input = document.create_element("input")?.dyn_into::<web_sys::HtmlInputElement>()?;
    input.set_type("file");
    let activated = window.navigator().user_activation().is_active();
    input.set_hidden(activated);
    if !activated {
        input.style().set_css_text("position:fixed;top:1rem;left:1rem;z-index:10000");
    }
    input.set_multiple(many);
    input.set_accept(&extensions.iter().map(|ext| format!(".{ext}")).collect::<Vec<_>>().join(","));
    let mut on_event = None;
    let ready = js_sys::Promise::new(&mut |resolve, _reject| {
        let input = input.clone();
        on_event = Some(Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if event.target().as_ref() == Some(input.as_ref()) {
                // Keep native keyboard activation/navigation, but let Escape dismiss it.
                if event.type_() == "pointerdown"
                    || (event.type_() == "keydown" && event.dyn_ref::<web_sys::KeyboardEvent>().is_none_or(|key| key.key() != "Escape"))
                {
                    return;
                }
                if event.type_() == "keydown" {
                    input.set_value("");
                }
            }
            let _ = resolve.call0(&JsValue::UNDEFINED);
        }));
    });
    let on_event = on_event.ok_or_else(|| JsValue::from_str("couldn't initialise file picker"))?;
    let mut selection = Selection { input, ready, on_event, listeners: Vec::new() };
    for (target, event) in [
        (selection.input.clone().unchecked_into::<web_sys::EventTarget>(), "change"),
        (selection.input.clone().unchecked_into(), "cancel"),
        (window.clone().unchecked_into(), "pointerdown"),
        (window.unchecked_into(), "keydown"),
    ] {
        target.add_event_listener_with_callback_and_bool(event, selection.on_event.as_ref().unchecked_ref(), true)?;
        selection.listeners.push((target, event));
    }
    body.append_child(&selection.input)?;
    if activated {
        selection.input.click();
    } else {
        // Keep a usable native control when automation has no transient activation.
        let _ = selection.input.focus();
    }
    Ok(selection)
}
