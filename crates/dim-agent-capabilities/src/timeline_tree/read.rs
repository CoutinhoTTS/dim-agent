use super::*;

pub struct DirectoryDisplay {
    pub entry: BranchDirectoryEntry,
    pub purpose: Option<String>,
    pub cut_messages: Option<u64>,
    pub end: Option<BranchEndData>,
}

impl TreeCommands {
    pub async fn directory_display(&self) -> Result<Vec<DirectoryDisplay>> {
        let tree = self
            .tree
            .upgrade()
            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))?;
        let mut entries = Vec::new();
        for entry in tree.directory() {
            let mut purpose = None;
            let mut cut_messages = None;
            if let (Some(parent), Some(anchor), Some(cut)) = (
                &entry.info.parent,
                entry.info.create_anchor,
                entry.info.branch_point,
            ) {
                let point = self
                    .query(
                        parent.clone(),
                        TimelineQuery {
                            scope: dim_agent_runtime::TimelineQueryScope::Branch,
                            from: Some(dim_agent_types::TimelinePosition {
                                branch_id: parent.clone(),
                                sequence: anchor,
                            }),
                            through: Some(anchor),
                            include_publishing: false,
                            query: QuerySpec {
                                filter: None,
                                select: QuerySelect::Fields {
                                    fields: vec!["data.prompt".into()],
                                },
                                group_by: vec![],
                                pick: None,
                                order_by: vec![],
                            },
                            page: Some(QueryPage {
                                limit: std::num::NonZeroU64::MIN,
                                after: None,
                            }),
                        },
                    )
                    .await?;
                let QueryOutput::Rows { rows } = point.result else {
                    unreachable!("branch purpose")
                };
                purpose = rows
                    .first()
                    .and_then(|row| match &row.values[0] {
                        QueryValue::Value { value } => value.as_str(),
                        _ => None,
                    })
                    .map(|text| text.lines().next().unwrap_or_default().trim().to_owned());
                let count = self
                    .prefix_query(
                        parent.clone(),
                        TimelineQuery {
                            scope: dim_agent_runtime::TimelineQueryScope::Chain,
                            from: None,
                            through: Some(cut),
                            include_publishing: false,
                            query: QuerySpec {
                                filter: Some(QueryFilter::And {
                                    conditions: vec![
                                        QueryFilter::In {
                                            value: QueryOperand::Field {
                                                path: "kind".into(),
                                            },
                                            values: vec!["input/message".into()],
                                        },
                                        QueryFilter::Present {
                                            value: QueryOperand::Field {
                                                path: "data.user".into(),
                                            },
                                        },
                                        QueryFilter::Not {
                                            condition: Box::new(QueryFilter::Exists {
                                                condition: Box::new(QueryFilter::And {
                                                    conditions: vec![
                                                        QueryFilter::In {
                                                            value: QueryOperand::Field {
                                                                path: "kind".into(),
                                                            },
                                                            values: vec!["input/deleted".into()],
                                                        },
                                                        QueryFilter::Compare {
                                                            left: QueryOperand::Field {
                                                                path: "data.input_id".into(),
                                                            },
                                                            comparison: QueryComparison::Eq,
                                                            right: QueryOperand::Outer {
                                                                path: "id".into(),
                                                            },
                                                        },
                                                    ],
                                                }),
                                            }),
                                        },
                                    ],
                                }),
                                select: QuerySelect::Count,
                                group_by: vec![],
                                pick: None,
                                order_by: vec![],
                            },
                            page: None,
                        },
                    )
                    .await?;
                let QueryOutput::Count { count } = count.result else {
                    unreachable!("branch cut message count")
                };
                cut_messages = Some(count);
            }
            let end = if let Some(at) = entry.end_event.filter(|at| *at <= entry.head) {
                let result = self
                    .query(
                        entry.info.id.clone(),
                        TimelineQuery {
                            scope: dim_agent_runtime::TimelineQueryScope::Branch,
                            from: Some(dim_agent_types::TimelinePosition {
                                branch_id: entry.info.id.clone(),
                                sequence: at,
                            }),
                            through: Some(at),
                            include_publishing: false,
                            query: QuerySpec {
                                filter: None,
                                select: QuerySelect::Fields {
                                    fields: vec!["data".into()],
                                },
                                group_by: vec![],
                                pick: None,
                                order_by: vec![],
                            },
                            page: Some(QueryPage {
                                limit: std::num::NonZeroU64::MIN,
                                after: None,
                            }),
                        },
                    )
                    .await?;
                let QueryOutput::Rows { rows } = result.result else {
                    unreachable!("branch end")
                };
                rows.first().map(|row| {
                    let QueryValue::Value { value } = &row.values[0] else {
                        unreachable!("branch end data")
                    };
                    serde_json::from_value(value.clone()).expect("branch end contract")
                })
            } else {
                None
            };
            entries.push(DirectoryDisplay {
                entry,
                purpose,
                cut_messages,
                end,
            });
        }
        Ok(entries)
    }

    async fn prefix_query(
        &self,
        id: String,
        query: TimelineQuery,
    ) -> Result<dim_agent_runtime::TimelineQueryReply> {
        let tree = self
            .tree
            .upgrade()
            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))?;
        if let Some(context) = tree.owned_context(&id) {
            return context.timeline().query(query).await;
        }
        let head = tree
            .ended_head(&id)
            .ok_or_else(|| AgentError::UnknownBranch(id.clone()))?;
        let through = query.through.unwrap_or(head);
        if through > head {
            return Err(AgentError::InvalidInput(
                "directory prefix exceeds branch head".into(),
            ));
        }
        let mut scope = tree.stored_scope(&id, 1, through);
        let mut info = tree
            .info(&id)
            .ok_or_else(|| AgentError::UnknownBranch(id.clone()))?;
        while let (Some(parent), Some(cut)) = (info.parent, info.branch_point) {
            scope
                .segments
                .extend(tree.stored_scope(&parent, 1, cut).segments);
            info = tree
                .info(&parent)
                .ok_or(AgentError::UnknownBranch(parent))?;
        }
        scope.segments.reverse();
        let result = tree
            .store
            .query(&scope, &query.query, query.page.as_ref())
            .await?;
        Ok(dim_agent_runtime::TimelineQueryReply {
            through,
            result: result.result,
            next: result.next,
        })
    }
}
