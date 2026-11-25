#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // The benchmark build is launched with `--config <path>`, which the command-line parser would reject.
    // The Lite edition has no subcommands: launching it with arguments starts the app.
    #[cfg(all(feature = "automation", not(feature = "performance-benchmark")))]
    if let Some(code) = hahakigami_lib::cli::run() {
        std::process::exit(code);
    }
    hahakigami_lib::run();
}
