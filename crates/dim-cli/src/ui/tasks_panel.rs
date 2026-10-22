//! The `/tasks` panel: what runs apart from the turn, a background command
//! or a member agent, from the session's snapshot. Command details show the
//! existing tail preview; member controls retain their current path.

use crate::model::view::{TaskKind, TaskOutputState, TaskTarget, TaskView};
use crate::sanitize::safe;
use crate::ui::i18n::t;
use crate::ui::list_panel::{Event, Item, ListPanel};
use crate::ui::prompts::Effect;
use crate::ui::render::panel::{bordered, key_hints};
use crate::ui::rows::title_plain;
use crate::ui::terminal::{KeyCode, KeyEvent, Modifiers};
use crate::ui::text::{truncate_cells, wrap_cells};
use crate::ui::theme::Palette;
use crate::ui::theme::ansi::fg;
use std::cell::Cell;

const HINT: &str = "Enter output / interrupt member · x stop command · Esc close";

pub struct TasksPanel {
    list: ListPanel,
    tasks: Vec<TaskView>,
    detail: Option<Detail>,
}

struct Detail {
    target: TaskTarget,
    title: String,
    text: String,
    state: TaskOutputState,
    scroll: Cell<usize>,
    page: Cell<usize>,
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
            detail: None,
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
                        TaskKind::Command(_) => t("command"),
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

    /// A paste goes into the filter.
    pub fn paste(&mut self, text: &str) {
        self.list.paste(text);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if let Some(detail) = &mut self.detail {
            match key.code {
                KeyCode::Esc => self.detail = None,
                KeyCode::Char('x')
                    if key.mods == Modifiers::NONE
                        && !matches!(&detail.state, TaskOutputState::Finished { .. }) =>
                {
                    return Effect::StopTask(detail.target.clone());
                }
                KeyCode::Up => detail.scroll.set(detail.scroll.get().saturating_add(1)),
                KeyCode::Down => detail.scroll.set(detail.scroll.get().saturating_sub(1)),
                KeyCode::PageUp => detail
                    .scroll
                    .set(detail.scroll.get().saturating_add(detail.page.get())),
                KeyCode::PageDown => detail
                    .scroll
                    .set(detail.scroll.get().saturating_sub(detail.page.get())),
                KeyCode::Home => detail.scroll.set(usize::MAX),
                KeyCode::End => detail.scroll.set(0),
                _ => {}
            }
            return Effect::None;
        }
        if key.code == KeyCode::Char('x')
            && key.mods == Modifiers::NONE
            && self.list.filter_is_empty()
            && let Some(id) = self.list.selected_id()
            && let Some(TaskView {
                kind: TaskKind::Command(target),
                ..
            }) = self.tasks.iter().find(|task| task.id == id)
        {
            return Effect::StopTask(target.clone());
        }
        match self.list.handle_key(key) {
            Event::None => Effect::None,
            Event::Close => Effect::Close,
            Event::Refresh => Effect::None,
            Event::Pick(id) => match self.tasks.iter().find(|task| task.id == id).cloned() {
                Some(TaskView {
                    kind: TaskKind::Command(target),
                    label,
                    ..
                }) => {
                    self.detail = Some(Detail {
                        target,
                        title: label,
                        text: String::new(),
                        state: TaskOutputState::Loading,
                        scroll: Cell::new(0),
                        page: Cell::new(1),
                    });
                    Effect::None
                }
                Some(TaskView {
                    kind:
                        TaskKind::Member {
                            session_id: Some(session_id),
                        },
                    ..
                }) => Effect::StopMember(session_id),
                Some(TaskView {
                    kind: TaskKind::Member { session_id: None },
                    ..
                }) => {
                    Effect::Notice(t("Agent is starting; no member Session to stop yet").to_owned())
                }
                _ => Effect::None,
            },
        }
    }

    pub fn cursor(&self, width: usize) -> Option<(usize, usize)> {
        if self.detail.is_some() {
            None
        } else {
            self.list.cursor(width)
        }
    }

    pub fn output_target(&self) -> Option<&TaskTarget> {
        self.detail
            .as_ref()
            .filter(|detail| !matches!(&detail.state, TaskOutputState::Finished { .. }))
            .map(|detail| &detail.target)
    }

    pub fn output(&mut self, target: &TaskTarget, text: Option<String>, state: TaskOutputState) {
        if let Some(detail) = &mut self.detail
            && detail.target == *target
        {
            if let Some(text) = text {
                detail.text = text;
            }
            detail.state = state;
        }
    }

    pub fn render(&self, width: usize, rows: usize, p: &Palette) -> Vec<String> {
        if let Some(detail) = &self.detail {
            let inner = width.saturating_sub(4).max(1);
            let (status, color) = match &detail.state {
                TaskOutputState::Loading | TaskOutputState::Live => (
                    t("Tail preview - last 2,000 lines / 50 KiB").to_owned(),
                    &p.muted,
                ),
                TaskOutputState::Unavailable => (t("Live output unavailable").to_owned(), &p.muted),
                TaskOutputState::Unsupported => (
                    t("Live output unavailable in this profile").to_owned(),
                    &p.muted,
                ),
                TaskOutputState::ReadError(error) => {
                    (format!("{}: {error}", t("Output read failed")), &p.danger)
                }
                TaskOutputState::Finished { status, failed } => (
                    t(status).to_owned(),
                    if *failed { &p.danger } else { &p.muted },
                ),
            };
            let text = if detail.text.is_empty() {
                match &detail.state {
                    TaskOutputState::Loading => t("Loading output..."),
                    TaskOutputState::Live => t("No output yet"),
                    _ => "",
                }
            } else {
                &detail.text
            };
            let lines = wrap_cells(&safe(text), inner);
            let page = rows.saturating_sub(3).max(1);
            detail.page.set(page);
            let scroll = detail.scroll.get().min(lines.len().saturating_sub(page));
            detail.scroll.set(scroll);
            let end = lines.len().saturating_sub(scroll);
            let mut content = vec![fg(&truncate_cells(&safe(&status), inner), color)];
            content.extend(
                lines[end.saturating_sub(page)..end]
                    .iter()
                    .map(|line| fg(line, &p.text)),
            );
            let hint = if matches!(&detail.state, TaskOutputState::Finished { .. }) {
                "Esc back"
            } else {
                "x stop command · Esc back"
            };
            return bordered(
                &content,
                width,
                Some(rows),
                &p.border,
                Some(&title_plain(&safe(&detail.title), inner, p)),
                Some(&key_hints(t(hint), p)),
            );
        }
        self.list.render(width, rows, p)
    }
}
