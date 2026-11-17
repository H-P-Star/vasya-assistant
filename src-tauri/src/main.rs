#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(all(windows, feature = "desktop"))]
    vasya::desktop::run();
    #[cfg(not(windows))]
    eprintln!("Vasya desktop requires Windows 10/11. Core tests can run on this platform.");
}
