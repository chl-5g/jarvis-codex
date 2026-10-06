//! Managed CipherPipe JSONL adapter. CipherPipe remains the encrypted transport;
//! Jarvis keeps ownership of model routing, tools, and permissions.
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, Mutex},
    time::{timeout, Duration},
};

const MAX_TEXT: usize = 4_000;
const TIMEOUT: Duration = Duration::from_secs(10);
struct Worker {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<Value>,
    next_id: u64,
}
pub struct CipherPipe {
    worker: Mutex<Option<Worker>>,
}
impl CipherPipe {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            worker: Mutex::new(None),
        })
    }
    async fn start(&self, app: &AppHandle, peer: Option<&str>) -> Result<(), String> {
        let root = std::env::var_os("JARVIS_CIPHERPIPE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("cipherpipe")
            });
        let script = app
            .path()
            .resource_dir()
            .map_err(|e| e.to_string())?
            .join("cipherpipe_bridge.py");
        if !script.is_file() {
            return Err(format!("CipherPipe bridge 不存在：{}", script.display()));
        }
        let proxy =
            std::env::var("JARVIS_CIPHERPIPE_PROXY").unwrap_or_else(|_| "127.0.0.1:8700".into());
        let keyfile = std::env::var_os("JARVIS_CIPHERPIPE_KEYFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("data/nostr.key"));
        let mut cmd = Command::new(crate::offline_speech::python_path());
        cmd.env("JARVIS_CIPHERPIPE_ROOT", &root);
        cmd.args(["-u"])
            .arg(script)
            .args(["--proxy", &proxy, "--keyfile"])
            .arg(keyfile);
        if let Some(peer) = peer {
            cmd.args(["--peer", peer]);
        }
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("无法启动 CipherPipe bridge：{e}"))?;
        let stdin = child.stdin.take().ok_or("CipherPipe stdin 不可用")?;
        let stdout = child.stdout.take().ok_or("CipherPipe stdout 不可用")?;
        let (tx, rx) = mpsc::channel(32);
        let app_events = app.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("event").and_then(Value::as_str) == Some("message") {
                    crate::events::emit(
                        &app_events,
                        "cipherpipe",
                        json!({"kind":"cipherpipe-message","from":value["from"],"text":value["text"],"id":value["id"]}),
                    );
                } else if tx.send(value).await.is_err() {
                    break;
                }
            }
        });
        *self.worker.lock().await = Some(Worker {
            child,
            stdin,
            responses: rx,
            next_id: 1,
        });
        Ok(())
    }
    pub async fn send(
        &self,
        app: &AppHandle,
        text: &str,
        peer: Option<&str>,
    ) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("CipherPipe 消息不能为空".into());
        }
        if text.chars().count() > MAX_TEXT {
            return Err("CipherPipe 消息过长".into());
        }
        let mut guard = self.worker.lock().await;
        if guard.is_none() {
            drop(guard);
            self.start(app, peer).await?;
            guard = self.worker.lock().await;
        }
        let worker = guard.as_mut().ok_or("CipherPipe 未启动")?;
        let id = worker.next_id;
        worker.next_id += 1;
        let mut line =
            serde_json::to_vec(&json!({"id": id, "op":"send", "text": text, "to": peer}))
                .map_err(|e| e.to_string())?;
        line.push(b'\n');
        worker
            .stdin
            .write_all(&line)
            .await
            .map_err(|e| e.to_string())?;
        worker.stdin.flush().await.map_err(|e| e.to_string())?;
        loop {
            let value = timeout(TIMEOUT, worker.responses.recv())
                .await
                .map_err(|_| "CipherPipe 响应超时".to_owned())?
                .ok_or("CipherPipe 已断开")?;
            if value.get("event").and_then(Value::as_str) == Some("ready") {
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id)
                || value.get("ok") != Some(&Value::Bool(true))
            {
                return Err(value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("CipherPipe 发送失败")
                    .into());
            }
            break;
        }
        Ok(())
    }
    pub async fn stop(&self) {
        if let Some(mut worker) = self.worker.lock().await.take() {
            let _ = worker.child.kill().await;
            let _ = worker.child.wait().await;
        }
    }
}
