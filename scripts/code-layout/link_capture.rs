use std::env;
use std::process::{exit, Command};

fn main() {
    let python = env::var_os("DIM_LAYOUT_PYTHON").expect("DIM_LAYOUT_PYTHON");
    let script = env::var_os("DIM_LAYOUT_SCRIPT").expect("DIM_LAYOUT_SCRIPT");
    let status = Command::new(python)
        .arg(script)
        .arg("--capture")
        .args(env::args_os().skip(1))
        .status()
        .expect("run the layout link capture");
    exit(status.code().unwrap_or(1));
}
