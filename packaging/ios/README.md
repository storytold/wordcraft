# WordCraft for iPad

An iPadOS build of the WordCraft desktop app, made for an iPad with a hardware keyboard and
trackpad (Magic Keyboard). It is the same app as on the desktop, with the ribbon, styles,
tables, track changes and .docx files, drawn with Metal.

The IPA is **for sideloading**: it isn't signed for the App Store, and you install it with your
own Apple ID.

## Get the IPA

1. On GitHub, open **Actions › iPad IPA** in this repository (enable Actions for the fork first
   if GitHub asks).
2. Open the latest green run, or press **Run workflow** on the `ios-ipad` branch.
3. Download the **wordcraft-ipados** artifact and unzip it: inside is
   `wordcraft-<version>-ipados.ipa`.

To build it yourself on a Mac with Xcode and rustup:

```bash
packaging/ios/package.sh
```

The IPA lands in `dist/release/`. Set `CRAFT_FONTS_DIR` and `CRAFT_I18N_DIR` to checkouts of
storytold/craft-fonts and storytold/craft-i18n to embed the fonts and translations, as the
workflow does.

## Install it

Any sideloading tool that signs with your Apple ID works. With a free Apple ID the app runs for
7 days, then needs signing again (AltStore and SideStore do that in the background). A paid
developer account gives a year.

- **Sideloadly** (Windows or macOS): connect the iPad by USB, drop the IPA on Sideloadly, sign in
  with your Apple ID and press Start.
- **AltStore or SideStore**: open the IPA from AltStore's **My Apps › +**.

Then, on the iPad:

1. **Settings › Privacy & Security › Developer Mode**: on (iPadOS 16 and later asks for this
   once, then restarts).
2. **Settings › General › VPN & Device Management**: trust your Apple ID's developer certificate.

## Using it

- **Files.** Documents live in the app's Documents folder, which the Files app shows as
  **On My iPad › WordCraft**. Copy files there to open them in WordCraft; files you save there
  appear in Files. File › Open and Save As use WordCraft's own file browser and start in that
  folder.
- **Keyboard.** Typing, Return, Tab, Delete, the arrow keys (with Shift, Option and Command), Page
  Up/Down, and Command and Control shortcuts all work, and held keys repeat. The on-screen
  keyboard appears only when WordCraft asks for text input and no hardware keyboard is attached.
- **Trackpad.** The pointer hovers over controls, a click is a mouse click, a two-finger click
  opens the context menu, and two-finger scrolling scrolls, with momentum.
- **Touch** works too: tap to place the caret and press buttons.
- **Clipboard.** Copy and paste go through the iPad's clipboard: plain text, HTML, RTF and
  pictures, to and from other apps. iPadOS may ask you to allow pasting from another app.
- **Multitasking.** Split View, Slide Over and Stage Manager resize the window.

## Known limits

- No native file picker yet: files outside WordCraft's folder (iCloud Drive, other apps) have to
  be copied into **On My iPad › WordCraft** first, and "Open in WordCraft" from other apps isn't
  offered.
- Printing doesn't reach AirPrint. Export to PDF and print the PDF from Files.
- The pointer keeps its round iPadOS shape; it doesn't turn into an I-beam over text.
- Pinch to zoom isn't wired up; use the zoom slider or View › Zoom.
- Messages the desktop app shows in a system dialog box (rare; for example, a failed start) only
  go to the log on iPad.
- No self-update: install a newer IPA the same way.

## How the build differs from the desktop one

`prepare-sources.sh` fetches two dependencies and patches them for iPadOS (`patches/`):

- **winit** (storytold/winit at the workspace's pinned rev): hardware keyboard key events with
  modifiers and repeat (UIKit presses), trackpad hover, two-finger scroll with momentum, trackpad
  clicks as mouse buttons, and an empty input view so the on-screen keyboard shows only for text
  input.
- **rfd** 0.17.2: an iOS backend with no native dialogs, so the app uses its own file chooser.

The app itself uses UIPasteboard for the clipboard (`apps/wordcraft/src/ios.rs`, through
`crates/ios-pasteboard`), keeps its settings in `Library/Application Support/WordCraft` inside
the app's container, and leaves out the desktop-only parts (X11/Wayland, arboard, the fixed
window size, self-update).
