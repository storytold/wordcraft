# Hosting WordCraft for the web

`wordcraft-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `wordcraft-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `wordcraft-web-<hash>.js` | wasm-bindgen glue (generated, ES module) |
| `wordcraft-web-<hash>_bg.wasm` | The app, about 13 MB, or 5 MB with compression |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## Any path works

All URLs in `index.html` are relative (`public_url = "./"` in `apps/wordcraft-web/Trunk.toml`),
so the site works at a domain root (`https://example.com/`), under a prefix
(`https://example.com/tools/wordcraft/`) and from a CDN bucket. The asset names carry a content
hash, so they can be cached forever. Only `index.html` needs revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm`. Browsers refuse to stream-compile it under
  any other type, and the app then loads slowly or not at all. Serve `.js` as `text/javascript`.
  Most hosts already do both. For nginx, check that `mime.types` has `application/wasm wasm;`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`. That takes the
  download from about 13 MB to about 5 MB. You can also precompress (`brotli -k *.wasm`) and let
  the server send `Content-Encoding: br`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed `.wasm` and
  `.js` files, and `no-cache` on `index.html`.
- **HTTPS:** WebGPU (and the clipboard) only work in a secure context, which means `https://`
  or `http://localhost`. Over plain HTTP elsewhere, the app falls back to WebGL2.
- **No special isolation headers:** WordCraft doesn't use `SharedArrayBuffer`, so it doesn't
  need `Cross-Origin-Opener-Policy` or `Cross-Origin-Embedder-Policy`. If your site already sends
  COEP `require-corp`, also send `Cross-Origin-Resource-Policy: same-origin` (or `cross-origin`
  when the files live on a CDN) on the app's files.

nginx example:

```nginx
location /wordcraft/ {
    types { application/wasm wasm; text/javascript js; text/html html; }
    gzip on;
    gzip_types application/wasm text/javascript text/html;
    location ~* \.(wasm|js)$ { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location ~* index\.html$ { add_header Cache-Control "no-cache"; }
}
```

Local test: `python3 -m http.server 8765` inside the folder, then open http://localhost:8765/.

## Embedding in a page (iframe)

```html
<iframe
  src="https://example.com/wordcraft/"
  title="WordCraft image editor"
  style="width: 100%; height: 720px; border: 0;"
  allow="fullscreen; clipboard-read; clipboard-write"
  allowfullscreen>
</iframe>
```

- The app fills the iframe and follows its size, so size the iframe and not the app.
- Keyboard shortcuts go to the iframe after the user clicks into it, as with any embedded app.
- **Cross-origin embeds** work. Preferences are kept in the iframe's `localStorage`. Browsers
  that partition or block third-party storage may forget them between visits, and the app
  then starts with defaults.
- **Sandboxed iframes** need at least
  `sandbox="allow-scripts allow-same-origin allow-downloads allow-popups"`. Without
  `allow-same-origin` there's no storage. Without `allow-downloads`, Save and Export (browser
  downloads) are blocked.
- Don't send `X-Frame-Options: DENY` or a `frame-ancestors` CSP that excludes the embedding page.

## The parent bridge (`?host=parent`)

A page framing WordCraft in a **same-origin** iframe can add `?host=parent` to the iframe's URL
(`https://example.com/wordcraft/?host=parent`) and have WordCraft hand its files over instead of
downloading them. The two sides talk `postMessage` with plain window messages that carry a `type`:

| Message | Direction | Fields |
|---|---|---|
| `wordcraft:ready` | app → page | `version`: the app is running and listening; send a file now |
| `wordcraft:open` | page → app | `name`, `bytes` (an `ArrayBuffer` or `Uint8Array`): open like File › Open |
| `wordcraft:save` | app → page | `name`, `saveAs`, `bytes` (a transferred `ArrayBuffer`): every file the app would otherwise download (Save, Save As, Export as PDF, Export Page as PNG). The page stores it and reports failures itself. `saveAs` says whether the file becomes the document's file: `false` after Save (the page's own file, written under its name) and for exports that are only copies (PDF, Markdown, plain text); `true` after Save As and for exports to a format that keeps everything (.docx, .odt, .rtf), after which the next Save carries that name with `false` |
| `wordcraft:dirty` | app → page | `dirty`: unsaved changes appeared or went away |
| `wordcraft:failed` | app → page | `error`: the app couldn't start (no WebGPU or WebGL2, for example) |

In host mode the app opens no sample document, so the framing page should send the file to edit
and guard leaving the page while changes are unsaved. `&author=` on the iframe's URL names the
user for the comments and tracked changes the app creates. Opening `?host=parent` outside an
iframe gives the normal app with downloads.

## Same-document host mode (no iframe)

A host application can also run WordCraft from its own document: it loads the wasm-bindgen module
itself and gives WordCraft one of its own canvases, so there is no iframe and no `?host=parent`.
It opts in with `data-host="page"` on that canvas — the exact value `page` — and the canvas keeps
the id the app looks up (`id="wordcraft_canvas"`).

```html
<canvas id="wordcraft_canvas" data-host="page" style="width: 100%; height: 720px; display: block;"></canvas>
```

The page and the app then talk with DOM `CustomEvent`s on the canvas instead of `postMessage`.
The app dispatches the same messages it would post to a parent — `wordcraft:ready`,
`wordcraft:save`, `wordcraft:dirty`, `wordcraft:failed` — with the message fields (without the
`type`) as the event's `detail`, and it opens a document when the page dispatches a
`wordcraft:open` event with `detail: { name, bytes }`. Events do not bubble, so listen on the
canvas itself (two editors on one page can't hear each other).

Add the listeners before `init`: the app announces itself with `wordcraft:ready` while `init` is
still running, and a file sent in reply would otherwise be missed.

```js
import init from "./wordcraft-web-<hash>.js"; // the wasm-bindgen glue shipped with the app

const canvas = document.getElementById("wordcraft_canvas");

canvas.addEventListener("wordcraft:ready", async () => {
  // The app is listening; hand it a document now, if you have one.
  const bytes = await documentBytes(); // an ArrayBuffer or Uint8Array
  canvas.dispatchEvent(new CustomEvent("wordcraft:open", {
    detail: { name: "Report.docx", bytes },
  }));
});

// Every file WordCraft would otherwise download (Save, Save As, Export as PDF) arrives here.
canvas.addEventListener("wordcraft:save", (event) => {
  const { name, saveAs, bytes } = event.detail; // bytes is an ArrayBuffer
  // Store the file under `name`; `saveAs` says whether it becomes the document's file.
  // Report your own errors.
  storeFile(name, bytes, saveAs);
});
canvas.addEventListener("wordcraft:dirty", (event) => {
  unsaved = event.detail.dirty; // warn before leaving, as you like
});
canvas.addEventListener("wordcraft:failed", (event) => {
  showError(event.detail.error); // the app couldn't start
});

await init({ module_or_path: "./wordcraft-web-<hash>_bg.wasm" });
```

- Saves are not downloads in this mode: the `bytes` in `wordcraft:save` hold the saved document,
  and the host decides where the file goes.
- `?author=` on the page's URL still names the user for new comments and tracked changes.
- An unframed page that uses `?host=parent` keeps the normal app with downloads; only the exact
  `data-host` value `page` turns this mode on.

## Renderer selection and fallback flags

WordCraft renders with wgpu. It uses **WebGPU** when the browser has it and falls back to
**WebGL2** on its own. URL query flags override this, and they work on the iframe `src` too:

| Flag | Effect |
|---|---|
| *(none)* | WebGPU if available, otherwise WebGL2 |
| `?webgl` | Force the WebGL2 backend (useful when a WebGPU driver misbehaves) |

For example: `<iframe src="https://example.com/wordcraft/?webgl" ...>`.

A browser with neither WebGPU nor WebGL2 gets a message in place of the app.
