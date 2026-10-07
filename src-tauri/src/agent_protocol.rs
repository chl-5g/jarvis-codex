use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: u8 = 1;
pub const MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub version: String,
    pub capabilities: AgentCardCapabilities,
    pub skills: Vec<AgentSkill>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCardCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_modes: Vec<String>,
    pub output_modes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePart {
    pub kind: String,
    pub text: Option<String>,
    pub data: Option<Value>,
    pub uri: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessage {
    pub role: String,
    pub parts: Vec<MessagePart>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub name: Option<String>,
    pub parts: Vec<MessagePart>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    AgentHello,
    TaskRequest,
    TaskResult,
    ApprovalRequest,
    TaskCancel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum TaskResultStatus {
    Completed,
    Failed,
    Rejected,
    Expired,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEnvelope {
    pub version: u8,
    pub message_id: String,
    pub kind: AgentMessageKind,
    pub task_id: Option<String>,
    pub context_id: Option<String>,
    pub reference_task_ids: Vec<String>,
    pub from: String,
    pub to: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRequestPayload {
    pub capability: String,
    pub input: Value,
    pub requires_approval: bool,
    pub message: Option<AgentMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TaskResultPayload {
    pub status: TaskResultStatus,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub artifacts: Vec<Artifact>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}-{}", now(), uuidish())
}
fn uuidish() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    format!("{:x}", N.fetch_add(1, Ordering::Relaxed))
}

pub fn validate_inbound(value: &AgentEnvelope, current: u64) -> Result<(), String> {
    if value.version != VERSION {
        return Err(crate::config::agent_message("unsupportedVersion").into());
    }
    if value.message_id.trim().is_empty()
        || value.from.trim().is_empty()
        || value.to.trim().is_empty()
    {
        return Err(crate::config::agent_message("identityRequired").into());
    }
    if value.expires_at < value.created_at || value.expires_at < current {
        return Err(crate::config::agent_message("expired").into());
    }
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err(crate::config::agent_message("tooLarge").into());
    }
    if matches!(
        value.kind,
        AgentMessageKind::TaskRequest
            | AgentMessageKind::TaskResult
            | AgentMessageKind::ApprovalRequest
            | AgentMessageKind::TaskCancel
    ) && value.task_id.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err(crate::config::agent_message("taskIdRequired").into());
    }
    Ok(())
}

pub fn task_result(
    from: &str,
    to: &str,
    task_id: &str,
    status: TaskResultStatus,
    output: Option<Value>,
    error: Option<String>,
    expires_at: u64,
) -> AgentEnvelope {
    AgentEnvelope {
        version: VERSION,
        message_id: new_id("msg"),
        kind: AgentMessageKind::TaskResult,
        task_id: Some(task_id.into()),
        context_id: None,
        reference_task_ids: Vec::new(),
        from: from.into(),
        to: to.into(),
        created_at: now(),
        expires_at,
        payload: serde_json::to_value(TaskResultPayload {
            status,
            output,
            error,
            artifacts: Vec::new(),
        })
        .unwrap_or(Value::Null),
    }
}

pub fn task_request(
    from: &str,
    to: &str,
    capability: &str,
    input: Value,
    expires_at: u64,
    requires_approval: bool,
) -> AgentEnvelope {
    AgentEnvelope {
        version: VERSION,
        message_id: new_id("msg"),
        kind: AgentMessageKind::TaskRequest,
        task_id: Some(new_id("task")),
        context_id: Some(new_id("context")),
        reference_task_ids: Vec::new(),
        from: from.into(),
        to: to.into(),
        created_at: now(),
        expires_at,
        payload: serde_json::to_value(TaskRequestPayload {
            capability: capability.into(),
            input,
            requires_approval,
            message: None,
        })
        .unwrap_or(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_request_round_trips() {
        let e = task_request(
            "a",
            "b",
            "read_file",
            serde_json::json!({"path":"x"}),
            now() + 60,
            false,
        );
        let bytes = serde_json::to_vec(&e).unwrap();
        let decoded: AgentEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.kind, AgentMessageKind::TaskRequest);
        assert!(validate_inbound(&decoded, now()).is_ok());
    }
    #[test]
    fn expired_and_missing_identity_rejected() {
        let mut e = task_request("a", "b", "read_file", serde_json::json!({}), 1, false);
        assert!(validate_inbound(&e, 2).is_err());
        e.expires_at = now() + 60;
        e.from.clear();
        assert!(validate_inbound(&e, now()).is_err());
    }
    #[test]
    fn unknown_version_rejected() {
        let mut e = task_request(
            "a",
            "b",
            "read_file",
            serde_json::json!({}),
            now() + 60,
            false,
        );
        e.version = 2;
        assert!(validate_inbound(&e, now()).is_err());
    }
}
