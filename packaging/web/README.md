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

## Opening and saving through your page (`?host`)

With `?host` on the iframe `src`, the embedding page opens and saves documents through
`postMessage`, so they can live on your server instead of the visitor's disk. WordCraft accepts
messages only from its parent window at the host's origin: its own origin by default, or the one
given as `?host=https://example.com`. In this mode Save sends the `.docx` back to your page and
never downloads it.

`?accent=RRGGBB` (for example `?host&accent=008080`) gives the editor your site's colour for its
buttons, tabs and selection.

From your page:

| Message | What it does |
|---|---|
| `{wordcraft: "open", id?, name, data}` | Open a document; `data` is an `ArrayBuffer` or `Uint8Array` |
| `{wordcraft: "run", id?, command, params?}` | Run any command, such as `file.setAuthor`, `review.trackChanges`, `review.restrict` or `review.compare` |
| `{wordcraft: "save", id?, name?}` | Send the document back as `.docx` |

From WordCraft:

| Message | When |
|---|---|
| `{wordcraft: "ready"}` | The editor is up and listening |
| `{wordcraft: "result", id, ok, result \| error}` | After `open` and `run`, and after a `save` that failed |
| `{wordcraft: "saved", id, name, data}` | The `.docx` bytes, after `save` or when the person chose Save (`id` is then `null`) |
| `{wordcraft: "dirty", value}` | The document gained or lost unsaved changes |

```js
const frame = document.querySelector('iframe'); // src="/wordcraft/?host&accent=008080"
const send = (msg) => frame.contentWindow.postMessage(msg, location.origin);
addEventListener('message', async (e) => {
  if (e.source !== frame.contentWindow) return;
  const m = e.data;
  if (m.wordcraft === 'ready') {
    const data = await (await fetch('/papers/42.docx')).arrayBuffer();
    send({ wordcraft: 'open', id: 1, name: 'paper.docx', data });
    send({ wordcraft: 'run', id: 2, command: 'file.setAuthor', params: { name: 'Ada Lovelace' } });
    send({ wordcraft: 'run', id: 3, command: 'review.trackChanges', params: { value: true } });
  }
  if (m.wordcraft === 'saved') await fetch('/papers/42.docx', { method: 'PUT', body: m.data });
});
```

Messages are handled in order, so a `run` sent right after `open` applies to the opened document.
To show what changed between two versions, open the older one and run `review.compare` with the
newer one's bytes as base64 `data`.

## Renderer selection and fallback flags

WordCraft renders with wgpu. It uses **WebGPU** when the browser has it and falls back to
**WebGL2** on its own. URL query flags override this, and they work on the iframe `src` too:

| Flag | Effect |
|---|---|
| *(none)* | WebGPU if available, otherwise WebGL2 |
| `?webgl` | Force the WebGL2 backend (useful when a WebGPU driver misbehaves) |
| `?cpu` | Force the CPU canvas path (slowest, most compatible) |

For example: `<iframe src="https://example.com/wordcraft/?webgl" ...>`.

A browser with neither WebGPU nor WebGL2 gets a message in place of the app.
