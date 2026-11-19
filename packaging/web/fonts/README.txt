Server fonts: fonts every visitor gets, with nothing to install.

1. Put the font files in this folder (TTF, OTF, TTC, OTC, WOFF or WOFF2, at most 50 MB each).
   Subfolders work: list them as "my-fonts/Moul.woff2".
2. List each file in manifest.json (a file name, or {"file": "name"}).
3. Reload the page. A font appears in the font list like one added with Add Font, and documents
   that name its family use it.

The sample manifest lists files that aren't here: replace the names with your own. A listed
file that is missing is skipped (the browser console says so); delete this README or leave it,
it isn't read. No manifest.json at all means no server fonts. Only use fonts whose licence lets
you serve them. Full notes: HOSTING.md ("Server fonts").
