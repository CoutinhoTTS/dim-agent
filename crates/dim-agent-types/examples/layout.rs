fn main() {
    println!(
        "AgentEvent={} EventKind={} ContextCompactionResult={} ToolResult={}",
        std::mem::size_of::<dim_agent_types::AgentEvent>(),
        std::mem::size_of::<dim_agent_types::EventKind>(),
        std::mem::size_of::<dim_agent_types::ContextCompactionResult>(),
        std::mem::size_of::<dim_agent_types::ToolResult>()
    );
}
