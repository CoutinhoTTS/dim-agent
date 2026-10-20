//! Extension events (ADR 001, issue #2): types the core does not define are
//! carried unchanged in the common envelope; the core's domains stay reserved
//! for its typed events.
use dim_agent_types::*;
use serde_json::json;

fn event(kind: serde_json::Value) -> serde_json::Value {
    let mut event = json!({"id":"e1","sequence":7,"create_at":1});
    event
        .as_object_mut()
        .unwrap()
        .extend(kind.as_object().unwrap().clone());
    event
}

#[test]
fn an_undefined_type_round_trips_as_an_extension_event() {
    let wire = event(json!({"type":"goal","name":"set","data":{"objective":"ship","n":[1,2]}}));
    let decoded: AgentEvent = serde_json::from_value(wire.clone()).unwrap();
    let EventKind::Extension(extension) = &decoded.kind else {
        panic!("{decoded:?}");
    };
    assert_eq!(
        (extension.domain(), extension.name(), decoded.kind.key()),
        ("goal", "set", "goal/set")
    );
    assert_eq!(extension.data(), &json!({"objective":"ship","n":[1,2]}));
    assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
    // The frozen v1 union has no view of it.
    assert!(v1::EventRef::new(&decoded).is_none());
}

#[test]
fn reserved_domains_decode_only_through_their_typed_contract() {
    for domain in RESERVED_DOMAINS {
        assert!(
            ExtensionEvent::new(domain, "anything", json!({})).is_err(),
            "{domain}"
        );
    }
    // A core event with a payload outside its contract, or an unknown name in
    // a core domain, is an error, never an extension event.
    for kind in [
        json!({"type":"input","name":"message","data":{"unexpected":1}}),
        json!({"type":"input","name":"forged","data":{}}),
        json!({"type":"turn","name":"end","data":null}),
    ] {
        let error = serde_json::from_value::<AgentEvent>(event(kind.clone())).unwrap_err();
        assert!(error.to_string().contains("reserved"), "{kind}: {error}");
    }
}

#[test]
fn the_envelope_needs_a_plain_domain_and_name() {
    for (domain, name) in [
        ("", "x"),
        ("x", ""),
        ("a/b", "x"),
        ("x", "a/b"),
        ("x", "*"),
        ("*", "x"),
    ] {
        assert!(
            ExtensionEvent::new(domain, name, json!(null)).is_err(),
            "{domain}/{name}"
        );
        let wire = event(json!({"type":domain,"name":name,"data":null}));
        assert!(
            serde_json::from_value::<AgentEvent>(wire).is_err(),
            "{domain}/{name}"
        );
    }
    let missing_data = event(json!({"type":"goal","name":"set"}));
    assert!(serde_json::from_value::<AgentEvent>(missing_data).is_err());
}
