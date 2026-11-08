//! Goal command forms and presentation of the owner's current-state read.

use crate::client::Client;

pub use dim_agent_types::{GoalView as Goal, GoalViewStatus as Status};

fn status_words(status: &Status) -> String {
    match status {
        Status::Active => "active".into(),
        Status::Paused(reason) => format!("paused ({})", reason.replace('_', " ")),
        Status::Completed => "completed".into(),
        Status::Blocked => "blocked".into(),
        Status::None => "no status".into(),
    }
}

/// The words after `/goal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The goal in force, or that the branch has none.
    Show,
    /// An objective typed without a method: `edit` while the goal in force
    /// is active or paused, so only the objective changes, and `set`
    /// otherwise, which starts a new goal.
    Objective(String),
    Set(String),
    Edit(String),
    Pause,
    Resume,
    Clear,
}

/// The method a request writes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Set,
    Edit,
    Pause,
    Resume,
    Clear,
}

impl Call {
    pub fn method(self) -> &'static str {
        match self {
            Self::Set => "session.goal.set",
            Self::Edit => "session.goal.edit",
            Self::Pause => "session.goal.pause",
            Self::Resume => "session.goal.resume",
            Self::Clear => "session.goal.clear",
        }
    }

    /// The word the command's output names the call with.
    pub fn words(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Edit => "edited",
            Self::Pause => "paused",
            Self::Resume => "resumed",
            Self::Clear => "cleared",
        }
    }
}

/// What `/goal` does with the words after it; `None` is a usage error.
///
/// `pause`, `resume` and `clear` name their method; `set` and `edit` name it
/// and take the objective after it, so an objective that is one of those
/// words is written as `/goal set pause`.
pub fn request(words: &str) -> Option<Request> {
    let words = words.trim();
    if words.is_empty() {
        return Some(Request::Show);
    }
    let (word, rest) = match words.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (words, ""),
    };
    Some(match word {
        "pause" if rest.is_empty() => Request::Pause,
        "resume" if rest.is_empty() => Request::Resume,
        "clear" if rest.is_empty() => Request::Clear,
        "set" => return (!rest.is_empty()).then(|| Request::Set(rest.to_owned())),
        "edit" => return (!rest.is_empty()).then(|| Request::Edit(rest.to_owned())),
        _ => Request::Objective(words.to_owned()),
    })
}

/// The objective a writing request carries.
pub fn objective(request: &Request) -> Option<&str> {
    match request {
        Request::Objective(text) | Request::Set(text) | Request::Edit(text) => Some(text),
        _ => None,
    }
}

/// The call a request makes, given the goal in force on the branch.
///
/// A typed objective edits only a goal that is still being worked toward
/// (active or paused); with no goal, no status, or a finished one, it sets a
/// new goal, which is what starts the branch working again (Goal §2).
pub fn call(request: &Request, goal: Option<&Goal>) -> Option<Call> {
    Some(match request {
        Request::Show => return None,
        Request::Objective(_)
            if goal
                .is_some_and(|goal| matches!(goal.status, Status::Active | Status::Paused(_))) =>
        {
            Call::Edit
        }
        Request::Objective(_) | Request::Set(_) => Call::Set,
        Request::Edit(_) => Call::Edit,
        Request::Pause => Call::Pause,
        Request::Resume => Call::Resume,
        Request::Clear => Call::Clear,
    })
}

/// Whether this daemon installs Goal: the Session's method table
/// (`session.methods`) lists `session.goal.set`, which Goal registers when
/// its Session part is loaded. A daemon that does not advertise the table is
/// trusted until the call itself answers `unknown_method`.
pub async fn available(client: &mut Client, session_id: &str) -> bool {
    if !client.offers("session.methods") {
        return true;
    }
    match client
        .typed::<dim_client::wire::MethodTable>(
            "session.methods",
            serde_json::json!({"sessionId": session_id}),
        )
        .await
    {
        Ok(table) => table
            .methods
            .iter()
            .any(|decl| decl.name == Call::Set.method()),
        Err(_) => true,
    }
}

/// What `/goal` says when this daemon installs no Goal.
pub const NOT_AVAILABLE: &str = "Goal is not available on this daemon.";

/// The usage shown by bare `/goal` and invalid forms.
pub const USAGE: &str =
    "Usage: /goal [OBJECTIVE | set OBJECTIVE | edit OBJECTIVE | pause | resume | clear]";

/// Reports the call and resulting goal state for `branch`; the read form
/// prefixes usage before the current state.
pub fn report(branch: &str, call: Option<Call>, goal: Option<&Goal>) -> Vec<String> {
    let mut lines = if call.is_none() {
        vec![USAGE.to_owned()]
    } else {
        Vec::new()
    };
    let Some(goal) = goal else {
        lines.push(match call {
            Some(Call::Clear) => format!("Goal cleared on {branch}."),
            _ => format!("No goal on {branch}."),
        });
        return lines;
    };
    let mut first = match call {
        Some(call) => format!("Goal {} on {branch}", call.words()),
        None => format!("Goal on {branch}"),
    };
    first.push_str(" · ");
    first.push_str(&status_words(&goal.status));
    if goal.evaluator {
        first.push_str(" · evaluator judge");
    }
    lines.push(first);
    for line in goal.objective.lines() {
        lines.push(format!("  {line}"));
    }
    lines
}

pub(crate) fn changed(event: &crate::wire::EventContent) -> bool {
    match event {
        crate::wire::EventContent::Input(data) => data.blocks.iter().any(|block| {
            let dim_agent_types::InputBlock::Raw { text, .. } = block;
            text.starts_with("<dim-goal ")
        }),
        crate::wire::EventContent::InputDeleted(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_typed_objective_edits_only_a_live_goal() {
        let goal = |status| Goal {
            objective: "Ship it".into(),
            status,
            evaluator: false,
        };
        let request = Request::Objective("Next thing".into());
        assert_eq!(call(&request, None), Some(Call::Set));
        assert_eq!(call(&request, Some(&goal(Status::None))), Some(Call::Set));
        assert_eq!(
            call(&request, Some(&goal(Status::Active))),
            Some(Call::Edit)
        );
        assert_eq!(
            call(&request, Some(&goal(Status::Paused("limit".into())))),
            Some(Call::Edit)
        );
        assert_eq!(
            call(&request, Some(&goal(Status::Completed))),
            Some(Call::Set)
        );
        assert_eq!(
            call(&request, Some(&goal(Status::Blocked))),
            Some(Call::Set)
        );
    }

    #[test]
    fn the_words_name_the_methods_and_an_objective_spelled_like_one() {
        assert_eq!(request(""), Some(Request::Show));
        assert_eq!(request("  "), Some(Request::Show));
        assert_eq!(request("pause"), Some(Request::Pause));
        assert_eq!(request("resume"), Some(Request::Resume));
        assert_eq!(request("clear"), Some(Request::Clear));
        assert_eq!(
            request("pause now"),
            Some(Request::Objective("pause now".into()))
        );
        assert_eq!(request("edit"), None);
        assert_eq!(request("set"), None);
        assert_eq!(request("set pause"), Some(Request::Set("pause".into())));
        assert_eq!(
            request("edit Ship it"),
            Some(Request::Edit("Ship it".into()))
        );
        assert_eq!(
            request("Ship it"),
            Some(Request::Objective("Ship it".into()))
        );
        assert_eq!(
            call(&Request::Objective("Ship it".into()), None),
            Some(Call::Set)
        );
        assert_eq!(call(&Request::Set("pause".into()), None), Some(Call::Set));
        assert_eq!(call(&Request::Show, None), None);
    }
}
