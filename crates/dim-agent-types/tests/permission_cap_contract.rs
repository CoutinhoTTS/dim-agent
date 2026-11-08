//! `PermissionPolicy::cap` and `within` against the shared policy-cap fixtures
//! that the App Server launch and the Scheduler both rely on.
use dim_agent_types::PermissionPolicy;
use serde_json::Value;
use std::collections::BTreeSet;

fn policy(value: &Value) -> PermissionPolicy {
    serde_json::from_value(value.clone()).unwrap()
}

fn names(list: &[String]) -> BTreeSet<&str> {
    list.iter().map(String::as_str).collect()
}

#[test]
fn cap_and_within_match_the_shared_fixtures() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/permission-policy-cap.json"
    ))
    .unwrap();
    for case in fixtures["cap"].as_array().unwrap() {
        let name = &case["name"];
        let requested = policy(&case["requested"]);
        let capped = requested.cap(&policy(&case["bound"]));
        let expected = policy(&case["capped"]);
        assert_eq!(capped.preset, expected.preset, "{name}");
        assert_eq!(
            names(&capped.allowed_tools),
            names(&expected.allowed_tools),
            "{name}"
        );
        assert_eq!(
            names(&capped.disallowed_tools),
            names(&expected.disallowed_tools),
            "{name}"
        );
        assert_eq!(
            Value::Bool(requested.within(&policy(&case["bound"]))),
            case["within"],
            "{name}"
        );
    }
    for case in fixtures["reuse"].as_array().unwrap() {
        let name = &case["name"];
        let actual = policy(&case["x"]);
        for (bound, expected) in [
            ("savedEffective", "withinSaved"),
            ("liveBound", "withinLive"),
        ] {
            assert_eq!(
                Value::Bool(actual.within(&policy(&case[bound]))),
                case[expected],
                "{name} {bound}"
            );
        }
    }
}
