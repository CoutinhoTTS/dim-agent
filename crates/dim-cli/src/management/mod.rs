//! The area adapters behind the native client's management surfaces
//! (cli-commands.md "Management surfaces").
//!
//! One adapter per management area talks to the daemon through the client
//! connection, so a subcommand needs no Session; turns its owner's reply into
//! one [`ManagementView`] at that boundary; performs the area's actions; and
//! refuses an action whose methods the daemon does not advertise, without
//! falling back. The panel and the subcommands both call [`apply`] and know no
//! area's methods.
//!
//! Each area is one table: the read that reports it and the method behind each
//! kind it has. [`supported`], [`actions`] and the views read that table, so
//! adding an area, or a kind to an area, writes the method once.

pub mod editor;
pub mod hooks;
pub mod scheduler;
pub mod skills;
pub mod workflow;

use std::path::Path;

use crate::client::Client;
use crate::model::session::{
    ActionKind, Area, ManagementAction, ManagementEntry, ManagementGroup, ManagementRefusal,
    ManagementRequest, ManagementResult,
};

/// One area's management methods: the read that reports the area, and the
/// methods behind each kind it has.
struct Methods {
    catalog: &'static str,
    /// Each kind and every method it calls. A kind is offered only when all
    /// of them are advertised.
    kinds: &'static [(ActionKind, &'static [&'static str])],
}

impl Methods {
    fn methods(&self, kind: ActionKind) -> Option<&'static [&'static str]> {
        self.kinds
            .iter()
            .find(|(has, _)| *has == kind)
            .map(|(_, methods)| *methods)
    }
}

const SKILLS: Methods = Methods {
    catalog: "skills.catalog",
    kinds: &[
        (ActionKind::Toggle, &["skills.set_enabled"]),
        (ActionKind::Reload, &["skills.refresh"]),
    ],
};

const HOOKS: Methods = Methods {
    catalog: "hooks.catalog",
    kinds: &[
        (ActionKind::Toggle, &["hooks.set_enabled"]),
        (ActionKind::Edit, &["hooks.save"]),
        // A reload re-reads the document, so it needs no rescan method.
        (ActionKind::Reload, &["hooks.catalog"]),
    ],
};

const WORKFLOW: Methods = Methods {
    catalog: "workflow.list",
    kinds: &[
        (ActionKind::Toggle, &["settings.read", "settings.update"]),
        (ActionKind::Edit, &["settings.read", "workflow.save"]),
        (ActionKind::Reload, &["workflow.list"]),
        (ActionKind::Add, &["workflow.save"]),
        (ActionKind::Run, &["workflow.start"]),
        (ActionKind::Inspect, &["workflow.status"]),
    ],
};

const SCHEDULED: Methods = Methods {
    catalog: "schedule.list",
    kinds: &[
        (ActionKind::Toggle, &["schedule.set_enabled"]),
        (ActionKind::Edit, &["schedule.save"]),
        // A reload re-reads the jobs, so it needs no rescan method.
        (ActionKind::Reload, &["schedule.list"]),
        (ActionKind::Add, &["schedule.save"]),
        (ActionKind::Remove, &["schedule.delete"]),
        (ActionKind::Run, &["schedule.run_now"]),
        (ActionKind::Inspect, &["schedule.history"]),
    ],
};

fn methods_of(area: Area) -> &'static Methods {
    match area {
        Area::Skills => &SKILLS,
        Area::Hooks => &HOOKS,
        Area::Scheduled => &SCHEDULED,
        Area::Workflow => &WORKFLOW,
    }
}

/// The kind an action exercises; a read has none.
fn kind(action: &ManagementAction) -> Option<ActionKind> {
    match action {
        ManagementAction::List => None,
        ManagementAction::Toggle { .. } => Some(ActionKind::Toggle),
        ManagementAction::Reload => Some(ActionKind::Reload),
        ManagementAction::Save { .. } => Some(ActionKind::Edit),
        ManagementAction::Add { .. } => Some(ActionKind::Add),
        ManagementAction::Remove { .. } => Some(ActionKind::Remove),
        ManagementAction::Run { .. } => Some(ActionKind::Run),
        ManagementAction::Inspect { .. } => Some(ActionKind::Inspect),
    }
}

/// Whether a kind acts on the area itself rather than on one entry.
fn acts_on_the_area(kind: ActionKind) -> bool {
    matches!(kind, ActionKind::Reload | ActionKind::Add)
}

/// The kinds of `available` that act on the area, in their order.
pub(super) fn area_kinds(available: &[ActionKind]) -> Vec<ActionKind> {
    available
        .iter()
        .copied()
        .filter(|kind| acts_on_the_area(*kind))
        .collect()
}

/// The kinds of `available` that act on one entry, in their order.
pub(super) fn entry_kinds(available: &[ActionKind]) -> Vec<ActionKind> {
    available
        .iter()
        .copied()
        .filter(|kind| !acts_on_the_area(*kind))
        .collect()
}

/// The action kinds the daemon can serve for `area`, in the order a panel
/// offers them. An area the daemon cannot read offers none.
pub fn actions(client: &Client, area: Area) -> Vec<ActionKind> {
    let methods = methods_of(area);
    if !client.supports(&[methods.catalog]) {
        return Vec::new();
    }
    methods
        .kinds
        .iter()
        .filter(|(_, needed)| client.supports(needed))
        .map(|(kind, _)| *kind)
        .collect()
}

/// Refuses a request the daemon cannot serve: an area without the method, or
/// a kind the area does not have. The client never falls back to a file,
/// another method or the host's own environment.
pub fn supported(client: &Client, request: &ManagementRequest) -> Result<(), String> {
    match kind(&request.action) {
        Some(kind) => supports_kind(client, request.area, kind),
        None => {
            let methods = methods_of(request.area);
            if client.supports(&[methods.catalog]) {
                Ok(())
            } else {
                Err(unsupported(request.area))
            }
        }
    }
}

/// Refuses a kind the daemon cannot serve for `area`, with the area's
/// sentence, for a surface that checks before it gathers anything.
pub fn supports_kind(client: &Client, area: Area, kind: ActionKind) -> Result<(), String> {
    let methods = methods_of(area);
    if !client.supports(&[methods.catalog]) {
        return Err(unsupported(area));
    }
    match methods.methods(kind) {
        Some(needed) if client.supports(needed) => Ok(()),
        _ => Err(unsupported(area)),
    }
}

pub(crate) fn unsupported(area: Area) -> String {
    match area {
        // The Scheduler is one owner by name, not a plural noun.
        Area::Scheduled => "This daemon cannot manage the Scheduler.".to_owned(),
        Area::Workflow => "This daemon cannot manage workflow definitions.".to_owned(),
        Area::Skills | Area::Hooks => format!("This daemon cannot manage {}.", area.name()),
    }
}

/// The Session and the line whose panel issued a request: what a run started
/// from the panel reports to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLine {
    pub session_id: String,
    pub branch_id: String,
}

/// Applies `request` to its area and reports what it came to.
pub async fn apply(
    client: &mut Client,
    request: &ManagementRequest,
    cwd: &Path,
    line: Option<&SessionLine>,
) -> ManagementResult {
    if let Err(error) = supported(client, request) {
        return ManagementResult::Refused(ManagementRefusal::Refused(error));
    }
    let actions = actions(client, request.area);
    match request.area {
        Area::Skills => match skills::apply(client, &request.action, cwd).await {
            skills::Outcome::Catalog(catalog) => ManagementResult::View {
                view: Box::new(skills::view(&catalog, &actions)),
                note: None,
            },
            skills::Outcome::ReadFailed(error) => ManagementResult::Applied {
                note: None,
                read_error: error,
            },
            skills::Outcome::Refused(message) => {
                ManagementResult::Refused(ManagementRefusal::Refused(message))
            }
        },
        Area::Hooks => match hooks::apply(client, &request.action, cwd).await {
            Ok(catalog) => ManagementResult::View {
                view: Box::new(hooks::view(&catalog, &actions)),
                note: None,
            },
            Err(hooks::Failure::ReadFailed(error)) => ManagementResult::Applied {
                note: None,
                read_error: error,
            },
            Err(hooks::Failure::Refused(message)) => {
                ManagementResult::Refused(ManagementRefusal::Refused(message))
            }
            Err(hooks::Failure::Conflict { message, catalog }) => {
                ManagementResult::Refused(ManagementRefusal::Conflict {
                    message,
                    view: Box::new(hooks::view(&catalog, &actions)),
                })
            }
        },
        Area::Workflow => match workflow::apply(client, &request.action, cwd, line).await {
            Ok(workflow::Answer::Definitions { listing, note }) => ManagementResult::View {
                view: Box::new(workflow::view(&listing, &actions, cwd)),
                note,
            },
            Ok(workflow::Answer::Detail(detail)) => ManagementResult::Detail(detail),
            Err(workflow::Failure::ReadFailed { note, error }) => ManagementResult::Applied {
                note,
                read_error: error,
            },
            Err(workflow::Failure::Refused(message)) => {
                ManagementResult::Refused(ManagementRefusal::Refused(message))
            }
            Err(workflow::Failure::Conflict { message, listing }) => {
                ManagementResult::Refused(ManagementRefusal::Conflict {
                    message,
                    view: Box::new(workflow::view(&listing, &actions, cwd)),
                })
            }
        },
        Area::Scheduled => match scheduler::apply(client, &request.action).await {
            Ok(scheduler::Answer::Jobs { catalog, note }) => ManagementResult::View {
                view: Box::new(scheduler::view(&catalog, &actions, cwd)),
                note,
            },
            Ok(scheduler::Answer::Detail(detail)) => ManagementResult::Detail(detail),
            Err(scheduler::Failure::ReadFailed { note, error }) => ManagementResult::Applied {
                note,
                read_error: error,
            },
            Err(scheduler::Failure::Refused(message)) => {
                ManagementResult::Refused(ManagementRefusal::Refused(message))
            }
            Err(scheduler::Failure::Conflict { message, catalog }) => {
                ManagementResult::Refused(ManagementRefusal::Conflict {
                    message,
                    view: Box::new(scheduler::view(&catalog, &actions, cwd)),
                })
            }
        },
    }
}

/// Entries collected under the headings their owner reports, in the order it
/// reports them.
#[derive(Default)]
struct Groups(Vec<ManagementGroup>);

impl Groups {
    fn push(&mut self, heading: &str, entry: ManagementEntry) {
        match self.0.iter_mut().find(|group| group.heading == heading) {
            Some(group) => group.entries.push(entry),
            None => self.0.push(ManagementGroup {
                heading: heading.to_owned(),
                entries: vec![entry],
            }),
        }
    }

    fn into_vec(self) -> Vec<ManagementGroup> {
        self.0
    }
}
