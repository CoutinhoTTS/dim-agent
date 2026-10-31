//! What this build reports as its version: the version the release flow set for
//! it, or the committed version marked `-source`.

/// The workspace version; the release flow sets it for the builds it produces.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether the release flow produced this build, which it marks with
/// `DIM_RELEASE_BUILD`. CI exports an unset variable as empty, so an empty
/// mark is no mark.
pub const RELEASE: bool = stamped(option_env!("DIM_RELEASE_BUILD"));

/// Whether `value` marks the build: an empty value is no mark.
const fn stamped(value: Option<&'static str>) -> bool {
    matches!(value, Some(value) if !value.is_empty())
}

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

    #[test]
    fn an_empty_release_mark_is_no_mark() {
        assert!(stamped(Some("1")));
        assert!(!stamped(Some("")));
        assert!(!stamped(None));
    }
}
