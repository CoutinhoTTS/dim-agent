fn main() {
    if let Err(error) = dim_app_server::cli::run(std::env::args().skip(1)) {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
