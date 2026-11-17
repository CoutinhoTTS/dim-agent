"""The compiler a layout build must use, with Cargo's own precedence."""

import os


def rustc():
    return os.environ.get("RUSTC") or os.environ.get("CARGO_BUILD_RUSTC") or "rustc"
