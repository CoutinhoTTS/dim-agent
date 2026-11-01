//! The management panel: one type draws every management area, on the list
//! every panel shares (cli-commands.md "Management surfaces").
//!
//! ```text
//! ╭─ Skills ──────────────────────────────────────╮
//! │ >                                             │
//! │                                               │
//! │   user                                        │
//! │ › commit      enabled                         │
//! │   review      disabled  shadowed by commit    │
//! │   project                                     │
//! │   deploy      enabled                         │
//! │   Issues                                      │
//! │     /w/.agents/skills/old/SKILL.md: no name   │
//! │   Writes commit messages from the staged diff.│
//! │   /home/u/.agents/skills/commit/SKILL.md      │
//! ╰───── Enter toggle · Ctrl+R reload · Esc close ┘
//! ```
//!
//! The entry's actions decide the keys it offers: the hint names only those,
//! and Enter, Space and Ctrl+R keep one meaning in every area.

use crate::model::session::{
    ActionKind, Area, EntryState, ManagementAction, ManagementEntry, ManagementReply,
    ManagementRequest, ManagementView,
};
use crate::ui::i18n::{t, tf};
use crate::ui::list_panel::{Event, Item, ListPanel};
use crate::ui::prompts::Effect;
use crate::ui::render::panel::wrap_words;
use crate::ui::terminal::{KeyCode, KeyEvent, Modifiers};
use crate::ui::theme::Palette;
use crate::ui::theme::ansi::fg;

/// Every English string the panel draws through [`t`]; each locale
/// translates them all.
#[cfg(test)]
pub const STRINGS: &[&str] = &[
    "Skills",
    "Esc close",
    "Enter toggle",
    "Ctrl+R reload",
    "Loading...",
    "No entries.",
    "No matching entry.",
    "Issues",
    "enabled",
    "disabled",
    "shadowed by {name}",
    "Reloaded.",
    "Enabled {name}.",
    "Disabled {name}.",
    "Reloaded {area}: {entries} entries, {disabled} disabled.",
];

pub struct ManagementPanel {
    area: Area,
    list: ListPanel,
    view: Option<ManagementView>,
    /// A request is in flight: only Esc answers.
    busy: bool,
    /// What the last request came to.
    message: Option<Result<String, String>>,
}

impl ManagementPanel {
    pub fn new(area: Area) -> Self {
        Self {
            area,
            list: ListPanel::new(
                title(area),
                hint(&[]),
                "Loading...",
                "No entries.",
                "No matching entry.",
                false,
            ),
            view: None,
            busy: true,
            message: None,
        }
    }

    /// The reply to this panel's request.
    pub fn set_result(&mut self, reply: &ManagementReply) {
        if reply.request.area != self.area {
            return;
        }
        self.busy = false;
        match &reply.result {
            Ok(view) => {
                // The read that opens a panel says nothing; every later
                // answer names what it did.
                self.message = match (&reply.request.action, self.view.is_some()) {
                    (ManagementAction::List, false) => None,
                    (ManagementAction::List, true) => Some(Ok(t("Reloaded.").to_owned())),
                    (action, _) => Some(Ok(outcome(action, view))),
                };
                self.set_view(view.clone());
            }
            Err(error) => {
                self.message = Some(Err(error.clone()));
                if self.view.is_none() {
                    // Nothing was shown yet: the list says why instead of
                    // keeping "Loading...".
                    self.list.set_items(Err(error.clone()), None);
                }
            }
        }
        self.refresh_hint();
    }

    fn set_view(&mut self, view: ManagementView) {
        let items = items(&view);
        let selected = self.list.selected_id();
        self.list.set_items(Ok(items), selected.as_deref());
        self.view = Some(view);
        self.refresh_hint();
    }

    pub fn paste(&mut self, text: &str) {
        self.list.paste(text);
        self.refresh_hint();
    }

    fn entry(&self, id: &str) -> Option<&ManagementEntry> {
        self.view
            .as_ref()?
            .groups
            .iter()
            .flat_map(|group| &group.entries)
            .find(|entry| entry.id == id)
    }

    fn selected(&self) -> Option<&ManagementEntry> {
        self.entry(&self.list.selected_id()?)
    }

    /// Whether the area or the selected entry offers `kind`.
    fn offers(&self, kind: ActionKind) -> bool {
        self.view
            .as_ref()
            .is_some_and(|view| view.actions.contains(&kind))
            || self
                .selected()
                .is_some_and(|entry| entry.actions.contains(&kind))
    }

    fn refresh_hint(&mut self) {
        let mut kinds = match &self.view {
            Some(view) => view.actions.clone(),
            // Nothing is shown yet: Ctrl+R repeats the read.
            None => vec![ActionKind::Reload],
        };
        if let Some(entry) = self.selected() {
            kinds.extend(entry.actions.iter().copied());
        }
        self.list.set_hint(hint(&kinds));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if key.code == KeyCode::Esc {
            return match self.list.handle_key(key) {
                Event::Close => Effect::Close,
                _ => {
                    self.refresh_hint();
                    Effect::None
                }
            };
        }
        if self.busy {
            return Effect::None;
        }
        if key.code == KeyCode::Char('r') && key.mods.contains(Modifiers::CTRL) {
            // A panel with nothing to act on still retries its read.
            if self.view.is_some() && !self.offers(ActionKind::Reload) {
                return Effect::None;
            }
            self.busy = true;
            return Effect::Management(ManagementRequest {
                area: self.area,
                action: ManagementAction::Reload,
            });
        }
        // Space toggles as Enter does, as in `/settings > Execution`.
        let key = if matches!(key.code, KeyCode::Char(' ')) {
            KeyEvent::new(KeyCode::Enter, key.mods)
        } else {
            key
        };
        let event = self.list.handle_key(key);
        self.refresh_hint();
        let Event::Pick(id) = event else {
            return Effect::None;
        };
        let Some(entry) = self.entry(&id) else {
            return Effect::None;
        };
        if !entry.actions.contains(&ActionKind::Toggle) {
            return Effect::None;
        }
        let request = ManagementRequest {
            area: self.area,
            action: ManagementAction::Toggle {
                id: entry.id.clone(),
                enabled: !entry.enabled,
            },
        };
        self.busy = true;
        Effect::Management(request)
    }

    pub fn render(&self, width: usize, rows: usize, p: &Palette) -> Vec<String> {
        let inner = width.saturating_sub(4).max(1);
        let mut footer = Vec::new();
        if let Some(entry) = self.selected() {
            for line in &entry.detail {
                for row in wrap_words(line, inner.saturating_sub(2).max(1)) {
                    footer.push(format!("  {}", fg(&row, &p.muted)));
                }
            }
        }
        if let Some(message) = &self.message {
            let (text, color) = match message {
                Ok(text) => (text, &p.success),
                Err(text) => (text, &p.danger),
            };
            for row in wrap_words(text, inner) {
                footer.push(fg(&row, color));
            }
        }
        if !footer.is_empty() {
            footer.insert(0, String::new());
        }
        footer.truncate(rows.saturating_sub(2 + 3 + 1));
        self.list.render_with_footer(width, rows, p, &footer)
    }
}

/// What a reload with no panel reports: the area's size, then what its owner
/// could not use.
pub fn reloaded(area: Area, view: &ManagementView) -> String {
    let entries: Vec<&ManagementEntry> = view
        .groups
        .iter()
        .flat_map(|group| &group.entries)
        .collect();
    let disabled = entries.iter().filter(|entry| !entry.enabled).count();
    let mut lines = vec![tf(
        "Reloaded {area}: {entries} entries, {disabled} disabled.",
        &[
            ("area", t(title(area))),
            ("entries", &entries.len().to_string()),
            ("disabled", &disabled.to_string()),
        ],
    )];
    lines.extend(view.issues.iter().map(|issue| format!("  {issue}")));
    lines.join("\n")
}

/// The panel title of `area`, in English: the list translates it as it
/// draws.
fn title(area: Area) -> &'static str {
    match area {
        Area::Skills => "Skills",
    }
}

/// One label per kind, in the actions table's order.
fn label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Toggle => "Enter toggle",
        ActionKind::Reload => "Ctrl+R reload",
    }
}

/// The keys the area and the selected entry offer, as one hint.
fn hint(kinds: &[ActionKind]) -> String {
    let mut parts: Vec<&str> = [ActionKind::Toggle, ActionKind::Reload]
        .into_iter()
        .filter(|kind| kinds.contains(kind))
        .map(label)
        .collect();
    parts.push("Esc close");
    parts.join(" · ")
}

/// The rows: each group's heading, its entries, then what the owner could
/// not use.
fn items(view: &ManagementView) -> Vec<Item> {
    let mut items = Vec::new();
    for group in &view.groups {
        items.push(Item::heading(group.heading.clone()));
        for entry in &group.entries {
            let mut item = Item::new(
                entry.id.clone(),
                entry.label.clone(),
                states(entry).join("  "),
            );
            item.search = format!("{} {}", entry.id, entry.detail.join(" "));
            items.push(item);
        }
    }
    if !view.issues.is_empty() {
        items.push(Item::heading(t("Issues")));
        items.extend(view.issues.iter().map(|issue| Item::heading(issue.clone())));
    }
    items
}

/// An entry's states: on or off, then the ones its owner adds.
fn states(entry: &ManagementEntry) -> Vec<String> {
    let mut states = vec![t(if entry.enabled { "enabled" } else { "disabled" }).to_owned()];
    for state in &entry.states {
        states.push(match state {
            EntryState::Shadowed { by } => tf("shadowed by {name}", &[("name", by)]),
        });
    }
    states
}

/// The notice a successful change leaves.
fn outcome(action: &ManagementAction, view: &ManagementView) -> String {
    let name = |id: &str| {
        view.groups
            .iter()
            .flat_map(|group| &group.entries)
            .find(|entry| entry.id == id)
            .map(|entry| entry.label.clone())
            .unwrap_or_else(|| id.to_owned())
    };
    match action {
        ManagementAction::Toggle { id, enabled } => tf(
            if *enabled {
                "Enabled {name}."
            } else {
                "Disabled {name}."
            },
            &[("name", &name(id))],
        ),
        ManagementAction::List | ManagementAction::Reload => t("Reloaded.").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::session::{ManagementGroup, ManagementReply};
    use crate::ui::render::panel::{strip_escapes, visible_width};
    use crate::ui::theme::{Appearance, ColorMode, palette::terminal_palette};

    fn palette() -> Palette {
        terminal_palette(ColorMode::TrueColor, Appearance::Dark)
    }

    fn entry(
        id: &str,
        label: &str,
        enabled: bool,
        states: Vec<EntryState>,
        actions: Vec<ActionKind>,
    ) -> ManagementEntry {
        ManagementEntry {
            id: id.into(),
            label: label.into(),
            detail: vec![
                format!("{label} does its work."),
                format!("/w/.agents/skills/{id}/SKILL.md"),
            ],
            enabled,
            states,
            actions,
        }
    }

    fn view() -> ManagementView {
        ManagementView {
            revision: None,
            document: None,
            actions: vec![ActionKind::Reload],
            groups: vec![
                ManagementGroup {
                    heading: "user".into(),
                    entries: vec![
                        entry(
                            "user:commit",
                            "commit",
                            true,
                            Vec::new(),
                            vec![ActionKind::Toggle],
                        ),
                        entry(
                            "user:review",
                            "review",
                            false,
                            vec![EntryState::Shadowed {
                                by: "commit".into(),
                            }],
                            vec![ActionKind::Toggle],
                        ),
                        entry("user:locked", "locked", true, Vec::new(), vec![]),
                    ],
                },
                ManagementGroup {
                    heading: "project".into(),
                    entries: vec![entry(
                        "workspace:/w:deploy",
                        "deploy",
                        true,
                        Vec::new(),
                        vec![ActionKind::Toggle],
                    )],
                },
            ],
            issues: vec!["/w/.agents/skills/old/SKILL.md: no name".into()],
        }
    }

    fn opened() -> ManagementPanel {
        let mut panel = ManagementPanel::new(Area::Skills);
        panel.set_result(&ManagementReply {
            request: ManagementRequest {
                area: Area::Skills,
                action: ManagementAction::List,
            },
            result: Ok(view()),
        });
        panel
    }

    fn shown(panel: &ManagementPanel, width: usize, rows: usize) -> Vec<String> {
        let lines = panel.render(width, rows, &palette());
        for line in &lines {
            assert_eq!(visible_width(line), width, "{line:?}");
        }
        lines.iter().map(|line| strip_escapes(line)).collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, Modifiers::NONE)
    }

    /// The rows group entries under their source, name each entry's state,
    /// and the selected one says what it is below the list.
    #[test]
    fn rows_group_by_source_and_name_the_selected_entry() {
        let panel = opened();
        let text = shown(&panel, 72, 20).join("\n");
        assert!(text.contains("user"), "{text}");
        assert!(text.contains("project"), "{text}");
        assert!(text.contains("enabled"), "{text}");
        assert!(text.contains("disabled  shadowed by commit"), "{text}");
        assert!(
            text.contains("commit does its work.")
                && text.contains("/w/.agents/skills/user:commit/SKILL.md"),
            "the selected entry says what it is: {text}"
        );
        assert!(
            text.contains("/w/.agents/skills/old/SKILL.md: no name"),
            "the issues follow the entries: {text}"
        );
        assert!(
            text.contains("Enter toggle · Ctrl+R reload · Esc close"),
            "{text}"
        );
        for width in [16, 24, 40] {
            for rows in [6, 10, 20] {
                let lines = shown(&panel, width, rows);
                assert!(lines.len() <= rows, "{width}x{rows}: {lines:?}");
            }
        }
    }

    /// The entry's actions decide its keys: Enter toggles what offers it,
    /// Ctrl+R reloads, and the hint names only what the selection offers.
    #[test]
    fn keys_follow_the_actions_of_the_selected_entry() {
        let mut panel = opened();
        match panel.handle_key(key(KeyCode::Enter)) {
            Effect::Management(request) => {
                assert_eq!(request.area, Area::Skills);
                assert_eq!(
                    request.action,
                    ManagementAction::Toggle {
                        id: "user:commit".into(),
                        enabled: false
                    }
                );
            }
            _ => panic!("expected a toggle"),
        }
        panel.set_result(&ManagementReply {
            request: ManagementRequest {
                area: Area::Skills,
                action: ManagementAction::Toggle {
                    id: "user:commit".into(),
                    enabled: false,
                },
            },
            result: Ok(view()),
        });
        assert!(
            shown(&panel, 72, 20)
                .join("\n")
                .contains("Disabled commit."),
            "{}",
            shown(&panel, 72, 20).join("\n")
        );
        panel.handle_key(key(KeyCode::End));
        let text = shown(&panel, 72, 20).join("\n");
        assert!(
            text.contains("Enter toggle · Ctrl+R reload · Esc close"),
            "{text}"
        );
        match panel.handle_key(KeyEvent::new(KeyCode::Char('r'), Modifiers::CTRL)) {
            Effect::Management(request) => assert_eq!(request.action, ManagementAction::Reload),
            _ => panic!("expected a reload"),
        }
        // A reload keeps the entry the user was on.
        panel.set_result(&ManagementReply {
            request: ManagementRequest {
                area: Area::Skills,
                action: ManagementAction::Reload,
            },
            result: Ok(view()),
        });
        assert!(shown(&panel, 72, 20).join("\n").contains("Reloaded."));
        // An entry that offers no toggle ignores Enter and says so by its
        // hint.
        let mut panel = opened();
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(key(KeyCode::Down));
        let text = shown(&panel, 72, 20).join("\n");
        assert!(
            text.contains("Ctrl+R reload · Esc close") && !text.contains("Enter toggle"),
            "{text}"
        );
        assert!(matches!(
            panel.handle_key(key(KeyCode::Enter)),
            Effect::None
        ));
        // Space toggles as Enter does.
        panel.handle_key(key(KeyCode::Home));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Char(' '))),
            Effect::Management(_)
        ));
    }

    /// A panel with nothing to show says so, and a failed read keeps the
    /// error rather than an empty list.
    #[test]
    fn an_empty_area_and_a_failed_read_say_so() {
        let mut panel = ManagementPanel::new(Area::Skills);
        assert!(shown(&panel, 40, 10).join("\n").contains("Loading..."));
        panel.set_result(&ManagementReply {
            request: ManagementRequest {
                area: Area::Skills,
                action: ManagementAction::List,
            },
            result: Ok(ManagementView::default()),
        });
        assert!(shown(&panel, 40, 10).join("\n").contains("No entries."));
        let mut panel = ManagementPanel::new(Area::Skills);
        panel.set_result(&ManagementReply {
            request: ManagementRequest {
                area: Area::Skills,
                action: ManagementAction::List,
            },
            result: Err("This daemon cannot manage skills.".into()),
        });
        assert!(
            shown(&panel, 60, 10)
                .join(" ")
                .contains("This daemon cannot manage skills.")
        );
    }

    /// Reload belongs to the area: it works on an empty area and after a
    /// failed read, where no entry is selected to offer it.
    #[test]
    fn reload_works_on_an_empty_area_and_after_a_failed_read() {
        let mut panel = ManagementPanel::new(Area::Skills);
        panel.set_result(&ManagementReply {
            request: ManagementRequest {
                area: Area::Skills,
                action: ManagementAction::List,
            },
            result: Ok(ManagementView {
                actions: vec![ActionKind::Reload],
                ..ManagementView::default()
            }),
        });
        let text = shown(&panel, 60, 12).join("\n");
        assert!(text.contains("No entries."), "{text}");
        assert!(text.contains("Ctrl+R reload · Esc close"), "{text}");
        match panel.handle_key(KeyEvent::new(KeyCode::Char('r'), Modifiers::CTRL)) {
            Effect::Management(request) => {
                assert_eq!(request.area, Area::Skills);
                assert_eq!(request.action, ManagementAction::Reload);
            }
            _ => panic!("expected a reload"),
        }
        // A read that failed leaves nothing to select; Ctrl+R reads again.
        let mut panel = ManagementPanel::new(Area::Skills);
        panel.set_result(&ManagementReply {
            request: ManagementRequest {
                area: Area::Skills,
                action: ManagementAction::List,
            },
            result: Err("This daemon cannot manage skills.".into()),
        });
        let text = shown(&panel, 60, 12).join("\n");
        assert!(text.contains("This daemon cannot manage skills."), "{text}");
        assert!(text.contains("Ctrl+R reload"), "{text}");
        match panel.handle_key(KeyEvent::new(KeyCode::Char('r'), Modifiers::CTRL)) {
            Effect::Management(request) => assert_eq!(request.action, ManagementAction::Reload),
            _ => panic!("expected a reload"),
        }
    }

    /// Filtering keeps the entry's keys: the hint follows the new selection.
    #[test]
    fn a_filter_narrows_the_list_and_keeps_the_keys_of_its_selection() {
        let mut panel = opened();
        for c in "lock".chars() {
            panel.handle_key(key(KeyCode::Char(c)));
        }
        let text = shown(&panel, 72, 20).join("\n");
        assert!(text.contains("locked"), "{text}");
        assert!(!text.contains("deploy"), "{text}");
        assert!(text.contains("Ctrl+R reload · Esc close"), "{text}");
        assert!(!text.contains("Enter toggle"), "{text}");
    }
}
