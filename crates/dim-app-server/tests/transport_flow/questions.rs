//! Questions through the public RPC interface (design Appendix D): Rpc
//! routes `session.question.respond` to the Session it names, the answer
//! arrives as the batch's `step/tool-result`, and a fork cannot cut a
//! pending question.
use super::deletion::Store;
use super::*;

const ASK: &str = r#"tool:ask_user {"questions":[{"header":"Database","question":"Which database?","options":[{"label":"PostgreSQL","description":"managed"},{"label":"SQLite"}]}]}"#;

async fn launch_questions(store: &Store) -> RunningServer {
    launch_with(
        Arc::new(MockTools),
        store.config(),
        dim_agent_questions::install,
    )
    .await
    .0
}
async fn session(client: &mut Client, interactive: bool) -> String {
    let mut params = create(json!(["ask_user"]));
    params["interactive"] = json!(interactive);
    let created = client.rpc("session.create", params).await;
    assert_ok(&created);
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    assert_ok(&client.rpc("session.attach", json!({"sessionId":sid})).await);
    sid
}
async fn submit(client: &mut Client, sid: &str, text: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":sid,"branchId":"main","text":text}),
            )
            .await,
    );
}
/// The next `session.event` step named `name`, as its notice params.
async fn step(client: &mut Client, name: &str) -> Value {
    loop {
        let notice = client.notice("session.event").await;
        let event = &notice["params"]["event"];
        if event["type"] == "step" && event["name"] == name {
            return notice["params"].clone();
        }
    }
}
fn respond(sid: &str, call: &str, answers: Value) -> Value {
    json!({"sessionId":sid,"branchId":"main","callId":call,"answers":answers})
}
fn app_error(reply: &Value) -> &str {
    reply["error"]["data"]["appError"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an error reply: {reply}"))
}
/// Waits until the handler registered `call`: an empty answer list is
/// refused as invalid only while the call waits.
async fn waiting(client: &mut Client, sid: &str, call: &str) {
    for _ in 0..500 {
        let reply = client
            .rpc("session.question.respond", respond(sid, call, json!([])))
            .await;
        match app_error(&reply) {
            "invalid_params" => return,
            "question_not_pending" => tokio::time::sleep(Duration::from_millis(20)).await,
            other => panic!("{other}: {reply}"),
        }
    }
    panic!("{call} never waited");
}

/// Acceptance 1, 4 and 8 on the wire: the method is listed for the Session,
/// rejected answers keep the call waiting, the first answer is the call's
/// result and a second finds nothing; the batch's `step/tool-result` carries
/// the answer and a fork at the pending call is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_answers_a_waiting_question_through_rpc() {
    let store = Store::new();
    let server = launch_questions(&store).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let sid = session(&mut client, true).await;
    let methods = client
        .rpc("session.methods", json!({"sessionId":sid}))
        .await;
    assert!(
        methods["result"]["methods"]
            .as_array()
            .unwrap()
            .contains(&json!({"name":"session.question.respond","mutating":true})),
        "{methods}"
    );

    submit(&mut client, &sid, ASK).await;
    let asked = step(&mut client, "tool-call").await;
    assert_eq!(asked["branchId"], "main");
    let call = &asked["event"]["data"]["calls"][0];
    assert_eq!(call["name"], "ask_user");
    let call_id = call["id"].as_str().unwrap().to_owned();
    let call_sequence = asked["event"]["sequence"].as_u64().unwrap();
    waiting(&mut client, &sid, &call_id).await;

    // A fork cuts only at a rest point, so never at a pending question.
    let fork = client
        .rpc("session.fork", json!({"sessionId":sid,"at":call_sequence}))
        .await;
    assert_eq!(app_error(&fork), "session_invalid_input");
    assert!(
        fork["error"]["message"]
            .as_str()
            .unwrap()
            .contains("without results"),
        "{fork}"
    );

    for (params, code) in [
        (
            json!({"sessionId":sid,"branchId":"main","callId":call_id}),
            "invalid_params",
        ),
        (
            respond(
                &sid,
                &call_id,
                json!([{"questionId":"1","selected":["MySQL"]}]),
            ),
            "invalid_params",
        ),
        (
            json!({"sessionId":sid,"branchId":"main","callId":call_id,"declined":true,"extra":1}),
            "invalid_params",
        ),
        (
            respond(
                &sid,
                "other",
                json!([{"questionId":"1","selected":["SQLite"]}]),
            ),
            "question_not_pending",
        ),
        (
            json!({"sessionId":sid,"branchId":"child","callId":call_id,"declined":true}),
            "question_not_pending",
        ),
    ] {
        let reply = client.rpc("session.question.respond", params).await;
        assert_eq!(app_error(&reply), code, "{reply}");
    }
    let answer = respond(
        &sid,
        &call_id,
        json!([{"questionId":"1","selected":["SQLite"]}]),
    );
    let accepted = client.rpc("session.question.respond", answer.clone()).await;
    assert_eq!(accepted["result"], json!({}), "{accepted}");
    let again = client.rpc("session.question.respond", answer).await;
    assert_eq!(app_error(&again), "question_not_pending");
    assert_eq!(again["error"]["code"], -32000);

    let settled = step(&mut client, "tool-result").await;
    let data = &settled["event"]["data"];
    assert_eq!(data["call_event_id"], asked["event"]["id"]);
    assert_eq!(data["results"][0]["call_id"], call_id.as_str());
    assert_eq!(
        data["results"][0]["result"],
        json!({
            "content": "User answered:\n1. Which database?\n   -> SQLite",
            "structuredContent": {"answers":[{"questionId":"1","selected":["SQLite"]}],"declined":false}
        })
    );
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":sid,"branchId":"main"}))
            .await,
    );
    server.shutdown();
    server.join().await.unwrap();
}

/// Acceptance 2 on the wire: `session.create {interactive: false}` declares
/// `ask_user` and lists the method, and the call returns the non-interactive
/// error at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_non_interactive_session_answers_ask_user_at_once() {
    let store = Store::new();
    let server = launch_questions(&store).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let sid = session(&mut client, false).await;
    let methods = client
        .rpc("session.methods", json!({"sessionId":sid}))
        .await;
    assert!(
        methods["result"]["methods"]
            .as_array()
            .unwrap()
            .contains(&json!({"name":"session.question.respond","mutating":true}))
    );
    submit(&mut client, &sid, ASK).await;
    let settled = step(&mut client, "tool-result").await;
    assert_eq!(
        settled["event"]["data"]["results"][0]["result"],
        json!({
            "content": "No one is available to answer questions in this session. Proceed without asking.",
            "isError": true
        })
    );
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":sid,"branchId":"main"}))
            .await,
    );
    server.shutdown();
    server.join().await.unwrap();
}
