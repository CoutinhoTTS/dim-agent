use crate::{
    methods::{Interruption, MethodDecl, RpcError, SessionMethods, params},
    timeline_tree::TreeCommands,
};
use dim_agent_runtime::*;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::{Ctx, Error};
use serde_json::json;
use std::{num::NonZeroU64, sync::Arc};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    #[serde(rename = "sessionId")]
    _session_id: String,
    branch_id: String,
    through: Option<u64>,
    turn_id: Option<String>,
}

pub struct LoopMethodsPlugin {
    tree: Key<TreeCommands>,
    methods: Key<SessionMethods>,
}
impl LoopMethodsPlugin {
    pub fn new(names: &Names<'_>) -> std::result::Result<Self, Error> {
        Ok(Self {
            tree: names.key()?,
            methods: names.key()?,
        })
    }
}
impl DynPlugin for LoopMethodsPlugin {
    fn name(&self) -> &str {
        "LoopMethods"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.tree.erase(), self.methods.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let tree = ctx.require_at(&self.tree)?;
            ctx.require_at(&self.methods)?.register(ctx, "session.turn", vec![MethodDecl::new("session.turn.get", false, Interruption::Report)], Arc::new(move |call, cancel, _| {
                let tree = tree.clone();
                Box::pin(async move {
                    let p: Params = params(call.params)?;
                    let mut conditions = vec![QueryFilter::In { value: QueryOperand::Field { path: "kind".into() }, values: vec![json!("turn/start"), json!("turn/end")] }];
                    if let Some(id) = &p.turn_id {
                        conditions.push(QueryFilter::In { value: QueryOperand::Field { path: "data.turn_id".into() }, values: vec![json!(id)] });
                    }
                    let query = TimelineQuery {
                        scope: TimelineQueryScope::Branch, from: None, through: p.through, include_publishing: false,
                        query: QuerySpec {
                            filter: Some(QueryFilter::And { conditions }),
                            select: QuerySelect::Fields { fields: ["kind", "position.sequence", "data.turn_id"].map(String::from).to_vec() },
                            group_by: vec![], pick: None,
                            order_by: vec![QueryOrder { key: QueryOrderKey::Position, direction: TimelineQueryDirection::Descending }],
                        },
                        page: Some(QueryPage { limit: NonZeroU64::MIN, after: None }),
                    };
                    let result = tokio::select! {
                        _ = cancel.cancelled() => return Err(RpcError::interrupted()),
                        result = tree.query(p.branch_id.clone(), query) => result?,
                    };
                    let QueryOutput::Rows { rows } = result.result else { unreachable!("turn fields") };
                    let turn = rows.into_iter().next().map(|row| {
                        let [QueryValue::Value { value: kind }, QueryValue::Value { value: sequence }, QueryValue::Value { value: id }] = row.values.as_slice() else { unreachable!("turn boundary") };
                        json!({"turnId":id,"sequence":sequence,"ended":kind == "turn/end"})
                    });
                    let mut output = None;
                    if let Some(turn) = &turn && turn["ended"] == false && p.turn_id.is_none() {
                        let mut request = TimelineQuery {
                            scope: TimelineQueryScope::Branch, from: None, through: Some(result.through), include_publishing: false,
                            query: QuerySpec {
                                filter: Some(QueryFilter::And { conditions: vec![
                                    QueryFilter::In { value: QueryOperand::Field { path: "kind".into() }, values: vec![json!("step/model-start")] },
                                    QueryFilter::In { value: QueryOperand::Field { path: "data.turn_id".into() }, values: vec![turn["turnId"].clone()] },
                                    QueryFilter::Not { condition: Box::new(QueryFilter::Exists { condition: Box::new(QueryFilter::And { conditions: vec![
                                        QueryFilter::In { value: QueryOperand::Field { path: "kind".into() }, values: vec![json!("step/model-output-end")] },
                                        QueryFilter::Compare { left: QueryOperand::Field { path: "data.request_id".into() }, comparison: QueryComparison::Eq, right: QueryOperand::Outer { path: "data.request_id".into() } },
                                    ] }) }) },
                                ] }),
                                select: QuerySelect::Fields { fields: vec!["data.request_id".into()] }, group_by: vec![], pick: Some(QueryPick::Last), order_by: vec![],
                            }, page: Some(QueryPage { limit: NonZeroU64::MIN, after: None }),
                        };
                        let current = tokio::select! {
                            _ = cancel.cancelled() => return Err(RpcError::interrupted()),
                            result = tree.query(p.branch_id.clone(), request.clone()) => result?,
                        };
                        let QueryOutput::Rows { rows } = current.result else { unreachable!("request identity") };
                        if let Some(row) = rows.first() {
                            let QueryValue::Value { value: id } = &row.values[0] else { unreachable!("request id") };
                            let mut active = dim_agent_types::ActiveOutput { request_id: id.as_str().expect("request id").into(), text_characters: 0, thinking_characters: 0 };
                            for (kind, count) in [("step/text", &mut active.text_characters), ("step/thinking", &mut active.thinking_characters)] {
                                request.query.filter = Some(QueryFilter::And { conditions: vec![
                                    QueryFilter::In { value: QueryOperand::Field { path: "kind".into() }, values: vec![json!(kind)] },
                                    QueryFilter::In { value: QueryOperand::Field { path: "data.request_id".into() }, values: vec![id.clone()] },
                                ] });
                                request.query.select = QuerySelect::Characters { field: "data.delta".into() };
                                request.query.pick = None;
                                request.page = None;
                                let sum = tokio::select! {
                                    _ = cancel.cancelled() => return Err(RpcError::interrupted()),
                                    result = tree.query(p.branch_id.clone(), request.clone()) => result?,
                                };
                                let QueryOutput::Count { count: total } = sum.result else { unreachable!("character total") };
                                *count = total;
                            }
                            output = Some(active);
                        }
                    }
                    Ok(json!({"branchId":p.branch_id,"through":result.through,"turn":turn,"output":output}))
                })
            }))?;
            Ok(Effect::Done)
        })
    }
}
