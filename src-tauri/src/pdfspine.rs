//! Managed JSONL connector for the optional pdfspine OCR worker.
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::{timeout, Duration},
};

const MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 200 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

pub struct PdfSpine {
    worker: Mutex<Option<Worker>>,
}

impl PdfSpine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            worker: Mutex::new(None),
        })
    }

    async fn start(&self, app: &AppHandle) -> Result<(), String> {
        let mut candidates = Vec::new();
        if let Some(configured) = std::env::var_os("JARVIS_PDFSPINE_BIN") {
            candidates.push(PathBuf::from(configured));
        }
        if let Ok(resource_dir) = app.path().resource_dir() {
            candidates.push(resource_dir.join("pdf-ocr-worker"));
            candidates.push(resource_dir.join("bin/pdf-ocr-worker"));
        }
        let project_root = PathBuf::from(crate::config::project_root());
        candidates.push(project_root.join("../pdfspine/target/release/pdf-ocr-worker"));
        candidates.push(project_root.join("../pdfspine/target/debug/pdf-ocr-worker"));
        let binary = candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                "pdfspine OCR worker 不可用：请设置 JARVIS_PDFSPINE_BIN 或构建 pdf-ocr-worker"
                    .to_owned()
            })?;
        let mut child = Command::new(binary)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("无法启动 pdfspine OCR worker：{e}"))?;
        let stdin = child.stdin.take().ok_or("pdfspine worker stdin 不可用")?;
        let stdout = child.stdout.take().ok_or("pdfspine worker stdout 不可用")?;
        *self.worker.lock().await = Some(Worker {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        });
        Ok(())
    }

    async fn kill_locked(worker: &mut Option<Worker>) {
        if let Some(mut worker) = worker.take() {
            let _ = worker.child.kill().await;
            let _ = worker.child.wait().await;
        }
    }

    pub async fn request(&self, app: &AppHandle, op: &str, params: Value) -> Result<Value, String> {
        let mut guard = self.worker.lock().await;
        if guard.is_none() {
            drop(guard);
            self.start(app).await?;
            guard = self.worker.lock().await;
        }
        let worker = guard.as_mut().ok_or("pdfspine worker 未启动")?;
        let id = worker.next_id;
        worker.next_id += 1;
        let request = json!({"id": id, "op": op, "params": params});
        let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        line.push(b'\n');
        if line.len() > 1_048_576 {
            return Err("pdfspine 请求过大".into());
        }
        worker
            .stdin
            .write_all(&line)
            .await
            .map_err(|e| e.to_string())?;
        worker.stdin.flush().await.map_err(|e| e.to_string())?;
        let mut response = String::new();
        let read = match timeout(REQUEST_TIMEOUT, worker.stdout.read_line(&mut response)).await {
            Ok(result) => result,
            Err(_) => {
                Self::kill_locked(&mut guard).await;
                return Err("pdfspine OCR 超时".into());
            }
        };
        if read.map_err(|e| e.to_string())? == 0 {
            Self::kill_locked(&mut guard).await;
            return Err("pdfspine worker 已退出".into());
        }
        let response: Value =
            serde_json::from_str(&response).map_err(|_| "pdfspine worker 返回格式无效")?;
        if response.get("id").and_then(Value::as_u64) != Some(id) {
            return Err("pdfspine 响应 id 不匹配".into());
        }
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("pdfspine OCR 失败")
                .to_owned());
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn shutdown(&self) {
        Self::kill_locked(&mut *self.worker.lock().await).await;
    }
}

fn permitted_path(workspace: &Path, raw: &str, output: bool) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(raw);
    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        workspace.join(candidate)
    };
    let base = workspace
        .canonicalize()
        .map_err(|e| format!("workspace 不可用：{e}"))?;
    let checked = if output {
        absolute
            .parent()
            .ok_or("输出路径无效")?
            .canonicalize()
            .map_err(|e| format!("输出目录不可用：{e}"))?
            .join(absolute.file_name().ok_or("输出路径无效")?)
    } else {
        absolute
            .canonicalize()
            .map_err(|e| format!("输入文件不可用：{e}"))?
    };
    if !checked.starts_with(&base) {
        return Err("文件路径必须位于当前 workspace 内".into());
    }
    if !output {
        let size = std::fs::metadata(&checked)
            .map_err(|e| e.to_string())?
            .len();
        if size > MAX_INPUT_BYTES {
            return Err("输入文件超过 100 MiB 限制".into());
        }
    }
    Ok(checked)
}

pub async fn request(
    app: &AppHandle,
    op: &str,
    workspace: &Path,
    mut params: Value,
) -> Result<Value, String> {
    let input = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or("OCR 缺少 path")?;
    let input = permitted_path(workspace, input, false)?;
    params["path"] = json!(input);
    if let Some(output) = params
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        let output = permitted_path(workspace, &output, true)?;
        if output == input {
            return Err("输出文件必须不同于输入文件".into());
        }
        params["output_path"] = json!(output);
        if output.exists()
            && std::fs::metadata(&output)
                .map(|m| m.len())
                .unwrap_or(MAX_OUTPUT_BYTES + 1)
                > MAX_OUTPUT_BYTES
        {
            return Err("输出文件超过限制".into());
        }
    }
    let connector = app.state::<crate::AppState>().pdfspine.clone();
    let result = connector.request(app, op, params).await;
    crate::events::emit(
        app,
        "connector",
        json!({"connector":"pdfspine","operation":op,"success":result.is_ok()}),
    );
    result
}
