//! The process owner's shared state and the parts of the interface that read
//! it: `declare`, `eventhub.sources`, `eventhub.subscriptions`, and the
//! deletion notice (design §2.1, §3.4, A.1, A.3).
use crate::{
    catalog::Catalog,
    error::EventHubError,
    model::{
        DeclareRequest, ListRequest, SubscribeRequest, UnsubscribeRequest, validate_declaration,
    },
    store::{Store, Subscription},
};
use rutis::Ctx;
use rutis_bridge::session::HostDispatch;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The state one generation of the process part owns.
pub(crate) struct Shared {
    pub(crate) ctx: Ctx,
    pub(crate) sessions: Arc<dyn HostDispatch>,
    pub(crate) catalog: Arc<Catalog>,
    /// Serializes every `subscribe` and `unsubscribe` from its first check to
    /// its last guide write or retire (design §3.3). It belongs to the plugin
    /// instance, not this generation: an in-flight creation from the previous
    /// generation still serializes with this one.
    pub(crate) creation: Arc<tokio::sync::Mutex<()>>,
    pub(crate) store: Arc<Store>,
    /// The generation's cancellation, passed to the files a publish stores.
    pub(crate) stop: CancellationToken,
}

impl Shared {
    pub(crate) async fn declare(
        &self,
        ctx: &Ctx,
        request: DeclareRequest,
    ) -> Result<(), EventHubError> {
        validate_declaration(&request)?;
        self.catalog.declare(ctx, &request)
    }

    pub(crate) fn sources(&self) -> Value {
        json!({ "sources": self.catalog.report() })
    }

    pub(crate) fn subscriptions(&self, request: ListRequest) -> Value {
        let subscriptions: Vec<Subscription> = self
            .store
            .read()
            .into_iter()
            .filter(|subscription| {
                request
                    .session_id
                    .as_deref()
                    .is_none_or(|session_id| subscription.target.session_id == session_id)
            })
            .collect();
        json!({ "subscriptions": subscriptions })
    }

    /// Removes every subscription whose target is the deleted Session, in one
    /// transaction (design §3.4). A failed write leaves them, which the
    /// design accepts as the cost of a missed notice; there is no caller to
    /// answer.
    pub(crate) async fn session_deleted(&self, session_id: &str) {
        let target = session_id.to_owned();
        let result = self
            .store
            .change(move |subscriptions| {
                let before = subscriptions.len();
                subscriptions.retain(|subscription| subscription.target.session_id != target);
                Ok((subscriptions.len() != before, ()))
            })
            .await;
        if let Err(error) = result {
            tracing::warn!(%error, session_id, "EventHub could not remove a deleted Session's subscriptions");
        }
    }

    pub(crate) async fn subscribe(
        &self,
        origin: crate::model::Origin,
        request: SubscribeRequest,
    ) -> Result<Value, EventHubError> {
        crate::subscribe::subscribe(self, origin, request).await
    }

    pub(crate) async fn unsubscribe(
        &self,
        request: UnsubscribeRequest,
    ) -> Result<Value, EventHubError> {
        crate::subscribe::unsubscribe(self, &request.subscription_id).await
    }
}
