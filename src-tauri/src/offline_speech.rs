//! Managed JSONL bridge for the offline speech-only Python worker.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::{timeout, Duration},
};

pub const MAX_TEXT: usize = 1_800;
pub const MAX_WAV: usize = 8 * 1024 * 1024;
const OP_TIMEOUT: Duration = Duration::from_secs(90);

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}
pub struct OfflineSpeech {
    worker: Mutex<Option<Worker>>,
}

impl OfflineSpeech {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            worker: Mutex::new(None),
        })
    }

    async fn start(&self, app: &AppHandle) -> Result<(), String> {
        let resource = app
            .path()
            .resource_dir()
            .map_err(|e| format!("无法定位语音资源：{e}"))?;
        let script = resource.join("controller.py");
        if !script.is_file() {
            return Err(format!("语音 worker 不存在：{}", script.display()));
        }
        let python = std::env::var_os("JARVIS_PYTHON").unwrap_or_else(|| "python3".into());
        let mut child = Command::new(python)
            .args(["-u"])
            .arg(script)
            .arg("--speech-worker")
            .env("JARVIS_MODEL_ROOT", resource.join("../../../models"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("无法启动语音 worker：{e}"))?;
        let stdin = child.stdin.take().ok_or("语音 worker stdin 不可用")?;
        let stdout = child.stdout.take().ok_or("语音 worker stdout 不可用")?;
        *self.worker.lock().await = Some(Worker {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        });
        Ok(())
    }

    async fn kill_locked(worker: &mut Option<Worker>) {
        if let Some(mut value) = worker.take() {
            let _ = value.child.kill().await;
            let _ = value.child.wait().await;
        }
    }

    pub async fn request(
        &self,
        app: &AppHandle,
        op: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value, String> {
        let mut guard = self.worker.lock().await;
        if guard.is_none() {
            drop(guard);
            self.start(app).await?;
            guard = self.worker.lock().await;
        }
        let value = guard.as_mut().ok_or("语音 worker 未启动")?;
        let id = value.next_id;
        value.next_id += 1;
        let mut request = params.as_object().cloned().unwrap_or_default();
        request.insert("id".into(), json!(id));
        request.insert("op".into(), json!(op));
        let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        line.push(b'\n');
        if line.len() > 12 * 1024 * 1024 {
            return Err("语音请求过大".into());
        }
        value
            .stdin
            .write_all(&line)
            .await
            .map_err(|e| e.to_string())?;
        value.stdin.flush().await.map_err(|e| e.to_string())?;
        let mut response = String::new();
        let read = timeout(deadline, value.stdout.read_line(&mut response)).await;
        let read = match read {
            Ok(result) => result,
            Err(_) => {
                Self::kill_locked(&mut guard).await;
                return Err("语音 worker 响应超时".into());
            }
        };
        if read.map_err(|e| e.to_string())? == 0 {
            Self::kill_locked(&mut guard).await;
            return Err("语音 worker 已退出".into());
        }
        let response: Value = serde_json::from_str(&response).map_err(|e| {
            let _ = e;
            "语音 worker 返回格式无效".to_owned()
        })?;
        if response.get("id").and_then(Value::as_u64) != Some(id) {
            return Err("语音 worker 响应 id 不匹配".into());
        }
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("语音 worker 操作失败")
                .to_owned());
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn synthesize(&self, app: &AppHandle, text: &str) -> Result<Vec<u8>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        if text.chars().count() > MAX_TEXT {
            return Err("语音文本过长".into());
        }
        let result = self
            .request(app, "synthesize", json!({"text": text}), OP_TIMEOUT)
            .await?;
        let encoded = result
            .get("wav")
            .and_then(Value::as_str)
            .ok_or("语音 worker 未返回 WAV")?;
        let wav = STANDARD
            .decode(encoded)
            .map_err(|_| "语音 worker WAV 编码无效")?;
        if wav.len() > MAX_WAV || wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
            return Err("语音 worker WAV 无效".into());
        }
        Ok(wav)
    }

    pub async fn cancel(&self) {
        Self::kill_locked(&mut *self.worker.lock().await).await;
    }
}
impl Drop for OfflineSpeech {
    fn drop(&mut self) {}
}
