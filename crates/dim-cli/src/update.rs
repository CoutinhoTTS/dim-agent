//! The update check: whether this build's npm channel has a newer version.
//! Alphas follow the `alpha` dist-tag, betas `beta` and releases `latest`,
//! so a channel only ever upgrades within itself, as the publish scripts
//! guarantee from the other side.

use std::time::Duration;

const PACKAGE: &str = "dimcode";
const REGISTRY: &str = "https://registry.npmjs.org";
const TIMEOUT: Duration = Duration::from_secs(4);

/// The dist-tag a version upgrades along: its prerelease identifier, or
/// `latest` for a release.
pub fn channel(version: &str) -> &str {
    match prerelease(version) {
        Some((identifier, _)) => identifier,
        None => "latest",
    }
}

/// `(identifier, number)` of `X.Y.Z-identifier.N`, `None` for a release.
fn prerelease(version: &str) -> Option<(&str, u64)> {
    let (_, tail) = version.split_once('-')?;
    let (identifier, number) = tail.split_once('.')?;
    Some((identifier, number.parse().ok()?))
}

/// `X.Y.Z` of a version; `None` for anything else.
fn release(version: &str) -> Option<[u64; 3]> {
    let core = version.split('-').next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let (Some(Some(a)), Some(Some(b)), Some(Some(c)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Some([a, b, c])
}

/// Whether `candidate` is a newer version of the same channel as `current`.
pub fn newer(current: &str, candidate: &str) -> bool {
    let (Some(now), Some(next)) = (release(current), release(candidate)) else {
        return false;
    };
    if next != now {
        return next > now;
    }
    match (prerelease(current), prerelease(candidate)) {
        (Some((a, n)), Some((b, m))) => a == b && m > n,
        _ => false,
    }
}

/// How to install `version` from this build's channel.
pub fn hint(current: &str, version: &str) -> String {
    let install = match channel(current) {
        "latest" => format!("npm i -g {PACKAGE}"),
        tag => format!("npm i -g {PACKAGE}@{tag}"),
    };
    format!("Update available: {PACKAGE} {version} (current {current}): {install}")
}

/// The hint for a newer version on the channel of `current`, or `None`
/// when there is none or the registry did not answer in time.
pub async fn available(current: &str) -> Option<String> {
    let client = reqwest::Client::builder().timeout(TIMEOUT).build().ok()?;
    let tags: serde_json::Value = client
        .get(format!("{REGISTRY}/-/package/{PACKAGE}/dist-tags"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    let version = tags.get(channel(current))?.as_str()?;
    newer(current, version).then(|| hint(current, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_only_upgrades_within_itself() {
        assert_eq!(channel("0.1.0-alpha.3"), "alpha");
        assert_eq!(channel("0.5.20-beta.0"), "beta");
        assert_eq!(channel("0.5.21"), "latest");
        assert!(newer("0.1.0-alpha.0", "0.1.0-alpha.1"));
        assert!(newer("0.1.0-alpha.9", "0.2.0-alpha.0"));
        assert!(!newer("0.1.0-alpha.1", "0.1.0-alpha.1"));
        assert!(!newer("0.1.0-alpha.1", "0.1.0-alpha.0"));
        // Another identifier or a release of the same core is not an upgrade.
        assert!(!newer("0.1.0-alpha.1", "0.1.0-beta.0"));
        assert!(!newer("0.1.0-alpha.1", "0.1.0"));
        assert!(newer("0.5.20", "0.5.21"));
        assert!(!newer("0.5.21", "0.5.21-beta.0"));
        assert_eq!(
            hint("0.1.0-alpha.0", "0.1.0-alpha.1"),
            "Update available: dimcode 0.1.0-alpha.1 (current 0.1.0-alpha.0): npm i -g dimcode@alpha"
        );
    }
}
