use super::*;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelQuery {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub query: Option<String>,
    pub after: Option<ModelId>,
    pub limit: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPage {
    pub models: Vec<ModelRow>,
    pub next: Option<ModelId>,
    pub empty: Vec<EmptyModelProvider>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub provider_id: String,
    pub provider_name: String,
    pub model: ProviderModel,
    pub version: u32,
    pub preferences: ModelPreferences,
    pub disabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyModelProvider {
    pub provider_id: String,
    pub provider_name: String,
    pub reason: EmptyModelReason,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EmptyModelReason {
    NoModels { listable: bool },
    Unreadable,
}

impl ProviderService {
    pub(super) async fn model_name(
        self: &Arc<Self>,
        provider_id: String,
        model_id: String,
        cancel: &CancellationToken,
    ) -> Result<String, ProviderError> {
        let id = provider_id;
        let target = model_id;
        let db = self.db.clone();
        self.owned(cancel, move |_| Box::pin(async move {
                db.run_read(move |connection| {
                    let mut query=connection.prepare("SELECT m.model_id,coalesce(json_extract(o.entry,'$.displayName'),json_extract(m.entry,'$.displayName'),json_extract(m.entry,'$.entry.displayName')),coalesce(json_extract(o.entry,'$.metadata.raw.magpie_label'),json_extract(m.entry,'$.metadata.raw.magpie_label'),json_extract(m.entry,'$.entry.metadata.raw.magpie_label')) FROM provider_models m JOIN provider_query_records p ON p.provider_id=m.provider_id LEFT JOIN provider_models o ON o.provider_id=m.provider_id AND o.model_id=m.model_id AND o.source='override' WHERE m.provider_id=?1 AND m.source!='override' ORDER BY m.source,m.rowid").map_err(storage)?;
                    let names=query.query_map([id], |row| Ok(naming::Name { model_id:row.get(0)?,display_name:row.get(1)?,label:row.get(2)? })).map_err(storage)?.collect::<rusqlite::Result<Vec<_>>>().map_err(storage)?;
                    let labels=naming::unique_names(&names);
                    names.iter().zip(labels).find_map(|(name,label)| (name.model_id==target).then_some(label)).ok_or_else(|| storage("Selected model disappeared"))
                }).await.map_err(ProviderError::from)
            })).await
    }

    pub async fn query_models(
        self: &Arc<Self>,
        input: ModelQuery,
        cancel: &CancellationToken,
    ) -> Result<ModelPage, ProviderError> {
        let limit = input.limit.unwrap_or(32);
        if !(1..=128).contains(&limit) {
            return Err(ProviderError::InvalidConfig(
                "limit must be between 1 and 128".into(),
            ));
        }
        let descriptors = self.oauth.providers(cancel).await?;
        let accounts = serde_json::to_string(&descriptors).map_err(|_| ProviderError::Failed)?;
        let db = self.db.clone();
        let candidates = self.owned(cancel, move |_| Box::pin(async move {
            db.run_read(move |connection| {
                let mut query = connection.prepare(r#"
WITH accounts AS (
    SELECT json_extract(value,'$.providerId') AS id,
           json_extract(value,'$.displayName') AS name FROM json_each(?7)
)
SELECT m.provider_id,m.model_id,coalesce(a.name,json_extract(p.record,'$.displayName'),m.provider_id)
FROM provider_models m JOIN provider_query_records p ON p.provider_id=m.provider_id
LEFT JOIN accounts a ON a.id=m.provider_id
LEFT JOIN provider_models o ON o.provider_id=m.provider_id AND o.model_id=m.model_id AND o.source='override'
WHERE m.source!='override' AND (?1 IS NULL OR m.provider_id=?1) AND (?2 IS NULL OR m.model_id=?2)
  AND (m.provider_id,m.model_id)>(coalesce(?3,''),coalesce(?4,''))
  AND (?5 IS NULL OR (lower(?5)='zdr' AND json_extract(m.entry,'$.zdr')=1)
       OR instr(lower(m.model_id),lower(?5))>0
       OR instr(lower(coalesce(json_extract(o.entry,'$.displayName'),json_extract(m.entry,'$.displayName'),json_extract(m.entry,'$.entry.displayName'))),lower(?5))>0
       OR instr(lower(coalesce(a.name,json_extract(p.record,'$.displayName'),m.provider_id)),lower(?5))>0)
  AND (p.kind='managed' OR p.enabled=1)
UNION ALL
SELECT p.provider_id,'',coalesce(a.name,json_extract(p.record,'$.displayName'),p.provider_id)
FROM provider_query_records p LEFT JOIN accounts a ON a.id=p.provider_id
WHERE (?1 IS NULL OR p.provider_id=?1) AND ?2 IS NULL
  AND (p.provider_id,'')>(coalesce(?3,''),coalesce(?4,''))
  AND (p.enabled=1 OR p.kind='managed')
  AND NOT EXISTS(SELECT 1 FROM provider_models m WHERE m.provider_id=p.provider_id AND m.source!='override')
  AND (?5 IS NULL OR instr(lower(p.provider_id),lower(?5))>0 OR instr(lower(coalesce(a.name,json_extract(p.record,'$.displayName'))),lower(?5))>0)
UNION ALL
SELECT a.id,'',a.name FROM accounts a
WHERE (?1 IS NULL OR a.id=?1) AND ?2 IS NULL
  AND (a.id,'')>(coalesce(?3,''),coalesce(?4,''))
  AND NOT EXISTS(SELECT 1 FROM provider_query_records p WHERE p.provider_id=a.id)
  AND (?5 IS NULL OR instr(lower(a.id),lower(?5))>0 OR instr(lower(a.name),lower(?5))>0)
ORDER BY 1,2 LIMIT ?6
"#).map_err(storage)?;
                query.query_map(params![input.provider_id,input.model_id,input.after.as_ref().map(|m| &m.provider_id),input.after.as_ref().map(|m| &m.model_id),input.query,limit+1,accounts], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(storage)?.collect::<rusqlite::Result<Vec<_>>>().map_err(storage)
            }).await.map_err(ProviderError::from)
        })).await?;
        let more = candidates.len() > limit as usize;
        let mut page = ModelPage {
            models: Vec::new(),
            next: None,
            empty: Vec::new(),
        };
        let mut last = None;
        for (provider_id, model_id, provider_name) in candidates.into_iter().take(limit as usize) {
            last = Some(ModelId {
                provider_id: provider_id.clone(),
                model_id: model_id.clone(),
            });
            if model_id.is_empty() {
                let listable = if descriptors
                    .iter()
                    .any(|descriptor| descriptor.provider_id == provider_id)
                {
                    if !self.account_available(&provider_id, cancel).await? {
                        continue;
                    }
                    true
                } else {
                    let id = provider_id.clone();
                    let db = self.db.clone();
                    let record = self
                        .owned(cancel, move |_| {
                            Box::pin(async move {
                                db.run_read(move |c| read_static(c, &id))
                                    .await
                                    .map_err(ProviderError::from)
                            })
                        })
                        .await?;
                    record
                        .as_ref()
                        .is_some_and(|record| self.remote_model_list(&provider_id, record))
                };
                page.empty.push(EmptyModelProvider {
                    provider_id,
                    provider_name,
                    reason: EmptyModelReason::NoModels { listable },
                });
                continue;
            }
            let catalog = match self
                .catalog_target(provider_id.clone(), model_id.clone(), cancel)
                .await
            {
                Ok(Some(catalog)) => catalog,
                Ok(None) => continue,
                Err(HostError::Cancelled(message)) => {
                    return Err(HostError::Cancelled(message).into());
                }
                Err(_) => {
                    self.check(cancel)?;
                    page.empty.push(EmptyModelProvider {
                        provider_id,
                        provider_name,
                        reason: EmptyModelReason::Unreadable,
                    });
                    continue;
                }
            };
            let disabled = catalog.disabled_model_ids.contains(&model_id);
            let version = catalog.version;
            let preferences = self
                .saved_preferences(&provider_id, &model_id, cancel)
                .await?;
            let mut model = catalog
                .models
                .into_iter()
                .find(|model| model.id == model_id)
                .expect("the selected catalog model");
            model.name = self
                .model_name(provider_id.clone(), model_id.clone(), cancel)
                .await?;
            page.models.push(ModelRow {
                provider_id,
                provider_name,
                model,
                version,
                preferences,
                disabled,
            });
        }
        if more {
            page.next = last;
        }
        Ok(page)
    }
}
