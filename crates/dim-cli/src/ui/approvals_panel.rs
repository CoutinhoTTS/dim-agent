//! The permission preset of the session's `main` branch
//! (`session.permission.get/update`), with a confirmation before full access.
//!
//! `/approvals` lists the three presets. `/approval` lists two choices over
//! the same presets: Ask is `workspace-write`, and Full access is
//! `full-access`.
//!
//! ```text
//! Approval
//! > Ask            current
//!   Full access
//! ↑↓/Tab move · Enter apply · Esc back
//! ```

use crate::ui::i18n::t;
use crate::ui::prompts::{CONFIRM_BODY, CONFIRM_HINT, Effect};
use crate::ui::render::panel::{IDLE_MARKER, SELECTED_MARKER, bordered, confirm_panel, key_hints};
use crate::ui::rows::title_plain;
use crate::ui::terminal::{KeyCode, KeyEvent};
use crate::ui::text::truncate_cells;
use crate::ui::theme::Palette;
use crate::ui::theme::ansi::{Paint, fg, paint};

/// `(preset, title, status label, description)`. The texts are English keys:
/// draw them through [`t`].
type Mode = (&'static str, &'static str, &'static str, &'static str);

/// The presets of `permission::PRESETS` in display order, by what each decides.
pub const MODES: [Mode; 3] = [
    (
        "full-access",
        "Full access",
        "full access",
        "Every tool call runs without asking",
    ),
    (
        "workspace-write",
        "Workspace write",
        "workspace write",
        "Reads and workspace file writes run; outside writes, commands, network and unclassified tools ask",
    ),
    (
        "read-only",
        "Read only",
        "read only",
        "Reads run; writes, commands, network and unclassified tools are denied",
    ),
];

/// `/approval`: Ask is the product default (`workspace-write`); Full access
/// is `full-access`. `read-only` stays on `/approvals` and `--permission`.
const CHOICES: [Mode; 2] = [
    (
        "workspace-write",
        "Ask",
        "workspace write",
        "Ask before commands, network, outside writes and unclassified tools",
    ),
    (
        "full-access",
        "Full access",
        "full access",
        "Every tool call runs without asking",
    ),
];

const HINT: &str = "↑↓/Tab move · Enter apply · Esc back";

/// The status-line label of a mode.
pub fn status_label(mode: &str) -> &'static str {
    MODES
        .iter()
        .find(|(m, ..)| *m == mode)
        .map_or(t("default"), |(_, _, status, _)| t(status))
}

/// The name a settled preset is announced with. Ask is the name of
/// `workspace-write` in `/approval`; the other presets keep their row titles.
pub fn announced_title(mode: &str) -> &'static str {
    let key = if mode == "workspace-write" {
        "Ask"
    } else {
        MODES
            .iter()
            .find(|(preset, ..)| *preset == mode)
            .map_or("", |(_, title, ..)| *title)
    };
    t(key)
}

pub struct ApprovalsPanel {
    modes: &'static [Mode],
    title: &'static str,
    current: String,
    selected: usize,
    /// `Some(focused button)` while full access awaits confirmation:
    /// 0 = Confirm, 1 = Cancel.
    confirming: Option<usize>,
}

impl ApprovalsPanel {
    /// `/approvals`: the three presets. An unknown current mode lands on
    /// workspace-write.
    pub fn new(current: &str) -> Self {
        Self::open(current, &MODES, "Tool Approvals", 1)
    }

    /// `/approval`: Ask or Full access. An unknown current mode, including
    /// read-only, lands on Ask without marking it current.
    pub fn choice(current: &str) -> Self {
        Self::open(current, &CHOICES, "Approval", 0)
    }

    fn open(current: &str, modes: &'static [Mode], title: &'static str, fallback: usize) -> Self {
        let selected = modes
            .iter()
            .position(|(mode, ..)| *mode == current)
            .unwrap_or(fallback);
        Self {
            modes,
            title,
            current: current.to_owned(),
            selected,
            confirming: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if let Some(focus) = &mut self.confirming {
            return match key.code {
                KeyCode::Esc => {
                    self.confirming = None;
                    Effect::None
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    *focus = 1 - *focus;
                    Effect::None
                }
                KeyCode::Char('y') => Effect::SetApprovals("full-access".into()),
                KeyCode::Char('n') => {
                    self.confirming = None;
                    Effect::None
                }
                KeyCode::Enter => {
                    if *focus == 0 {
                        Effect::SetApprovals("full-access".into())
                    } else {
                        self.confirming = None;
                        Effect::None
                    }
                }
                _ => Effect::None,
            };
        }
        match key.code {
            KeyCode::Esc => Effect::Close,
            KeyCode::Up | KeyCode::BackTab => {
                self.selected = self.selected.saturating_sub(1);
                Effect::None
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(self.modes.len() - 1);
                Effect::None
            }
            KeyCode::Enter => {
                let mode = self.modes[self.selected].0;
                if mode == self.current {
                    Effect::Close
                } else if mode == "full-access" {
                    self.confirming = Some(1);
                    Effect::None
                } else {
                    Effect::SetApprovals(mode.to_owned())
                }
            }
            _ => Effect::None,
        }
    }

    pub fn render(&self, width: usize, max_rows: usize, p: &Palette) -> Vec<String> {
        if let Some(focus) = self.confirming {
            return confirm_panel(
                t("Confirm Full Access"),
                &[t(CONFIRM_BODY).to_owned()],
                &[t("Confirm").to_owned(), t("Cancel").to_owned()],
                focus,
                t(CONFIRM_HINT),
                width,
                p,
            );
        }
        let inner = width.saturating_sub(4).max(1);
        let build = |roomy: bool, detail_lines: usize| -> Vec<String> {
            let mut content = Vec::new();
            if roomy {
                content.push(String::new());
            }
            for (index, (mode, name, _, description)) in self.modes.iter().enumerate() {
                let selected = index == self.selected;
                let marker = if selected {
                    paint(
                        SELECTED_MARKER,
                        Paint {
                            fg: Some(&p.accent),
                            bold: true,
                            ..Paint::default()
                        },
                    )
                } else {
                    IDLE_MARKER.to_owned()
                };
                let label = paint(
                    &truncate_cells(t(name), inner.saturating_sub(2 + 9)),
                    Paint {
                        fg: Some(if selected { &p.accent } else { &p.text }),
                        bold: selected,
                        ..Paint::default()
                    },
                );
                let badge = if *mode == self.current {
                    format!("  {}", fg(t("current"), &p.success))
                } else {
                    String::new()
                };
                content.push(format!("{marker}{label}{badge}"));
                let color = if selected { &p.muted } else { &p.faint };
                let width = inner.saturating_sub(2).max(1);
                let wrapped = crate::ui::render::panel::wrap_words(t(description), width);
                let mut lines: Vec<String> = wrapped.iter().take(detail_lines).cloned().collect();
                if wrapped.len() > detail_lines
                    && let Some(last) = lines.last_mut()
                {
                    *last = format!("{last} {}", wrapped[detail_lines]);
                }
                for line in lines {
                    content.push(format!(
                        "{IDLE_MARKER}{}",
                        fg(&truncate_cells(&line, width), color)
                    ));
                }
            }
            if roomy {
                content.push(String::new());
            }
            content
        };
        let content = [build(true, 2), build(true, 1), build(false, 1)]
            .into_iter()
            .find(|lines| lines.len() + 2 <= max_rows)
            .unwrap_or_else(|| build(false, 1));
        bordered(
            &content,
            width,
            None,
            &p.border,
            Some(&title_plain(t(self.title), inner, p)),
            Some(&key_hints(t(HINT), p)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::terminal::{KeyCode, KeyEvent, Modifiers};
    use crate::ui::theme::{Appearance, ColorMode};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, Modifiers::NONE)
    }

    fn screen(panel: &ApprovalsPanel) -> String {
        let palette =
            crate::ui::theme::palette::terminal_palette(ColorMode::TrueColor, Appearance::Dark);
        panel.render(80, 24, &palette).join("\n")
    }

    #[test]
    fn approval_offers_ask_and_full_access() {
        crate::ui::i18n::set(crate::ui::i18n::Locale::En);
        let mut panel = ApprovalsPanel::choice("workspace-write");
        let shown = screen(&panel);
        assert!(shown.contains("Ask"));
        assert!(shown.contains("Full access"));
        assert!(shown.contains("Ask before commands"));
        assert!(shown.contains("current"));
        assert!(!shown.contains("Read only"));
        assert!(!shown.contains("Workspace write"));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Enter)),
            Effect::Close
        ));

        let mut panel = ApprovalsPanel::choice("workspace-write");
        assert!(matches!(panel.handle_key(key(KeyCode::Down)), Effect::None));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Enter)),
            Effect::None
        ));
        assert!(screen(&panel).contains("Confirm Full Access"));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Char('n'))),
            Effect::None
        ));
        assert!(!screen(&panel).contains("Confirm Full Access"));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Enter)),
            Effect::None
        ));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Char('y'))),
            Effect::SetApprovals(mode) if mode == "full-access"
        ));

        let mut panel = ApprovalsPanel::choice("read-only");
        let shown = screen(&panel);
        assert!(!shown.contains("current"));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Enter)),
            Effect::SetApprovals(mode) if mode == "workspace-write"
        ));

        let mut panel = ApprovalsPanel::choice("full-access");
        assert!(matches!(
            panel.handle_key(key(KeyCode::Enter)),
            Effect::Close
        ));
        assert!(matches!(panel.handle_key(key(KeyCode::Up)), Effect::None));
        assert!(matches!(
            panel.handle_key(key(KeyCode::Enter)),
            Effect::SetApprovals(mode) if mode == "workspace-write"
        ));
        assert_eq!(announced_title("workspace-write"), "Ask");
        assert_eq!(announced_title("full-access"), "Full access");
        assert_eq!(announced_title("read-only"), "Read only");
    }

    #[test]
    fn the_preset_panel_still_lists_read_only() {
        crate::ui::i18n::set(crate::ui::i18n::Locale::En);
        let shown = screen(&ApprovalsPanel::new("workspace-write"));
        assert!(shown.contains("Workspace write"));
        assert!(shown.contains("Read only"));
        assert!(shown.contains("Full access"));
        assert!(shown.contains("current"));
    }
}
