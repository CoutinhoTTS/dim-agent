use crate::{
    Error, Migration, ObjectResult, Outcome, Result, TaskState,
    engine::{check, publish, request},
    source::{Source, fingerprint},
};
use dim_client::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Imported {
    status: String,
    job_id: String,
}

pub(crate) async fn import(
    owner: &Migration,
    task: &mut TaskState,
    source: &Source,
    client: &mut Client,
) -> Result<()> {
    for issue in &source.inventory.issues {
        owner
            .ledger
            .object(
                &task.id,
                &ObjectResult {
                    kind: "file".into(),
                    id: "schedule-tui.sqlite".into(),
                    outcome: Outcome::Failed,
                    target_id: None,
                    reason: Some(issue.clone()),
                    omitted: Vec::new(),
                },
            )
            .await?;
    }
    let sessions = owner
        .ledger
        .report(&task.id)
        .await?
        .objects
        .into_iter()
        .filter(|result| result.kind == "session")
        .map(|result| (result.id.clone(), result))
        .collect::<BTreeMap<_, _>>();
    let jobs = source.job_ids()?;
    task.total = Some(jobs.len() as u64);
    task.processed = 0;
    for (terminal, id) in jobs {
        check(owner, &task.id).await?;
        let source_id = format!("legacy-{}:{id}", if terminal { "cli" } else { "desktop" });
        task.current = Some(source_id.clone());
        publish(owner, task).await?;
        let result=async {
            let job=source.job(terminal,&id)?;
            let mut patch=json!({"name":job.name,"prompt":job.prompt,"cwd":job.cwd,"model":job.model,"enabled":job.enabled,"mode":if job.session_mode=="new" {"create"}else{"reuse"}});
            if let Some(bound)=&job.bound_session {
                let imported=sessions.get(bound).ok_or_else(||Error::Deferred("Bound Session has not been imported".into()))?;
                if imported.outcome==Outcome::Deferred {return Err(Error::Deferred("Bound Session is deferred; its job remains in the legacy home".into()));}
                if !matches!(imported.outcome,Outcome::Imported|Outcome::AlreadyImported) {return Err(Error::Deferred("Bound Session is unavailable in the target".into()));}
                patch["session_id"]=json!(imported.target_id.as_ref().ok_or_else(||Error::Source("Imported Session receipt lacks its target identity".into()))?);
            }
            let recurrence=json!({"rrule":job.rrule,"created_at":job.created_at,"next_run_at":job.next_run_at,"last_run_at":job.last_run_at});
            let digest=fingerprint(&(&source_id,&patch,&recurrence))?;
            let params=json!({"source_id":source_id,"fingerprint":digest,"job":patch,"recurrence":recurrence});
            let imported:Imported=request(client,"schedule.import_legacy",params).await?;
            let outcome=match imported.status.as_str(){"imported"=>Outcome::Imported,"already_imported"=>Outcome::AlreadyImported,"conflict"=>Outcome::Conflict,_=>return Err(Error::Target("Unknown Scheduler import outcome".into()))};
            Ok(ObjectResult {kind:"scheduled_job".into(),id:source_id.clone(),outcome,target_id:Some(imported.job_id),reason:None,omitted:if job.reasoning_effort.is_some(){vec!["reasoningEffort".into()]}else{Vec::new()}})
        }.await;
        let result = match result {
            Ok(result) => result,
            Err(Error::Object(reason)) => ObjectResult {
                kind: "scheduled_job".into(),
                id: source_id.clone(),
                outcome: Outcome::Failed,
                target_id: None,
                reason: Some(reason),
                omitted: Vec::new(),
            },
            Err(Error::Unsupported(reason)) => ObjectResult {
                kind: "scheduled_job".into(),
                id: source_id.clone(),
                outcome: Outcome::Unsupported,
                target_id: None,
                reason: Some(reason),
                omitted: Vec::new(),
            },
            Err(Error::Deferred(reason)) => ObjectResult {
                kind: "scheduled_job".into(),
                id: source_id.clone(),
                outcome: Outcome::Deferred,
                target_id: None,
                reason: Some(reason),
                omitted: Vec::new(),
            },
            Err(error) => return Err(error),
        };
        owner.ledger.object(&task.id, &result).await?;
        task.processed += 1;
        publish(owner, task).await?;
    }
    Ok(())
}
