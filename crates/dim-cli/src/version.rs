//! What this build reports as its version: the version the release flow set for
//! it, or the committed version marked `-source`.

/// The workspace version; the release flow sets it for the builds it produces.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether the release flow produced this build, which it marks with
/// `DIM_RELEASE_BUILD`.
pub const RELEASE: bool = option_env!("DIM_RELEASE_BUILD").is_some();

/// The marker of a build the release flow did not produce.
const SOURCE: &str = "-source";

/// `version` for a released build, the same version marked `SOURCE` otherwise.
pub fn label(version: &str, release: bool) -> String {
    if release {
        version.to_owned()
    } else {
        format!("{version}{SOURCE}")
    }
}

/// What this build reports.
pub fn current() -> String {
    label(VERSION, RELEASE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_released_build_reports_its_version_and_a_source_build_marks_it() {
        assert_eq!(label("0.1.0-alpha.8", true), "0.1.0-alpha.8");
        assert_eq!(label("0.1.0-alpha.8", false), "0.1.0-alpha.8-source");
    }
}
