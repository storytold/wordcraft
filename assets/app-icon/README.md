# WordCraft app icon

A **page and pen**: a sheet of paper with a folded corner and ruled lines, with a fountain pen
lying across its lower right, on the app's blue field. It is drawn in the canvas palette the app's
own artwork uses (`#3b5bdb` field, `#efe9dc` paper, `#0b0b0c` ink).

`wordcraft.svg` is the canonical master: a full-bleed 512-unit tile with a rounded square
(`rx=112`) and no border. Windows and Linux use the tile as it is; the macOS renders sit on
Apple's grid, an 824/1024 body with a transparent margin.

## Files

- `wordcraft.svg`: canonical master (also the hicolor scalable icon).
- `wordcraft-1024.png`: 1024 px full-bleed render.
- `wordcraft-macos-512.png`: the padded macOS render (Dock, window icon; embedded by the app).
- `wordcraft.icns`: macOS bundle icon (`CFBundleIconFile`), padded frames.
- `wordcraft.ico`: Windows icon, 16–256 px, embedded in `wordcraft.exe` by
  `apps/wordcraft/build.rs`.
- `hicolor/<size>/apps/ai.storyteller.wordcraft.png` (16–512) and `hicolor/scalable/...svg`:
  Linux icon theme.

## Regenerate

Replace `wordcraft.svg`, then run `packaging/icons.sh`. It needs `resvg`, plus `iconutil` on macOS;
`cargo run -p xtask -- ico` packs the Windows icon.

## Nightly channel

The Nightly icons keep the drawing, its alpha and the macOS padding exactly as the stable renders
have them. Only the background contribution changes: from the app blue to the deeper Nightly
background `#232c67`. There is no letter badge, crescent or other mark; the app and the Launcher
name the channel in text.

`packaging/nightly-icons.py` derives the four files from the committed stable renders (the `.ico`
frames, the `.icns` slots, the 256 px hicolor render and the padded 512 px macOS master) and
records their hashes in [`nightly-provenance.json`](nightly-provenance.json). `--check` re-derives
them and compares. The three smallest macOS slots (16, 32 and 64 px) are box-filtered from the
padded master; every other frame is the same size as the stable frame it comes from, so nothing is
upscaled. The stable files are not modified.
