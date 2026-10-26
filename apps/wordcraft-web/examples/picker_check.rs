//! Browser regression harness for the real picker module. Build for wasm32, run through
//! wasm-bindgen --target web, then call `pick_files` from browser automation. See the
//! dismissal/selection checks in docs/document-tabs-validation.md.

#[cfg(target_arch = "wasm32")]
const MAX_PICKED_FILES: usize = 2000;
#[cfg(target_arch = "wasm32")]
#[path = "../src/web/file_picker.rs"]
mod file_picker;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn pick_files(many: bool) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue> {
    let selection = file_picker::pick(&["txt"], many)?;
    let files = selection.files().await;
    let names = js_sys::Array::new();
    for file in files {
        names.push(&wasm_bindgen::JsValue::from_str(&file.name()));
    }
    Ok(names.into())
}

/// Self-check normal cancellation, silent-dismissal fallback and DOM cleanup.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn check_dismissal() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast as _;
    let window = web_sys::window().ok_or_else(|| wasm_bindgen::JsValue::from_str("no window"))?;
    let document = window.document().ok_or_else(|| wasm_bindgen::JsValue::from_str("no document"))?;
    for kind in ["change", "cancel", "pointerdown", "keydown"] {
        let selection = file_picker::pick(&["txt"], false)?;
        let input = document.query_selector("input[type=file]")?.ok_or_else(|| wasm_bindgen::JsValue::from_str("missing picker input"))?;
        let target: &web_sys::EventTarget = if matches!(kind, "change" | "cancel") { input.unchecked_ref() } else { window.as_ref() };
        target.dispatch_event(&web_sys::Event::new(kind)?)?;
        if !selection.files().await.is_empty() || input.is_connected() {
            return Err(wasm_bindgen::JsValue::from_str("dismissal must return no files and remove its input"));
        }
    }
    Ok(())
}

fn main() {}
