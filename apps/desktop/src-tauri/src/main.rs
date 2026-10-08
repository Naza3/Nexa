#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(any(windows, test))]
mod close_request;
#[cfg(any(windows, test))]
mod diagnostics;
#[cfg(any(windows, test))]
mod directory_selection;
#[cfg(windows)]
mod download_component;
#[cfg(any(windows, test))]
mod file_selection;
#[cfg(any(windows, test))]
mod layout;
#[cfg(any(windows, test))]
mod local_path;
#[cfg(any(windows, test))]
mod selection;
#[cfg(windows)]
mod windows;

fn main() {
    #[cfg(windows)]
    windows::run();
    #[cfg(not(windows))]
    {
        eprintln!("Nexa desktop currently supports Windows x64 only");
        std::process::exit(1);
    }
}
