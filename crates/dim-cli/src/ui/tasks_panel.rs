//! The `/tasks` panel: what runs apart from the turn, a background command
//! or a member agent, from the session's snapshot. Command details show the
//! existing tail preview; member controls retain their current path.

use crate::model::view::{TaskKind, TaskOutputState, TaskTarget, TaskView, ViewSnapshot};
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

const HINT: &str = "Enter output / interrupt member · x stop command · Ctrl+X stop all · Esc close";

pub struct TasksPanel {
    session_id: String,
    pub projection_id: u64,
    history_incomplete: bool,
    updated_second: u64,
    list: ListPanel,
    tasks: Vec<TaskView>,
    detail: Option<Detail>,
}

struct Detail {
    target: Option<TaskTarget>,
    title: String,
    text: String,
    state: TaskOutputState,
    scroll: Cell<usize>,
    page: Cell<usize>,
}

impl TasksPanel {
    pub fn new(snapshot: &ViewSnapshot) -> Self {
        let mut panel = Self {
            session_id: snapshot.session_id.clone(),
            projection_id: snapshot.projection_id,
            history_incomplete: false,
            updated_second: 0,
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
        panel.set_tasks(snapshot);
        panel
    }

    pub fn set_tasks(&mut self, snapshot: &ViewSnapshot) {
        let selected = self.list.selected_id();
        self.tasks = snapshot.tasks.clone();
        self.history_incomplete =
            snapshot.task_history_loading != 0 || !snapshot.task_history_errors.is_empty();
        let mut items = Vec::new();
        if snapshot.task_history_loading != 0 {
            let mut item = Item::heading(t("Loading branch history... Stop-all unavailable"));
            item.sticky = true;
            items.push(item);
        }
        for error in &snapshot.task_history_errors {
            let mut item = Item::heading(format!(
                "{}: {}",
                t("Branch history failed; reopen to retry"),
                safe(error)
            ));
            item.sticky = true;
            items.push(item);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.updated_second = now / 1000;
        items.extend(self.tasks.iter().map(|task| {
            let kind = match &task.kind {
                TaskKind::Command(_) => t("command"),
                TaskKind::Member { session_id: None } if task.finished.is_none() => {
                    t("agent (starting)")
                }
                TaskKind::Member { .. } => t("agent"),
            };
            let status = task.finished.as_ref().map_or_else(
                || match &task.kind {
                    TaskKind::Member { session_id: None } => t("Starting"),
                    _ => t("Running"),
                },
                |finish| t(finish.status),
            );
            let elapsed = task
                .finished
                .as_ref()
                .map_or(now, |finish| finish.at)
                .saturating_sub(task.started_at)
                / 1000;
            let exit = task
                .finished
                .as_ref()
                .and_then(|finish| finish.exit_code)
                .map_or(String::new(), |code| format!(" · exit {code}"));
            Item::new(
                task.id.clone(),
                safe(&task.label),
                format!("{kind} · {status} · {elapsed}s{exit}"),
            )
        }));
        self.list.set_items(Ok(items), selected.as_deref());
        if let Some(detail) = &mut self.detail
            && let Some(task) = self
                .tasks
                .iter()
                .find(|task| match (&task.kind, &detail.target) {
                    (TaskKind::Command(target), Some(at)) => target == at,
                    _ => false,
                })
            && let Some(finish) = &task.finished
        {
            detail.text = finish.text.clone();
            detail.state = TaskOutputState::Finished {
                status: finish.status,
                failed: finish.failed,
            };
        }
    }

    pub fn tick(&mut self, snapshot: &ViewSnapshot) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now != self.updated_second && self.tasks.iter().any(|task| task.finished.is_none()) {
            self.set_tasks(snapshot);
            true
        } else {
            false
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if let Some(detail) = &mut self.detail {
            match key.code {
                KeyCode::Esc => self.detail = None,
                KeyCode::Char('x')
                    if key.mods == Modifiers::NONE
                        && !matches!(&detail.state, TaskOutputState::Finished { .. }) =>
                {
                    if let Some(target) = &detail.target {
                        return Effect::StopTask {
                            session_id: self.session_id.clone(),
                            projection_id: self.projection_id,
                            target: target.clone(),
                        };
                    }
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
        if key.code == KeyCode::Char('x') && key.mods == Modifiers::CTRL {
            return if self.history_incomplete {
                Effect::Notice(t("Branch history is incomplete; stop-all unavailable").into())
            } else {
                Effect::StopTasks {
                    session_id: self.session_id.clone(),
                    projection_id: self.projection_id,
                }
            };
        }
        if key.code == KeyCode::Char('x')
            && key.mods == Modifiers::NONE
            && self.list.filter_is_empty()
            && let Some(id) = self.list.selected_id()
            && let Some(TaskView {
                kind: TaskKind::Command(target),
                finished: None,
                ..
            }) = self.tasks.iter().find(|task| task.id == id)
        {
            return Effect::StopTask {
                session_id: self.session_id.clone(),
                projection_id: self.projection_id,
                target: target.clone(),
            };
        }
        match self.list.handle_key(key) {
            Event::None => Effect::None,
            Event::Close => Effect::Close,
            Event::Refresh => Effect::None,
            Event::Pick(id) => match self.tasks.iter().find(|task| task.id == id).cloned() {
                Some(TaskView {
                    kind: TaskKind::Command(target),
                    label,
                    finished,
                    ..
                }) => {
                    self.detail = Some(Detail {
                        target: Some(target),
                        title: label,
                        text: finished
                            .as_ref()
                            .map_or(String::new(), |finish| finish.text.clone()),
                        state: finished.map_or(TaskOutputState::Loading, |finish| {
                            TaskOutputState::Finished {
                                status: finish.status,
                                failed: finish.failed,
                            }
                        }),
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
                    finished: None,
                    ..
                }) => Effect::StopMember {
                    session_id: self.session_id.clone(),
                    projection_id: self.projection_id,
                    member_id: session_id,
                },
                Some(TaskView {
                    finished: Some(finish),
                    label,
                    ..
                }) => {
                    self.detail = Some(Detail {
                        target: None,
                        title: format!("{}: {label}", t("Delegation result")),
                        text: finish.text,
                        state: TaskOutputState::Finished {
                            status: finish.status,
                            failed: finish.failed,
                        },
                        scroll: Cell::new(0),
                        page: Cell::new(1),
                    });
                    Effect::None
                }
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
            .and_then(|detail| detail.target.as_ref())
    }

    pub fn output(&mut self, target: &TaskTarget, text: Option<String>, state: TaskOutputState) {
        if let Some(detail) = &mut self.detail
            && detail.target.as_ref() == Some(target)
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
