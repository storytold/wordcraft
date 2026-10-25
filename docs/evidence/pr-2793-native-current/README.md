# Native evidence after the egui 0.37 merge

This session qualifies `ee28fe88afc381b41362cc7780854db989c2d89e`, which merges upstream `7ef2633a`. It uses the current CI font snapshot `b076417b5880bd4154bd22f00695b1d28c6bea83`. It does not qualify later upstream code. [Source/runtime binding](source-and-runtime.json), [focused checks and lint](focused-validation.json), [native window](screenshots/native-window.png).

The actual WordCraft executable ran on Debian 12 aarch64, Xvfb/Openbox, Vulkan llvmpipe, at 1 pixel per UI point and 100% document zoom. XTest delivered native clicks, drags and Ctrl+Z through winit. RPC supplied setup, inspection, saving and export.

| Case | Entered editing | Rotated | After native Undo | Numeric and input records |
|---|---|---|---|---|
| body-ordinary | [editing](screenshots/body-ordinary/entered.png) | [rotation](screenshots/body-ordinary/rotated.png) | [Undo](screenshots/body-ordinary/undo.png) | [states](cases/body-ordinary.json), [events](logs/body-ordinary.jsonl) |
| body-upright | [editing](screenshots/body-upright/entered.png) | [rotation](screenshots/body-upright/rotated.png) | [Undo](screenshots/body-upright/undo.png) | [states](cases/body-upright.json), [events](logs/body-upright.jsonl) |
| header-ordinary | [editing](screenshots/header-ordinary/entered.png) | [rotation](screenshots/header-ordinary/rotated.png) | [Undo](screenshots/header-ordinary/undo.png) | [states](cases/header-ordinary.json), [events](logs/header-ordinary.jsonl) |
| header-upright | [editing](screenshots/header-upright/entered.png) | [rotation](screenshots/header-upright/rotated.png) | [Undo](screenshots/header-upright/undo.png) | [states](cases/header-upright.json), [events](logs/header-upright.jsonl) |
| footer-ordinary | [editing](screenshots/footer-ordinary/entered.png) | [rotation](screenshots/footer-ordinary/rotated.png) | [Undo](screenshots/footer-ordinary/undo.png) | [states](cases/footer-ordinary.json), [events](logs/footer-ordinary.jsonl) |
| footer-upright | [editing](screenshots/footer-upright/entered.png) | [rotation](screenshots/footer-upright/rotated.png) | [Undo](screenshots/footer-upright/undo.png) | [states](cases/footer-upright.json), [events](logs/footer-upright.jsonl) |

Each case entered and left text editing, then moved, resized and rotated from 30° to 50°. Ordinary boxes used straight editing handles; upright boxes kept their turned frame. Native Ctrl+Z restored each full host paragraph after every gesture. Final body blocks and part text matched their initial states. Saved DOCX files retained upright flags. Each case includes numeric before/after/Undo states, selection, paragraph hashes and ordered native events. [Fixtures](../pr-2793-native/fixtures/) are byte-identical to the earlier synthetic fixtures.

The [current exported PDF](pdf/turned-link.pdf) opened in native Evince. An actual click on its displayed turned text dispatched the expected URI. A distinct handler control succeeded; an off-link click produced no activation and left viewer geometry unchanged. [Activation receipt](pdf/activation.json), [input log](logs/pdf-click.jsonl). Runtime networking was disabled. The URI recorder logs dispatch, not an HTTP response. No viewer UI image is included.

The startup update check remained enabled. Its offline DNS failure did not block the covered interactions. `pdfinfo` emitted `Suspects object is wrong type (boolean)`; this diagnostic is disclosed without source attribution. [Setup corrections and limits](notes.json).

This is Linux 1x software-rendered evidence for synthetic Latin root boxes only. Groups, cells, independent nested/linked boxes, complex scripts, selected-only gestures and broader pointer paths remain outside it. Atomic row-group splitting is not implemented. No Word pixel-parity or other-platform claim is made.

The [older bundle](../pr-2793-native/README.md) remains unchanged and qualifies only `40e60513` with the old font snapshot. Its captures are not evidence for this egui 0.37 session.
