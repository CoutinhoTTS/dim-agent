use dim_agent_types::MemberApprovalDescriptor;

pub fn lines(descriptor: &MemberApprovalDescriptor) -> Vec<String> {
    let member = &descriptor.member;
    let mut lines = vec![
        format!(
            "Member: {}  Session: {}",
            member.agent_id, member.session_id
        ),
        format!("Member branch: {}", member.branch_id),
        format!("Tool: {}", descriptor.call.name),
        format!("Member reason: {}", descriptor.reason),
    ];
    lines.extend(
        serde_json::to_string_pretty(&descriptor.call.input)
            .expect("tool input serializes")
            .lines()
            .map(str::to_owned),
    );
    lines
        .into_iter()
        .map(|line| crate::sanitize::safe(&line))
        .collect()
}
