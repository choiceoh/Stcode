// Disable command line from opening on release mode
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bootstrap;
mod reliability;
mod zed;

fn main() {
    bootstrap::run(bootstrap::LaunchMode::Stcode);
}
