#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if let Some(code) = hahakigami_lib::cli::run() {
        std::process::exit(code);
    }
    hahakigami_lib::run();
}
