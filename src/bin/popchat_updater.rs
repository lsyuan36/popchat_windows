//! Independent GUI helper: no console window and no shell-based installer.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--request")) {
        std::process::exit(2);
    }
    let Some(path) = args.next() else {
        std::process::exit(2);
    };
    if args.next().is_some() {
        std::process::exit(2);
    }
    if popchat_windows::updater::helper_main(std::path::Path::new(&path)).is_err() {
        // Bounded failure details are recorded next to the trusted request.
        // Do not create a second UI or launch anything after a failed update.
        std::process::exit(1);
    }
}
