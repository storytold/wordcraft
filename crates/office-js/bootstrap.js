// WordCraft's Office.js bootstrap: the one hand-written script in WordCraft (crates/office-js/src/lib.rs).
//
// An add-in's page calls `Office.onReady(...)` (or sets `Office.initialize`) while it loads, but
// the library is wasm, which WebView2 won't compile synchronously at this size. So this defines
// stand-ins for `Office` and `Word` that hold early calls, starts the library, and hands the
// calls over once the real globals (defined by the library, not replaceable) are in place.
// The desktop app puts this, then wasm-bindgen's generated glue, then a call to
// `__craftOfficeStart(wasm_bindgen, <the wasm, base64>)` before the page's own scripts.
(function () {
  "use strict";
  if (globalThis.__craftOfficeStart) return;
  var waiting = [];
  function later(get) {
    return function () {
      var args = arguments;
      return new Promise(function (resolve, reject) {
        waiting.push(function () {
          try { resolve(get().apply(null, args)); } catch (e) { reject(e); }
        });
      });
    };
  }
  var standIn = {
    Office: { onReady: later(function () { return globalThis.Office.onReady; }) },
    Word: { run: later(function () { return globalThis.Word.run; }) },
    OfficeExtension: {}
  };
  Object.keys(standIn).forEach(function (name) {
    Object.defineProperty(globalThis, name, { value: standIn[name], writable: true, configurable: true });
  });
  // The library posts its requests here; the desktop web view's channel is `window.ipc`.
  if (!globalThis.__craftOffice && globalThis.ipc && globalThis.ipc.postMessage) {
    var mac = /Mac/.test(navigator.platform || "");
    globalThis.__craftOffice = { post: function (s) { globalThis.ipc.postMessage(s); }, platform: mac ? "Mac" : "PC" };
  }
  globalThis.__craftOfficeStart = function (bindgen, wasmBase64) {
    var bytes = Uint8Array.from(atob(wasmBase64), function (c) { return c.charCodeAt(0); });
    bindgen({ module_or_path: bytes }).then(function () {
      // The library's start function has defined the real globals by now.
      if (typeof standIn.Office.initialize === "function") {
        globalThis.Office.initialize = standIn.Office.initialize;
      }
      waiting.splice(0).forEach(function (call) { call(); });
    }, function (e) {
      console.error("WordCraft Office.js failed to start", e);
    });
  };
})();
