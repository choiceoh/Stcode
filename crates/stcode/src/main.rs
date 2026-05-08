// Disable command line from opening on release mode
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    zed::bootstrap::run(zed::bootstrap::LaunchMode::Stcode);
}
