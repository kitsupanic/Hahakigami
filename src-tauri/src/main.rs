#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // The benchmark build is launched with `--config <path>`, which the command-line parser would reject.
    #[cfg(not(feature = "performance-benchmark"))]
    if let Some(code) = hahakigami_lib::cli::run() {
        std::process::exit(code);
    }
    hahakigami_lib::run();
}
