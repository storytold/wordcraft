//! Windows only: embed the app icon and version info (VERSIONINFO) into `wordcraft.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab, and delay-load the print-only
//! system DLLs so the app starts on Windows installations without them.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `WORDCRAFT_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/wordcraft.ico");
    println!("cargo:rerun-if-env-changed=WORDCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Printing (craft-print through winprint) draws the PDF with `PdfCreateRenderer` from
    // Windows.Data.Pdf.dll and sets the job's options through the Print Ticket API in prntvpt.dll.
    // Stripped Windows installations (NTLite and similar) lack both, and a load-time import stops
    // the exe from starting at all, so load them on first use instead. Only the print helper
    // process calls into them, and it checks that they load first (`print::isolated`).
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:windows.data.pdf.dll");
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:prntvpt.dll");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/wordcraft.ico")
        .set("ProductName", "WordCraft")
        .set("FileDescription", "WordCraft word processor")
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the WordCraft authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "wordcraft.exe")
        .set("InternalName", "wordcraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("WORDCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=wordcraft.exe built without icon/version resources: {e}");
    }
}
