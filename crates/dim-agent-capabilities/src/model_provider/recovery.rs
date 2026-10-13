use crate::host::{CredentialRecoveryPhase, CredentialRejection, ModelFailureFacts};
use std::time::Duration;

/// Invocation-local I/O control, not replayable business state or a service.
#[derive(Default)]
pub(super) struct RecoveryState {
    pub attempts: u8,
    pub waited: Duration,
    pub phase: CredentialRecoveryPhase,
}

pub(super) enum FailureAction {
    Stop,
    SettleCredentials,
    Retry(Duration),
    RecoverCredentials(Duration),
}

impl RecoveryState {
    pub fn decide(
        &self,
        facts: Option<&ModelFailureFacts>,
        rejection: CredentialRejection,
        forbidden: bool,
        replay_safe: bool,
        jitter: u8,
    ) -> FailureAction {
        // Settlement is not replay: it remains necessary at every replay limit.
        if rejection == CredentialRejection::Terminal {
            return FailureAction::SettleCredentials;
        }
        if forbidden || !replay_safe || self.attempts >= 3 {
            return FailureAction::Stop;
        }
        let recovering = rejection == CredentialRejection::Recoverable;
        if recovering {
            if self.phase == CredentialRecoveryPhase::Recovered {
                return FailureAction::Stop;
            }
        } else if !facts.is_some_and(|facts| facts.transient) {
            return FailureAction::Stop;
        }
        let delay = facts
            .and_then(|facts| facts.retry_after)
            .unwrap_or_else(|| {
                if recovering {
                    Duration::ZERO
                } else {
                    Duration::from_secs_f64(
                        0.5 * f64::from(self.attempts) * (0.75 + 0.25 * f64::from(jitter) / 255.0),
                    )
                }
            });
        if delay > Duration::from_secs(60).saturating_sub(self.waited) {
            return FailureAction::Stop;
        }
        if recovering {
            FailureAction::RecoverCredentials(delay)
        } else {
            FailureAction::Retry(delay)
        }
    }
}
