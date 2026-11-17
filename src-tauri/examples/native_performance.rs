fn main() {
    let result = hahakigami_lib::performance::native_main();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
