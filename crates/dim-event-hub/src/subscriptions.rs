//! The `eventSubscriptions` process service (design §6, Appendix F): the
//! subscription core for the `events` tool, scoped to the caller's own
//! Session.
//!
//! It is Rust only — not a shared name, no host dispatch — so a row of another
//! language cannot manage a Session's subscriptions and `eventHub` stays
//! producer-only. Each method answers what the management core answers; the
//! service adds only the caller's scope.
use crate::{
    error::EventHubError,
    hub::Shared,
    model::{ListRequest, Origin, SubscribeRequest, SubscribeTarget, UnsubscribeRequest},
    subscribe::subscription_unknown,
};
use serde_json::Value;
use std::sync::Arc;

/// The subscription service: `names::name_of::<EventSubscriptions>()` is
/// `eventSubscriptions` (design Appendix F). The Session part's `events` tool
/// reads it with `Key<EventSubscriptions>`.
pub struct EventSubscriptions {
    shared: Arc<Shared>,
}

impl EventSubscriptions {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    /// The declared sources and their types, what `eventhub.sources` answers
    /// (design A.3).
    pub fn sources(&self) -> Value {
        self.shared.sources()
    }

    /// Creates one subscription for `session_id`'s `branch_id` with origin
    /// `tool`; the lock, guide write and storage rules are
    /// `eventhub.subscribe`'s own (design A.3, §3.3).
    pub async fn subscribe(
        &self,
        session_id: &str,
        branch_id: Option<&str>,
        mut request: SubscribeRequest,
    ) -> Result<Value, EventHubError> {
        request.target = SubscribeTarget {
            session_id: session_id.to_owned(),
            branch_id: branch_id.map(str::to_owned),
        };
        self.shared.subscribe(Origin::Tool, request).await
    }

    /// Removes one of `session_id`'s subscriptions. A subscription of another
    /// Session answers `subscription_unknown` in the same words as an unknown
    /// id, so the tool learns nothing about it (design §6). A subscription's
    /// target never changes, so the check needs no lock of its own.
    pub async fn unsubscribe(
        &self,
        session_id: &str,
        subscription_id: &str,
    ) -> Result<Value, EventHubError> {
        let belongs = self.shared.store.read().into_iter().any(|subscription| {
            subscription.id == subscription_id && subscription.target.session_id == session_id
        });
        if !belongs {
            return Err(subscription_unknown(subscription_id));
        }
        self.shared
            .unsubscribe(UnsubscribeRequest {
                subscription_id: subscription_id.to_owned(),
            })
            .await
    }

    /// The subscriptions of `session_id`, what `eventhub.subscriptions
    /// {sessionId}` answers (design A.3).
    pub fn list(&self, session_id: &str) -> Value {
        self.shared.subscriptions(ListRequest {
            session_id: Some(session_id.to_owned()),
        })
    }
}
