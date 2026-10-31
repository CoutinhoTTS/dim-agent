use crate::ApprovalCallData;
use serde::{Deserialize, Serialize};

pub const MEMBER_APPROVAL_ALLOW: &str = "member_approval_allow";
pub const MEMBER_APPROVAL_REJECT: &str = "member_approval_reject";
pub const MEMBER_APPROVAL_GET: &str = "session.collaboration.approval.get";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberApprovalTarget {
    pub agent_id: String,
    pub session_id: String,
    pub branch_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberApprovalDescriptor {
    pub delegation_id: String,
    pub member: MemberApprovalTarget,
    pub request_id: String,
    pub call: ApprovalCallData,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase", deny_unknown_fields)]
pub enum MemberApprovalDetail {
    Pending {
        #[serde(rename = "approvalId")]
        approval_id: String,
        descriptor: Box<MemberApprovalDescriptor>,
    },
    Ended,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberApprovalEnded {
    pub branch_id: String,
    pub approval_id: String,
}
