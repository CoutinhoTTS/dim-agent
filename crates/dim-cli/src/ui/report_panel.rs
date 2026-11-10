use std::cell::Cell;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};

use crate::report::{Action, Captured, Draft, Plan, Preview, Reply};
use crate::ui::i18n::t;
use crate::ui::line_input::LineInput;
use crate::ui::render::panel::{bordered, follow_window, wrap};
use crate::ui::terminal::{KeyCode, KeyEvent};
use crate::ui::text::truncate_cells;
use crate::ui::theme::{Palette, ansi::fg};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Prepare,
    Plan,
    Generate,
}

pub struct Panel {
    pub id: u64,
    pub draft: Option<Draft>,
    message: String,
    model: Option<dim_agent_types::ModelId>,
    preview: Option<Preview>,
    plan: Option<Plan>,
    busy: Option<Busy>,
    note: String,
    redact: bool,
    editing: bool,
    options: LineInput,
    instructions: String,
    focus: usize,
    file: usize,
    offset: usize,
    page: Cell<usize>,
    caret: Cell<Option<(usize, usize)>>,
    saved: Option<PathBuf>,
}

pub enum Input {
    None,
    Send(Action),
    Close,
}

impl Panel {
    pub fn new(id: u64, error: &Captured) -> Self {
        Self {
            id,
            draft: None,
            message: crate::sanitize::safe_line(&error.message),
            model: None,
            preview: None,
            plan: None,
            busy: Some(Busy::Prepare),
            note: String::new(),
            redact: true,
            editing: false,
            options: LineInput::new(""),
            instructions: String::new(),
            focus: 0,
            file: 0,
            offset: 0,
            page: Cell::new(1),
            caret: Cell::new(None),
            saved: None,
        }
    }

    pub fn reply(&mut self, reply: Reply) {
        self.busy = None;
        self.note.clear();
        match reply {
            Reply::Prepared(session, prepared) => {
                self.model = prepared.default_model;
                self.draft = Some(Draft {
                    session,
                    report_id: prepared.report_id,
                    plan_id: prepared.plan.plan_id.clone(),
                });
                self.plan = Some(prepared.plan);
            }
            Reply::Plan(plan) => {
                if let Some(draft) = &mut self.draft {
                    draft.plan_id.clone_from(&plan.plan_id);
                }
                self.plan = Some(plan);
            }
            Reply::Generated(preview) => {
                self.draft = None;
                self.preview = Some(preview);
                self.focus = 0;
                self.offset = 0;
            }
            Reply::Failed(error) => self.note = crate::sanitize::safe_line(&error),
        }
    }

    pub fn disconnected(&mut self, message: &str) {
        self.draft = None;
        self.busy = None;
        self.note = crate::sanitize::safe_line(message);
    }

    pub fn paste(&mut self, text: &str) {
        if self.editing {
            self.options.paste(text);
        }
    }

    fn replan(&mut self) -> Input {
        let Some(draft) = self.draft.clone() else {
            return Input::None;
        };
        self.busy = Some(Busy::Plan);
        self.plan = None;
        self.note.clear();
        Input::Send(Action::Plan {
            draft,
            redact: self.redact,
            instructions: self.instructions.clone(),
        })
    }

    pub fn key(&mut self, key: KeyEvent, workspace: &Path) -> Input {
        if key.code == KeyCode::Esc {
            if self.editing {
                self.editing = false;
                return Input::None;
            }
            if self.busy == Some(Busy::Generate)
                && let Some(draft) = self.draft.clone()
            {
                self.note = t("Cancelling generation…").into();
                return Input::Send(Action::Cancel {
                    draft,
                    generation: true,
                });
            }
            return Input::Close;
        }
        if self.editing {
            if key.code == KeyCode::Enter {
                if self.options.text().len() > 1024 {
                    self.note = t("Options must fit within 1 KiB").into();
                    return Input::None;
                }
                self.instructions = self.options.text().to_owned();
                self.editing = false;
                return self.replan();
            }
            self.options.handle_key(key);
            return Input::None;
        }
        let count = self.controls().len();
        match key.code {
            KeyCode::Tab => self.focus = (self.focus + 1) % count,
            KeyCode::BackTab => self.focus = (self.focus + count - 1) % count,
            KeyCode::Up => self.offset = self.offset.saturating_sub(1),
            KeyCode::Down => self.offset = self.offset.saturating_add(1),
            KeyCode::PageUp => self.offset = self.offset.saturating_sub(self.page.get()),
            KeyCode::PageDown => self.offset = self.offset.saturating_add(self.page.get()),
            KeyCode::Enter if self.preview.is_some() => return self.preview_key(workspace),
            KeyCode::Enter if self.focus == 4 || self.draft.is_none() && self.busy.is_none() => {
                return Input::Close;
            }
            KeyCode::Enter if self.busy.is_none() => match self.focus {
                0 => self.note = t("Local generation makes zero model requests.").into(),
                1 => {
                    self.redact = !self.redact;
                    return self.replan();
                }
                2 => {
                    self.options = LineInput::new(&self.instructions);
                    self.editing = true;
                }
                3 if self.plan.is_none() => return self.replan(),
                3 => {
                    if let Some(draft) = self.draft.clone() {
                        self.busy = Some(Busy::Generate);
                        self.plan = None;
                        self.note.clear();
                        return Input::Send(Action::Generate(draft));
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Input::None
    }

    fn preview_key(&mut self, workspace: &Path) -> Input {
        let preview = self.preview.as_ref().expect("open preview");
        match self.focus {
            0..=2 => {
                self.file = self.focus;
                self.offset = 0;
            }
            3 => {
                self.note = match self
                    .saved
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(|| save_zip(workspace, preview))
                {
                    Ok(path) => {
                        self.saved = Some(path.clone());
                        format!("{} {}", t("Saved ZIP"), path.display())
                    }
                    Err(error) => format!("{} {error}", t("Could not save ZIP")),
                };
            }
            4 | 5 => {
                let (url, host, path) = if self.focus == 4 {
                    (
                        &preview.sharing.github_issue_url,
                        "github.com",
                        "/arcships/dim/issues/new",
                    )
                } else {
                    (&preview.sharing.discord_url, "discord.gg", "/ds35K3TadP")
                };
                let valid = url::Url::parse(url).ok().filter(|url| {
                    url.scheme() == "https"
                        && url.host_str() == Some(host)
                        && url.path() == path
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.port().is_none()
                });
                self.note = if valid.is_some_and(|url| crate::browser::open(url.as_str())) {
                    if self.focus == 5 {
                        crate::clipboard::copy_local(&preview.sharing.description);
                        t("Opened Discord")
                    } else {
                        t("Opened GitHub issue draft")
                    }
                } else {
                    t("Could not open the share destination")
                }
                .into();
            }
            _ => return Input::Close,
        }
        Input::None
    }

    fn controls(&self) -> Vec<&str> {
        if self.preview.is_some() {
            vec![
                t("Preview"),
                "report.md",
                "diagnostics.json",
                t("Save ZIP"),
                "GitHub",
                "Discord",
                t("Close"),
            ]
        } else {
            vec![
                t("Model · Generate locally"),
                if self.redact {
                    t("Redact [x]")
                } else {
                    t("Redact [ ]")
                },
                t("Options"),
                t("Generate report"),
                t("Cancel"),
            ]
        }
    }

    pub fn cursor(&self) -> Option<(usize, usize)> {
        self.caret.get()
    }

    pub fn render(&self, width: usize, budget: usize, p: &Palette) -> Vec<String> {
        self.caret.set(None);
        if budget < 3 || width < 8 {
            return vec![truncate_cells(t("Report this error"), width)];
        }
        let inner = width - 4;
        let height = budget - 2;
        let mut rows = Vec::new();
        let title = if self.preview.is_some() {
            t("Report preview")
        } else {
            t("Report this error")
        };
        if self.editing {
            rows.push(truncate_cells(
                t("Options · for example: omit timeline, keep error codes"),
                inner,
            ));
            if height > 1 {
                let (text, column) = self.options.view(inner);
                rows.push(text);
                self.caret.set(Some((2, 2 + column)));
                if height > 2 && !self.note.is_empty() {
                    rows.push(truncate_cells(&self.note, inner));
                }
            }
        } else {
            let controls = self.controls();
            let visible = controls.len().min((height / 2).max(1));
            let first = follow_window(0, self.focus, visible, controls.len());
            let body_rows = height.saturating_sub(visible);
            self.page.set(body_rows.max(1));
            let body = self.body();
            let lines: Vec<_> = crate::sanitize::safe(&body)
                .split('\n')
                .flat_map(|line| wrap(line, inner))
                .collect();
            let from = self
                .offset
                .min(lines.len().saturating_sub(body_rows.max(1)));
            rows.extend(lines.into_iter().skip(from).take(body_rows));
            for (index, label) in controls.into_iter().enumerate().skip(first).take(visible) {
                let selected = self.focus == index;
                let line = truncate_cells(
                    &format!("{} {label}", if selected { "›" } else { " " }),
                    inner,
                );
                rows.push(fg(&line, if selected { &p.accent } else { &p.text_soft }));
            }
        }
        let hint = if self.editing {
            t("Enter apply · Esc back")
        } else {
            t("Tab focus · Enter select · ↑↓ scroll · Esc close")
        };
        bordered(&rows, width, None, &p.border, Some(title), Some(hint))
    }

    fn body(&self) -> String {
        let mut body = if let Some(preview) = &self.preview {
            match self.file {
                1 => preview.files.get("report.md").cloned().unwrap_or_default(),
                2 => preview
                    .files
                    .get("diagnostics.json")
                    .cloned()
                    .unwrap_or_default(),
                _ => format!("{}\n\n{}", preview.title, preview.body),
            }
        } else {
            let mut body = self.message.clone();
            if let Some(model) = &self.model {
                body.push_str(&format!(
                    "\n{} {}/{}",
                    t("Session model"),
                    model.provider_id,
                    model.model_id
                ));
            }
            body.push_str(&format!(
                "\n{}",
                t("Local generation makes zero model requests.")
            ));
            if let Some(plan) = &self.plan {
                body.push('\n');
                body.push_str(if plan.evidence.timeline.is_some() {
                    t("Evidence · Timeline failure captured")
                } else {
                    t("Evidence · Observed error only")
                });
                if matches!(
                    plan.instructions_status.as_str(),
                    "partially_applied" | "unsupported"
                ) {
                    body.push('\n');
                    body.push_str(t("Some options were unsupported."));
                }
            }
            body
        };
        let note = match self.busy {
            Some(Busy::Prepare) => t("Preparing report…"),
            Some(Busy::Plan) => t("Updating report plan…"),
            Some(Busy::Generate) if self.note.is_empty() => t("Generating report…"),
            _ => &self.note,
        };
        if !note.is_empty() {
            body = format!("{note}\n{body}");
        }
        body
    }
}

fn save_zip(workspace: &Path, preview: &Preview) -> crate::Result<PathBuf> {
    let bytes = STANDARD.decode(&preview.archive.base64)?;
    if bytes.len() > 64 * 1024 || !bytes.starts_with(b"PK\x03\x04") {
        return Err("Invalid report ZIP".into());
    }
    let path = workspace.join(format!("dim-report-{}.zip", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(error.into());
    }
    Ok(path)
}
