use super::Worker;
use crate::model::session::Update;
use crate::report::{Action, Captured, Draft, Prepared, Reply, Request};
use serde_json::{Value, json};

impl Worker {
    pub(super) fn report_available(&self) -> bool {
        self.client.offers("report.prepare") || self.methods.contains("session.report.prepare")
    }

    pub(super) fn reportable(&self, error: Captured, show_error: bool) -> bool {
        if !self.report_available() {
            return false;
        }
        self.send(Update::Reportable { error, show_error });
        true
    }

    pub(super) async fn report_request(&mut self, request: Request) {
        let Request { panel, action } = request;
        let reply = match action {
            Action::Prepare(error) => self.prepare_report(error).await,
            Action::Plan {
                draft,
                redact,
                instructions,
            } => {
                let (method, params) = report_call(
                    &draft,
                    "plan",
                    json!({
                        "reportId": draft.report_id, "generation": {"kind":"local"},
                        "redact": redact, "instructions": instructions,
                    }),
                );
                self.client.typed(&method, params).await.map(Reply::Plan)
            }
            Action::Generate(draft) => {
                let (method, params) = report_call(
                    &draft,
                    "generate",
                    json!({
                        "reportId":draft.report_id,"planId":draft.plan_id,
                    }),
                );
                match self.client.send(&method, params).await {
                    Ok(id) => {
                        self.report_generations.insert(id, panel);
                        return;
                    }
                    Err(error) => Err(error),
                }
            }
            Action::Cancel { draft, generation } => {
                let params = if generation {
                    json!({"reportId":draft.report_id,"target":"generation","planId":draft.plan_id})
                } else {
                    json!({"reportId":draft.report_id,"target":"report"})
                };
                let (method, params) = report_call(&draft, "cancel", params);
                let _ = self.client.send(&method, params).await;
                return;
            }
        };
        self.report_reply(panel, reply);
    }

    async fn prepare_report(&mut self, error: Captured) -> crate::Result<Reply> {
        if !error.session.is_empty() {
            let mut params = error.params(true);
            params["sessionId"] = json!(error.session);
            match self
                .client
                .typed::<Prepared>("session.report.prepare", params)
                .await
            {
                Ok(prepared) => return Ok(Reply::Prepared(Some(error.session), prepared)),
                Err(failure)
                    if !self.client.offers("report.prepare")
                        || !session_admission_failed(&failure) =>
                {
                    return Err(failure);
                }
                Err(_) => {}
            }
        }
        if !self.client.offers("report.prepare") {
            return Err("Error reporting is unavailable on this App Server".into());
        }
        self.client
            .typed::<Prepared>("report.prepare", error.params(false))
            .await
            .map(|prepared| Reply::Prepared(None, prepared))
    }

    pub(super) fn report_reply(&self, panel: u64, result: crate::Result<Reply>) {
        self.send(Update::ReportReply {
            panel,
            reply: result.unwrap_or_else(|error| Reply::Failed(error.to_string())),
        });
    }
}

fn report_call(draft: &Draft, suffix: &str, mut params: Value) -> (String, Value) {
    let namespace = match &draft.session {
        Some(session) => {
            params["sessionId"] = json!(session);
            "session.report"
        }
        None => "report",
    };
    (format!("{namespace}.{suffix}"), params)
}

fn session_admission_failed(error: &crate::Error) -> bool {
    matches!(
        dim_client::app_error(error),
        Some("session_closed" | "session_build_failed" | "session_protocol_version")
    ) || error
        .downcast_ref::<dim_client::RpcError>()
        .is_some_and(|rpc| rpc.code == -32603 && rpc.message == "Session operation failed")
}
