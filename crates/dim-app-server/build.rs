//! Exports existing Session and branch services to TypeScript parts by name
//! (branch-typescript.md), generating their dispatch and declarations.
fn main() {
    rutis_bridge::build::Exports::new()
        .service_in("treeCommands", "dim_agent_capabilities::timeline_tree::TreeCommands", "session")
        .service_in("dispatcher", "dim_agent_runtime::Dispatcher", "branch")
        .service_in("branchLifecycle", "dim_agent_runtime::BranchLifecycle", "branch")
        .service_in("timelineIdentity", "dim_agent_runtime::TimelineIdentity", "branch")
        // Its sequence and timestamp use custom serde; the shape is npm/plugin.d.ts's AgentEvent.
        .declared(
            "dim_agent_types::AgentEvent",
            "{ id: string; sequence: number | bigint; create_at: number; tags?: string[]; type: string; name: string; data: Record<string, unknown> }",
        )
        // Inline, so the data does not share the service's TypeScript name.
        .declared(
            "dim_agent_runtime::TimelineIdentity",
            "{ sessionId: string; branchId: string }",
        )
        .generate()
        .expect("generate the exported services");
}
