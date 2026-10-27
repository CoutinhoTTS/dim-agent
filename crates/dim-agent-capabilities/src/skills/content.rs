//! Immutable content services owned by real Session loader instances.
use super::{CatalogView, SkillDefinition, SkillStatus, SkillView, SkillsError};
use dim_agent_runtime::SessionLoader;
use rutis::host::{CtxHost, FiberView, InstanceId, Key, TypeKey};
use rutis::{Ctx, Plugin};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

pub(super) const GROUP: &str = "SkillContentInstances";

#[derive(Debug, thiserror::Error)]
pub(super) enum ContentError {
    #[error(transparent)]
    Catalog(#[from] SkillsError),
    #[error("Skill content lifecycle failed: {0}")]
    Lifecycle(String),
}

/// One content instance's configuration: its definition, the key it provides
/// it at, and the collection of the Skills generation that created it.
#[derive(Clone)]
pub(super) struct ContentConfig {
    definition: Arc<SkillDefinition>,
    key: Key<Contribution>,
    contributions: Arc<SkillContributions>,
}
struct Contribution(Arc<SkillDefinition>);
/// The ready content of one Skills generation. Skills holds it and hands it
/// to each content instance it creates, which adds itself while it runs; it
/// is no service, so an instance does not wait for Skills, which waits for
/// its instances.
#[derive(Default)]
pub(super) struct SkillContributions(Mutex<HashMap<TypeKey, Arc<Contribution>>>);
pub(super) struct ContentPlugin(pub(super) ContentConfig);
impl Plugin for ContentPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "SkillContent"
    }
    async fn apply(&self, ctx: &Ctx, (): ()) -> rutis::Result {
        let contributions = self.0.contributions.clone();
        let service = Arc::new(Contribution(self.0.definition.clone()));
        ctx.provide_at(self.0.key.clone(), service.clone())?;
        let key = self.0.key.erase();
        let cleanup = contributions.clone();
        ctx.effect(move || async move {
            cleanup.0.lock().unwrap().remove(&key);
            Ok(())
        })?;
        contributions
            .0
            .lock()
            .unwrap()
            .insert(self.0.key.erase(), service);
        Ok(())
    }
}

pub(super) struct Contents {
    /// The Session instance the content services are keyed in.
    session: InstanceId,
    contributions: Arc<SkillContributions>,
    instances: BTreeMap<String, (FiberView, TypeKey)>,
}
impl Contents {
    pub(super) fn new(session: InstanceId) -> Self {
        Self {
            session,
            contributions: Arc::default(),
            instances: BTreeMap::new(),
        }
    }
    pub(super) async fn close(&mut self) -> Result<(), ContentError> {
        let mut error = None;
        for (_, (instance, _)) in std::mem::take(&mut self.instances) {
            if let Err(failure) = instance.shutdown().await {
                error = Some(ContentError::Lifecycle(failure.to_string()));
            } else if let Some(failure) = instance.state().error {
                error = Some(ContentError::Lifecycle(failure.to_string()));
            }
        }
        error.map_or(Ok(()), Err)
    }
    pub(super) async fn update(
        &mut self,
        view: Result<CatalogView, SkillsError>,
        loader: &SessionLoader,
        cancel: &CancellationToken,
    ) -> Result<Arc<SkillView>, SkillsError> {
        let mut result = self.reconcile(view, loader, cancel).await;
        if result.is_err()
            && let Err(error) = self.close().await
        {
            result = Err(error);
        }
        result.map_err(|error| match error {
            ContentError::Catalog(error) => error,
            ContentError::Lifecycle(error) => {
                tracing::error!(%error, "Skill contents unavailable");
                SkillsError::Closed
            }
        })
    }
    async fn reconcile(
        &mut self,
        view: Result<CatalogView, SkillsError>,
        loader: &SessionLoader,
        cancel: &CancellationToken,
    ) -> Result<Arc<SkillView>, ContentError> {
        let view = view?;
        let selected: BTreeMap<_, _> = view
            .entries
            .iter()
            .filter(|entry| entry.status == SkillStatus::Active && entry.enabled)
            .map(|entry| (entry.definition.id.clone(), entry.definition.clone()))
            .collect();
        let disabled: BTreeSet<_> = view
            .entries
            .iter()
            .filter(|entry| entry.status == SkillStatus::Active && !entry.enabled)
            .map(|entry| entry.definition.name.clone())
            .collect();
        let obsolete: Vec<_> = {
            let ready = self.contributions.0.lock().unwrap();
            self.instances
                .iter()
                .filter(|(id, (_, key))| {
                    selected.get(*id).is_none_or(|definition| {
                        ready
                            .get(key)
                            .is_none_or(|service| **definition != *service.0)
                    })
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in obsolete {
            let (instance, _) = self.instances.remove(&id).unwrap();
            instance
                .shutdown()
                .await
                .map_err(|error| ContentError::Lifecycle(error.to_string()))?;
            if let Some(error) = instance.state().error {
                return Err(ContentError::Lifecycle(error.to_string()));
            }
        }
        for (id, definition) in selected {
            if cancel.is_cancelled() {
                return Err(SkillsError::Closed.into());
            }
            if self.instances.contains_key(&id) {
                continue;
            }
            let provided = Key::<Contribution>::dynamic(id.clone()).instance(self.session);
            let key = provided.erase();
            let instance = loader
                .create_instance(GROUP)
                .with(ContentConfig {
                    definition,
                    key: provided,
                    contributions: self.contributions.clone(),
                })
                .await
                .map_err(|error| ContentError::Lifecycle(error.to_string()))?;
            let ready = self.contributions.0.lock().unwrap().contains_key(&key);
            if cancel.is_cancelled() || !ready {
                instance
                    .view
                    .shutdown()
                    .await
                    .map_err(|error| ContentError::Lifecycle(error.to_string()))?;
                if let Some(error) = instance.view.state().error {
                    return Err(ContentError::Lifecycle(error.to_string()));
                }
                return Err(if cancel.is_cancelled() {
                    SkillsError::Closed.into()
                } else {
                    ContentError::Lifecycle("content instance did not become ready".into())
                });
            }
            self.instances.insert(id, (instance.view, key));
        }
        let ready = self.contributions.0.lock().unwrap();
        Ok(Arc::new(SkillView::from_contributions(
            self.instances
                .values()
                .filter_map(|(_, key)| ready.get(key))
                .map(|service| (service.0.name.clone(), service.0.clone()))
                .collect(),
            disabled,
        )))
    }
}
