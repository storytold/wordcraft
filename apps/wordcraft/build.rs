//! On Windows, embed the app icon and version info (VERSIONINFO) into `wordcraft.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab.
//!
//! A missing Windows resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `WORDCRAFT_REQUIRE_WINRES=1` turns it
//! into an error (for release builds). On Linux, set the AppImage launcher’s relative library path.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/wordcraft.ico");
    println!("cargo:rerun-if-env-changed=WORDCRAFT_REQUIRE_WINRES");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => {
            // Only AppRun's fallback hardlink has this sibling directory. DT_RPATH lets
            // the X11 bridge find its matching base library without changing the environment.
            println!("cargo:rustc-link-arg-bin=wordcraft=-Wl,--disable-new-dtags,-rpath,$ORIGIN/wordcraft-x11-libraries");
            return;
        }
        Ok("windows") => {}
        _ => return,
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
