//! The `/tasks` panel: what runs apart from the turn, a background command
//! or a member agent, from the session's snapshot. Enter stops the command
//! or interrupts the delivered member's current turn.

use crate::model::view::{TaskKind, TaskView};
use crate::ui::i18n::t;
use crate::ui::list_panel::{Event, Item, ListPanel};
use crate::ui::prompts::Effect;
use crate::ui::terminal::KeyEvent;
use crate::ui::theme::Palette;

const HINT: &str = "Enter stop selected task · Esc close";

pub struct TasksPanel {
    list: ListPanel,
    tasks: Vec<TaskView>,
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
            tasks: Vec::new(),
        };
        panel.set_tasks(tasks);
        panel
    }

    /// Follows the snapshot: a task that ended leaves the list.
    pub fn set_tasks(&mut self, tasks: &[TaskView]) {
        let selected = self.list.selected_id();
        self.tasks = tasks.to_vec();
        self.list.set_items(
            Ok(tasks
                .iter()
                .map(|task| {
                    let kind = match &task.kind {
                        TaskKind::Command => t("command"),
                        TaskKind::Member { session_id: None } => t("agent (starting)"),
                        TaskKind::Member {
                            session_id: Some(_),
                        } => t("agent"),
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
            Event::Pick(id) => match self
                .tasks
                .iter()
                .find(|task| task.id == id)
                .map(|task| &task.kind)
            {
                Some(TaskKind::Command) => Effect::StopTask(id),
                Some(TaskKind::Member {
                    session_id: Some(session_id),
                }) => Effect::StopMember(session_id.clone()),
                Some(TaskKind::Member { session_id: None }) => {
                    Effect::Notice(t("Agent is starting; no member Session to stop yet").to_owned())
                }
                None => Effect::None,
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
