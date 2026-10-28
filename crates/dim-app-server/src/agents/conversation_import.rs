use super::*;
use dim_agent_types::{
    ConversationBegin, ConversationChunk, ConversationFileChunk, ConversationFileReceipt,
    ConversationReceipt, ConversationStatus, ModelSelection,
};

impl AgentIndex {
    fn imported(&self, receipt: ConversationReceipt) -> ConversationReceipt {
        if receipt.status == ConversationStatus::Committed {
            self.insert_version(&receipt.session_id, &receipt.agent_id, 3);
        }
        receipt
    }

    pub(crate) async fn import_begin(
        &self,
        request: ConversationBegin,
    ) -> dim_agent_runtime::Result<ConversationReceipt> {
        if !Path::new(&request.cwd).is_absolute() || request.cwd.contains('\0') {
            return Err(AgentError::InvalidInput(
                "Imported cwd must be an absolute path".into(),
            ));
        }
        if request
            .model
            .as_ref()
            .is_some_and(|model| model.provider_id.is_empty() || model.model_id.is_empty())
        {
            return Err(AgentError::InvalidInput(
                "Imported model identity must not be empty".into(),
            ));
        }
        let host = self
            .host(None)
            .ok_or_else(|| AgentError::Reloading("default Agent".into()))?;
        let mut config = session_config(&host.profile, &self.session_defaults);
        config.cwd = request.cwd.clone();
        if let Some(model) = &request.model {
            config.model = ModelSelection {
                provider_id: model.provider_id.clone(),
                model_id: model.model_id.clone(),
                capability_snapshot: None,
                config: None,
                metadata: None,
            };
        }
        config.permission_policy = sessions::cap(
            request
                .policy
                .as_ref()
                .unwrap_or(&dim_agent_types::PermissionPolicy::default()),
            &self.session_defaults.permission_policy,
        );
        config.interactive = true;
        let receipt = self
            .import_store()?
            .begin_conversation(request, config, host.agent_id.clone())
            .await?;
        Ok(self.imported(receipt))
    }

    fn import_store(&self) -> dim_agent_runtime::Result<&SessionStore> {
        self.session_store.as_deref().ok_or_else(|| {
            AgentError::InvalidInput("Conversation import needs native storage".into())
        })
    }

    pub(crate) async fn import_chunk(
        &self,
        request: ConversationChunk,
    ) -> dim_agent_runtime::Result<ConversationReceipt> {
        self.import_store()?.append_conversation(request).await
    }

    pub(crate) async fn import_file(
        &self,
        request: ConversationFileChunk,
        cancel: CancellationToken,
    ) -> dim_agent_runtime::Result<Option<ConversationFileReceipt>> {
        self.import_store()?
            .conversation_file(request, cancel)
            .await
    }

    pub(crate) async fn import_finish(
        &self,
        key: String,
    ) -> dim_agent_runtime::Result<ConversationReceipt> {
        let receipt = self.import_store()?.finish_conversation(key).await?;
        Ok(self.imported(receipt))
    }
}
