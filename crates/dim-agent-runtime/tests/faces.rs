use dim_agent_runtime::{AgentBuilder, AgentConfig};

struct Demo;

#[test]
fn internal_reasons_are_recorded_by_catalog_name() {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.internal_name("secretHost", "secret");
    builder.internal::<Demo>("owner-api");

    assert_eq!(builder.faces().reason("secretHost"), Some("secret"));
    assert_eq!(builder.faces().reason("demo"), Some("owner-api"));
    assert_eq!(builder.faces().reason("dataHost"), None);
    assert_eq!(builder.faces().iter().count(), 2);
}
