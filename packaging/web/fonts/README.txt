Server fonts: fonts every visitor gets, with nothing to install.

1. Put the font files in this folder (TTF, OTF, TTC, OTC, WOFF or WOFF2, at most 50 MB each).
   Subfolders work: list them as "my-fonts/Moul.woff2".
2. First setup only: copy manifest.sample.json to manifest.json, then list each font file in it
   (a file name, or {"file": "name"}). Releases never ship manifest.json, so updating the app
   won't overwrite yours.
3. Reload the page. A font appears in the font list like one added with Add Font, and documents
   that name its family use it.

manifest.sample.json lists files that aren't here: replace the names with your own. A listed
file that is missing is skipped (the browser console says so); delete this README or leave it,
it isn't read. The app reads only manifest.json: without it there are no server fonts.
Only use fonts whose licence lets you serve them. Full notes: HOSTING.md ("Server fonts").
