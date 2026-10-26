fn main() {
    if let Err(error) = rix_chrome::page_worker::run_stdio_worker() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
