use super::*;
use dim_agent_runtime::{AgentBuilder, AgentConfig, SessionConfig};
use dim_agent_types::EventKind;
use std::time::Duration;

mod deferred_inputs;
use deferred_inputs::DeferredInputs;

/// The typed entry is crate-private; exercise its full server/Session flow
/// here without adding a public test access API.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn typed_input_retains_admitted_io_and_accepts_each_submission_once() {
    let (store, mut held) = DeferredInputs::new();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(store);
    dim_agent_capabilities::defaults::install(&mut builder);
    dim_agent_capabilities::defaults::mock_model_hosts(&mut builder);
    dim_agent_capabilities::defaults::tool_host(
        &mut builder,
        Arc::new(dim_agent_capabilities::adapters::MockTools),
    );
    let server = crate::serve(builder, crate::ServerConfig::default())
        .await
        .unwrap();
    let session = server
        .agent()
        .unwrap()
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let index = server.root.get::<AgentIndex>().unwrap();
    let write = |submission: &str| InputWrite {
        session_id: session.id().into(),
        branch_id: "main".into(),
        mode: InputMode::Append,
        submission_id: submission.into(),
        raw_blocks: vec![RawBlock {
            text: "Typed input settlement".into(),
            key: None,
        }],
        content: vec![],
    };
    let request = write("settlement:typed:publication");
    let caller = index.clone();
    let call = tokio::spawn(async move { caller.input(request).await });
    let waiting = tokio::time::timeout(Duration::from_secs(5), held.recv())
        .await
        .unwrap()
        .unwrap();
    let (restarted, ()) = tokio::join!(server.restart("Agents"), async {
        index.stopping.cancelled().await;
        waiting.release.send(()).unwrap();
    });
    restarted.unwrap();
    let receipt = call.await.unwrap().unwrap();
    assert!(session.publication("main").unwrap().sequence() >= receipt.sequence);
    assert!(matches!(
        index.input(write("test:retired-index")).await,
        Err(SessionsError::Reloading { .. })
    ));

    let index = server.root.get::<AgentIndex>().unwrap();
    assert!(matches!(
        index
            .lookup(LookupRequest {
                session_id: session.id().into(),
                branch_id: "main".into(),
                submission_id: "test:retired-index".into(),
            })
            .await
            .unwrap(),
        LookupReply::Unknown
    ));
    let request = write("settlement:typed:dropped");
    let call = tokio::spawn(async move { index.input(request).await });
    let waiting = tokio::time::timeout(Duration::from_secs(5), held.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(waiting.submission_id, "settlement:typed:dropped");
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    // A resubmission while the first write is still unpublished writes
    // nothing and replies its receipt once it is published.
    let resubmitted = tokio::spawn({
        let index = server.root.get::<AgentIndex>().unwrap();
        let request = write("settlement:typed:dropped");
        async move { index.input(request).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!resubmitted.is_finished());
    waiting.release.send(()).unwrap();
    let receipt = tokio::time::timeout(Duration::from_secs(5), resubmitted)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(receipt.sequence, waiting.sequence);
    assert!(held.try_recv().is_err());
    tokio::time::timeout(
        Duration::from_secs(5),
        published(&session.publication("main").unwrap(), waiting.sequence),
    )
    .await
    .unwrap()
    .unwrap();
    let index = server.root.get::<AgentIndex>().unwrap();
    let recovered = index
        .lookup(LookupRequest {
            session_id: session.id().into(),
            branch_id: "main".into(),
            submission_id: "settlement:typed:dropped".into(),
        })
        .await
        .unwrap();
    let LookupReply::Accepted { input_id, sequence } = recovered else {
        panic!("the dropped input was not found");
    };
    assert_eq!(
        (input_id.as_str(), sequence),
        (receipt.input_id.as_str(), receipt.sequence)
    );
    // After publication too, a resubmission replies the first receipt.
    let again = index
        .input(write("settlement:typed:dropped"))
        .await
        .unwrap();
    assert_eq!((again.input_id, again.sequence), (input_id, sequence));
    // Concurrent submissions with one id write one input and share its receipt.
    let (one, two) = tokio::join!(
        index.input(write("test:concurrent")),
        index.input(write("test:concurrent"))
    );
    let (one, two) = (one.unwrap(), two.unwrap());
    assert_eq!((one.input_id, one.sequence), (two.input_id, two.sequence));
    let history = session.local("main", ..).await.unwrap();
    for submission in [
        "settlement:typed:publication",
        "settlement:typed:dropped",
        "test:concurrent",
    ] {
        assert_eq!(history.iter().filter(|event| matches!(&event.kind, EventKind::InputMessage(data) if data.submission_id.as_deref() == Some(submission))).count(), 1);
    }
    server.shutdown();
    server.join().await.unwrap();
}
