//! The flows that need the product loader and a controllable `sessions`
//! service: the factory rebuild keeps the install's state, a delete racing a
//! subscribe, a retry after a failed put, a retry while the first append is
//! paused, a changed guide written before the next event, and the install's
//! shared delivery budget (design §2.1, §2.4, §2.5, §3.2, §3.3).
mod support;

use dim_agent_types::InputMode;
use dim_event_hub::{DeclareRequest, PublishRequest, ToTarget, TypeDeclaration};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use support::fixture::{CountingSessionFiles, FailingSessionFiles, LoaderFixture};
use support::wait_for;

fn im_declaration() -> DeclareRequest {
    DeclareRequest {
        source: "im".into(),
        guide: "The IM guide.".into(),
        types: vec![TypeDeclaration {
            r#type: "im.message".into(),
            description: "An instant message".into(),
            mode: InputMode::Queue,
        }],
    }
}

fn event(id: &str) -> PublishRequest {
    PublishRequest {
        source: "im".into(),
        r#type: "im.message".into(),
        id: Some(id.into()),
        title: None,
        body: "body".into(),
        attributes: Default::default(),
        to: Vec::new(),
    }
}

fn event_to(id: &str, sessions: &[String]) -> PublishRequest {
    let mut request = event(id);
    request.to = sessions
        .iter()
        .map(|session| ToTarget {
            session_id: session.clone(),
            branch_id: None,
            mode: None,
        })
        .collect();
    request
}

/// A row built anew from its factory when an injected row reloads keeps every
/// declaration: the producer declared once, and its publish after the reapply
/// is delivered without declaring again (design §2.1, Appendix D item 12).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reapply_keeps_the_declaration() {
    let fixture = LoaderFixture::mount().await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    fixture.subscribe("reload", &["im.*"], None).await.unwrap();

    // `pluginFiles` is one of the rows EventHub injects: reloading it builds
    // EventHub again from its factory.
    fixture.host.loader().restart("PluginFiles").await.unwrap();
    wait_for("the factory's next generation", || {
        fixture.producer.hub().is_some()
    })
    .await;

    let receipt = fixture.producer.publish(event("kept")).await.unwrap();
    assert_eq!(
        receipt["deliveries"][0]["status"],
        json!("delivered"),
        "{receipt}"
    );
    assert_eq!(fixture.producer.declares(), 1, "one declaration");
}

/// A delete that arrives while a subscribe holds the Session lock runs after
/// it: the record is stored first and the handler then removes it, so no
/// subscription of that Session remains (design §3.3, Appendix D item 13).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delete_racing_a_subscribe_leaves_no_subscription() {
    let fixture = Arc::new(LoaderFixture::mount().await);
    fixture.producer.declare(im_declaration()).await.unwrap();
    // The guide write is held in flight: the subscribe owns the Session lock.
    let gate = fixture.sessions.gate_live();
    let subscribing = {
        let fixture = fixture.clone();
        tokio::spawn(async move { fixture.subscribe("branch", &["im.*"], None).await })
    };
    wait_for("the guide write in flight", || gate.max() >= 1).await;
    let deleting = {
        let fixture = fixture.clone();
        tokio::spawn(async move { fixture.session_deleted("branch").await })
    };
    // The handler waits for the lock the subscribe holds; without it the
    // handler runs before the record exists and the subscribe stores it
    // after.
    tokio::time::sleep(Duration::from_millis(200)).await;
    gate.release();
    subscribing.await.unwrap().unwrap();
    deleting.await.unwrap();
    assert!(
        fixture.subscriptions(Some("branch")).await.is_empty(),
        "the delete removed the record the subscribe stored"
    );
}

/// The first publish's put fails and its preview is delivered `truncated`; a
/// retry with the same id answers `repeated: true` with the original ids, no
/// `truncated`, and writes no file and no input (design §3.2, Appendix D
/// item 14).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retry_after_a_failed_put_answers_repeated() {
    let fixture =
        LoaderFixture::mount_with_session_files(Some(Arc::new(FailingSessionFiles))).await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    fixture.subscribe("branch", &["im.*"], None).await.unwrap();
    let mut request = event("huge");
    request.body = format!("{}\n{}", "x".repeat(6000), "tail");

    let first = fixture.producer.publish(request.clone()).await.unwrap();
    let delivery = &first["deliveries"][0];
    assert_eq!(delivery["status"], json!("delivered"), "{first}");
    assert_eq!(delivery["truncated"], json!(true), "{first}");
    assert!(delivery.get("repeated").is_none(), "{first}");
    let (input_id, sequence) = (delivery["inputId"].clone(), delivery["sequence"].clone());
    let calls = fixture.sessions.calls().len();

    let retry = fixture.producer.publish(request).await.unwrap();
    let delivery = &retry["deliveries"][0];
    assert_eq!(delivery["status"], json!("delivered"), "{retry}");
    assert_eq!(delivery["repeated"], json!(true), "{retry}");
    assert!(delivery.get("truncated").is_none(), "{retry}");
    assert_eq!(delivery["inputId"], input_id, "{retry}");
    assert_eq!(delivery["sequence"], sequence, "{retry}");
    assert_eq!(
        fixture.sessions.calls().len(),
        calls,
        "the retry writes nothing"
    );
}

/// A publish that holds its event lock runs in a task EventHub owns: dropping
/// the caller while its input is in flight frees neither the lock nor the
/// write, so a retry with the same id waits for it and answers `repeated` with
/// one put in all (design §2.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retry_while_the_first_append_is_paused_answers_repeated() {
    let files = Arc::new(CountingSessionFiles::default());
    let fixture = LoaderFixture::mount_with_session_files(Some(files.clone())).await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    fixture.subscribe("branch", &["im.*"], None).await.unwrap();
    let gate = fixture.sessions.gate();
    let mut request = event("huge");
    request.body = format!("{}\n{}", "x".repeat(6000), "tail");

    // The first publish's put lands, then its input call is held at the gate.
    let first = {
        let producer = fixture.producer.clone();
        let request = request.clone();
        tokio::spawn(async move { producer.publish(request).await })
    };
    wait_for("the first input in flight", || gate.max() >= 1).await;

    // Dropping the caller's future frees no lock: the publish task owns it.
    first.abort();
    let _ = first.await;

    let retry = {
        let producer = fixture.producer.clone();
        let request = request.clone();
        tokio::spawn(async move { producer.publish(request).await })
    };
    // A retry that did not wait would look up `unknown` while the first input
    // is held and put a second file; one put and `repeated` below are the proof.
    gate.release();
    let receipt = retry.await.unwrap().unwrap();
    let delivery = &receipt["deliveries"][0];
    assert_eq!(delivery["status"], json!("delivered"), "{receipt}");
    assert_eq!(delivery["repeated"], json!(true), "{receipt}");
    assert!(delivery.get("truncated").is_none(), "{receipt}");
    // The subscribe's guide write took `input-1`, so the event's input is
    // the second the fixture committed.
    assert_eq!(delivery["inputId"], json!("input-2"), "{receipt}");
    assert_eq!(delivery["sequence"], json!(2), "{receipt}");
    assert_eq!(files.puts().len(), 1, "one put for the one event");
}

/// A refusal the store re-check answers after the guide inputs were written
/// retires them too, not only a failed write (design §3.3, Appendix C). Two
/// Sessions race for the total limit's last slot; the loser leaves one guide
/// write and its retire on its branch, and no stored subscription.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_limit_refusal_retires_the_guide_the_subscribe_wrote() {
    let fixture = LoaderFixture::mount().await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    let mut other = im_declaration();
    other.source = "other".into();
    other.guide = "The other guide.".into();
    other.types = vec![TypeDeclaration {
        r#type: "other.note".into(),
        description: "A note".into(),
        mode: InputMode::Queue,
    }];
    fixture.producer.declare(other).await.unwrap();

    // The total limit is 1024: the view starts at 1023, so the two subscribes
    // below both pass the early check and only the store re-check refuses one.
    let subscriptions: Vec<serde_json::Value> = (0..1023)
        .map(|index| {
            json!({
                "id": format!("sub_seed_{index}"),
                "types": ["im.*"],
                "target": {"sessionId": format!("seed-{index}"), "branchId": "main"},
                "origin": "client",
                "createdAt": "2026-11-08T09:15:04.120Z",
            })
        })
        .collect();
    let text = json!({"version": 1, "subscriptions": subscriptions}).to_string();
    std::fs::write(
        fixture.file(),
        serde_json::to_vec(&json!({"version": 1, "text": text})).unwrap(),
    )
    .unwrap();
    fixture.host.loader().restart("eventhub").await.unwrap();
    assert_eq!(
        fixture.subscriptions(None).await.len(),
        1023,
        "the seeded view"
    );

    let (first, second) = tokio::join!(
        fixture.subscribe("a", &["im.*"], None),
        fixture.subscribe("b", &["other.*"], None)
    );
    let (loser_session, loser_source, winner_session) = match (&first, &second) {
        (Ok(_), Err(error)) => ("b", "other", "a"),
        (Err(error), Ok(_)) => ("a", "im", "b"),
        other => panic!("one subscribe wins the last slot: {other:?}"),
    };
    let loser_error = match (&first, &second) {
        (_, Err(error)) | (Err(error), _) => error,
        _ => unreachable!(),
    };
    assert_eq!(
        loser_error
            .data
            .as_ref()
            .map(|data| data.app_error.as_str()),
        Some("limit_reached")
    );

    let calls = fixture.sessions.calls();
    let loser: Vec<_> = calls
        .iter()
        .filter(|call| call.session_id == loser_session)
        .collect();
    assert_eq!(
        loser.len(),
        2,
        "one guide write and its retire on {loser_session}: {loser:?}"
    );
    assert_eq!(loser[0].key_use(), Some("live"), "{loser:?}");
    assert_eq!(
        loser[0].key_name(),
        Some(format!("eventhub:guide:{loser_source}").as_str())
    );
    assert_eq!(loser[1].key_use(), Some("retire"), "{loser:?}");
    assert_eq!(
        loser[1].key_name(),
        Some(format!("eventhub:guide:{loser_source}").as_str())
    );
    let winner: Vec<_> = calls
        .iter()
        .filter(|call| call.session_id == winner_session)
        .collect();
    assert_eq!(winner.len(), 1, "the winner keeps its guide: {winner:?}");
    assert_eq!(winner[0].key_use(), Some("live"), "{winner:?}");

    assert_eq!(
        fixture.subscriptions(None).await.len(),
        1024,
        "only the winner's subscription was stored"
    );
}

/// A delivery in flight outlives an EventHub generation: its change lands in
/// the install's subscription view, so the next generation lists and routes
/// by what the file now holds (design §2.1, A.5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delivery_in_flight_updates_the_view_the_next_generation_reads() {
    let fixture = LoaderFixture::mount().await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    fixture.subscribe("branch", &["im.*"], None).await.unwrap();
    fixture.sessions.fail("branch", "branch_ended");
    let gate = fixture.sessions.gate();
    let publishing = {
        let producer = fixture.producer.clone();
        tokio::spawn(async move { producer.publish(event("gone")).await })
    };
    wait_for("the delivery in flight", || gate.max() >= 1).await;

    // Only EventHub is rebuilt; the publish task holds the old generation's
    // file handle, so its change must land in the install's view.
    fixture.host.loader().restart("eventhub").await.unwrap();
    wait_for("the next generation", || fixture.producer.hub().is_some()).await;

    gate.release();
    let receipt = publishing.await.unwrap().unwrap();
    assert_eq!(
        receipt["deliveries"][0]["status"],
        json!("gone"),
        "{receipt}"
    );
    assert_eq!(
        receipt["deliveries"][0]["code"],
        json!("branch_ended"),
        "{receipt}"
    );

    assert!(
        fixture.subscriptions(None).await.is_empty(),
        "the next generation lists no subscription the file dropped"
    );
    let receipt = fixture.producer.publish(event("after")).await.unwrap();
    assert_eq!(receipt["deliveries"], json!([]), "{receipt}");
}

/// A partial publish's retry reuses the files the first attempt stored: one
/// file per Session and event, even across a retry (design §3.2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retry_reuses_the_files_of_a_partial_publish() {
    let files = Arc::new(CountingSessionFiles::default());
    let fixture = LoaderFixture::mount_with_session_files(Some(files.clone())).await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    fixture.subscribe("ok", &["im.*"], None).await.unwrap();
    fixture.subscribe("slow", &["im.*"], None).await.unwrap();
    fixture.sessions.fail("slow", "rejected");
    let mut request = event("huge");
    request.body = format!("{}\n{}", "x".repeat(6000), "tail");

    // The first publish is held at its inputs, so the retry waits for the
    // event lock while that attempt is still in flight.
    let gate = fixture.sessions.gate();
    let first = {
        let producer = fixture.producer.clone();
        let request = request.clone();
        tokio::spawn(async move { producer.publish(request).await })
    };
    wait_for("the first publish's input in flight", || gate.max() >= 1).await;
    let retry = {
        let producer = fixture.producer.clone();
        tokio::spawn(async move { producer.publish(request).await })
    };
    gate.release();
    let first = first.await.unwrap().unwrap();
    assert_eq!(
        first["deliveries"][0]["status"],
        json!("delivered"),
        "{first}"
    );
    assert_eq!(first["deliveries"][1]["status"], json!("failed"), "{first}");
    assert_eq!(files.puts().len(), 2, "one file per Session");

    let retry = retry.await.unwrap().unwrap();
    assert_eq!(retry["deliveries"][0]["repeated"], json!(true), "{retry}");
    assert_eq!(
        files.puts().len(),
        2,
        "the retry writes no second file: {:?}",
        files.puts()
    );
}

/// A redeclared source's new guide is written to a subscribed branch before
/// its next event, and a later publish writes no guide again (design §3.3,
/// Appendix D item 15).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_changed_guide_is_written_before_the_next_event() {
    let fixture = LoaderFixture::mount().await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    fixture.subscribe("branch", &["im.*"], None).await.unwrap();
    fixture.sessions.clear();

    let mut redeclared = im_declaration();
    redeclared.guide = "The new IM guide.".into();
    fixture.producer.declare(redeclared).await.unwrap();
    fixture.producer.publish(event("refreshed")).await.unwrap();
    let calls = fixture.sessions.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(
        calls[0]
            .submission_id
            .starts_with("eventhub-guide-refresh:im:"),
        "{calls:?}"
    );
    assert_eq!(calls[0].key_use(), Some("live"), "{calls:?}");
    assert!(calls[0].text.contains("The new IM guide."), "{calls:?}");
    assert!(
        calls[1].submission_id.starts_with("eventhub:im:"),
        "{calls:?}"
    );

    // The record now carries the new digest: no second guide write.
    fixture.sessions.clear();
    fixture.producer.publish(event("second")).await.unwrap();
    let calls = fixture.sessions.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].submission_id.starts_with("eventhub:im:"),
        "{calls:?}"
    );
}

/// Two concurrent publishes of eight targets each never run more than eight
/// `sessions.input` calls together: the budget belongs to the install, not to
/// one publish (design §2.5, Appendix D item 16).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_publishes_share_the_install_delivery_budget() {
    let fixture = LoaderFixture::mount().await;
    fixture.producer.declare(im_declaration()).await.unwrap();
    let gate = fixture.sessions.gate();
    let targets =
        |prefix: &str| -> Vec<String> { (0..8).map(|index| format!("{prefix}-{index}")).collect() };
    let first = event_to("one", &targets("a"));
    let second = event_to("two", &targets("b"));
    let one = {
        let producer = fixture.producer.clone();
        tokio::spawn(async move { producer.publish(first).await })
    };
    let two = {
        let producer = fixture.producer.clone();
        tokio::spawn(async move { producer.publish(second).await })
    };
    wait_for("eight deliveries in flight", || gate.max() >= 8).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        gate.max(),
        8,
        "one budget for the install, not one per publish"
    );
    gate.release();
    for task in [one, two] {
        let receipt = task.await.unwrap().unwrap();
        let deliveries = receipt["deliveries"].as_array().unwrap();
        assert_eq!(deliveries.len(), 8, "{receipt}");
        assert!(
            deliveries
                .iter()
                .all(|delivery| delivery["status"] == json!("delivered")),
            "{receipt}"
        );
    }
    assert_eq!(gate.max(), 8);
}
