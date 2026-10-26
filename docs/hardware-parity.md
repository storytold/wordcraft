# Hardware parity with Microsoft Word

> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-26 · **Change:** minor (Android: the touch surface — page pan, long-press menu, tap, double-tap — and stamped keyboard chords, Tab S11-verified; previously trivial: Pen / stylus row: two-stroke and closed-head arrows; previously trivial: Pen / stylus row: ink, eraser and Ink to Shape incl. arrows landed; previously trivial: a window too big for its screen shrinks to fit when it opens, #1215; previously trivial: 32-bit Windows starts on OpenGL; previously major: first version) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4 and Word for Windows

> **Printing update:** 2026-10-12 · direct desktop printing through craft-print; details below.

Word is not a hardware-heavy app, but a few devices matter to its users: printers above all, then
pens, microphones (Dictate) and speakers (Read Aloud), and displays.

**Dimension: ~40% (estimated), 20–40 h to full** (pen/ink hours are counted with the Draw tab in
[`target-app-parity.md`](target-app-parity.md)).

| Hardware | Word | WordCraft macOS | Windows | Linux / BSD | Web | Parity | Hours |
|---|---|---|---|---|---|---|---|
| Printers: native print dialog, printer choice, copies, duplex, page ranges, tray, scaling | ✅ (`WordPDE.plugin` print-dialog extension on Mac) | 🟡 Direct CUPS printing; printer, copies, collation, duplex, ranges, grayscale and scale to paper size | 🟡 Direct spooler printing; printer, copies, collation, duplex, ranges, grayscale and scale to paper size | 🟡 Direct spooler printing; printer, copies, collation, duplex, ranges, grayscale and scale to paper size | ✅ browser print dialog (#209) | 35% | 5–8 |
| GPU rendering, HiDPI/Retina, pixel-aligned text | ✅ | ✅ wgpu + vello_cpu rasteriser, whole-pixel text (#110, #165) | ✅ DirectX 12 default (#50), OpenGL when DirectX 12 has no GPU driver (#316) or on 32-bit; startup crash on Intel UHD (#170) | ✅ (soft text on KDE at some zooms, #140) | ✅ | 80% | 3–5 (bugs) |
| Pen / stylus: ink, pressure, eraser, ink to shape/math | ✅ (Draw tab, `InkRender.bundle`) | 🟡 ink, stroke eraser, Ink to Shape incl. one- and two-stroke arrows with open or closed heads (#307, #395); pressure and Ink to Math missing | 🟡 same | 🟡 same | 🟡 same | 50% | (Draw tab, 15–25) |
| Multiple monitors, per-monitor DPI, remembered window position, a window that fits its screen | ✅ | ✅ (#38, #76, #1215) | ✅ (winit DPI fix pending, #155) | ✅ | — | 85% | 1 |
| Microphone: Dictate | ✅ (cloud speech service) | ❌ | ❌ | ❌ | ❌ | 0% | 8–15 + owner (speech model choice) |
| Speakers: Read Aloud | ✅ | ✅ system speech, speed, sentence skip (#190) | ✅ | 🟡 | 🟡 | 75% | 1–2 |
| Touch screens and touchpads: pinch zoom, inertial scroll | ✅ | ✅ pinch (#179) | ✅ | ✅ inertial scroll (#252) | ✅ | 85% | 0–1 |
| Scanners/cameras (Insert from device / Continuity Camera on Mac) | ✅ | ❌ | ❌ | ❌ | ❌ | 0% | 2–4 |

**On Android** (the arm64 build, verified on a Samsung Galaxy Tab S11, SM-X736B, Android 16): the
same interface runs on the touch screen — a finger pans the page and scrolls the browser's storage
list, a still hold opens the context menu, a tap places the caret and a double-tap selects a word;
the drag-growth paths stand down for touch, so a finger never drag-selects. Keyboard modifiers are
stamped onto each frame's key events, so chords reach egui (the on-device pass exercised typing;
no Bluetooth keyboard was attached), and the vendored winit fills Android's mouse path, so a mouse
attached to the tablet keeps hover, wheel and right button (exercised by host tests; no mouse was
attached on device). Options › General offers a 50% interface size for the tablet's
2560×1600 landscape screen.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | minor | Android (Tab S11): the touch surface — page pan, long-press menu, tap and double-tap — and stamped keyboard modifiers verified on device; no Bluetooth keyboard or mouse was attached |
| 2026-10-15 | trivial | Pen / stylus row: Ink to Shape also makes two-stroke arrows and arrows with a closed triangle head |
| 2026-10-17 | minor | File › Print's Scale to paper size (#2108): pick Letter, Legal, A4 or any Layout › Size paper and every page is scaled to it and centred, the paper turned to each page's orientation, in the preview, the print job (with the spooler's media) and Save these pages as PDF; the document keeps its page size. `paper` param on `ui.print`/`file.printPdf` |
| 2026-10-14 | trivial | Pen / stylus row corrected: ink, eraser and Ink to Shape (with arrows) work; pressure and Ink to Math missing |
| 2026-10-13 | trivial | A window too big for its screen shrinks to fit when it opens (#1215): eframe sized the first window to the whole monitor, frame and taskbar not counted, so on a laptop screen its bottom, with the end of the scroll bar, was past the screen edge. A window saved that size is fitted on its next launch too |
| 2026-10-12 | trivial | 32-bit Windows starts on OpenGL: DirectX 12 graphics pipeline creation returns "The parameter is incorrect" and the app exited before the window appeared |
| 2026-10-12 | minor | Desktop File > Print now discovers printers and submits PDF jobs through pinned craft-print with printer choice, copies, duplex and grayscale; browser printing and PDF export remain available. The upstream preview, page ranges, parity and Print Markup remain available. Remaining print parity includes driver-specific tray/options and pages-per-sheet UI; physical printer fidelity needs hardware validation. |
| 2026-10-10 | trivial | Windows falls back to OpenGL when DirectX 12 offers only the software adapter (#316) |
| 2026-10-10 | trivial | Desktop File › Print opens the PDF in the system viewer (#286) |
| 2026-10-10 | trivial | Inertial touchpad scrolling landed (#252) |
| 2026-10-10 | major | First version |
