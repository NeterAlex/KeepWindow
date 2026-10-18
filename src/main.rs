#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod calibration;
#[cfg(windows)]
mod config;
#[cfg(windows)]
mod events;
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod instance;
#[cfg(windows)]
mod locale;
#[cfg(windows)]
mod mouse;
#[cfg(windows)]
mod picker;
#[cfg(windows)]
mod protection;
#[cfg(windows)]
mod recovery;
#[cfg(windows)]
mod runtime;
#[cfg(windows)]
mod startup;
#[cfg(windows)]
mod storage;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod window;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("KeepWindow requires Windows 10 or later.");
}
