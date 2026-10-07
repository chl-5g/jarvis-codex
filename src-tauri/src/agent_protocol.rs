use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: u8 = 1;
pub const MAX_BYTES: usize = 64 * 1024;

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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TaskResultPayload {
    pub status: TaskResultStatus,
    pub output: Option<Value>,
    pub error: Option<String>,
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
        return Err("unsupported agent protocol version".into());
    }
    if value.message_id.trim().is_empty()
        || value.from.trim().is_empty()
        || value.to.trim().is_empty()
    {
        return Err("agent envelope identity is required".into());
    }
    if value.expires_at < value.created_at || value.expires_at < current {
        return Err("agent message expired".into());
    }
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("agent message too large".into());
    }
    if matches!(
        value.kind,
        AgentMessageKind::TaskRequest
            | AgentMessageKind::TaskResult
            | AgentMessageKind::ApprovalRequest
            | AgentMessageKind::TaskCancel
    ) && value.task_id.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err("task_id is required".into());
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
        from: from.into(),
        to: to.into(),
        created_at: now(),
        expires_at,
        payload: serde_json::to_value(TaskResultPayload {
            status,
            output,
            error,
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
        from: from.into(),
        to: to.into(),
        created_at: now(),
        expires_at,
        payload: serde_json::to_value(TaskRequestPayload {
            capability: capability.into(),
            input,
            requires_approval,
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
