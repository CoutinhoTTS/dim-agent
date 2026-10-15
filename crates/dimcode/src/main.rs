//! One executable for the product: the terminal client, which starts this
//! same file as its App Server daemon with the hidden `app-server`
//! subcommand.

fn main() {
    if std::env::args().nth(1).as_deref() == Some("app-server") {
        if let Err(error) = dim_app_server::cli::run(std::env::args().skip(2)) {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
        return;
    }
    std::process::exit(dim_cli::run_cli());
}
