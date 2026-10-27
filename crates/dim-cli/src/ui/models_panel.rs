//! The `/models` picker (port of `ModelPanel` and `buildModelRows` for the
//! models a daemon offers; reasoning-effort levels and "Edit models" are not
//! built yet), and its "Subagent models" action when the daemon manages that
//! policy:
//!
//! ```text
//! Select a model                  Subagent models
//!   Current provider / model        Agent choice (default)
//!   Other provider / model          Inherit requesting branch
//!   Edit models                     Fixed model > the model list again
//!   Subagent models: Agent choice
//!                                   Applies to new members, including named Agents.
//!                                   Existing members keep their models.
//! ```
//!
//! Choosing a policy writes only `collaboration.set_model_policy`; the leader's
//! model and the provider default are untouched.

use crate::model::session::{ModelItem, ModelList, ModelPolicyUpdate, SubagentModelPolicy};
use crate::ui::i18n::{t, tf};
use crate::ui::list_panel::{Event, Item, ListPanel};
use crate::ui::prompts::Effect;
use crate::ui::terminal::{KeyCode, KeyEvent, Modifiers};
use crate::ui::theme::Palette;
use dim_agent_types::ModelId;

const HINT: &str = "Enter select · Ctrl+X clear saved · Esc clear/close";
const POLICY_HINT: &str = "Enter select · Esc back";
const EDIT_MODELS: &str = "__edit_models__";
const SUBAGENT_MODELS: &str = "__subagent_models__";
const SECTION: &str = "Models";
const AGENT: &str = "agent";
const INHERIT: &str = "inherit";
const FIXED: &str = "fixed";

#[derive(Clone, Copy, PartialEq)]
enum View {
    Models,
    Policy,
    /// The model list again, choosing the fixed subagent model.
    Fixed,
}

/// The policy as last read; `revision` is `None` after a conflict or
/// failure, when a choice rereads instead of saving.
struct Policy {
    revision: Option<String>,
    current: Option<SubagentModelPolicy>,
}

pub struct ModelsPanel {
    view: View,
    list: ListPanel,
    listed: Option<Result<ModelList, String>>,
    /// `None` while the daemon does not manage the policy: no action.
    policy: Option<Policy>,
}

impl ModelsPanel {
    pub fn new() -> Self {
        Self {
            view: View::Models,
            list: list(View::Models),
            listed: None,
            policy: None,
        }
    }

    pub fn set_models(&mut self, models: Result<ModelList, String>) {
        self.listed = Some(models);
        self.rebuild(None);
    }

    /// Returns the notice the update calls for.
    pub fn set_policy(&mut self, update: ModelPolicyUpdate) -> Option<String> {
        let saved = matches!(update, ModelPolicyUpdate::Read { saved: true, .. });
        let policy = self.policy.get_or_insert(Policy {
            revision: None,
            current: None,
        });
        let notice = match update {
            ModelPolicyUpdate::Read {
                revision,
                policy: current,
                ..
            } => {
                policy.revision = Some(revision);
                policy.current = Some(current);
                saved.then(|| t("Subagent model policy saved").to_owned())
            }
            ModelPolicyUpdate::Conflict => {
                policy.revision = None;
                Some(
                    t("Subagent model policy changed elsewhere; choose again to reread it")
                        .to_owned(),
                )
            }
            ModelPolicyUpdate::Failed(error) => {
                policy.revision = None;
                Some(tf("Subagent model policy: {error}", &[("error", &error)]))
            }
        };
        if saved {
            self.open(View::Models, Some(SUBAGENT_MODELS));
        } else {
            self.rebuild(None);
        }
        notice
    }

    fn current(&self) -> Option<&SubagentModelPolicy> {
        self.policy.as_ref()?.current.as_ref()
    }

    fn open(&mut self, view: View, preselect: Option<&str>) {
        self.view = view;
        self.list = list(view);
        self.rebuild(preselect);
    }

    /// Fills the list of the current view, keeping its selection.
    fn rebuild(&mut self, preselect: Option<&str>) {
        let selected = self.list.selected_id();
        let mode = match self.current() {
            Some(SubagentModelPolicy::Agent) => Some(AGENT),
            Some(SubagentModelPolicy::Inherit) => Some(INHERIT),
            Some(SubagentModelPolicy::Fixed { .. }) => Some(FIXED),
            None => None,
        };
        let marked = match (self.view, self.current()) {
            (View::Models, _) => self
                .listed
                .as_ref()
                .and_then(|listed| listed.as_ref().ok()?.current.as_ref())
                .map(|(p, m)| key(p, m)),
            (View::Fixed, Some(SubagentModelPolicy::Fixed { model })) => {
                Some(key(&model.provider_id, &model.model_id))
            }
            (View::Fixed, _) => None,
            (View::Policy, _) => mode.map(str::to_owned),
        };
        let items = match self.view {
            View::Policy => Some(Ok(self.policy_rows(mode))),
            View::Models | View::Fixed => self.listed.as_ref().map(|listed| {
                listed.clone().map(|list| {
                    let mut items = model_rows(&list.models, marked.as_deref());
                    if self.view == View::Models {
                        items.push(Item::new(EDIT_MODELS, t("Edit models"), ""));
                        if let Some(policy) = &self.policy {
                            let label = self.label(policy.current.as_ref());
                            items.push(Item::new(
                                SUBAGENT_MODELS,
                                tf("Subagent models: {policy}", &[("policy", &label)]),
                                "",
                            ));
                        }
                    }
                    items
                })
            }),
        };
        if let Some(items) = items {
            let preselect = preselect.or(selected.as_deref()).or(marked.as_deref());
            self.list.set_items(items, preselect);
        }
    }

    fn policy_rows(&self, mode: Option<&str>) -> Vec<Item> {
        let detail = |id| if mode == Some(id) { t("current") } else { "" };
        let mut fixed = Item::new(FIXED, t("Fixed model"), detail(FIXED));
        if let Some(SubagentModelPolicy::Fixed { model }) = self.current() {
            fixed.detail = format!("{}  {}", self.model_name(model), t("current"));
        }
        vec![
            Item::new(AGENT, t("Agent choice (default)"), detail(AGENT)),
            Item::new(INHERIT, t("Inherit requesting branch"), detail(INHERIT)),
            fixed,
            Item::heading(""),
            Item::heading(t("Applies to new members, including named Agents.")),
            Item::heading(t("Existing members keep their models.")),
        ]
    }

    fn models(&self) -> &[ModelItem] {
        match &self.listed {
            Some(Ok(list)) => &list.models,
            _ => &[],
        }
    }

    fn model_name(&self, model: &ModelId) -> String {
        let provider = self
            .models()
            .iter()
            .find(|item| item.provider_id == model.provider_id)
            .map_or(model.provider_id.as_str(), provider_name);
        format!("{provider} / {}", model.model_id)
    }

    fn label(&self, policy: Option<&SubagentModelPolicy>) -> String {
        match policy {
            Some(SubagentModelPolicy::Agent) => t("Agent choice").to_owned(),
            Some(SubagentModelPolicy::Inherit) => t("Inherit requesting branch").to_owned(),
            Some(SubagentModelPolicy::Fixed { model }) => self.model_name(model),
            None => t("unknown").to_owned(),
        }
    }

    /// Saves `policy`, or rereads first when the revision is not known.
    fn choose(&self, policy: SubagentModelPolicy) -> Effect {
        match self.policy.as_ref().and_then(|p| p.revision.clone()) {
            Some(expected_revision) => Effect::SetModelPolicy {
                expected_revision,
                policy,
            },
            None => Effect::ReadModelPolicy,
        }
    }

    /// A paste goes into the filter.
    pub fn paste(&mut self, text: &str) {
        self.list.paste(text);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if self.view == View::Models
            && key.code == KeyCode::Char('x')
            && key.mods.contains(Modifiers::CTRL)
        {
            return self
                .list
                .selected_id()
                .and_then(|id| {
                    self.models()
                        .iter()
                        .find(|model| id == self::key(&model.provider_id, &model.model_id))
                })
                .map(|model| Effect::ClearModelPreferences {
                    provider_id: model.provider_id.clone(),
                    model_id: model.model_id.clone(),
                    expected_version: model.provider_version,
                })
                .unwrap_or(Effect::None);
        }
        let event = self.list.handle_key(key);
        match (self.view, event) {
            (_, Event::None | Event::Refresh) => Effect::None,
            (View::Models, Event::Close) => Effect::Close,
            (View::Policy, Event::Close) => {
                self.open(View::Models, Some(SUBAGENT_MODELS));
                Effect::None
            }
            (View::Fixed, Event::Close) => {
                self.open(View::Policy, Some(FIXED));
                Effect::None
            }
            (View::Models, Event::Pick(id)) if id == EDIT_MODELS => {
                Effect::Notice(t("Editing models is not available yet").to_owned())
            }
            (View::Models, Event::Pick(id)) if id == SUBAGENT_MODELS => {
                self.open(View::Policy, None);
                Effect::None
            }
            (View::Policy, Event::Pick(id)) if id == FIXED => {
                self.open(View::Fixed, None);
                Effect::None
            }
            (View::Policy, Event::Pick(id)) if id == INHERIT => {
                self.choose(SubagentModelPolicy::Inherit)
            }
            (View::Policy, Event::Pick(_)) => self.choose(SubagentModelPolicy::Agent),
            (view, Event::Pick(id)) => match id.split_once("::") {
                Some((provider, model)) if view == View::Fixed => {
                    self.choose(SubagentModelPolicy::Fixed {
                        model: ModelId {
                            provider_id: provider.to_owned(),
                            model_id: model.to_owned(),
                        },
                    })
                }
                Some((provider, model)) => Effect::SelectModel {
                    provider_id: provider.to_owned(),
                    model_id: model.to_owned(),
                },
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

fn list(view: View) -> ListPanel {
    let models = |title, hint| {
        ListPanel::new(
            title,
            hint,
            "Loading models...",
            "No models available. Connect a provider first.",
            "No matching models.",
            false,
        )
    };
    match view {
        View::Models => models("Select a model", HINT),
        View::Fixed => models("Fixed subagent model", POLICY_HINT),
        View::Policy => {
            ListPanel::new("Subagent models", POLICY_HINT, "", "", "", false).without_filter()
        }
    }
}

fn model_rows(models: &[ModelItem], marked: Option<&str>) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    if !models.is_empty() {
        items.push(Item::heading(t(SECTION)));
    }
    for model in models {
        let id = key(&model.provider_id, &model.model_id);
        let detail = if Some(id.as_str()) == marked {
            t("current")
        } else {
            ""
        };
        let mut row = Item::new(
            id,
            format!("{} / {}", provider_name(model), model.model_id),
            detail,
        );
        row.search = model.display_name.clone();
        items.push(row);
    }
    items
}

fn provider_name(model: &ModelItem) -> &str {
    if model.provider_name.is_empty() {
        model.provider_id.as_str()
    } else {
        model.provider_name.as_str()
    }
}

fn key(provider: &str, model: &str) -> String {
    format!("{provider}::{model}")
}
