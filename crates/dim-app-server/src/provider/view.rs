//! Safe edit snapshots are projected from one admitted Provider record.
use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementView {
    pub drivers: Vec<ManagementDriver>,
    pub providers: Vec<ManagementProvider>,
    pub errors: Vec<ProviderRecordError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementDriver {
    pub driver_kind: &'static str,
    pub auth_types: Vec<&'static str>,
    pub remote_model_list: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementInfo {
    pub provider_id: String,
    pub display_name: String,
    pub source: &'static str,
    pub credential_authority: &'static str,
    pub version: Option<u32>,
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<ManagementAccount>,
    pub actions: Vec<&'static str>,
    pub connection: Option<ManagementConnection>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementProvider {
    #[serde(flatten)]
    pub info: ManagementInfo,
    pub models: Option<Vec<ManagementModel>>,
    pub preferences: BTreeMap<String, ModelPreferences>,
    /// What an update did with the record's models; absent from the reads and
    /// from a patch that did not change `baseUrl`, `credential` or `headers`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_listed: Option<ModelsListed>,
}

/// What `provider.update` did with the record's models when the patch changed
/// the endpoint or the account it reaches.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ModelsListed {
    /// It listed the new endpoint's models and stored them in the same write
    /// as the endpoint.
    Listed,
    /// The record keeps its models, which are the bundled catalog's.
    Bundled,
    /// The driver cannot list models, so the connection was saved alone.
    Unsupported,
    /// The record is disabled, so nothing was listed.
    Disabled,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementAccount {
    pub namespace: &'static str,
    pub login_methods: Vec<String>,
    pub state: AccountState,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AccountState {
    Native(managed_oauth::AuthStatus),
    Package(AccountSnapshot),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementConnection {
    pub driver_kind: String,
    pub auth: StaticAuth,
    pub base_url: String,
    pub credential_configured: bool,
    pub header_names: Vec<String>,
    pub proxy_url: Option<String>,
    pub proxy_auth_configured: bool,
    pub timeout_ms: Option<u64>,
    /// The prefix this Provider's listed ids carry, absent when it states
    /// none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glued_prefix: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementModel {
    pub model: ModelEntry,
    pub source: &'static str,
    pub enabled: bool,
    pub endpoint: Option<ModelProtocol>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_limits: Vec<String>,
    pub unavailable_reason: Option<String>,
    pub actions: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_override: Option<NewApiEndpoint>,
    /// The model this entry's id stands for, when the provider says so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// The name `provider.catalog` publishes for this row, when it differs from
    /// `model.displayName`: a client that picks models from this read shows
    /// what the catalog shows, and an absent field means the display name is
    /// already the one to show.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_name: Option<String>,
}

impl ProviderService {
    pub(super) fn remote_model_list(
        &self,
        provider_id: &str,
        record: &StaticProviderRecord,
    ) -> bool {
        super::can_list_models(&self.bundled, provider_id, record)
    }

    pub(super) async fn static_management_info(
        &self,
        provider_id: String,
        record: &StaticProviderRecord,
        cancel: &CancellationToken,
    ) -> Result<ManagementInfo, ProviderError> {
        let (authority, account, available) = match record.credential_authority {
            CredentialAuthority::Static => ("static", None, true),
            authority => {
                let (namespace, method, state) = match authority {
                    CredentialAuthority::OpenaiCodex => {
                        ("account.codex", "browser", self.codex.status(cancel).await?)
                    }
                    CredentialAuthority::Grok => {
                        ("account.grok", "device", self.grok.status(cancel).await?)
                    }
                    CredentialAuthority::Static => unreachable!(),
                };
                let available = state.authenticated;
                (
                    if authority == CredentialAuthority::OpenaiCodex {
                        "openai_codex"
                    } else {
                        "grok"
                    },
                    Some(ManagementAccount {
                        namespace,
                        login_methods: vec![method.into()],
                        state: AccountState::Native(state),
                    }),
                    available,
                )
            }
        };
        let mut actions = vec![
            "update",
            "delete",
            "models.edit",
            "models.reset",
            "models.set_enabled",
            "preferences.update",
        ];
        if authority == "static" {
            actions.extend(["models.create", "models.delete"]);
            if self.remote_model_list(&provider_id, record)
                && (record.auth == StaticAuth::Anonymous || record.credential.is_some())
            {
                actions.push("refresh");
            }
        } else {
            actions.push("login");
            if available {
                actions.push("logout");
            }
        }
        let (proxy_url, proxy_auth_configured) = match &record.network.proxy {
            Some(proxy) => {
                let mut url = reqwest::Url::parse(proxy).map_err(|_| ProviderError::Failed)?;
                let authenticated = !url.username().is_empty() || url.password().is_some();
                url.set_username("").map_err(|_| ProviderError::Failed)?;
                url.set_password(None).map_err(|_| ProviderError::Failed)?;
                (Some(url.to_string()), authenticated)
            }
            None => (None, false),
        };
        let mut header_names: Vec<_> = record.headers.keys().cloned().collect();
        header_names.sort();
        Ok(ManagementInfo {
            provider_id,
            display_name: record.display_name.clone(),
            source: if record.kind == StaticRecordKind::Builtin {
                "builtin"
            } else {
                "custom"
            },
            credential_authority: authority,
            version: Some(record.version),
            enabled: Some(record.enabled),
            account,
            actions,
            connection: Some(ManagementConnection {
                driver_kind: record.driver_kind.clone(),
                auth: record.auth.clone(),
                base_url: record.base_url.clone(),
                credential_configured: if authority == "static" {
                    record.credential.is_some()
                } else {
                    available
                },
                header_names,
                proxy_url,
                proxy_auth_configured,
                timeout_ms: record.network.timeout_ms,
                glued_prefix: record.glued_prefix.clone(),
            }),
        })
    }

    pub(super) async fn static_management(
        &self,
        provider_id: String,
        record: StaticProviderRecord,
        cancel: &CancellationToken,
    ) -> Result<ManagementProvider, ProviderError> {
        let info = self
            .static_management_info(provider_id.clone(), &record, cancel)
            .await?;
        let available = info
            .connection
            .as_ref()
            .is_some_and(|c| c.credential_configured)
            || record.credential_authority == CredentialAuthority::Static;
        let entries = static_entries(&self.bundled, &provider_id, &record);
        let catalog = static_models_from_entries(&record, available, entries.clone());
        let models = entries
            .into_iter()
            .zip(catalog)
            .map(|(model, effective)| {
                let user = record
                    .user_models
                    .iter()
                    .any(|entry| entry.model_id == model.model_id);
                let overridden = record
                    .model_config_overrides
                    .iter()
                    .any(|entry| entry.model_id == model.model_id);
                // The set governs the entries the provider can use: an
                // unavailable entry keeps the enablement it has, so the
                // owner offers no toggle for it.
                let mut actions = if effective.unavailable_reason.is_none() {
                    vec!["set_enabled", "edit"]
                } else {
                    vec!["edit"]
                };
                if user {
                    actions.push("delete");
                } else if overridden {
                    actions.push("reset");
                }
                let catalog_name = (effective.name != model.display_name).then_some(effective.name);
                ManagementModel {
                    enabled: !record.disabled_model_ids.contains(&model.model_id),
                    endpoint_override: record.endpoint_overrides.get(&model.model_id).copied(),
                    alias: record.model_aliases.get(&model.model_id).cloned(),
                    model,
                    source: if user {
                        "user"
                    } else if overridden {
                        "override"
                    } else {
                        "default"
                    },
                    endpoint: effective.endpoint,
                    missing_limits: effective.missing_limits,
                    unavailable_reason: effective.unavailable_reason,
                    actions,
                    catalog_name,
                }
            })
            .collect();
        Ok(ManagementProvider {
            info,
            models: Some(models),
            preferences: record.model_preferences,
            models_listed: None,
        })
    }

    pub(super) fn managed_management_info(
        descriptor: crate::managed_provider::ProviderDescriptor,
        account: AccountSnapshot,
        record: Option<&ProviderRecord>,
    ) -> ManagementInfo {
        let unavailable = if account.account.is_none() {
            Some("account_required")
        } else if record
            .as_ref()
            .is_some_and(|record| !same_account(&record.account, &account))
        {
            Some("account_changed")
        } else {
            None
        };
        let mut actions = vec!["login", "usage"];
        if account.account.is_some() {
            actions.extend(["logout", "refresh"]);
        }
        if record.is_some() && unavailable.is_none() {
            actions.extend([
                "models.edit",
                "models.reset",
                "models.set_enabled",
                "preferences.update",
            ]);
        }
        let version = record.as_ref().map(|record| record.version);
        ManagementInfo {
            provider_id: descriptor.provider_id,
            display_name: descriptor.display_name,
            source: "managed",
            credential_authority: "account",
            version,
            enabled: None,
            account: Some(ManagementAccount {
                namespace: "account",
                login_methods: descriptor.login_methods,
                state: AccountState::Package(account),
            }),
            actions,
            connection: None,
        }
    }

    pub(super) fn managed_management(
        descriptor: crate::managed_provider::ProviderDescriptor,
        account: AccountSnapshot,
        record: Option<ProviderRecord>,
        bundled: &BundledCatalog,
    ) -> ManagementProvider {
        let info = Self::managed_management_info(descriptor, account.clone(), record.as_ref());
        let unavailable = if account.account.is_none() {
            Some("account_required")
        } else if record
            .as_ref()
            .is_some_and(|record| !same_account(&record.account, &account))
        {
            Some("account_changed")
        } else {
            None
        };
        let (models, preferences) = match record {
            Some(record) => {
                let effective = record.effective_models(bundled);
                let names = naming::unique(effective.iter().map(|model| &model.entry));
                let models = effective
                    .into_iter()
                    .zip(names)
                    .map(|(effective, name)| {
                        // The account itself can be unavailable, a fact the
                        // record does not hold; that reason stands on top of
                        // the one the effective limits derive.
                        let unavailable_reason = unavailable.map(str::to_owned).or_else(|| {
                            effective.unavailable.map(|unavailable| unavailable.reason)
                        });
                        // The set governs the entries the account can use: an
                        // unavailable entry keeps the enablement it has, so the
                        // owner offers no toggle for it.
                        let mut actions = if unavailable.is_some() {
                            Vec::new()
                        } else if unavailable_reason.is_none() {
                            vec!["set_enabled", "edit"]
                        } else {
                            vec!["edit"]
                        };
                        if effective.overridden && unavailable.is_none() {
                            actions.push("reset");
                        }
                        let enabled = !record
                            .disabled_model_ids
                            .contains(&effective.entry.model_id);
                        let alias = record.model_aliases.get(&effective.entry.model_id).cloned();
                        let catalog_name = (name != effective.entry.display_name).then_some(name);
                        ManagementModel {
                            enabled,
                            model: effective.entry,
                            source: if effective.overridden {
                                "override"
                            } else {
                                "default"
                            },
                            endpoint: Some(effective.source.protocol),
                            missing_limits: effective.missing_limits,
                            unavailable_reason,
                            actions,
                            endpoint_override: None,
                            alias,
                            catalog_name,
                        }
                    })
                    .collect();
                (Some(models), record.model_preferences)
            }
            None => (None, BTreeMap::new()),
        };
        ManagementProvider {
            info,
            models,
            preferences,
            models_listed: None,
        }
    }

    pub async fn management(
        self: &Arc<Self>,
        provider_id: String,
        model_id: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<ManagementView, ProviderError> {
        let db = self.db.clone();
        let id = provider_id.clone();
        let record = self
            .owned(cancel, move |_| {
                Box::pin(async move {
                    db.run_read(move |connection| {
                        let row: Option<(u32, String)> = connection
                            .query_row(
                                "SELECT version,record FROM providers WHERE provider_id=?1",
                                [&id],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .optional()
                            .map_err(storage)?;
                        row.map(|(version, body)| {
                            parse_provider_row(&id, version, &body)
                                .map_err(|error| storage(error.to_string()))
                        })
                        .transpose()
                    })
                    .await
                    .map_err(ProviderError::from)
                })
            })
            .await?;
        let mut errors = Vec::new();
        let mut provider = match record {
            Some(PersistedProviderRecord::Static(record)) => {
                if record.kind == StaticRecordKind::Builtin
                    && let Some(definition) = bundled_seed_for(&self.bundled, &provider_id)?
                    && record.definition_fingerprint != definition.definition_fingerprint
                {
                    let candidate = reseed_candidate(&record, definition);
                    let collisions: Vec<_> = candidate
                        .user_models
                        .iter()
                        .filter(|user| {
                            candidate
                                .models
                                .iter()
                                .any(|model| model.model_id == user.model_id)
                        })
                        .map(|entry| entry.model_id.as_str())
                        .collect();
                    errors.push(ProviderRecordError { provider_id: provider_id.clone(), message: if collisions.is_empty() {
                        "Bundled definition changed; restart to apply it".into()
                    } else { format!("Bundled definition conflicts with user models: {}; delete them and restart", collisions.join(", ")) } });
                }
                self.static_management(provider_id, *record, cancel).await?
            }
            record => {
                let descriptor = self
                    .oauth
                    .providers(cancel)
                    .await?
                    .into_iter()
                    .find(|descriptor| descriptor.provider_id == provider_id)
                    .ok_or(if record.is_some() {
                        ProviderError::Unsupported("Managed account package is unavailable".into())
                    } else {
                        ProviderError::NotFound
                    })?;
                let account = self.oauth.status(&provider_id, cancel).await?;
                let record = record.map(|record| match record {
                    PersistedProviderRecord::Managed(record) => *record,
                    PersistedProviderRecord::Static(_) => unreachable!(),
                });
                Self::managed_management(descriptor, account, record, &self.bundled)
            }
        };
        if let Some(model_id) = model_id {
            let models = provider.models.as_mut().ok_or(ProviderError::NotFound)?;
            models.retain(|model| model.model.model_id == model_id);
            if models.is_empty() {
                return Err(ProviderError::NotFound);
            }
            provider.preferences.retain(|id, _| id == &model_id);
        }
        self.check(cancel)?;
        Ok(ManagementView {
            drivers: management_drivers(),
            providers: vec![provider],
            errors,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagementPageRequest {
    pub provider_id: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
    #[serde(default)]
    pub accounts_only: bool,
    pub query: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementPage {
    pub drivers: Vec<ManagementDriver>,
    pub providers: Vec<ManagementInfo>,
    pub errors: Vec<ProviderRecordError>,
    pub next: Option<String>,
}

impl ProviderService {
    pub async fn management_page(
        self: &Arc<Self>,
        request: ManagementPageRequest,
        cancel: &CancellationToken,
    ) -> Result<ManagementPage, ProviderError> {
        let limit = request.limit.unwrap_or(32);
        if !(1..=128).contains(&limit) || (request.provider_id.is_some() && request.after.is_some())
        {
            return Err(ProviderError::InvalidConfig(
                "Use a page size of 1..128 and either providerId or after".into(),
            ));
        }
        let mut descriptors = self.oauth.providers(cancel).await?;
        let db = self.db.clone();
        let target = request.provider_id.clone();
        let after = request.after.clone();
        let accounts_only = request.accounts_only;
        let query_text = request.query.clone().unwrap_or_default();
        let mut records = self.owned(cancel, move |_| Box::pin(async move {
            db.run_read(move |connection| {
                let sql = if target.is_some() {
                    "SELECT provider_id FROM providers WHERE provider_id=?1"
                } else {
                    "SELECT provider_id FROM providers WHERE provider_id>coalesce(?1,'') AND CASE WHEN json_valid(record) THEN (instr(lower(provider_id),lower(?3))>0 OR instr(lower(json_extract(record,'$.displayName')),lower(?3))>0) AND (?4=0 OR json_extract(record,'$.credentialAuthority') IN ('openai_codex','grok') OR json_extract(record,'$.kind')='managed') ELSE 1 END ORDER BY provider_id LIMIT ?2"
                };
                let mut query = connection.prepare(sql).map_err(storage)?;
                let mut rows = if let Some(target) = target {
                    query.query(params![target]).map_err(storage)?
                } else {
                    query.query(params![after, limit+1, query_text, accounts_only]).map_err(storage)?
                };
                let mut records = Vec::new();
                while let Some(row) = rows.next().map_err(storage)? {
                    let id: String = row.get(0).map_err(storage)?;
                    records.push(id);
                }
                Ok(records)
            }).await.map_err(ProviderError::from)
        })).await?;
        for descriptor in &descriptors {
            if request.query.as_ref().is_none_or(|query| {
                format!("{} {}", descriptor.provider_id, descriptor.display_name)
                    .to_lowercase()
                    .contains(&query.to_lowercase())
            }) && request
                .provider_id
                .as_ref()
                .is_none_or(|id| id == &descriptor.provider_id)
                && request
                    .after
                    .as_ref()
                    .is_none_or(|id| id < &descriptor.provider_id)
                && !records.iter().any(|id| id == &descriptor.provider_id)
            {
                records.push(descriptor.provider_id.clone());
            }
        }
        records.sort();
        let next = (records.len() > limit as usize).then(|| records[limit as usize - 1].clone());
        records.truncate(limit as usize);
        let db = self.db.clone();
        let records = self
            .owned(cancel, move |_| {
                Box::pin(async move {
                    db.run_read(move |connection| {
                        let mut query = connection
                            .prepare("SELECT version,record FROM providers WHERE provider_id=?1")
                            .map_err(storage)?;
                        records
                            .into_iter()
                            .map(|id| {
                                let row: Option<(u32, String)> = query
                                    .query_row([&id], |row| Ok((row.get(0)?, row.get(1)?)))
                                    .optional()
                                    .map_err(storage)?;
                                let record = row.map(|(version, body)| {
                                    parse_provider_row(&id, version, &body)
                                        .map_err(ProviderError::from)
                                });
                                Ok((id, record))
                            })
                            .collect::<dim_agent_runtime::Result<Vec<_>>>()
                    })
                    .await
                    .map_err(ProviderError::from)
                })
            })
            .await?;
        let mut page = ManagementPage {
            drivers: management_drivers(),
            providers: Vec::new(),
            errors: Vec::new(),
            next,
        };
        for (id, record) in records {
            let result = match record.transpose() {
                Ok(Some(PersistedProviderRecord::Static(record))) => {
                    self.static_management_info(id.clone(), &record, cancel)
                        .await
                }
                Ok(record) => {
                    if let Some(index) = descriptors
                        .iter()
                        .position(|descriptor| descriptor.provider_id == id)
                    {
                        let descriptor = descriptors.remove(index);
                        match self.oauth.status(&id, cancel).await {
                            Ok(account) => Ok(Self::managed_management_info(
                                descriptor,
                                account,
                                record.as_ref().and_then(|record| match record {
                                    PersistedProviderRecord::Managed(record) => {
                                        Some(record.as_ref())
                                    }
                                    _ => None,
                                }),
                            )),
                            Err(error) => Err(error.into()),
                        }
                    } else {
                        Err(if record.is_some() {
                            ProviderError::Unsupported(
                                "Managed account package is unavailable".into(),
                            )
                        } else {
                            ProviderError::NotFound
                        })
                    }
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(info) => page.providers.push(info),
                Err(error) if request.provider_id.is_some() => return Err(error),
                Err(error) => page.errors.push(ProviderRecordError {
                    provider_id: id,
                    message: error.to_string(),
                }),
            }
        }
        if request.provider_id.is_some() && page.providers.is_empty() {
            return Err(ProviderError::NotFound);
        }
        self.check(cancel)?;
        Ok(page)
    }
}

fn management_drivers() -> Vec<ManagementDriver> {
    drivers::custom_drivers()
        .map(|driver| ManagementDriver {
            driver_kind: driver,
            auth_types: if driver == "amazon-bedrock" {
                vec!["aws_bedrock"]
            } else {
                vec!["api_key", "bearer", "custom_header", "anonymous"]
            },
            remote_model_list: drivers::remote_model_list(driver),
        })
        .collect()
}
