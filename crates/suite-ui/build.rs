//! macOS: compile the suite's app menu (`data/app-menu.ui`, with "Update Automatically",
//! ADR 0012) into a GResource bundle that `chrome::macos_app_menu` registers over GTK's own.
//! Needs GLib's `glib-compile-resources` (Homebrew's glib, which gtk4 brings), or
//! `GLIB_COMPILE_RESOURCES` naming it. Nothing elsewhere.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=data");
    println!("cargo::rerun-if-env-changed=GLIB_COMPILE_RESOURCES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("app-menu.gresource");
    let tool = std::env::var_os("GLIB_COMPILE_RESOURCES")
        .unwrap_or_else(|| "glib-compile-resources".into());
    let status = Command::new(&tool)
        .arg("--sourcedir=data")
        .arg(format!("--target={}", out.display()))
        .arg("data/app-menu.gresource.xml")
        .status()
        .unwrap_or_else(|e| panic!("{}: {e} (brew install glib)", tool.to_string_lossy()));
    assert!(status.success(), "glib-compile-resources failed: {status}");
}
