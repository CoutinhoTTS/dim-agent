use crate::{
    branch::{goal_inputs, live_inputs},
    fold::Fold,
    text,
};
use dim_agent_capabilities::timeline_tree::TreeCommands;
use dim_agent_runtime::{
    AgentError, QueryComparison, QueryFilter, QueryOperand, QueryOutput, QueryPage, QueryPick,
    QuerySelect, QuerySpec, QueryValue, TimelineQuery, TimelineQueryScope,
};
use dim_agent_types::{GoalRead, GoalView, GoalViewStatus, InputBlock, TimelinePosition};
use std::num::NonZeroU64;
use tokio_util::sync::CancellationToken;

pub(crate) async fn current(
    tree: &TreeCommands,
    branch_id: &str,
    cancel: &CancellationToken,
) -> Result<GoalRead, AgentError> {
    let mut query = TimelineQuery {
        scope: TimelineQueryScope::Branch,
        from: None,
        through: None,
        include_publishing: false,
        query: QuerySpec {
            rows: None,
            filter: Some(QueryFilter::In {
                value: QueryOperand::Field {
                    path: "kind".into(),
                },
                values: vec!["branch/start".into(), "session/forked".into()],
            }),
            select: QuerySelect::Fields {
                fields: vec!["position.sequence".into()],
            },
            group_by: vec![],
            pick: Some(QueryPick::Last),
            order_by: vec![],
        },
        page: Some(QueryPage {
            limit: NonZeroU64::MIN,
            after: None,
        }),
    };
    let opening = tree.query(branch_id.to_owned(), query.clone()).await?;
    let through = opening.through;
    let QueryOutput::Rows { rows } = opening.result else {
        unreachable!("opening position")
    };
    let start = rows.first().map(|row| {
        let QueryValue::Value { value } = &row.values[0] else {
            unreachable!("opening position")
        };
        value.as_u64().expect("opening sequence")
    });
    query.from = start.map(|sequence| TimelinePosition {
        branch_id: branch_id.into(),
        sequence,
    });
    query.through = Some(through);
    query.query.pick = None;
    query.page.as_mut().unwrap().limit = NonZeroU64::new(256).unwrap();
    query.query.filter = Some(QueryFilter::And {
        conditions: vec![
            kind("input/message"),
            QueryFilter::Exists {
                condition: Box::new(QueryFilter::And {
                    conditions: vec![
                        kind("input/deleted"),
                        QueryFilter::Compare {
                            left: QueryOperand::Field {
                                path: "data.input_id".into(),
                            },
                            comparison: QueryComparison::Eq,
                            right: QueryOperand::Outer { path: "id".into() },
                        },
                    ],
                }),
            },
        ],
    });
    let mut fold = Fold::default();
    loop {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled("Goal read".into()));
        }
        let page = tree.query(branch_id.to_owned(), query.clone()).await?;
        let QueryOutput::Rows { rows } = page.result else {
            unreachable!("deleted input positions")
        };
        for row in rows {
            let QueryValue::Value { value } = &row.values[0] else {
                unreachable!("input position")
            };
            fold.deleted_sequences
                .insert(value.as_u64().expect("input sequence"));
        }
        query.page.as_mut().unwrap().after = page.next;
        if query.page.as_ref().unwrap().after.is_none() {
            break;
        }
    }
    query.query.filter = Some(live_inputs(goal_inputs()));
    query.query.select = QuerySelect::Fields {
        fields: [
            "position.sequence",
            "id",
            "data.submission_id",
            "data.blocks",
        ]
        .map(String::from)
        .to_vec(),
    };
    loop {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled("Goal read".into()));
        }
        let page = tree.query(branch_id.to_owned(), query.clone()).await?;
        let QueryOutput::Rows { rows } = page.result else {
            unreachable!("Goal input fields")
        };
        for row in rows {
            let [
                QueryValue::Value { value: sequence },
                QueryValue::Value { value: id },
                submission,
                QueryValue::Value { value: blocks },
            ] = row.values.as_slice()
            else {
                unreachable!("Goal input fields")
            };
            let sequence = sequence.as_u64().expect("input sequence");
            if let Some(next) = fold.held_next()
                && next < sequence
            {
                pair(tree, branch_id, &query, &mut fold, next).await?;
            }
            let submission = match submission {
                QueryValue::Missing => None,
                QueryValue::Value { value } => Some(value.as_str().expect("submission id")),
                _ => unreachable!("submission scalar"),
            };
            let blocks: Vec<InputBlock> =
                serde_json::from_value(blocks.clone()).expect("input blocks contract");
            fold.goal_input(
                sequence,
                id.as_str().expect("input id"),
                submission,
                &blocks,
            );
            fold.compact(start);
        }
        query.page.as_mut().unwrap().after = page.next;
        if query.page.as_ref().unwrap().after.is_none() {
            break;
        }
    }
    if let Some(next) = fold.held_next()
        && next <= through
    {
        pair(tree, branch_id, &query, &mut fold, next).await?;
    }
    Ok(GoalRead {
        branch_id: branch_id.into(),
        through,
        goal: fold.objective.map(|objective| GoalView {
            objective,
            status: match fold.state {
                text::State::Active => GoalViewStatus::Active,
                text::State::Paused(reason) => GoalViewStatus::Paused(reason),
                text::State::Completed(_) => GoalViewStatus::Completed,
                text::State::Blocked(_) => GoalViewStatus::Blocked,
                text::State::None => GoalViewStatus::None,
            },
            evaluator: fold.judge == text::Judge::Evaluator,
        }),
    })
}

fn kind(name: &str) -> QueryFilter {
    QueryFilter::In {
        value: QueryOperand::Field {
            path: "kind".into(),
        },
        values: vec![name.into()],
    }
}

async fn pair(
    tree: &TreeCommands,
    branch_id: &str,
    scope: &TimelineQuery,
    fold: &mut Fold,
    sequence: u64,
) -> Result<(), AgentError> {
    let mut query = scope.clone();
    query.query.filter = Some(live_inputs(QueryFilter::In {
        value: QueryOperand::Field {
            path: "position.sequence".into(),
        },
        values: vec![sequence.into()],
    }));
    query.query.select = QuerySelect::Fields {
        fields: vec!["data.submission_id".into()],
    };
    query.page = Some(QueryPage {
        limit: NonZeroU64::MIN,
        after: None,
    });
    let page = tree.query(branch_id.to_owned(), query).await?;
    let QueryOutput::Rows { rows } = page.result else {
        unreachable!("trigger identity")
    };
    let submission = rows.first().and_then(|row| match &row.values[0] {
        QueryValue::Missing => None,
        QueryValue::Value { value } => Some(value.as_str().expect("submission id")),
        _ => unreachable!("submission scalar"),
    });
    fold.pair_next(sequence, submission);
    Ok(())
}
