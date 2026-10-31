fn main() {
    #[cfg(target_os = "linux")]
    unsafe {
        unsafe extern "C" {
            fn mallopt(param: i32, value: i32) -> i32;
        }
        mallopt(-3, 128 * 1024);
        mallopt(-8, 2);
    }
    if let Err(error) = dim_app_server::cli::run(std::env::args().skip(1)) {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
