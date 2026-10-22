//! Embeds the account runtime archive that `DIMCODE_EMBEDDED_RUNTIME` names
//! (from scripts/stage-runtime.mjs) and its SHA-256. A build without it embeds
//! none and supports API-key providers only.
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=DIMCODE_EMBEDDED_RUNTIME");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let archive = match std::env::var_os("DIMCODE_EMBEDDED_RUNTIME").filter(|path| !path.is_empty())
    {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            std::fs::read(&path).unwrap_or_else(|error| {
                panic!("DIMCODE_EMBEDDED_RUNTIME {}: {error}", path.display())
            })
        }
        None => Vec::new(),
    };
    let digest = if archive.is_empty() {
        String::new()
    } else {
        format!("{:x}", Sha256::digest(&archive))
    };
    std::fs::write(out.join("runtime.bin"), &archive).expect("write runtime.bin");
    std::fs::write(out.join("runtime.sha256"), digest).expect("write runtime.sha256");
}
