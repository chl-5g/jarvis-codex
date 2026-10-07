use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

const MAX_AGENTS: usize = 128;
const MAX_CAPABILITIES: usize = 64;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecord {
    pub public_key: String,
    pub name: String,
    pub device: String,
    pub trusted: bool,
    pub capabilities: Vec<String>,
    pub last_seen: Option<u64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    agents: Vec<AgentRecord>,
}
#[derive(Clone)]
pub struct AgentRegistry {
    path: PathBuf,
}
impl AgentRegistry {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn list(&self) -> Vec<AgentRecord> {
        self.load().agents
    }
    pub fn upsert(&self, record: AgentRecord) -> Result<AgentRecord, String> {
        validate(&record)?;
        let mut file = self.load();
        if let Some(old) = file
            .agents
            .iter_mut()
            .find(|a| a.public_key == record.public_key)
        {
            *old = record.clone();
        } else {
            if file.agents.len() >= MAX_AGENTS {
                return Err("agent registry is full".into());
            }
            file.agents.push(record.clone());
        }
        self.persist(&file)?;
        Ok(record)
    }
    pub fn remove(&self, key: &str) -> Result<(), String> {
        let mut file = self.load();
        file.agents.retain(|a| a.public_key != key);
        self.persist(&file)
    }
    pub fn authorize(&self, peer: &str, capability: &str) -> bool {
        self.list().into_iter().any(|a| {
            a.public_key == peer && a.trusted && a.capabilities.iter().any(|c| c == capability)
        })
    }
    fn load(&self) -> RegistryFile {
        fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn persist(&self, file: &RegistryFile) -> Result<(), String> {
        if let Some(p) = self.path.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(tmp, &self.path).map_err(|e| e.to_string())
    }
}
fn validate(a: &AgentRecord) -> Result<(), String> {
    if a.public_key.trim().is_empty() || a.name.trim().is_empty() {
        return Err("agent identity is required".into());
    }
    if a.capabilities.len() > MAX_CAPABILITIES
        || a.capabilities
            .iter()
            .any(|c| c.trim().is_empty() || c.len() > 128 || c == "run_command")
    {
        return Err("invalid or unsafe agent capability".into());
    }
    Ok(())
}
pub fn default_path() -> PathBuf {
    std::env::var_os("JARVIS_AGENT_REGISTRY")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".jarvis/agents.json")))
        .unwrap_or_else(|| PathBuf::from(".jarvis/agents.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!("jarvis-registry-{}.json", std::process::id()))
    }
    #[test]
    fn persists_and_authorizes() {
        let p = path();
        let _ = fs::remove_file(&p);
        let r = AgentRegistry::new(p.clone());
        r.upsert(AgentRecord {
            public_key: "peer".into(),
            name: "Peer".into(),
            device: "Mac".into(),
            trusted: true,
            capabilities: vec!["read_file".into()],
            last_seen: None,
        })
        .unwrap();
        assert!(r.authorize("peer", "read_file"));
        assert!(!r.authorize("peer", "run_command"));
        assert_eq!(r.list().len(), 1);
        let _ = fs::remove_file(p);
    }
    #[test]
    fn rejects_unsafe_capability() {
        let r = AgentRegistry::new(path());
        let x = AgentRecord {
            public_key: "p".into(),
            name: "P".into(),
            device: "d".into(),
            trusted: true,
            capabilities: vec!["run_command".into()],
            last_seen: None,
        };
        assert!(r.upsert(x).is_err());
    }
}
