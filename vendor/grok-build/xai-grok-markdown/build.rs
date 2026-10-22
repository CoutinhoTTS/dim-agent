// Modified by dim-agent: omit the extra Swift syntax asset pending license verification.
use std::env;
use std::path::Path;

use syntect::dumps::dump_to_uncompressed_file;

fn main() {
    // Keep the syntax set precompiled without the unverified extra Swift asset.
    let set = two_face::syntax::extra_newlines();
    dump_to_uncompressed_file(
        &set,
        Path::new(&env::var("OUT_DIR").unwrap()).join("syntaxes.bin"),
    )
    .expect("dump syntaxes");
}
