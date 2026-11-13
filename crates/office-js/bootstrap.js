// WordCraft's Office.js bootstrap: the one hand-written script in WordCraft (crates/office-js/src/lib.rs).
//
// An add-in's page calls `Office.onReady(...)`, sets `Office.initialize` or calls
// `Office.actions.associate(...)` while it loads, but the library is wasm, which WebView2 won't
// compile synchronously at this size. So this script, which runs before the page's own:
//
// - keeps Microsoft's office.js from running: a <script> whose source is a known office.js URL
//   (parser-inserted or added later) gets an empty script instead;
// - defines `Office`, `Word`, `OfficeExtension` and `OfficeRuntime` as properties nobody can
//   replace or delete: they answer stand-ins that hold early calls until the library (wasm) hands
//   over its objects (`__craftOfficeAdopt`), then the library's objects. A script that assigns its
//   own `Office` anyway has the assignment ignored (an `initialize` function on it is kept);
// - starts the library, then hands it what the page did meanwhile (`onReady` and `Word.run`
//   calls, `Office.initialize`, `Office.actions.associate`).
//
// The desktop app puts this, then wasm-bindgen's generated glue, then a call to
// `__craftOfficeStart(wasm_bindgen, <the wasm, base64>)` before the page's own scripts.
(function () {
  "use strict";
  var g = globalThis;
  if (g.__craftOfficeStart) return;
  function hidden(name, value) {
    Object.defineProperty(g, name, { value: value, writable: false, configurable: false, enumerable: false });
  }

  // ---------------------------------------------------------------- Microsoft's office.js

  // Microsoft's CDNs (any script there) and npm copies of @microsoft/office-js.
  var CDN = /^https?:\/\/(appsforoffice\.microsoft\.com|officeapis\.public\.onecdn\.static\.microsoft|[a-z0-9.-]+\.officeapps\.live\.com)\//i;
  var NPM = /\/@microsoft\/office-js\/.*\.js([?#].*)?$/i;
  var EMPTY = "data:text/javascript,%2F*%20WordCraft%3A%20Office.js%20is%20built%20in%20*%2F";
  function isOfficeJs(url) {
    var u = String(url || "");
    try { u = new URL(u, g.document ? g.document.baseURI : undefined).href; } catch (e) { /* keep it as written */ }
    return CDN.test(u) || NPM.test(u);
  }
  if (typeof document === "object" && typeof Element === "function" && typeof HTMLScriptElement === "function") {
    var setAttribute = Element.prototype.setAttribute;
    var neutralise = function (node) {
      if (node && node.tagName === "SCRIPT" && isOfficeJs(node.getAttribute("src"))) {
        setAttribute.call(node, "src", EMPTY);
      }
    };
    // Scripts made by a page's code: `script.src = …` / `setAttribute("src", …)`.
    var src = Object.getOwnPropertyDescriptor(HTMLScriptElement.prototype, "src");
    if (src && src.set && src.configurable) {
      Object.defineProperty(HTMLScriptElement.prototype, "src", {
        get: src.get,
        set: function (v) { src.set.call(this, isOfficeJs(v) ? EMPTY : v); },
        configurable: true,
        enumerable: src.enumerable
      });
    }
    Element.prototype.setAttribute = function (name, value) {
      if (this instanceof HTMLScriptElement && String(name).toLowerCase() === "src" && isOfficeJs(value)) {
        value = EMPTY;
      }
      return setAttribute.call(this, name, value);
    };
    // Scripts in the page's HTML: the parser inserts the element, and observers run before it
    // runs the script (at the end tag), so the source can still change.
    if (typeof MutationObserver === "function") {
      new MutationObserver(function (records) {
        records.forEach(function (r) {
          r.addedNodes.forEach(function (n) {
            neutralise(n);
            if (n.querySelectorAll) n.querySelectorAll("script[src]").forEach(neutralise);
          });
        });
      }).observe(document, { childList: true, subtree: true });
    }
  }

  // ---------------------------------------------------------------- stand-ins

  var real = {};
  var started = false;
  var waiting = [];
  var early = { initialize: undefined, actions: [] };
  function later(get) {
    return function () {
      var args = arguments;
      if (started) return get().apply(null, args);
      return new Promise(function (resolve, reject) {
        waiting.push(function () {
          try { resolve(get().apply(null, args)); } catch (e) { reject(e); }
        });
      });
    };
  }
  function fixed(o) {
    Object.keys(o).forEach(function (k) {
      Object.defineProperty(o, k, { value: o[k], writable: false, enumerable: true, configurable: false });
    });
    return o;
  }
  var office = fixed({
    onReady: later(function () { return real.Office.onReady; }),
    actions: fixed({
      associate: function (name, fn) {
        if (started) return real.Office.actions.associate(name, fn);
        early.actions.push([name, fn]);
      }
    })
  });
  Object.defineProperty(office, "initialize", {
    get: function () { return started ? real.Office.initialize : early.initialize; },
    set: function (f) { if (started) real.Office.initialize = f; else early.initialize = f; },
    enumerable: true,
    configurable: false
  });
  var standIn = {
    Office: office,
    Word: fixed({ run: later(function () { return real.Word.run; }) }),
    OfficeRuntime: {},
    OfficeExtension: {}
  };
  Object.keys(standIn).forEach(function (name) {
    Object.defineProperty(g, name, {
      get: function () { return real[name] || standIn[name]; },
      // Another `Office` (Microsoft's office.js, a page's own) doesn't replace WordCraft's; a
      // page's `initialize` on it still counts.
      set: function (v) {
        if (name === "Office" && v && v !== g.Office && typeof v.initialize === "function") g.Office.initialize = v.initialize;
      },
      enumerable: true,
      configurable: false
    });
  });
  // The library hands over its objects here, once each.
  hidden("__craftOfficeAdopt", function (name, value) {
    if (Object.prototype.hasOwnProperty.call(standIn, name) && !real[name]) real[name] = value;
  });

  // ---------------------------------------------------------------- the channel and the start

  // The library posts its requests here; the desktop web view's channel is `window.ipc`.
  if (!g.__craftOffice && g.ipc && g.ipc.postMessage) {
    var mac = typeof navigator === "object" && /Mac/.test(navigator.platform || "");
    var ipc = g.ipc;
    hidden("__craftOffice", Object.freeze({ post: function (s) { ipc.postMessage(s); }, platform: mac ? "Mac" : "PC" }));
  }
  hidden("__craftOfficeStart", function (bindgen, wasmBase64) {
    var bytes = Uint8Array.from(atob(wasmBase64), function (c) { return c.charCodeAt(0); });
    bindgen({ module_or_path: bytes }).then(function () {
      // The library's start function has defined the real objects by now.
      started = !!real.Office;
      if (!started) throw new Error("the library didn't define Office");
      if (typeof early.initialize === "function") real.Office.initialize = early.initialize;
      early.actions.splice(0).forEach(function (a) {
        try { real.Office.actions.associate(a[0], a[1]); } catch (e) { console.error(e); }
      });
      waiting.splice(0).forEach(function (call) { call(); });
    }).catch(function (e) {
      console.error("WordCraft Office.js failed to start", e);
    });
  });
})();
