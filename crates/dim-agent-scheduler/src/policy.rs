//! The authority a run may use (design §3.3): the conservative cap of a
//! requested policy by the job's saved authority, computed at save and launch.
use crate::runs::SchedulerError;
use dim_agent_types::{PermissionPolicy, PermissionPreset, ToolApproval};
use std::collections::HashSet;

/// Whether `actual` is already inside `bound`: capping it changes nothing. An
/// invalid pair is not inside.
pub fn within(actual: &PermissionPolicy, bound: &PermissionPolicy) -> bool {
    let Ok(capped) = cap(actual, bound) else {
        return false;
    };
    let same_names = |left: &[String], right: &[String]| {
        left.iter().collect::<HashSet<_>>() == right.iter().collect::<HashSet<_>>()
    };
    actual.preset == capped.preset
        && actual.file_access() == capped.file_access()
        && actual.tool_approval() == capped.tool_approval()
        && same_names(&actual.allowed_tools, &capped.allowed_tools)
        && same_names(&actual.disallowed_tools, &capped.disallowed_tools)
}

fn rank(preset: PermissionPreset) -> u8 {
    match preset {
        PermissionPreset::ReadOnly => 0,
        PermissionPreset::WorkspaceWrite => 1,
        PermissionPreset::FullAccess => 2,
    }
}

/// `cap(requested, bound)`: the lower preset, the union of the denials, and
/// only the allowed tools both sides allow unconditionally. The result is
/// never wider than either side. An explicit Ask cannot be capped by a legacy
/// read-only policy, because the two do not have one representation.
pub fn cap(
    requested: &PermissionPolicy,
    bound: &PermissionPolicy,
) -> Result<PermissionPolicy, SchedulerError> {
    let legacy_read_only = [requested, bound].iter().any(|policy| {
        policy.tool_approval.is_none() && policy.preset == PermissionPreset::ReadOnly
    });
    if legacy_read_only
        && [requested, bound]
            .iter()
            .any(|policy| policy.tool_approval == Some(ToolApproval::Ask))
    {
        return Err(SchedulerError::new(
            "schedule_invalid",
            "Explicit Ask tool approval cannot be capped by a legacy read-only policy",
        ));
    }
    let allows = |policy: &PermissionPolicy, tool: &String| {
        policy.tool_approval() == ToolApproval::Bypass || policy.allowed_tools.contains(tool)
    };
    let mut disallowed_tools = Vec::new();
    for tool in requested
        .disallowed_tools
        .iter()
        .chain(&bound.disallowed_tools)
    {
        if !disallowed_tools.contains(tool) {
            disallowed_tools.push(tool.clone());
        }
    }
    let mut allowed_tools = Vec::new();
    for tool in requested.allowed_tools.iter().chain(&bound.allowed_tools) {
        if allows(requested, tool)
            && allows(bound, tool)
            && !disallowed_tools.contains(tool)
            && !allowed_tools.contains(tool)
        {
            allowed_tools.push(tool.clone());
        }
    }
    Ok(PermissionPolicy {
        preset: if rank(requested.preset) <= rank(bound.preset) {
            requested.preset
        } else {
            bound.preset
        },
        file_access: (requested.file_access.is_some() || bound.file_access.is_some())
            .then(|| requested.file_access().min(bound.file_access())),
        tool_approval: if legacy_read_only {
            None
        } else if requested.tool_approval.is_some() || bound.tool_approval.is_some() {
            Some(
                if requested.tool_approval() == ToolApproval::Bypass
                    && bound.tool_approval() == ToolApproval::Bypass
                {
                    ToolApproval::Bypass
                } else {
                    ToolApproval::Ask
                },
            )
        } else {
            None
        },
        allowed_tools,
        disallowed_tools,
    })
}
