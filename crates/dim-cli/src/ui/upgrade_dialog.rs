//! The update dialog (port of the TS `upgrade-dialog`): a newer version on
//! this build's channel, installed in place with the package manager this
//! copy came from, then a restart onto the same session.
//!
//! ```text
//! ╭──────────────────────────────────────────────────────────────╮
//! │ Update available                                             │
//! │                                                              │
//! │ Version 0.1.0-alpha.2 is available (you have 0.1.0-alpha.1). │
//! │ Will run: npm install -g --prefer-online … dimcode@0.1.0-…   │
//! │                                                              │
//! │ › Update now   Skip this version   Later                     │
//! │                                                              │
//! │ ←→ move · Enter confirm · Esc close                          │
//! ╰──────────────────────────────────────────────────────────────╯
//! ```

use crate::ui::i18n::{t, tf};
use crate::ui::prompts::Effect;
use crate::ui::render::panel::confirm_panel;
use crate::ui::terminal::{KeyCode, KeyEvent};
use crate::ui::theme::Palette;
use crate::update::Offer;

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Offer,
    Installing,
    Installed,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Choice {
    Install,
    Skip,
    Later,
    Close,
    Restart,
}

impl Choice {
    fn label(self) -> &'static str {
        match self {
            Self::Install => t("Update now"),
            Self::Skip => t("Skip this version"),
            Self::Later => t("Later"),
            Self::Close => t("Close"),
            Self::Restart => t("Restart now"),
        }
    }
}

pub struct UpgradeDialog {
    pub offer: Offer,
    pub stage: Stage,
    focused: usize,
}

impl UpgradeDialog {
    pub fn new(offer: Offer) -> Self {
        Self {
            offer,
            stage: Stage::Offer,
            focused: 0,
        }
    }

    /// The install ended; the dialog moves on to restarting or the error.
    pub fn installed(&mut self, result: Result<(), String>) {
        self.stage = match result {
            Ok(()) => Stage::Installed,
            Err(error) => Stage::Failed(error),
        };
        self.focused = 0;
    }

    /// How an install that ended after the dialog closed is told.
    pub fn outcome(version: &str, result: &Result<(), String>) -> String {
        match result {
            Ok(()) => tf(
                "Updated to {version}. Restart DimCode to use it.",
                &[("version", version)],
            ),
            Err(error) => tf("Update failed: {error}", &[("error", error)]),
        }
    }

    fn choices(&self) -> &'static [Choice] {
        match self.stage {
            Stage::Offer if self.offer.install.is_some() => {
                &[Choice::Install, Choice::Skip, Choice::Later]
            }
            Stage::Offer => &[Choice::Skip, Choice::Close],
            Stage::Installing | Stage::Failed(_) => &[Choice::Close],
            Stage::Installed => &[Choice::Restart, Choice::Close],
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        let count = self.choices().len();
        match key.code {
            KeyCode::Right | KeyCode::Tab => self.focused = (self.focused + 1) % count,
            KeyCode::Left | KeyCode::BackTab => self.focused = (self.focused + count - 1) % count,
            KeyCode::Esc => return Effect::Close,
            KeyCode::Enter => {
                return match self.choices()[self.focused.min(count - 1)] {
                    Choice::Install => {
                        self.stage = Stage::Installing;
                        self.focused = 0;
                        match &self.offer.install {
                            Some(install) => Effect::UpgradeInstall(install.clone()),
                            None => Effect::None,
                        }
                    }
                    Choice::Skip => Effect::UpgradeSkip(self.offer.version.clone()),
                    Choice::Later | Choice::Close => Effect::Close,
                    Choice::Restart => Effect::Restart { confirmed: false },
                };
            }
            _ => {}
        }
        Effect::None
    }

    pub fn render(&self, width: usize, p: &Palette) -> Vec<String> {
        let version = self.offer.version.as_str();
        let body = match &self.stage {
            Stage::Offer => vec![
                tf(
                    "Version {latest} is available (you have {current}).",
                    &[("latest", version), ("current", &self.offer.current)],
                ),
                match &self.offer.install {
                    Some(install) => tf(
                        "Will run: {command}",
                        &[("command", &install.command_line())],
                    ),
                    None => tf(
                        "To update, run: {command}",
                        &[("command", &self.offer.manual)],
                    ),
                },
            ],
            Stage::Installing => vec![tf("Updating to {version}...", &[("version", version)])],
            Stage::Installed => vec![Self::outcome(version, &Ok(()))],
            Stage::Failed(error) => vec![Self::outcome(version, &Err(error.clone()))],
        };
        let labels: Vec<String> = self
            .choices()
            .iter()
            .map(|choice| choice.label().to_owned())
            .collect();
        confirm_panel(
            t("Update available"),
            &body,
            &labels,
            self.focused,
            t("←→ move · Enter confirm · Esc close"),
            width,
            p,
        )
    }
}
