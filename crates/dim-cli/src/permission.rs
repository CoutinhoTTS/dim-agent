//! Permission controls over existing App Server methods: the startup preset,
//! `/approvals`, and the rule an "always" approval answer proposes. The App
//! Server decides and enforces every call; the CLI only selects and writes.
use crate::{
    Result,
    client::{self, Client},
    display::Display,
    input::Input,
};
use dim_agent_types::{
    ApprovalCallData, COMMAND_SEPARATORS, CommandMatch, CommandRule, CommandRuleDecision,
    PermissionPolicy, PermissionPreset,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::future::Future;

#[cfg(unix)]
pub type Interrupts = tokio::signal::unix::Signal;
#[cfg(windows)]
pub type Interrupts = tokio::signal::windows::CtrlC;

const PRESETS: [PermissionPreset; 3] = [
    PermissionPreset::ReadOnly,
    PermissionPreset::WorkspaceWrite,
    PermissionPreset::FullAccess,
];
pub const PRESET_METHODS: [&str; 2] = ["session.permission.get", "session.permission.update"];
pub const RULE_METHODS: [&str; 2] = ["permission.rules", "permission.set_rules"];
/// Attempts for a write whose outcome does not depend on what was read.
pub const ATTEMPTS: usize = 3;
const USAGE: &str = "Usage: /approvals [PRESET | allow|deny exact|prefix COMMAND | remove N]";

pub fn parse_preset(value: &str) -> Result<PermissionPreset> {
    PRESETS
        .into_iter()
        .find(|preset| name(*preset) == value)
        .ok_or_else(|| {
            "Unknown permission preset; use read-only, workspace-write or full-access".into()
        })
}

pub fn name(preset: PermissionPreset) -> &'static str {
    match preset {
        PermissionPreset::ReadOnly => "read-only",
        PermissionPreset::WorkspaceWrite => "workspace-write",
        PermissionPreset::FullAccess => "full-access",
    }
}

/// What Permission's preset step decides, by risk category.
fn summary(preset: PermissionPreset) -> &'static str {
    match preset {
        PermissionPreset::ReadOnly => {
            "reads run; writes, commands, network and unclassified tools are denied"
        }
        PermissionPreset::WorkspaceWrite => {
            "reads and file writes run; commands, network and unclassified tools ask"
        }
        PermissionPreset::FullAccess => "every tool call runs without asking",
    }
}

/// The dispatched policy of `main` and the version a change must expect.
#[derive(Deserialize)]
pub struct State {
    pub version: u64,
    pub policy: PermissionPolicy,
}

pub async fn read(client: &mut Client, session_id: &str) -> Result<State> {
    client
        .typed(
            "session.permission.get",
            json!({"sessionId":session_id,"branchId":"main"}),
        )
        .await
}

/// Replaces only the preset; the tool lists that were read are kept.
async fn update(
    client: &mut Client,
    session_id: &str,
    state: &State,
    preset: PermissionPreset,
) -> Result<u64> {
    #[derive(Deserialize)]
    struct Updated {
        version: u64,
    }
    let policy = PermissionPolicy {
        preset,
        ..state.policy.clone()
    };
    let updated: Updated = client
        .typed(
            "session.permission.update",
            json!({"sessionId":session_id,"branchId":"main","expectedVersion":state.version,"policy":policy}),
        )
        .await?;
    Ok(updated.version)
}

/// `--permission` for a resumed Session. The requested preset does not depend
/// on the read, so a version conflict reads again and retries.
pub async fn select(client: &mut Client, session_id: &str, preset: PermissionPreset) -> Result<()> {
    if !client.supports(&PRESET_METHODS) {
        return Err(
            "This daemon cannot change a Session's permission preset; omit --permission".into(),
        );
    }
    for _ in 0..ATTEMPTS {
        let state = read(client, session_id).await?;
        if state.policy.preset == preset {
            return Ok(());
        }
        match update(client, session_id, &state, preset).await {
            Err(error) if client::app_error(&error) == Some("permission_conflict") => continue,
            result => return result.map(drop),
        }
    }
    Err("The Session's permission preset kept changing; run the command again".into())
}

/// The user's command rules and the Settings revision a write must expect.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    pub revision: String,
    pub command_rules: Vec<CommandRule>,
}

async fn rules(client: &mut Client) -> Result<Rules> {
    client.typed("permission.rules", json!({})).await
}

pub fn set_rules_params(revision: &str, rules: &[CommandRule]) -> Value {
    json!({"expectedRevision":revision,"commandRules":rules})
}

async fn set_rules(client: &mut Client, revision: &str, rules: &[CommandRule]) -> Result<String> {
    #[derive(Deserialize)]
    struct Revision {
        revision: String,
    }
    let written: Revision = client
        .typed("permission.set_rules", set_rules_params(revision, rules))
        .await?;
    Ok(written.revision)
}

pub fn describe(rule: &CommandRule) -> String {
    format!(
        "{} {} `{}`",
        decision_name(rule.decision),
        match_name(rule.matching),
        rule.command
    )
}

fn decision_name(decision: CommandRuleDecision) -> &'static str {
    match decision {
        CommandRuleDecision::Allow => "allow",
        CommandRuleDecision::Deny => "deny",
    }
}

fn match_name(matching: CommandMatch) -> &'static str {
    match matching {
        CommandMatch::Exact => "exact",
        CommandMatch::Prefix => "prefix",
    }
}

/// The list with `rule` appended, or `None` when an equal rule exists.
/// Rules compare word by word, as Permission matches them.
pub fn with_rule(rules: &[CommandRule], rule: &CommandRule) -> Option<Vec<CommandRule>> {
    let present = rules.iter().any(|existing| {
        existing.decision == rule.decision
            && existing.matching == rule.matching
            && existing
                .command
                .split_whitespace()
                .eq(rule.command.split_whitespace())
    });
    (!present).then(|| {
        let mut rules = rules.to_vec();
        rules.push(rule.clone());
        rules
    })
}

/// The command an "always" answer may turn into an allow rule: a simple
/// command of `exec` or `execute`, the tools F2 names. The request carries no
/// risk category; a rule for another tool would never match.
pub fn always_command(call: &ApprovalCallData) -> Option<&str> {
    if !matches!(call.name.as_str(), "exec" | "execute") {
        return None;
    }
    let Some(Value::String(command)) = call.input.get("command") else {
        return None;
    };
    (!command.contains(COMMAND_SEPARATORS) && command.split_whitespace().next().is_some())
        .then_some(command.as_str())
}

/// The allow rule for the words typed at the rule prompt: none keeps the
/// exact command, its leading words make a prefix rule.
pub fn proposed(command: &str, typed: &str) -> Option<CommandRule> {
    let words: Vec<_> = command.split_whitespace().collect();
    let chosen: Vec<_> = typed.split_whitespace().collect();
    let (matching, chosen) = if chosen.is_empty() || chosen == words {
        (CommandMatch::Exact, words)
    } else if words.starts_with(&chosen) {
        (CommandMatch::Prefix, chosen)
    } else {
        return None;
    };
    Some(CommandRule {
        command: chosen.join(" "),
        matching,
        decision: CommandRuleDecision::Allow,
    })
}

/// The rule list the user last saw; `remove N` refers to it.
pub struct Listing {
    revision: String,
    rules: Vec<CommandRule>,
}

/// Waits for an RPC; interrupts are reported and consumed until it settles.
async fn settle<T>(interrupts: &mut Interrupts, operation: impl Future<Output = T>) -> T {
    tokio::pin!(operation);
    loop {
        tokio::select! {
            biased;
            _ = interrupts.recv() => {
                println!("\nWaiting for the permission result before accepting more input.");
            }
            result = &mut operation => break result,
        }
    }
}

fn print_state(display: &mut Display, state: &State) -> Result<()> {
    let current = state.policy.preset;
    display.record(&format!("Preset (main branch): {}", name(current)))?;
    for preset in PRESETS {
        display.record(&format!(
            "{} {:<16} {}",
            if preset == current { "*" } else { " " },
            name(preset),
            summary(preset)
        ))?;
    }
    for (label, tools) in [
        ("allowed", &state.policy.allowed_tools),
        ("disallowed", &state.policy.disallowed_tools),
    ] {
        if !tools.is_empty() {
            display.record(&format!("  {label} tools: {}", tools.join(", ")))?;
        }
    }
    Ok(())
}

fn print_rules(display: &mut Display, rules: &[CommandRule]) -> Result<()> {
    if rules.is_empty() {
        return display.record("Command rules: none");
    }
    display.record("Command rules (user scope, every Session):")?;
    for (index, rule) in rules.iter().enumerate() {
        display.record(&format!(
            "  {}  {:<5}  {:<6}  {}",
            index + 1,
            decision_name(rule.decision),
            match_name(rule.matching),
            rule.command
        ))?;
    }
    Ok(())
}

fn require_rules(client: &Client) -> Result<()> {
    if client.supports(&RULE_METHODS) {
        Ok(())
    } else {
        Err("This daemon does not provide command rules".into())
    }
}

/// `/approvals` and `/permissions` at the idle prompt.
pub async fn command(
    client: &mut Client,
    input: &mut Input,
    interrupts: &mut Interrupts,
    session_id: &str,
    text: &str,
    display: &mut Display,
    listing: &mut Option<Listing>,
) -> Result<()> {
    let words: Vec<_> = text.split_whitespace().collect();
    match words[1..] {
        [] => show(client, interrupts, session_id, display, listing).await,
        ["remove", number] => {
            require_rules(client)?;
            let Some(seen) = listing.as_ref() else {
                return Err("Run /approvals to list the rules first".into());
            };
            let index = number
                .parse::<usize>()
                .ok()
                .filter(|index| (1..=seen.rules.len()).contains(index))
                .ok_or("No such rule in the last /approvals listing")?;
            let mut next = seen.rules.clone();
            let removed = next.remove(index - 1);
            let revision = seen.revision.clone();
            match settle(interrupts, set_rules(client, &revision, &next)).await {
                Ok(revision) => {
                    display.record(&format!("Rule removed: {}", describe(&removed)))?;
                    *listing = Some(Listing {
                        revision,
                        rules: next,
                    });
                }
                Err(error) if client::app_error(&error) == Some("permission_rules_conflict") => {
                    display.record("Command rules changed elsewhere; nothing was removed.")?;
                    let current = settle(interrupts, rules(client)).await?;
                    print_rules(display, &current.command_rules)?;
                    *listing = Some(Listing {
                        revision: current.revision,
                        rules: current.command_rules,
                    });
                }
                Err(error) => return Err(error),
            }
            Ok(())
        }
        [
            decision @ ("allow" | "deny"),
            matching @ ("exact" | "prefix"),
            ref command @ ..,
        ] if !command.is_empty() => {
            require_rules(client)?;
            let command = command.join(" ");
            if command.contains(COMMAND_SEPARATORS) {
                return Err("COMMAND must be a simple command without ; & | ` ( ) < > or $".into());
            }
            let rule = CommandRule {
                command,
                matching: if matching == "exact" {
                    CommandMatch::Exact
                } else {
                    CommandMatch::Prefix
                },
                decision: if decision == "allow" {
                    CommandRuleDecision::Allow
                } else {
                    CommandRuleDecision::Deny
                },
            };
            // Adding does not depend on the other rules: a conflict retries.
            for _ in 0..ATTEMPTS {
                let current = settle(interrupts, rules(client)).await?;
                let Some(next) = with_rule(&current.command_rules, &rule) else {
                    return display.record(&format!("Rule already present: {}", describe(&rule)));
                };
                match settle(interrupts, set_rules(client, &current.revision, &next)).await {
                    Ok(revision) => {
                        // The listing stays valid only if nothing else changed.
                        if listing
                            .as_ref()
                            .is_some_and(|seen| seen.revision == current.revision)
                        {
                            *listing = Some(Listing {
                                revision,
                                rules: next,
                            });
                        }
                        return display.record(&format!("Rule added: {}", describe(&rule)));
                    }
                    Err(error)
                        if client::app_error(&error) == Some("permission_rules_conflict") => {}
                    Err(error) => return Err(error),
                }
            }
            Err("Command rules kept changing; run the command again".into())
        }
        [preset] if !matches!(preset, "remove" | "allow" | "deny") => {
            switch(
                client,
                input,
                interrupts,
                session_id,
                parse_preset(preset)?,
                display,
            )
            .await
        }
        _ => Err(USAGE.into()),
    }
}

async fn show(
    client: &mut Client,
    interrupts: &mut Interrupts,
    session_id: &str,
    display: &mut Display,
    listing: &mut Option<Listing>,
) -> Result<()> {
    if client.supports(&PRESET_METHODS) {
        let state = settle(interrupts, read(client, session_id)).await?;
        print_state(display, &state)?;
    } else {
        display.record("Permission presets: this daemon does not provide them")?;
    }
    if client.supports(&RULE_METHODS) {
        let current = settle(interrupts, rules(client)).await?;
        print_rules(display, &current.command_rules)?;
        *listing = Some(Listing {
            revision: current.revision,
            rules: current.command_rules,
        });
    } else {
        display.record("Command rules: this daemon does not provide them")?;
    }
    display.record(USAGE)
}

/// Switches `main`. The user decided from the preset that was read, so a
/// version conflict is reported rather than retried.
async fn switch(
    client: &mut Client,
    input: &mut Input,
    interrupts: &mut Interrupts,
    session_id: &str,
    preset: PermissionPreset,
    display: &mut Display,
) -> Result<()> {
    if !client.supports(&PRESET_METHODS) {
        return Err("This daemon cannot change the permission preset".into());
    }
    let state = settle(interrupts, read(client, session_id)).await?;
    if state.policy.preset == preset {
        return display.record(&format!("Preset (main branch) is already {}", name(preset)));
    }
    if preset == PermissionPreset::FullAccess {
        display.record("Full access runs every tool call, including commands, without asking.")?;
        input.prompt("Switch the main branch to full-access? [y/N] ")?;
        // Only y/yes confirms; Ctrl+C and EOF decline.
        let line = tokio::select! {
            biased;
            _ = interrupts.recv() => {
                println!();
                None
            }
            line = input.line() => line?,
        };
        if !line
            .is_some_and(|line| matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
        {
            return display.record(&format!("Preset unchanged: {}", name(state.policy.preset)));
        }
    }
    match settle(interrupts, update(client, session_id, &state, preset)).await {
        Ok(version) => {
            // The live `permission/changed` for this write is already reported.
            display.permission_seen = display.permission_seen.max(version);
            display.record(&format!(
                "Preset (main branch): {}. Child branches created later inherit it; agent members keep the Session's starting preset.",
                name(preset)
            ))
        }
        Err(error) if client::app_error(&error) == Some("permission_conflict") => {
            let current = settle(interrupts, read(client, session_id)).await?;
            display.record(&format!(
                "The preset changed elsewhere and is now {}; nothing was changed. Run the command again to switch.",
                name(current.policy.preset)
            ))
        }
        Err(error) => Err(error),
    }
}
