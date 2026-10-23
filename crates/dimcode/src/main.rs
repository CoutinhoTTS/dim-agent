//! One executable for the product: the terminal client, which starts this
//! same file as its App Server daemon with the hidden `app-server`
//! subcommand.

// With the CLI and the App Server in one debug binary, `__eh_frame` passes
// the 16 MB that Apple's ld can reference from the compact unwind table. ld
// then reports that only panics unwind more slowly, which does not matter here.
#![allow(linker_messages)]

mod ui_leaf;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("app-server") {
        if let Err(error) = dim_app_server::cli::run(std::env::args().skip(2)) {
            eprintln!("Error: {error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("ui-leaf") {
        std::process::exit(ui_leaf::run(std::env::args().skip(2)));
    }
    if std::env::args().nth(1).as_deref() == Some("ui-tui") {
        std::process::exit(dim_cli::run_tui_child());
    }
    std::process::exit(dim_cli::run_cli());
}
