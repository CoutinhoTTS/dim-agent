//! The `/tasks` panel: what runs apart from the turn, a background command
//! or a member agent, from the session's snapshot. Command details show the
//! existing tail preview; members have a separate read-only conversation.

use crate::model::view::{
    MemberConversationState, RowKind, TaskKind, TaskOutputState, TaskTarget, TaskView, ViewSnapshot,
};
use crate::sanitize::safe;
use crate::ui::i18n::t;
use crate::ui::list_panel::{Event, Item, ListPanel};
use crate::ui::prompts::Effect;
use crate::ui::render::markdown::MarkdownRenderer;
use crate::ui::render::panel::{bordered, key_hints};
use crate::ui::rows::title_plain;
use crate::ui::terminal::{KeyCode, KeyEvent, Modifiers};
use crate::ui::text::{truncate_cells, wrap_cells};
use crate::ui::theme::Palette;
use crate::ui::theme::ansi::fg;
use crate::ui::transcript::block::{opens_group, project_row};
use crate::ui::transcript::tool_group::Detail as ToolDetail;
use std::cell::Cell;

const HINT: &str = "Enter details · x stop selected · Ctrl+X stop all · Esc close";

pub struct TasksPanel {
    session_id: String,
    pub projection_id: u64,
    history_incomplete: bool,
    updated_second: u64,
    list: ListPanel,
    tasks: Vec<TaskView>,
    detail: Option<Box<Detail>>,
}

struct Detail {
    task: TaskView,
    body: DetailBody,
    scroll: Cell<usize>,
    page: Cell<usize>,
}

enum DetailBody {
    Command {
        text: String,
        state: TaskOutputState,
    },
    Member {
        snapshot: Box<ViewSnapshot>,
        state: MemberConversationState,
    },
    Result,
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
                .find(|task| task.id == detail.task.id && task.branch_id == detail.task.branch_id)
        {
            detail.task = task.clone();
            if let DetailBody::Command { text, state } = &mut detail.body
                && let Some(finish) = &task.finished
            {
                *text = finish.text.clone();
                *state = TaskOutputState::Finished {
                    status: finish.status,
                    failed: finish.failed,
                };
            }
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

    fn stop(&self, task: &TaskView) -> Effect {
        if task.finished.is_some() {
            return Effect::None;
        }
        match &task.kind {
            TaskKind::Command(target) => Effect::StopTask {
                session_id: self.session_id.clone(),
                projection_id: self.projection_id,
                target: target.clone(),
            },
            TaskKind::Member {
                session_id: Some(member_id),
            } => Effect::StopMember {
                session_id: self.session_id.clone(),
                projection_id: self.projection_id,
                member_id: member_id.clone(),
            },
            TaskKind::Member { session_id: None } => {
                Effect::Notice(t("Agent is starting; no member Session to stop yet").into())
            }
        }
    }

    /// A paste edits the visible list filter.
    pub fn paste(&mut self, text: &str) {
        if self.detail.is_none() {
            self.list.paste(text);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if let Some(detail) = &self.detail {
            if key.code == KeyCode::Char('x') && key.mods == Modifiers::NONE {
                if matches!(
                    &detail.body,
                    DetailBody::Member {
                        state: MemberConversationState::Closed,
                        ..
                    }
                ) {
                    return Effect::None;
                }
                return self.stop(&detail.task);
            }
            match key.code {
                KeyCode::Esc => self.detail = None,
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
            && let Some(task) = self.tasks.iter().find(|task| task.id == id)
        {
            return self.stop(task);
        }
        match self.list.handle_key(key) {
            Event::None | Event::Refresh => Effect::None,
            Event::Close => Effect::Close,
            Event::Pick(id) => {
                let Some(task) = self.tasks.iter().find(|task| task.id == id).cloned() else {
                    return Effect::None;
                };
                let body =
                    match &task.kind {
                        TaskKind::Command(_) => DetailBody::Command {
                            text: task
                                .finished
                                .as_ref()
                                .map_or(String::new(), |finish| finish.text.clone()),
                            state: task.finished.as_ref().map_or(
                                TaskOutputState::Loading,
                                |finish| TaskOutputState::Finished {
                                    status: finish.status,
                                    failed: finish.failed,
                                },
                            ),
                        },
                        TaskKind::Member {
                            session_id: Some(_),
                        } => DetailBody::Member {
                            snapshot: Box::default(),
                            state: MemberConversationState::Loading,
                        },
                        TaskKind::Member { session_id: None } if task.finished.is_some() => {
                            DetailBody::Result
                        }
                        TaskKind::Member { session_id: None } => {
                            return Effect::Notice(
                                t("Agent is starting; no member Session to stop yet").into(),
                            );
                        }
                    };
                self.detail = Some(Box::new(Detail {
                    task,
                    body,
                    scroll: Cell::new(0),
                    page: Cell::new(1),
                }));
                Effect::None
            }
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
        let detail = self.detail.as_ref()?;
        match (&detail.task.kind, &detail.body) {
            (TaskKind::Command(target), DetailBody::Command { state, .. })
                if !matches!(state, TaskOutputState::Finished { .. }) =>
            {
                Some(target)
            }
            _ => None,
        }
    }

    pub fn member_target(&self) -> Option<&str> {
        let detail = self.detail.as_ref()?;
        match (&detail.task.kind, &detail.body) {
            (
                TaskKind::Member {
                    session_id: Some(id),
                },
                DetailBody::Member { .. },
            ) => Some(id),
            _ => None,
        }
    }

    pub fn output(&mut self, target: &TaskTarget, text: Option<String>, state: TaskOutputState) {
        if let Some(Detail {
            task:
                TaskView {
                    kind: TaskKind::Command(at),
                    ..
                },
            body:
                DetailBody::Command {
                    text: current,
                    state: status,
                },
            ..
        }) = self.detail.as_deref_mut()
            && at == target
        {
            if let Some(text) = text {
                *current = text;
            }
            *status = state;
        }
    }

    pub fn member(
        &mut self,
        session_id: &str,
        snapshot: Box<ViewSnapshot>,
        state: MemberConversationState,
    ) {
        if let Some(Detail {
            task:
                TaskView {
                    kind:
                        TaskKind::Member {
                            session_id: Some(id),
                        },
                    ..
                },
            body:
                DetailBody::Member {
                    snapshot: current,
                    state: status,
                },
            ..
        }) = self.detail.as_deref_mut()
            && id == session_id
        {
            *current = snapshot;
            *status = state;
        }
    }

    pub fn render(
        &self,
        width: usize,
        rows: usize,
        p: &Palette,
        markdown: &MarkdownRenderer,
    ) -> Vec<String> {
        let Some(detail) = &self.detail else {
            return self.list.render(width, rows, p);
        };
        let inner = width.saturating_sub(4).max(1);
        let mut content = Vec::new();
        let mut lines = Vec::new();
        let mut hint = "Esc back";
        let mut title = safe(&detail.task.label);
        match &detail.body {
            DetailBody::Command { text, state } => {
                let (status, color) = match state {
                    TaskOutputState::Loading | TaskOutputState::Live => (
                        t("Tail preview - last 2,000 lines / 50 KiB").to_owned(),
                        &p.muted,
                    ),
                    TaskOutputState::Unavailable => {
                        (t("Live output unavailable").to_owned(), &p.muted)
                    }
                    TaskOutputState::Unsupported => (
                        t("Live output unavailable in this profile").to_owned(),
                        &p.muted,
                    ),
                    TaskOutputState::ReadError(error) => (
                        format!("{}: {}", t("Output read failed"), safe(error)),
                        &p.danger,
                    ),
                    TaskOutputState::Finished { status, failed } => (
                        t(status).to_owned(),
                        if *failed { &p.danger } else { &p.muted },
                    ),
                };
                content.push(fg(&truncate_cells(&status, inner), color));
                let text = if text.is_empty() {
                    match state {
                        TaskOutputState::Loading => t("Loading output..."),
                        TaskOutputState::Live => t("No output yet"),
                        _ => "",
                    }
                } else {
                    text
                };
                lines.extend(
                    wrap_cells(&safe(text), inner)
                        .into_iter()
                        .map(|line| fg(&line, &p.text)),
                );
                if detail.task.finished.is_none() {
                    hint = "x stop command · Esc back";
                }
            }
            DetailBody::Member { snapshot, state } => {
                title = format!("{} - {}", title, t("read-only"));
                let TaskKind::Member {
                    session_id: Some(id),
                } = &detail.task.kind
                else {
                    unreachable!("member detail has a delivered identity")
                };
                content.push(fg(
                    &truncate_cells(&format!("{}: {}", t("Member Session"), safe(id)), inner),
                    &p.muted,
                ));
                let status = match state {
                    MemberConversationState::Loading => t("Loading conversation...").to_owned(),
                    MemberConversationState::Live if snapshot.rows.is_empty() => {
                        t("No conversation yet").to_owned()
                    }
                    MemberConversationState::Live if snapshot.running => t("Running").to_owned(),
                    MemberConversationState::Live => t("Observing conversation").to_owned(),
                    MemberConversationState::ReadError(error) => {
                        format!("{}: {}", t("Conversation unavailable"), safe(error))
                    }
                    MemberConversationState::Closed => t("Session closed").to_owned(),
                };
                content.push(fg(
                    &truncate_cells(&status, inner),
                    if matches!(state, MemberConversationState::ReadError(_)) {
                        &p.danger
                    } else {
                        &p.muted
                    },
                ));
                lines.push(fg(t("Member conversation"), &p.muted));
                for (index, row) in snapshot.rows.iter().enumerate() {
                    if row.kind == RowKind::ToolCall && !opens_group(&snapshot.rows, index) {
                        continue;
                    }
                    let block = project_row(&snapshot.rows, index, ToolDetail::Expanded);
                    lines.extend(crate::ui::transcript::blocks::render_block(
                        &block, inner, p, markdown,
                    ));
                    lines.push(String::new());
                }
                if let Some(finish) = &detail.task.finished {
                    lines.push(fg(
                        &format!("{}: {}", t("Delegation result"), t(finish.status)),
                        &p.muted,
                    ));
                    lines.extend(
                        wrap_cells(&safe(&finish.text), inner)
                            .into_iter()
                            .map(|line| fg(&line, &p.text)),
                    );
                    lines.push(String::new());
                }
                if detail.task.finished.is_none()
                    && !matches!(state, MemberConversationState::Closed)
                {
                    hint = "x interrupt member · Esc back";
                }
            }
            DetailBody::Result => {
                let finish = detail
                    .task
                    .finished
                    .as_ref()
                    .expect("a delivery result is terminal");
                title = format!("{}: {title}", t("Delegation result"));
                content.push(fg(
                    t(finish.status),
                    if finish.failed { &p.danger } else { &p.muted },
                ));
                lines.extend(
                    wrap_cells(&safe(&finish.text), inner)
                        .into_iter()
                        .map(|line| fg(&line, &p.text)),
                );
            }
        }
        let page = rows.saturating_sub(content.len() + 2).max(1);
        detail.page.set(page);
        let scroll = detail.scroll.get().min(lines.len().saturating_sub(page));
        detail.scroll.set(scroll);
        let end = lines.len().saturating_sub(scroll);
        content.extend(lines[end.saturating_sub(page)..end].iter().cloned());
        bordered(
            &content,
            width,
            Some(rows),
            &p.border,
            Some(&title_plain(&title, inner, p)),
            Some(&key_hints(t(hint), p)),
        )
    }
}
