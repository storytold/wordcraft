# Hardware parity with Microsoft Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first version) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4 and Word for Windows

Word is not a hardware-heavy app, but a few devices matter to its users: printers above all, then
pens, microphones (Dictate) and speakers (Read Aloud), and displays.

**Dimension: ~40% (estimated), 20–40 h to full** (pen/ink hours are counted with the Draw tab in
[`target-app-parity.md`](target-app-parity.md)).

| Hardware | Word | WordCraft macOS | Windows | Linux / BSD | Web | Parity | Hours |
|---|---|---|---|---|---|---|---|
| Printers: native print dialog, printer choice, copies, duplex, page ranges, tray, scaling | ✅ (`WordPDE.plugin` print-dialog extension on Mac) | ❌ via PDF export | ❌ via PDF | ❌ via PDF | ✅ browser print dialog (#209) | 25% | 6–10 |
| GPU rendering, HiDPI/Retina, pixel-aligned text | ✅ | ✅ wgpu + vello_cpu rasteriser, whole-pixel text (#110, #165) | ✅ DirectX 12 default (#50); startup crash on Intel UHD (#170) | ✅ (soft text on KDE at some zooms, #140) | ✅ | 80% | 3–5 (bugs) |
| Multiple monitors, per-monitor DPI, remembered window position | ✅ | ✅ (#38, #76) | ✅ (winit DPI fix pending, #155) | ✅ | — | 85% | 1 |
| Pen / stylus: ink, pressure, eraser, ink to shape/math | ✅ (Draw tab, `InkRender.bundle`) | ❌ | ❌ | ❌ | ❌ | 0% | (Draw tab, 15–25) |
| Microphone: Dictate | ✅ (cloud speech service) | ❌ | ❌ | ❌ | ❌ | 0% | 8–15 + owner (speech model choice) |
| Speakers: Read Aloud | ✅ | ✅ system speech, speed, sentence skip (#190) | ✅ | 🟡 | 🟡 | 75% | 1–2 |
| Touch screens and touchpads: pinch zoom, inertial scroll | ✅ | ✅ pinch (#179) | ✅ | 🟡 inertial scroll pending (#252, #122) | ✅ | 70% | 1–2 |
| Scanners/cameras (Insert from device / Continuity Camera on Mac) | ✅ | ❌ | ❌ | ❌ | ❌ | 0% | 2–4 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First version |
