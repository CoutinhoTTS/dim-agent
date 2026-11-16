use super::{SkillsCatalog, SkillsError};
use crate::port_faces::{Empty, request, unknown_method};
use rutis_bridge::session::{HostDispatch, Reply, Value, arg};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    plugin: String,
    root: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plugin {
    plugin: String,
}

fn reply<T: Serialize>(result: Result<T, SkillsError>) -> Reply {
    match result {
        Ok(value) => arg(&value),
        Err(error) => Err(rutis_bridge::exports::failure(
            "skillsCatalog",
            error.code(),
            error.to_string(),
            None,
        )),
    }
}

impl HostDispatch for SkillsCatalog {
    fn methods(&self) -> Option<serde_json::Value> {
        Some(
            serde_json::json!({"register":"async","withdraw":"async","plugins":"async","issues":"async"}),
        )
    }

    fn invoke(&self, method: &str, args: Value) -> Reply {
        match method {
            "register" => {
                let Register { plugin, root } = request("skillsCatalog", args)?;
                let owner = self.clone();
                Ok(Value::future(async move {
                    reply(owner.register_plugin(&plugin, root.into()).await)
                }))
            }
            "withdraw" => {
                let Plugin { plugin } = request("skillsCatalog", args)?;
                let owner = self.clone();
                Ok(Value::future(async move {
                    reply(owner.withdraw_plugin(&plugin).await)
                }))
            }
            "plugins" => {
                let Empty {} = request("skillsCatalog", args)?;
                let owner = self.clone();
                Ok(Value::future(async move { reply(owner.plugins().await) }))
            }
            "issues" => {
                let Plugin { plugin } = request("skillsCatalog", args)?;
                let owner = self.clone();
                Ok(Value::future(async move {
                    reply(owner.plugin_issues(&plugin).await)
                }))
            }
            _ => Err(unknown_method("skillsCatalog", method)),
        }
    }
}
