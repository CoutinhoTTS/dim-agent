//! Dim's identity on its model requests: the User-Agent, `X-Title` and
//! `HTTP-Referer` every request carries unless its provider names its own.
//!
//! The headers go in the provider layer of [`crate::driver`], so the library's
//! own User-Agent parts follow them, and a provider's per-call header still
//! overrides them.
use aimux_provider_utils::HeaderMapOpt;
use std::{collections::HashMap, sync::RwLock};

/// The title dim sends with its requests.
const X_TITLE: &str = "DimCode";
/// The attribution URL dim sends with its requests.
const REFERER: &str = "https://dimagent.com/";
/// The variable that replaces the built-in User-Agent for one process.
const ENVIRONMENT_VARIABLE: &str = "DIM_USER_AGENT";
/// The longest override [`checked_user_agent`] accepts.
const MAX_USER_AGENT: usize = 512;

/// `dim/<version> DimAgent/<version>`: `dim` first, so a gateway that reads
/// the first token names dim, then the older relay identity.
fn default_user_agent() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!("dim/{version} DimAgent/{version}")
}

/// `value` when it is an override a request can carry: 1 to 512 printable
/// ASCII characters.
fn checked_user_agent(value: &str) -> Option<String> {
    ((1..=MAX_USER_AGENT).contains(&value.len())
        && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)))
    .then(|| value.to_owned())
}

/// The overrides in effect, highest scope first.
#[derive(Default)]
struct Overrides {
    /// [`ENVIRONMENT_VARIABLE`], read by the process that hosts models.
    environment: Option<String>,
    /// The `provider` settings source's `userAgent`.
    configured: Option<String>,
}

/// Process state, as a model connection carries no owner that could hold it:
/// the driver builds a model per call and the App Server holds the setting.
static OVERRIDES: RwLock<Overrides> = RwLock::new(Overrides {
    environment: None,
    configured: None,
});

/// The User-Agent every model request carries: the environment's override,
/// then the configured one, then [`default_user_agent`].
pub fn user_agent() -> String {
    let overrides = OVERRIDES.read().unwrap_or_else(|error| error.into_inner());
    overrides
        .environment
        .clone()
        .or_else(|| overrides.configured.clone())
        .unwrap_or_else(default_user_agent)
}

/// The three headers dim adds at the provider layer.
pub fn headers() -> HeaderMapOpt {
    HashMap::from([
        ("user-agent".into(), Some(user_agent())),
        ("x-title".into(), Some(X_TITLE.into())),
        ("http-referer".into(), Some(REFERER.into())),
    ])
}

/// Publishes [`ENVIRONMENT_VARIABLE`], which wins over a configured value.
pub fn set_environment_user_agent(user_agent: Option<String>) {
    let mut overrides = OVERRIDES.write().unwrap_or_else(|error| error.into_inner());
    overrides.environment = user_agent;
}

/// Publishes the configured override: the `provider` settings source's
/// `userAgent`.
pub fn set_configured_user_agent(user_agent: Option<String>) {
    let mut overrides = OVERRIDES.write().unwrap_or_else(|error| error.into_inner());
    overrides.configured = user_agent;
}

/// [`ENVIRONMENT_VARIABLE`]'s value, as where the model host mounts reads it.
/// `None` when it is unset; a value a request cannot carry is refused in the
/// words the startup inputs use.
fn environment_user_agent(variable: Option<String>) -> Result<Option<String>, &'static str> {
    let Some(value) = variable else {
        return Ok(None);
    };
    checked_user_agent(&value)
        .map(Some)
        .ok_or("DIM_USER_AGENT must be 1 to 512 printable ASCII characters")
}

/// Reads [`ENVIRONMENT_VARIABLE`] where the model host mounts and publishes
/// it when it is set. A value a request cannot carry fails that mount, which
/// fails the startup of the process that hosts models.
///
/// The environment does not change under a running process, so a row that
/// remounts reads the same value. An unset variable publishes nothing: a host
/// that sets the identity in code keeps what it set.
pub fn install_environment() -> Result<(), &'static str> {
    let variable = match std::env::var(ENVIRONMENT_VARIABLE) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("DIM_USER_AGENT must be valid Unicode");
        }
    };
    if let Some(user_agent) = environment_user_agent(variable)? {
        set_environment_user_agent(Some(user_agent));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_override_is_refused_when_a_request_cannot_carry_it() {
        assert_eq!(environment_user_agent(None), Ok(None));
        assert_eq!(
            environment_user_agent(Some("private-client/7".into())).unwrap(),
            Some("private-client/7".into())
        );
        // A space is printable ASCII, so a value may carry one; a control
        // character, a byte outside ASCII and an empty value may not.
        for refused in ["", "dim\u{e9}", "a\nb", "a\u{7f}b"] {
            let error = environment_user_agent(Some(refused.into())).unwrap_err();
            assert!(
                error.contains("DIM_USER_AGENT") && !error.contains(refused) || refused.is_empty(),
                "{refused:?}: the refusal names no value: {error}"
            );
        }
    }

    #[test]
    fn an_override_is_one_to_512_printable_ascii_characters() {
        assert_eq!(checked_user_agent("dim/1.0").as_deref(), Some("dim/1.0"));
        assert!(
            checked_user_agent(&"a".repeat(MAX_USER_AGENT)).is_some(),
            "the longest accepted value is accepted"
        );
        // A space is printable ASCII and a value may carry one; an empty
        // value, a control character and a byte outside ASCII may not.
        for refused in ["", "a\nb", "a\u{7f}b", "dím/1.0"] {
            assert_eq!(checked_user_agent(refused), None, "{refused:?}");
        }
        assert_eq!(
            checked_user_agent(&"a".repeat(MAX_USER_AGENT + 1)),
            None,
            "a value one over the limit is refused, not trimmed"
        );
    }
}
