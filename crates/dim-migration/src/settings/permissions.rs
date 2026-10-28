use super::{fill, record};
use crate::{Migration, Outcome, Result, TaskState, source::fingerprint};
use dim_agent_types::{COMMAND_SEPARATORS, CommandMatch, CommandRule, CommandRuleDecision};
use dim_client::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Decision {
    Allow,
    Deny,
    Ask,
}

struct Pattern {
    id: String,
    words: Vec<String>,
    prefix: bool,
    decision: Decision,
    simple: bool,
}
impl Pattern {
    fn covers(&self, other: &Self) -> bool {
        if self.prefix {
            other.words.starts_with(&self.words)
        } else {
            !other.prefix && self.words == other.words
        }
    }
    fn score(&self) -> usize {
        self.words
            .iter()
            .filter(|word| word.as_str() != "*")
            .map(|word| word.encode_utf16().count() + 2)
            .sum()
    }
}

pub(super) async fn import(
    owner: &Migration,
    task: &TaskState,
    client: &mut Client,
    document: Value,
) -> Result<()> {
    let unsupported = |id: String, reason: &str| {
        record(owner, task, id, Outcome::Unsupported, Some(reason.into()))
    };
    if document
        .get("version")
        .is_some_and(|version| version != &json!(1))
        || document
            .as_object()
            .expect("source document object")
            .keys()
            .any(|key| !matches!(key.as_str(), "version" | "execCommandPermissions"))
    {
        return unsupported(
            "permissions.json".into(),
            "Legacy permissions schema is unsupported",
        )
        .await;
    }
    let Some(config) = document.get("execCommandPermissions") else {
        return Ok(());
    };
    if !config.as_object().is_some_and(|config| {
        config
            .keys()
            .all(|key| matches!(key.as_str(), "rules" | "defaultAction"))
    }) {
        return unsupported(
            "permissions.json".into(),
            "Legacy command permission fields are invalid",
        )
        .await;
    }
    if let Some(default) = config.get("defaultAction") {
        if serde_json::from_value::<Decision>(default.clone()).is_err() {
            return unsupported(
                "permissions.json".into(),
                "Legacy default command decision is invalid",
            )
            .await;
        }
        unsupported(
            "permission.defaultAction".into(),
            "Legacy default approvals do not map to a native permission preset",
        )
        .await?;
    }
    let Some(raw) = config.get("rules") else {
        return unsupported(
            "permissions.json".into(),
            "Legacy command rules are missing",
        )
        .await;
    };
    let Ok(rules) = serde_json::from_value::<BTreeMap<String, Decision>>(raw.clone()) else {
        return unsupported(
            "permissions.json".into(),
            "Legacy command rules are invalid",
        )
        .await;
    };
    if rules.len() > 1024 {
        return unsupported("permissions.json".into(),"Legacy command rules exceed the 1024-rule conversion limit; no rules were truncated or applied").await;
    }
    let mut normalized = BTreeSet::new();
    let mut patterns = Vec::new();
    for (raw, decision) in rules {
        let mut words = raw
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if !normalized.insert(words.join(" ")) {
            return unsupported(
                "permissions.json".into(),
                "Legacy command rules contain colliding normalized patterns",
            )
            .await;
        }
        let prefix = words.last().is_some_and(|word| word == "*");
        if prefix {
            words.pop();
        }
        patterns.push(Pattern {
            id: format!("permission.rule:{}", fingerprint(&raw)?),
            words,
            prefix,
            decision,
            simple: !raw.contains(COMMAND_SEPARATORS),
        });
    }
    let mut blocked = patterns
        .iter()
        .enumerate()
        .filter(|(_, pattern)| {
            !pattern.words.is_empty() && (!pattern.simple || pattern.decision == Decision::Ask)
        })
        .map(|(index, _)| index)
        .collect::<BTreeSet<_>>();
    for (index, a) in patterns.iter().enumerate() {
        for (other, b) in patterns.iter().enumerate().skip(index + 1) {
            if a.words.is_empty()
                || b.words.is_empty()
                || a.decision == b.decision
                || !(a.covers(b) || b.covers(a))
            {
                continue;
            }
            let winner = match a.score().cmp(&b.score()) {
                std::cmp::Ordering::Greater => Some(a.decision),
                std::cmp::Ordering::Less => Some(b.decision),
                std::cmp::Ordering::Equal if a.words == b.words && a.prefix != b.prefix => {
                    Some(if a.prefix { b.decision } else { a.decision })
                }
                std::cmp::Ordering::Equal => None,
            };
            let equivalent = if a.decision == Decision::Ask || b.decision == Decision::Ask {
                winner.is_some_and(|winner| winner != Decision::Ask)
            } else {
                winner == Some(Decision::Deny)
            };
            if !equivalent {
                blocked.extend([index, other]);
            }
        }
    }
    loop {
        let previous = blocked.len();
        for (index, pattern) in patterns.iter().enumerate() {
            if pattern.words.is_empty() || blocked.contains(&index) {
                continue;
            }
            if blocked
                .iter()
                .any(|other| pattern.covers(&patterns[*other]) || patterns[*other].covers(pattern))
            {
                blocked.insert(index);
            }
        }
        if blocked.len() == previous {
            break;
        }
    }
    let empty = patterns.is_empty();
    let mut rules = Vec::new();
    for (index, pattern) in patterns.into_iter().enumerate() {
        if pattern.words.is_empty()
            || !pattern.simple
            || pattern.decision == Decision::Ask
            || blocked.contains(&index)
        {
            unsupported(pattern.id,"Legacy command rule or its precedence has no equivalent native rule; no approximation is applied").await?;
            continue;
        }
        rules.push(CommandRule {
            command: pattern.words.join(" "),
            matching: if pattern.prefix {
                CommandMatch::Prefix
            } else {
                CommandMatch::Exact
            },
            decision: match pattern.decision {
                Decision::Allow => CommandRuleDecision::Allow,
                Decision::Deny => CommandRuleDecision::Deny,
                Decision::Ask => unreachable!(),
            },
        });
    }
    if empty || !rules.is_empty() {
        fill(
            owner,
            task,
            client,
            "permission",
            json!({"commandRules":rules}),
        )
        .await?;
    }
    Ok(())
}
