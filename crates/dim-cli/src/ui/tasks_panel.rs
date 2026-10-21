//! The `/tasks` panel: what runs apart from the turn, a background command
//! or a member agent, from the session's snapshot. Enter stops a command;
//! a member ends with its result or with the leader's turn.

use crate::model::view::{TaskKind, TaskView};
use crate::ui::i18n::t;
use crate::ui::list_panel::{Event, Item, ListPanel};
use crate::ui::prompts::Effect;
use crate::ui::terminal::KeyEvent;
use crate::ui::theme::Palette;

const HINT: &str = "Enter stop command · Esc close";

pub struct TasksPanel {
    list: ListPanel,
    kinds: Vec<(String, TaskKind)>,
}

impl TasksPanel {
    pub fn new(tasks: &[TaskView]) -> Self {
        let mut panel = Self {
            list: ListPanel::new(
                "Background tasks",
                HINT,
                "",
                "Nothing runs in the background.",
                "No matching tasks.",
                false,
            ),
            kinds: Vec::new(),
        };
        panel.set_tasks(tasks);
        panel
    }

    /// Follows the snapshot: a task that ended leaves the list.
    pub fn set_tasks(&mut self, tasks: &[TaskView]) {
        let selected = self.list.selected_id();
        self.kinds = tasks
            .iter()
            .map(|task| (task.id.clone(), task.kind))
            .collect();
        self.list.set_items(
            Ok(tasks
                .iter()
                .map(|task| {
                    let kind = match task.kind {
                        TaskKind::Command => t("command"),
                        TaskKind::Member => t("agent"),
                    };
                    Item::new(task.id.clone(), task.label.clone(), kind)
                })
                .collect()),
            selected.as_deref(),
        );
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        match self.list.handle_key(key) {
            Event::None => Effect::None,
            Event::Close => Effect::Close,
            Event::Refresh => Effect::None,
            Event::Pick(id) => match self.kinds.iter().find(|(task, _)| *task == id) {
                Some((_, TaskKind::Command)) => Effect::StopTask(id),
                _ => Effect::Notice(
                    t("An agent stops with its result, or with the turn (Esc)").to_owned(),
                ),
            },
        }
    }

    pub fn cursor(&self, width: usize) -> Option<(usize, usize)> {
        self.list.cursor(width)
    }

    pub fn render(&self, width: usize, rows: usize, p: &Palette) -> Vec<String> {
        self.list.render(width, rows, p)
    }
}
