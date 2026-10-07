use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RemoteTaskStatus {
    Queued,
    AwaitingApproval,
    Running,
    Completed,
    Failed,
    Rejected,
    Expired,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTask {
    pub id: String,
    pub message_id: String,
    pub peer: String,
    pub capability: String,
    pub input: Value,
    pub status: RemoteTaskStatus,
    pub expires_at: u64,
    pub requires_approval: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TaskFile {
    tasks: Vec<RemoteTask>,
    seen: Vec<String>,
}
#[derive(Clone)]
pub struct TaskStore {
    path: PathBuf,
}
#[allow(dead_code)]
impl TaskStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn list(&self) -> Vec<RemoteTask> {
        self.load().tasks
    }
    pub fn create(&self, t: RemoteTask) -> Result<RemoteTask, String> {
        let mut f = self.load();
        if f.tasks.iter().any(|x| x.id == t.id) {
            return Err("task already exists".into());
        }
        f.tasks.push(t.clone());
        self.persist(&f)?;
        Ok(t)
    }
    pub fn accept_once(&self, msg: &str) -> Result<bool, String> {
        let mut f = self.load();
        if f.seen.iter().any(|x| x == msg) {
            return Ok(false);
        }
        f.seen.push(msg.into());
        if f.seen.len() > 2048 {
            f.seen.drain(..512);
        }
        self.persist(&f)?;
        Ok(true)
    }
    pub fn transition(&self, id: &str, next: RemoteTaskStatus) -> Result<RemoteTask, String> {
        let mut f = self.load();
        let t = f
            .tasks
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or("task not found")?;
        if terminal(&t.status) {
            return Err("task is terminal".into());
        }
        if !allowed(&t.status, &next) {
            return Err("invalid task transition".into());
        }
        t.status = next;
        let out = t.clone();
        self.persist(&f)?;
        Ok(out)
    }
    fn load(&self) -> TaskFile {
        fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn persist(&self, f: &TaskFile) -> Result<(), String> {
        if let Some(p) = self.path.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?
        }
        let tmp = self.path.with_extension("tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(f).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(tmp, &self.path).map_err(|e| e.to_string())
    }
}
#[allow(dead_code)]
fn terminal(s: &RemoteTaskStatus) -> bool {
    matches!(
        s,
        RemoteTaskStatus::Completed
            | RemoteTaskStatus::Failed
            | RemoteTaskStatus::Rejected
            | RemoteTaskStatus::Expired
            | RemoteTaskStatus::Cancelled
    )
}
#[allow(dead_code)]
fn allowed(a: &RemoteTaskStatus, b: &RemoteTaskStatus) -> bool {
    matches!(
        (a, b),
        (
            RemoteTaskStatus::Queued,
            RemoteTaskStatus::Running
                | RemoteTaskStatus::AwaitingApproval
                | RemoteTaskStatus::Rejected
                | RemoteTaskStatus::Expired
                | RemoteTaskStatus::Cancelled
        ) | (
            RemoteTaskStatus::AwaitingApproval,
            RemoteTaskStatus::Running
                | RemoteTaskStatus::Rejected
                | RemoteTaskStatus::Expired
                | RemoteTaskStatus::Cancelled
        ) | (
            RemoteTaskStatus::Running,
            RemoteTaskStatus::Completed | RemoteTaskStatus::Failed | RemoteTaskStatus::Cancelled
        )
    )
}
pub fn default_path() -> PathBuf {
    std::env::var_os("JARVIS_AGENT_TASKS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".jarvis/agent-tasks.json"))
        })
        .unwrap_or_else(|| PathBuf::from(".jarvis/agent-tasks.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "jarvis-tasks-{}-{}.json",
            std::process::id(),
            crate::agent_protocol::new_id("test")
        ))
    }
    fn task() -> RemoteTask {
        RemoteTask {
            id: "t".into(),
            message_id: "m".into(),
            peer: "p".into(),
            capability: "read_file".into(),
            input: serde_json::json!({}),
            status: RemoteTaskStatus::Queued,
            expires_at: 9999999999,
            requires_approval: false,
        }
    }
    #[test]
    fn deduplicates_and_transitions() {
        let p = path();
        let _ = fs::remove_file(&p);
        let s = TaskStore::new(p.clone());
        s.create(task()).unwrap();
        assert!(s.accept_once("m").unwrap());
        assert!(!s.accept_once("m").unwrap());
        s.transition("t", RemoteTaskStatus::Running).unwrap();
        assert_eq!(
            s.transition("t", RemoteTaskStatus::Completed)
                .unwrap()
                .status,
            RemoteTaskStatus::Completed
        );
        assert!(s.transition("t", RemoteTaskStatus::Running).is_err());
        let _ = fs::remove_file(p);
    }
    #[test]
    fn approval_gate_is_explicit() {
        let p = path();
        let _ = fs::remove_file(&p);
        let s = TaskStore::new(p.clone());
        let mut t = task();
        t.requires_approval = true;
        t.status = RemoteTaskStatus::AwaitingApproval;
        s.create(t).unwrap();
        assert!(s.transition("t", RemoteTaskStatus::Completed).is_err());
        assert!(s.transition("t", RemoteTaskStatus::Running).is_ok());
        let _ = fs::remove_file(p);
    }
}
