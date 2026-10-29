# Hardware parity with Microsoft Word

> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-26 · **Change:** trivial (Web Read Aloud marked partial until browser playback is verified; previously trivial: browser Read Aloud: a player that loses the speech pauses; previously trivial: browser Read Aloud review fixes; previously trivial: Read Aloud in the browser; previously trivial: Read Aloud voice list and stray letters; previously trivial: Read Aloud voice review fixes: the chosen voice and the language tag, MBROLA voices, the voice list; previously trivial: Read Aloud voice choice; previously trivial: Linux Read Aloud: a sentence a screen reader cuts off is said again; previously trivial: Linux Read Aloud review fixes: Speech Dispatcher address, language region, pausing when another program stops the sentence; previously trivial: Linux Read Aloud falls back to Speech Dispatcher when neither espeak-ng nor espeak is installed, and names the program to install; previously trivial: Pen / stylus row: two-stroke and closed-head arrows; previously trivial: Pen / stylus row: ink, eraser and Ink to Shape incl. arrows landed; previously trivial: a window too big for its screen shrinks to fit when it opens, #1215; previously trivial: 32-bit Windows starts on OpenGL; previously major: first version) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4 and Word for Windows

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
| Speakers: Read Aloud | ✅ | ✅ system speech, speed, sentence skip (#190), voice choice (Options › Accessibility) | ✅ | 🟡 espeak-ng, then espeak, then Speech Dispatcher (over its socket), each sentence in its language's voice; packages recommend a speech program; silent in the Flatpak (no audio access, no speech program) | 🟡 the browser's speech synthesis (Web Speech API), its voices in the voice list: implemented, not yet verified by playback in Chrome or Firefox | 80% | 1–2 |
| Touch screens and touchpads: pinch zoom, inertial scroll | ✅ | ✅ pinch (#179) | ✅ | ✅ inertial scroll (#252) | ✅ | 85% | 0–1 |
| Scanners/cameras (Insert from device / Continuity Camera on Mac) | ✅ | ❌ | ❌ | ❌ | ❌ | 0% | 2–4 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | trivial | Speakers: Read Aloud row: the Web cell is partial (🟡) and the row back at 80% until playback in Chrome and Firefox is shown; the browser path is implemented and covered by native tests only |
| 2026-10-26 | trivial | Browser Read Aloud: when another document starts reading, the one it cut off pauses on its sentence instead of staying in Playing |
| 2026-10-26 | trivial | Read Aloud voice: a sentence counts as Latin when less than a fifth of its letters are in another script (a µ, a ligature or a Greek letter keeps the chosen voice), a voice without a named language reads Latin text, opening the voice list may start Speech Dispatcher once to list its voices, and a good list is never replaced by an empty one |
| 2026-10-26 | trivial | Linux Read Aloud: a sentence another program's message cancels (a screen reader's announcement) is said again after a short pause, twice at most; cancelled again, paused by another program, or never begun, it pauses reading with a message |
| 2026-10-21 | trivial | Browser Read Aloud review fixes: a browser without speech synthesis says so instead of failing; a sentence cancelled from outside the player is said again (twice at most, then reading pauses), one the browser can't speak in its language or voice is said in its default voice, and one the browser never finishes stops reading with a message; only the reading player stops the browser's speech; the voice list follows the browser's `voiceschanged` |
| 2026-10-21 | trivial | Read Aloud voice review fixes: the chosen voice reads every sentence of Latin letters whatever its language tag (Word and new documents tag all text `en-US`), while text in another script, Cyrillic, Devanagari or Georgian among them, keeps its language's voice; MBROLA voices are listed when `mbrola` and their database are installed; the voice list is asked for again when it was empty or is a minute old, never starts Speech Dispatcher, and a listing program that hangs is stopped |
| 2026-10-21 | trivial | Linux Read Aloud review fixes: Speech Dispatcher is found at `SPEECHD_ADDRESS` (`unix_socket`, `inet_socket`) or the default socket, gets the sentence's language and region, and a sentence another program cancels or pauses (a screen reader's message, a stop for every program) pauses reading on it with a message instead of being skipped or said over the other program |
| 2026-10-18 | trivial | Read Aloud in the browser: the Web Speech API speaks each sentence in its language at the reading speed, the browser's voices join the voice list, and pause, skip, stop and a new speed cancel and restart the sentence; row 80% → 85% (still missing: the word being read highlighted) |
| 2026-10-18 | trivial | Read Aloud voice choice (`readAloud.voice`, Options › Accessibility › Read Aloud › Voice), saved with the settings: the chosen voice reads sentences in its language and those without one, others keep their language's voice; row 75% → 80% (still missing: the word being read highlighted, browser speech) |
| 2026-10-18 | trivial | Linux Read Aloud: Speech Dispatcher (over its socket, with the sentence's language; stopping cancels only WordCraft's own speech) when neither espeak-ng nor espeak is installed, a message naming the program to install when none is, and a speech program recommended by the `.deb` (`espeak-ng` or `speech-dispatcher`) and `.rpm` (`espeak-ng`) packages |
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
