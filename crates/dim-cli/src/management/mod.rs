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

use std::path::Path;

use crate::client::Client;
use crate::model::session::{
    ActionKind, Area, ManagementAction, ManagementEntry, ManagementGroup, ManagementRefusal,
    ManagementRequest, ManagementResult, ManagementView,
};
use crate::ui::i18n::{self, t, tf};

/// One area's management methods: the read that reports the area, the
/// method behind each kind it has, and the name the area goes by.
struct Methods {
    catalog: &'static str,
    kinds: &'static [(ActionKind, &'static str)],
    /// What the area manages, as the unsupported sentence names it: the
    /// noun behind "This daemon cannot manage ...".
    manages: &'static str,
}

impl Methods {
    fn method(&self, kind: ActionKind) -> Option<&'static str> {
        self.kinds
            .iter()
            .find(|(has, _)| *has == kind)
            .map(|(_, method)| *method)
    }
}

const SKILLS: Methods = Methods {
    catalog: "skills.catalog",
    manages: "skills",
    kinds: &[
        (ActionKind::Toggle, "skills.set_enabled"),
        (ActionKind::Reload, "skills.refresh"),
    ],
};

const HOOKS: Methods = Methods {
    catalog: "hooks.catalog",
    manages: "hooks",
    kinds: &[
        (ActionKind::Toggle, "hooks.set_enabled"),
        (ActionKind::Edit, "hooks.save"),
        // A reload re-reads the document, so it needs no rescan method.
        (ActionKind::Reload, "hooks.catalog"),
    ],
};

const SCHEDULED: Methods = Methods {
    catalog: "schedule.list",
    manages: "scheduled jobs",
    kinds: &[
        (ActionKind::Toggle, "schedule.set_enabled"),
        (ActionKind::Edit, "schedule.save"),
        // A reload re-reads the jobs, so it needs no rescan method.
        (ActionKind::Reload, "schedule.list"),
        (ActionKind::Add, "schedule.save"),
        (ActionKind::Remove, "schedule.delete"),
        (ActionKind::Run, "schedule.run_now"),
        (ActionKind::Inspect, "schedule.history"),
    ],
};

fn methods_of(area: Area) -> &'static Methods {
    match area {
        Area::Skills => &SKILLS,
        Area::Hooks => &HOOKS,
        Area::Scheduled => &SCHEDULED,
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
        .filter(|(_, method)| client.supports(&[method]))
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
    match methods.method(kind) {
        Some(method) if client.supports(&[method]) => Ok(()),
        _ => Err(unsupported(area)),
    }
}

/// The unsupported-daemon sentence: what the area manages, translated
/// where it is used, so the shared module names no area.
fn unsupported(area: Area) -> String {
    let methods = methods_of(area);
    tf(
        "This daemon cannot manage {area}.",
        &[("area", t(methods.manages))],
    )
}

/// The id of the entry `action` names, when it names one.
pub fn named_id(action: &ManagementAction) -> Option<&str> {
    match action {
        ManagementAction::Toggle { id, .. }
        | ManagementAction::Remove { id }
        | ManagementAction::Run { id }
        | ManagementAction::Save { id: Some(id), .. } => Some(id),
        _ => None,
    }
}

/// The label of the entry `action` names in `view`, when the view has it.
pub fn entry_name<'a>(view: &'a ManagementView, action: &ManagementAction) -> Option<&'a str> {
    let id = named_id(action)?;
    view.groups
        .iter()
        .flat_map(|group| &group.entries)
        .find(|entry| entry.id == id)
        .map(|entry| entry.label.as_str())
}

/// The line an applied action reports. `name` is the entry's label from the
/// freshest read the surface holds; without one the line names the id the
/// action was sent with. An adapter with a better line — a create's name, a
/// run's answer — answers with it instead.
pub fn applied_line(action: &ManagementAction, name: Option<&str>) -> String {
    match action {
        ManagementAction::Toggle { id, enabled } => tf(
            if *enabled {
                "Enabled {name}."
            } else {
                "Disabled {name}."
            },
            &[("name", name.unwrap_or(id))],
        ),
        ManagementAction::Save { id: Some(id), .. } => {
            tf("Saved {name}.", &[("name", name.unwrap_or(id))])
        }
        ManagementAction::Save { id: None, .. } => t("Saved.").to_owned(),
        ManagementAction::Remove { id } => tf("Removed {name}.", &[("name", name.unwrap_or(id))]),
        ManagementAction::Reload => t("Reloaded.").to_owned(),
        ManagementAction::Add { .. } => t("Added.").to_owned(),
        ManagementAction::Run { id } => tf("Ran {name}.", &[("name", name.unwrap_or(id))]),
        // A read applies nothing: the adapters answer with one only after an
        // action of the kinds above.
        ManagementAction::List | ManagementAction::Inspect { .. } => {
            unreachable!("a read does not report an applied action")
        }
    }
}

/// Sets the language management text is composed in, from a locale code
/// (`config.json` stores the same spelling). The session thread composes the
/// panel's text, so the interface hands it the language it chose.
pub fn use_language(code: &str) {
    if let Some(locale) = i18n::Locale::from_code(code) {
        i18n::set(locale);
    }
}

/// Applies `request` to its area and reports what it came to.
pub async fn apply(
    client: &mut Client,
    request: &ManagementRequest,
    cwd: &Path,
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
                    view: catalog.map(|catalog| Box::new(hooks::view(&catalog, &actions))),
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
                    view: Ok(Box::new(scheduler::view(&catalog, &actions, cwd))),
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
