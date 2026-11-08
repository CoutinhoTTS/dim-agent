use dim_agent_runtime::{
    AgentError, QueryComparison, QueryFilter, QueryOperand, QueryOutput, QueryPage, QueryPick,
    QuerySelect, QuerySpec, QueryValue, Timeline, TimelineQuery, TimelineQueryScope,
};
use dim_agent_types::*;
use std::num::NonZeroU64;

/// Reason of the Cancelled terminal written for a request that a steer input
/// replaced before it had any model output (ADR agent-029 decision 5).
pub const REPLACED_BY_STEER: &str = "replaced by steer";

/// Folded by the owners of a request's preparation and model call from the
/// events they consume: the latest `step/model-start` and whether an input
/// that drives the Loop was applied after it. Such an input is never part of
/// that request: inputs a request consumes are applied before its
/// `step/model-start`. A request handed over after such an input was
/// replaced while its previous owner still held it.
#[derive(Default)]
pub(crate) struct SteerMark(Option<(String, bool)>);
impl SteerMark {
    pub async fn restore(timeline: &Timeline, through: u64) -> dim_agent_runtime::Result<Self> {
        let mut request = TimelineQuery {
            scope: TimelineQueryScope::Branch,
            from: None,
            through: Some(through),
            include_publishing: true,
            query: QuerySpec {
                filter: Some(QueryFilter::Compare {
                    left: QueryOperand::Field {
                        path: "kind".into(),
                    },
                    comparison: QueryComparison::Eq,
                    right: QueryOperand::Value {
                        value: "step/model-start".into(),
                    },
                }),
                select: QuerySelect::Fields {
                    fields: vec![
                        "data.request_id".into(),
                        "position.sequence".into(),
                        "position.branch_id".into(),
                    ],
                },
                group_by: Vec::new(),
                pick: Some(QueryPick::Last),
                order_by: Vec::new(),
            },
            page: Some(QueryPage {
                limit: NonZeroU64::new(1).expect("one latest request"),
                after: None,
            }),
        };
        let reply = timeline.query(request.clone()).await?;
        let QueryOutput::Rows { rows } = reply.result else {
            unreachable!("steer recovery selects request fields")
        };
        let Some(row) = rows.into_iter().next() else {
            return Ok(Self::default());
        };
        let Ok(
            [
                QueryValue::Value {
                    value: serde_json::Value::String(request_id),
                },
                QueryValue::Value {
                    value: serde_json::Value::Number(sequence),
                },
                QueryValue::Value {
                    value: serde_json::Value::String(branch_id),
                },
            ],
        ) = <[_; 3]>::try_from(row.values)
        else {
            return Err(AgentError::fault(
                "steer",
                "a model start has no request identity or position",
            ));
        };
        request.from = Some(TimelinePosition {
            branch_id,
            sequence: sequence
                .as_u64()
                .expect("event sequences are unsigned integers"),
        });
        request.page = None;
        request.query.pick = None;
        request.query.select = QuerySelect::Exists;
        request.query.filter = Some(QueryFilter::And {
            conditions: vec![
                QueryFilter::Compare {
                    left: QueryOperand::Field {
                        path: "kind".into(),
                    },
                    comparison: QueryComparison::Eq,
                    right: QueryOperand::Value {
                        value: "input/applied".into(),
                    },
                },
                QueryFilter::Not {
                    condition: Box::new(QueryFilter::Compare {
                        left: QueryOperand::Field {
                            path: "data.mode".into(),
                        },
                        comparison: QueryComparison::Eq,
                        right: QueryOperand::Value {
                            value: "append".into(),
                        },
                    }),
                },
            ],
        });
        let reply = timeline.query(request).await?;
        let QueryOutput::Exists { exists } = reply.result else {
            unreachable!("steer recovery checks adoption existence")
        };
        Ok(Self(Some((request_id, exists))))
    }

    pub fn apply(&mut self, event: &AgentEvent) {
        match &event.kind {
            EventKind::ModelStart(data) => self.0 = Some((data.request_id.clone(), false)),
            EventKind::InputApplied(data) if data.mode.unwrap_or_default() != InputMode::Append => {
                if let Some((_, steered)) = self.0.as_mut() {
                    *steered = true;
                }
            }
            _ => {}
        }
    }
    /// A driving input was applied after `request_id` started.
    pub fn steered(&self, request_id: &str) -> bool {
        matches!(&self.0, Some((request, true)) if request == request_id)
    }
}
