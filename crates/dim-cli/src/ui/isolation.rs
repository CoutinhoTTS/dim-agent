//! The node's `host-isolation.json`: whether model commands run in a sandbox.
//!
//! A missing file means isolation is on (the `workspace` tier). Turning it off
//! writes `"sandbox": "host"` on version 2, or `"enabled": false` on version 1,
//! and leaves every other field. Turning it on removes that field and deletes
//! the file when only `version` remains. An invalid file is left untouched.

use std::path::Path;

use serde_json::{Map, Value};

/// Whether the next model command is isolated. A missing file is isolated.
pub fn enabled(path: &Path) -> Result<bool, String> {
    let Some(object) = read(path)? else {
        return Ok(true);
    };
    Ok(isolated(&object))
}

/// `on` restores the default tier by removing the off switch. `Err` leaves
/// the file unchanged.
pub fn set_enabled(path: &Path, on: bool) -> Result<(), String> {
    let Some(mut object) = read(path)? else {
        if on {
            return Ok(());
        }
        return write(path, &serde_json::json!({"version": 2, "sandbox": "host"}));
    };
    if object.get("version").and_then(Value::as_u64) == Some(1) {
        if on {
            object.remove("enabled");
        } else {
            object.insert("enabled".into(), Value::Bool(false));
        }
    } else if on {
        object.remove("sandbox");
    } else {
        object.insert("sandbox".into(), Value::String("host".into()));
    }
    if object.len() == 1 && object.contains_key("version") {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    write(path, &Value::Object(object))
}

fn isolated(object: &Map<String, Value>) -> bool {
    if object.get("version").and_then(Value::as_u64) == Some(1) {
        object.get("enabled") != Some(&Value::Bool(false))
    } else {
        object.get("sandbox").and_then(Value::as_str) != Some("host")
    }
}

fn read(path: &Path) -> Result<Option<Map<String, Value>>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(parse(&text)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn parse(text: &str) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        format!(
            "Invalid host-isolation.json at line {}, column {}",
            error.line(),
            error.column()
        )
    })?;
    let object = match value {
        Value::Object(object) => object,
        _ => return Err("host-isolation.json must be a JSON object".into()),
    };
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .filter(|version| *version == 1 || *version == 2)
        .ok_or("host-isolation.json version must be 1 or 2")?;
    let allowed: &[&str] = if version == 1 {
        &[
            "version",
            "enabled",
            "network",
            "readOnlyPaths",
            "writablePaths",
            "environment",
        ]
    } else {
        &[
            "version",
            "sandbox",
            "escalation",
            "readOnlyPaths",
            "writablePaths",
            "environment",
        ]
    };
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("host-isolation.json has an unknown field: {key}"));
    }
    if version == 1 {
        expect_bool(&object, "enabled")?;
        expect_bool(&object, "network")?;
    } else {
        expect_one_of(&object, "sandbox", &["workspace", "network", "host"])?;
        expect_one_of(&object, "escalation", &["deny", "ask"])?;
    }
    expect_strings(&object, "readOnlyPaths")?;
    expect_strings(&object, "writablePaths")?;
    expect_strings(&object, "environment")?;
    Ok(object)
}

fn expect_bool(object: &Map<String, Value>, key: &str) -> Result<(), String> {
    match object.get(key) {
        None | Some(Value::Bool(_)) => Ok(()),
        Some(_) => Err(format!("host-isolation.json {key} must be a boolean")),
    }
}

fn expect_one_of(object: &Map<String, Value>, key: &str, allowed: &[&str]) -> Result<(), String> {
    if !object.contains_key(key) {
        return Ok(());
    }
    match object.get(key).and_then(Value::as_str) {
        Some(value) if allowed.contains(&value) => Ok(()),
        _ => Err(format!(
            "host-isolation.json {key} must be one of {}",
            allowed.join(", ")
        )),
    }
}

fn expect_strings(object: &Map<String, Value>, key: &str) -> Result<(), String> {
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    match value.as_array() {
        Some(items) if items.iter().all(Value::is_string) => Ok(()),
        _ => Err(format!(
            "host-isolation.json {key} must be an array of strings"
        )),
    }
}

fn write(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())? + "\n";
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        && !parent.exists()
    {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent).map_err(|error| error.to_string())?;
    }
    let mut tmp_name = path
        .file_name()
        .ok_or_else(|| format!("Isolation policy has no file name: {}", path.display()))?
        .to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let write_tmp = (|| -> Result<(), String> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&tmp).map_err(|error| error.to_string())?;
        std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())
    })();
    if let Err(error) = write_tmp {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        error.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "dim-isolation-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn value(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_missing_file_is_on_and_turning_it_off_then_on_removes_it() {
        let scratch = Scratch::new();
        let path = scratch.path("host-isolation.json");
        assert!(enabled(&path).unwrap());
        set_enabled(&path, true).unwrap();
        assert!(!path.exists());
        set_enabled(&path, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 2,\n  \"sandbox\": \"host\"\n}\n"
        );
        assert!(!enabled(&path).unwrap());
        set_enabled(&path, true).unwrap();
        assert!(!path.exists());
        assert!(enabled(&path).unwrap());
    }

    #[test]
    fn other_fields_survive_turning_isolation_off_and_on() {
        let scratch = Scratch::new();
        let path = scratch.path("host-isolation.json");
        std::fs::write(
            &path,
            "{\n  \"version\": 2,\n  \"escalation\": \"ask\",\n  \"readOnlyPaths\": [\"/usr\"]\n}\n",
        )
        .unwrap();
        set_enabled(&path, false).unwrap();
        assert_eq!(
            value(&path),
            json!({
                "version": 2,
                "escalation": "ask",
                "readOnlyPaths": ["/usr"],
                "sandbox": "host"
            })
        );
        set_enabled(&path, true).unwrap();
        assert_eq!(
            value(&path),
            json!({"version": 2, "escalation": "ask", "readOnlyPaths": ["/usr"]})
        );
        assert!(enabled(&path).unwrap());
    }

    #[test]
    fn an_explicit_network_tier_returns_to_the_default_when_turned_back_on() {
        let scratch = Scratch::new();
        let path = scratch.path("host-isolation.json");
        std::fs::write(&path, "{\"version\":2,\"sandbox\":\"network\"}\n").unwrap();
        assert!(enabled(&path).unwrap());
        set_enabled(&path, false).unwrap();
        assert!(!enabled(&path).unwrap());
        set_enabled(&path, true).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_version_1_file_uses_enabled_and_keeps_network() {
        let scratch = Scratch::new();
        let path = scratch.path("host-isolation.json");
        std::fs::write(
            &path,
            "{\"version\":1,\"enabled\":false,\"network\":true}\n",
        )
        .unwrap();
        assert!(!enabled(&path).unwrap());
        set_enabled(&path, true).unwrap();
        assert_eq!(value(&path), json!({"version": 1, "network": true}));
        assert!(enabled(&path).unwrap());
        set_enabled(&path, false).unwrap();
        assert_eq!(
            value(&path),
            json!({"version": 1, "network": true, "enabled": false})
        );
    }

    #[test]
    fn an_invalid_file_is_not_rewritten() {
        let scratch = Scratch::new();
        let path = scratch.path("host-isolation.json");
        let text = "{ not json\n";
        std::fs::write(&path, text).unwrap();
        let error = set_enabled(&path, false).unwrap_err();
        assert!(error.contains("Invalid host-isolation.json"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(enabled(&path).is_err());

        std::fs::write(&path, "{\"version\":2,\"sandbox\":\"host\",\"extra\":1}\n").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(
            set_enabled(&path, true)
                .unwrap_err()
                .contains("unknown field")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_and_an_existing_parent_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        let parent = scratch.path("home");
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = parent.join("host-isolation.json");
        set_enabled(&path, false).unwrap();
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(file_mode & 0o077, 0, "policy file mode {file_mode:o}");
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o755
        );

        let created = scratch.path("rust").join("host-isolation.json");
        set_enabled(&created, false).unwrap();
        let dir_mode = std::fs::metadata(created.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o077, 0, "created home mode {dir_mode:o}");
    }
}
