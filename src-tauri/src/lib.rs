use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{oneshot, Mutex, RwLock},
    time::{timeout, Duration},
};

mod bridge;
mod cipherpipe;
mod config;
mod events;
mod knowledge;
mod logging;
mod memory;
mod offline_speech;
mod on_device_model;
mod pdfspine;
mod skills;
mod tasks;
mod tools;
mod workflow;

const JARVIS_MODEL: &str = "gpt-5.6-sol";

fn memory_store() -> memory::MemoryStore {
    memory::MemoryStore::default()
}

#[tauri::command]
fn skills_list() -> Vec<skills::SkillMetadata> {
    skills::SkillsRegistry::default().list()
}

#[tauri::command]
fn skills_match(query: String, limit: Option<usize>) -> Vec<skills::SkillMatch> {
    skills::SkillsRegistry::default().route(&query, limit.unwrap_or(4))
}

#[tauri::command]
fn skills_context(query: String, max_chars: Option<usize>) -> String {
    skills::SkillsRegistry::default().context(&query, max_chars.unwrap_or(4_000))
}

#[tauri::command]
fn connector_list() -> Value {
    config::connectors()
}

struct AppState {
    runtime: Mutex<Option<Arc<CodexRuntime>>>,
    speech: Mutex<Option<Child>>,
    offline_speech: Arc<offline_speech::OfflineSpeech>,
    pdfspine: Arc<pdfspine::PdfSpine>,
    cipherpipe: Arc<cipherpipe::CipherPipe>,
    speaker_access: RwLock<SpeakerAccess>,
    cold_wake_pending: AtomicBool,
    background_start: bool,
    wake_enabled: AtomicBool,
    wake_ready: AtomicBool,
    wake_supervisor_running: AtomicBool,
    wake_pid: AtomicU32,
    wake_authorization: RwLock<String>,
}

#[tauri::command]
fn startup_is_background(state: State<'_, AppState>) -> bool {
    state.background_start
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn request_microphone_permission() -> Result<String, String> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    use std::sync::Mutex as StdMutex;

    let media_type =
        unsafe { AVMediaTypeAudio }.ok_or_else(|| "macOS 未提供音频授权类型".to_owned())?;
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
    match status {
        AVAuthorizationStatus::Authorized => return Ok("authorized".to_owned()),
        AVAuthorizationStatus::Denied => return Ok("denied".to_owned()),
        AVAuthorizationStatus::Restricted => return Ok("restricted".to_owned()),
        _ => {}
    }

    let (sender, receiver) = oneshot::channel::<bool>();
    let sender = Arc::new(StdMutex::new(Some(sender)));
    {
        let completion_sender = sender.clone();
        let completion = RcBlock::new(move |granted: Bool| {
            if let Ok(mut guard) = completion_sender.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(granted.as_bool());
                }
            }
        });
        unsafe {
            AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &completion);
        }
    }
    let granted = receiver
        .await
        .map_err(|_| "macOS 麦克风授权回调中断".to_owned())?;
    Ok(if granted { "authorized" } else { "denied" }.to_owned())
}

#[tauri::command]
async fn verify_speaker(app: AppHandle, audio: String) -> Result<Value, String> {
    let bytes = STANDARD
        .decode(audio)
        .map_err(|_| "声纹音频编码无效".to_owned())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("声纹音频过大".to_owned());
    }
    let resource = app.path().resource_dir().map_err(|e| e.to_string())?;
    let script = if resource.join("speaker_identity.py").is_file() {
        resource.join("speaker_identity.py")
    } else {
        PathBuf::from(config::project_root()).join("src-tauri/speaker_identity.py")
    };
    let file = std::env::temp_dir().join(format!("jarvis-speaker-{}.wav", std::process::id()));
    fs::write(&file, bytes).map_err(|e| format!("写入声纹样本失败：{e}"))?;
    let python = offline_speech::speaker_python_path();
    logging::text(
        "jarvis-runtime",
        &format!(
            "speaker verification started: {}",
            PathBuf::from(&python).display()
        ),
    );
    let output = Command::new(python).arg(&script).arg("--verify").arg(&file)
        .env("JARVIS_SPEAKER_MODEL", std::env::var("JARVIS_SPEAKER_MODEL").unwrap_or_else(|_| "/Users/caihaolun/models/speaker/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx".to_owned()))
        .env("JARVIS_SPEAKER_PROFILE", std::env::var("JARVIS_SPEAKER_PROFILE").unwrap_or_else(|_| "/Users/caihaolun/.config/jarvis/speakers/allen.json".to_owned()))
        .env("JARVIS_SPEAKER_THRESHOLD", std::env::var("JARVIS_SPEAKER_THRESHOLD").unwrap_or_else(|_| "0.85".to_owned()))
        .output().await.map_err(|e| format!("启动声纹验证失败：{e}"))?;
    let _ = fs::remove_file(&file);
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        logging::text(
            "jarvis-runtime",
            &format!("speaker verification failed: {error}"),
        );
        return Err(error);
    }
    let result: Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("声纹结果无效：{e}"))?;
    logging::text(
        "jarvis-runtime",
        &format!("speaker verification result: {}", result),
    );
    Ok(result)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn request_microphone_permission() -> Result<String, String> {
    Ok("authorized".to_owned())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionStatus {
    microphone: String,
    speech_recognition: String,
    location: String,
}

/// Read capability state without requesting any new macOS authorization.
/// Actual prompts are opened only by the capability that needs them.
#[cfg(target_os = "macos")]
#[tauri::command]
fn permission_status() -> PermissionStatus {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    let microphone = unsafe { AVMediaTypeAudio }
        .map(|media_type| unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) })
        .map(|status| match status {
            AVAuthorizationStatus::Authorized => "authorized",
            AVAuthorizationStatus::Denied => "denied",
            AVAuthorizationStatus::Restricted => "restricted",
            AVAuthorizationStatus::NotDetermined => "notDetermined",
            _ => "unknown",
        })
        .unwrap_or("unknown")
        .to_owned();
    PermissionStatus {
        microphone,
        speech_recognition: "not_checked".to_owned(),
        location: "not_checked".to_owned(),
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn permission_status() -> PermissionStatus {
    PermissionStatus {
        microphone: "authorized".to_owned(),
        speech_recognition: "not_checked".to_owned(),
        location: "not_checked".to_owned(),
    }
}

struct CodexRuntime {
    writer: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next_id: AtomicU64,
    thread_id: RwLock<Option<String>>,
    active_turn: RwLock<Option<String>>,
    voice_active: AtomicBool,
    voice_phase: RwLock<String>,
    realtime_session_id: RwLock<Option<String>>,
    permission_mode: PermissionMode,
    speaker_access: SpeakerAccess,
    workspace: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum PermissionMode {
    Safe,
    Auto,
    Full,
}

/// Local speaker verification is intentionally a separate gate from the
/// Codex permission mode.  `Unknown` may receive ordinary answers, but the
/// model must not use Computer Use or other desktop-control tools.  A
/// verifier can promote the session to `Allen`; a failed verification maps to
/// `Rejected` and is handled before any task is sent to Codex.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum SpeakerAccess {
    #[default]
    Unknown,
    Allen,
    Rejected,
}

/// Speaker verification is enabled whenever an Allen profile is installed.
/// The environment variable can explicitly disable or enable the gate for
/// development and recovery.
fn speaker_gate_enabled() -> bool {
    match std::env::var("JARVIS_SPEAKER_GATE").as_deref() {
        Ok("0") | Ok("false") | Ok("no") => false,
        Ok("1") | Ok("true") | Ok("yes") => true,
        _ => PathBuf::from("/Users/caihaolun/.config/jarvis/speakers/allen.json").is_file(),
    }
}

fn effective_speaker_access(requested: SpeakerAccess) -> SpeakerAccess {
    if speaker_gate_enabled() {
        requested
    } else {
        SpeakerAccess::Allen
    }
}

impl SpeakerAccess {
    fn instructions(self) -> &'static str {
        match self {
            Self::Allen => "The local speaker verifier identified Allen. Allen's private profile, Computer Use, and desktop-control tools are available under the selected permission mode.",
            Self::Unknown => "The local speaker verifier did not identify the speaker. Answer ordinary questions normally, but do not use Computer Use, desktop-control, screen-control, or other interactive UI tools. Never reveal, confirm, guess, infer, or accept a claimed identity for Allen or Cai Haolun from memory, prior turns, profile data, conversation context, or the user's words. If asked who the user is, say that the speaker is not verified and address them neutrally. Explain that speaker verification is required before computer control.",
            Self::Rejected => "The local speaker verifier rejected the speaker. Do not execute or send the requested task; respond with exactly: 未识别的说话人",
        }
    }
}

struct PermissionProfile {
    approval_policy: &'static str,
    sandbox: &'static str,
    instructions: &'static str,
}

impl PermissionMode {
    fn profile(self) -> PermissionProfile {
        match self {
            Self::Safe => PermissionProfile {
                approval_policy: "on-request",
                sandbox: "workspace-write",
                instructions: "Require explicit confirmation when Codex requests approval for actions outside the workspace boundary or for risky operations.",
            },
            Self::Auto => PermissionProfile {
                approval_policy: "never",
                sandbox: "workspace-write",
                instructions: "Work autonomously inside the selected workspace. Never request elevated access; if an action is blocked by the sandbox, explain the blocked boundary and continue with the safest in-workspace alternative.",
            },
            Self::Full => PermissionProfile {
                approval_policy: "never",
                sandbox: "danger-full-access",
                instructions: "Full filesystem and network access is enabled. Still avoid destructive or irreversible actions unless the user explicitly requested the exact action and target.",
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionInfo {
    thread_id: String,
    cwd: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectVoiceInfo {
    codex_connected: bool,
    voice_active: bool,
    phase: String,
    protocol: &'static str,
    thread_id: Option<String>,
    realtime_session_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WakeStatus {
    enabled: bool,
    ready: bool,
    authorization: String,
}

fn raise_jarvis_window(app: &AppHandle) {
    let app_handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        {
            use objc2::MainThreadMarker;
            use objc2_app_kit::NSApplication;

            if let Some(mtm) = MainThreadMarker::new() {
                let application = NSApplication::sharedApplication(mtm);
                #[allow(deprecated)]
                application.activateIgnoringOtherApps(true);
            }
        }

        if let Some(window) = app_handle.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            // A short floating interval lets macOS finish switching the
            // active application before the window returns to normal level.
            let _ = window.set_always_on_top(true);
            let _ = window.set_focus();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(700)).await;
                let _ = window.set_always_on_top(false);
            });
        }
    });
}

impl CodexRuntime {
    async fn spawn(
        app: AppHandle,
        permission_mode: PermissionMode,
        speaker_access: SpeakerAccess,
        workspace: String,
    ) -> Result<Arc<Self>, String> {
        let speaker_access = effective_speaker_access(speaker_access);
        let codex_binary = codex_binary_path(&app)?;
        let mut command = Command::new(&codex_binary);
        command
            // Realtime is an experimental app-server surface. Enable it only
            // for this isolated Jarvis child; never mutate ~/.codex/config.toml.
            .args(["app-server", "--enable", "realtime_conversation", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if speaker_access != SpeakerAccess::Allen {
            // Unknown speakers can still ask questions, but the process does
            // not expose either local desktop-control MCP entry point.
            command.args([
                "-c",
                "mcp_servers.node_repl.enabled=false",
                "-c",
                "mcp_servers.cua_repl.enabled=false",
            ]);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("无法启动 codex app-server：{error}"))?;
        let writer = child.stdin.take().ok_or("无法连接 Codex stdin")?;
        let stdout = child.stdout.take().ok_or("无法连接 Codex stdout")?;
        let stderr = child.stderr.take().ok_or("无法连接 Codex stderr")?;
        let runtime = Arc::new(Self {
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            thread_id: RwLock::new(None),
            active_turn: RwLock::new(None),
            voice_active: AtomicBool::new(false),
            voice_phase: RwLock::new("standby".to_owned()),
            realtime_session_id: RwLock::new(None),
            permission_mode,
            speaker_access,
            workspace,
        });

        let weak = Arc::downgrade(&runtime);
        let event_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                eprintln!(
                    "codex rpc: id={} method={}",
                    message
                        .get("id")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "-".to_owned()),
                    message.get("method").and_then(Value::as_str).unwrap_or("-")
                );
                if matches!(
                    message.get("method").and_then(Value::as_str),
                    Some("error") | Some("thread/realtime/error")
                ) {
                    eprintln!(
                        "codex diagnostic: {}",
                        message
                            .pointer("/params/message")
                            .and_then(Value::as_str)
                            .or_else(|| message.pointer("/error/message").and_then(Value::as_str))
                            .unwrap_or("unknown")
                    );
                }
                if message.get("method").is_none() {
                    if let Some(id) = message.get("id").and_then(Value::as_u64) {
                        if let Some(runtime) = weak.upgrade() {
                            if let Some(sender) = runtime.pending.lock().await.remove(&id) {
                                let result = if let Some(error) = message.get("error") {
                                    Err(error
                                        .get("message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Codex request failed")
                                        .to_owned())
                                } else {
                                    Ok(message.get("result").cloned().unwrap_or(Value::Null))
                                };
                                let _ = sender.send(result);
                            }
                        }
                    }
                    continue;
                }
                if message.get("method").and_then(Value::as_str) == Some("item/tool/call") {
                    if let Some(runtime) = weak.upgrade() {
                        let app = event_app.clone();
                        let request = message.clone();
                        tauri::async_runtime::spawn(async move {
                            let params = &request["params"];
                            let result = tools::execute(
                                app,
                                &PathBuf::from(&runtime.workspace),
                                params["tool"].as_str().unwrap_or(""),
                                params["arguments"].clone(),
                                runtime.permission_mode == PermissionMode::Full,
                            )
                            .await;
                            let content = if result.success {
                                result.output.clone()
                            } else {
                                result.error.clone().unwrap_or_default()
                            };
                            let mut content_items = vec![json!({
                                "type": "inputText",
                                "text": content
                            })];
                            if result.success && result.tool_name == "capture_camera" {
                                if let Ok(photo) = serde_json::from_str::<Value>(&result.output) {
                                    if let Some(path) = photo.get("path").and_then(Value::as_str) {
                                        if let Ok(bytes) = fs::read(path) {
                                            content_items.push(json!({
                                                "type": "inputImage",
                                                "imageUrl": format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))
                                            }));
                                        }
                                    }
                                }
                            }
                            let _ = runtime.write(&json!({
                                "id": request["id"],
                                "result": {"success": result.success, "contentItems": content_items}
                            })).await;
                        });
                    }
                    continue;
                }
                if let Some(runtime) = weak.upgrade() {
                    match message.get("method").and_then(Value::as_str) {
                        Some("turn/started") => {
                            *runtime.active_turn.write().await = message
                                .pointer("/params/turn/id")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                        }
                        Some("turn/completed") => *runtime.active_turn.write().await = None,
                        Some("thread/realtime/started") => {
                            runtime.voice_active.store(true, Ordering::SeqCst);
                            *runtime.voice_phase.write().await = "connected".to_owned();
                            *runtime.realtime_session_id.write().await = message
                                .pointer("/params/realtimeSessionId")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                        }
                        Some("thread/realtime/error") => {
                            runtime.voice_active.store(false, Ordering::SeqCst);
                            *runtime.voice_phase.write().await = "error".to_owned();
                        }
                        Some("thread/realtime/closed") => {
                            runtime.voice_active.store(false, Ordering::SeqCst);
                            *runtime.voice_phase.write().await = "closed".to_owned();
                            *runtime.realtime_session_id.write().await = None;
                        }
                        _ => {}
                    }
                }
                let _ = event_app.emit("codex-event", message.clone());
                if message.get("method").and_then(Value::as_str) == Some("item/completed") {
                    if let Some(text) = message.pointer("/params/item/text").and_then(Value::as_str)
                    {
                        crate::logging::conversation("assistant", text, "codex");
                    }
                }
                crate::events::emit(&event_app, "codex", message);
            }
        });
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("codex stderr: {line}");
                if line.contains("ERROR") {
                    let _ = app.emit("codex-diagnostic", line);
                }
            }
        });
        Ok(runtime)
    }

    async fn write(&self, message: &Value) -> Result<(), String> {
        let mut payload = serde_json::to_vec(message).map_err(|error| error.to_string())?;
        payload.push(b'\n');
        let mut writer = self.writer.lock().await;
        writer
            .write_all(&payload)
            .await
            .map_err(|error| error.to_string())?;
        writer.flush().await.map_err(|error| error.to_string())
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        if let Err(error) = self
            .write(&json!({"id": id, "method": method, "params": params}))
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        timeout(Duration::from_secs(90), receiver)
            .await
            .map_err(|_| format!("{method} 响应超时"))?
            .map_err(|_| format!("{method} 响应通道关闭"))?
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"method": method, "params": params}))
            .await
    }

    async fn thread(&self) -> Result<String, String> {
        self.thread_id
            .read()
            .await
            .clone()
            .ok_or("Jarvis 尚未连接 Codex 线程".to_owned())
    }
}

fn codex_binary_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(configured) = std::env::var("JARVIS_CODEX_BIN") {
        let path = PathBuf::from(configured);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join("codex");
        if bundled.is_file() {
            return Ok(bundled);
        }
    }
    let mut candidates = vec![
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
        PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join(".local/bin/codex"));
        candidates.push(PathBuf::from(home).join(".cargo/bin/codex"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "未找到 Codex 可执行文件；请安装 Codex，或设置 JARVIS_CODEX_BIN。".to_owned()
        })
}

async fn runtime(state: &State<'_, AppState>) -> Result<Arc<CodexRuntime>, String> {
    state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or("Jarvis runtime 尚未启动".to_owned())
}

async fn direct_voice_info(state: &State<'_, AppState>) -> DirectVoiceInfo {
    let runtime = state.runtime.lock().await.clone();
    let Some(runtime) = runtime else {
        return DirectVoiceInfo {
            codex_connected: false,
            voice_active: false,
            phase: "standby".to_owned(),
            protocol: "Codex app-server V3 · WebRTC",
            thread_id: None,
            realtime_session_id: None,
        };
    };
    let phase = runtime.voice_phase.read().await.clone();
    let thread_id = runtime.thread_id.read().await.clone();
    let realtime_session_id = runtime.realtime_session_id.read().await.clone();
    DirectVoiceInfo {
        codex_connected: true,
        voice_active: runtime.voice_active.load(Ordering::SeqCst),
        phase,
        protocol: "Codex app-server V3 · WebRTC",
        thread_id,
        realtime_session_id,
    }
}

#[tauri::command]
async fn direct_voice_status(state: State<'_, AppState>) -> Result<DirectVoiceInfo, String> {
    Ok(direct_voice_info(&state).await)
}

fn wake_helper_path(app: &AppHandle) -> Result<PathBuf, String> {
    let relative = PathBuf::from("wake-helper/JarvisWakeListener.app");
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join(&relative);
        if bundled.exists() {
            return Ok(bundled);
        }
    }
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    if development.exists() {
        return Ok(development);
    }
    Err("Jarvis 唤醒监听器未找到".to_owned())
}

async fn wake_speech_permission_status(app: &AppHandle) -> Result<String, String> {
    let helper = wake_helper_path(app)?;
    let event_file =
        std::env::temp_dir().join(format!("jarvis-speech-status-{}.jsonl", std::process::id()));
    let _ = fs::write(&event_file, "");
    let mut child = Command::new("/usr/bin/open")
        .args(["-n", "-W"])
        .arg(&helper)
        .args(["--args", "--status-only", "--event-file"])
        .arg(&event_file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("启动语音权限检查失败：{error}"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let content = fs::read_to_string(&event_file).unwrap_or_default();
        for line in content.lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if value.get("type").and_then(Value::as_str) == Some("authorization") {
                let status = value
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = fs::remove_file(&event_file);
                return Ok(status);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = fs::remove_file(&event_file);
            return Err("语音识别权限检查超时".to_owned());
        }
        if child.try_wait().ok().flatten().is_some() {
            let _ = fs::remove_file(&event_file);
            return Err("语音识别权限检查进程提前退出".to_owned());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tauri::command]
async fn speech_permission_status(app: AppHandle) -> Result<String, String> {
    wake_speech_permission_status(&app).await
}

fn location_helper_path(app: &AppHandle) -> Result<PathBuf, String> {
    let relative = PathBuf::from("location-helper/JarvisLocationHelper.app");
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join(&relative);
        if bundled.exists() {
            return Ok(bundled);
        }
    }
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    if development.exists() {
        return Ok(development);
    }
    Err("Jarvis 位置能力组件未找到".to_owned())
}

fn camera_helper_path(app: &AppHandle) -> Result<PathBuf, String> {
    let relative = PathBuf::from("camera-helper/JarvisCameraHelper.app");
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join(&relative);
        if bundled.exists() {
            return Ok(bundled);
        }
    }
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    if development.exists() {
        return Ok(development);
    }
    Err("Jarvis 摄像头能力组件未找到".to_owned())
}

pub(crate) async fn request_current_location(app: AppHandle) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        return Err("当前系统没有 Jarvis 原生位置能力".to_owned());
    }
    #[cfg(target_os = "macos")]
    {
        let helper = location_helper_path(&app)?;
        let event_file =
            std::env::temp_dir().join(format!("jarvis-location-{}.jsonl", std::process::id()));
        let _ = fs::write(&event_file, "");
        let mut child = Command::new("/usr/bin/open")
            .args(["-n", "-W"])
            .arg(&helper)
            .args(["--args", "--event-file"])
            .arg(&event_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("启动位置能力失败：{error}"))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        loop {
            let content = fs::read_to_string(&event_file).unwrap_or_default();
            for line in content.lines() {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                match value.get("type").and_then(Value::as_str) {
                    Some("location") => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = fs::remove_file(&event_file);
                        return Ok(value.to_string());
                    }
                    Some("error") => {
                        let message = value
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("位置能力失败")
                            .to_owned();
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = fs::remove_file(&event_file);
                        if message.contains("permission denied") {
                            let _ = open_capability_settings("location").await;
                        }
                        return Err(message);
                    }
                    _ => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = fs::remove_file(&event_file);
                return Err("位置能力超时".to_owned());
            }
            if child.try_wait().ok().flatten().is_some() {
                let _ = fs::remove_file(&event_file);
                return Err("位置能力进程提前退出".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

pub(crate) async fn request_camera_capture(app: AppHandle) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        return Err("当前系统没有 Jarvis 原生摄像头能力".to_owned());
    }
    #[cfg(target_os = "macos")]
    {
        let helper = camera_helper_path(&app)?;
        let event_file =
            std::env::temp_dir().join(format!("jarvis-camera-{}.jsonl", std::process::id()));
        let _ = fs::write(&event_file, "");
        let mut child = Command::new("/usr/bin/open")
            .args(["-n", "-W"])
            .arg(&helper)
            .args(["--args", "--event-file"])
            .arg(&event_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("启动摄像头能力失败：{error}"))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        loop {
            let content = fs::read_to_string(&event_file).unwrap_or_default();
            for line in content.lines() {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                match value.get("type").and_then(Value::as_str) {
                    Some("photo") => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = fs::remove_file(&event_file);
                        return Ok(value.to_string());
                    }
                    Some("error") => {
                        let message = value
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("摄像头能力失败")
                            .to_owned();
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = fs::remove_file(&event_file);
                        if message.contains("permission denied") {
                            let _ = open_capability_settings("camera").await;
                        }
                        return Err(message);
                    }
                    _ => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = fs::remove_file(&event_file);
                return Err("摄像头能力超时".to_owned());
            }
            if child.try_wait().ok().flatten().is_some() {
                let _ = fs::remove_file(&event_file);
                return Err("摄像头能力进程提前退出".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

pub(crate) async fn request_pdfspine(
    app: AppHandle,
    workspace: &std::path::Path,
    operation: &str,
    params: Value,
) -> Result<Value, String> {
    pdfspine::request(&app, operation, workspace, params).await
}

#[tauri::command]
async fn cancel_pdfspine(state: State<'_, AppState>) -> Result<(), String> {
    state.pdfspine.shutdown().await;
    Ok(())
}

#[tauri::command]
async fn request_location(app: AppHandle) -> Result<String, String> {
    request_current_location(app).await
}

#[tauri::command]
async fn request_capability(capability: String) -> Result<String, String> {
    open_capability_settings(&capability).await
}

#[tauri::command]
async fn request_all_capabilities() -> Result<String, String> {
    let mut opened = 0usize;
    for capability in config::permission_capabilities() {
        if open_capability_settings(&capability).await.is_ok() {
            opened += 1;
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    }
    if opened == 0 {
        return Err("没有可打开的系统授权页面".to_owned());
    }
    Ok(config::prompt("allCapabilitiesOpened").to_owned())
}

pub(crate) async fn open_capability_settings(capability: &str) -> Result<String, String> {
    let url = config::permission_settings_url(capability.trim());
    if url.is_empty() {
        return Err(format!("未注册的系统能力：{}", capability.trim()));
    }
    let status = Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .await
        .map_err(|error| format!("打开系统权限设置失败：{error}"))?;
    if !status.success() {
        return Err(format!("系统拒绝打开权限设置：{}", capability.trim()));
    }
    Ok(format!("已打开 {} 的系统隐私授权页面", capability.trim()))
}

fn host_app_bundle_path(app: &AppHandle) -> Option<PathBuf> {
    let resource_dir = app.path().resource_dir().ok()?;
    let contents_dir = resource_dir.parent()?;
    let bundle = contents_dir.parent()?;
    (bundle.extension().and_then(|value| value.to_str()) == Some("app"))
        .then(|| bundle.to_path_buf())
}

async fn wake_status_value(state: &AppState) -> WakeStatus {
    WakeStatus {
        enabled: state.wake_enabled.load(Ordering::SeqCst),
        ready: state.wake_ready.load(Ordering::SeqCst),
        authorization: state.wake_authorization.read().await.clone(),
    }
}

fn start_wake_supervisor(app: AppHandle) {
    let state = app.state::<AppState>();
    if state.wake_supervisor_running.swap(true, Ordering::SeqCst) {
        return;
    }
    state.wake_enabled.store(true, Ordering::SeqCst);

    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let helper = match wake_helper_path(&app) {
            Ok(path) => path,
            Err(error) => {
                *state.wake_authorization.write().await = error.clone();
                state.wake_enabled.store(false, Ordering::SeqCst);
                state.wake_supervisor_running.store(false, Ordering::SeqCst);
                let _ = app.emit("jarvis-wake-status", wake_status_value(&state).await);
                return;
            }
        };
        // A previous host may have exited while its LaunchServices helper
        // remained alive. Keep exactly one microphone listener.
        let _ = Command::new("/usr/bin/pkill")
            .args(["-x", "JarvisWakeListener"])
            .status()
            .await;

        let mut woke = false;
        let mut wake_speaker_access = SpeakerAccess::Unknown;
        while state.wake_enabled.load(Ordering::SeqCst) {
            state.wake_ready.store(false, Ordering::SeqCst);
            let event_file =
                std::env::temp_dir().join(format!("jarvis-wake-{}.jsonl", std::process::id()));
            let _ = fs::remove_file(&event_file);
            if let Err(error) = fs::write(&event_file, "") {
                *state.wake_authorization.write().await = format!("无法创建唤醒事件通道：{error}");
                break;
            }
            if !state.wake_enabled.load(Ordering::SeqCst) {
                let _ = fs::remove_file(&event_file);
                break;
            }

            // LaunchServices is required so macOS attributes microphone and
            // speech-recognition permissions to the helper app bundle.
            let mut command = Command::new("/usr/bin/open");
            command
                .args(["-n", "-W"])
                .arg(&helper)
                .args(["--args", "--event-file"])
                .arg(&event_file);
            if let Some(host_app) = host_app_bundle_path(&app) {
                command.arg("--host-app").arg(host_app);
            }
            let mut child = match command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    *state.wake_authorization.write().await =
                        format!("无法启动唤醒监听器：{error}");
                    break;
                }
            };
            state
                .wake_pid
                .store(child.id().unwrap_or(0), Ordering::SeqCst);
            let mut processed = 0usize;

            loop {
                let content = fs::read_to_string(&event_file).unwrap_or_default();
                let lines: Vec<&str> = content.lines().collect();
                for line in lines.iter().skip(processed) {
                    let Ok(message) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    match message.get("type").and_then(Value::as_str) {
                        Some("authorization") => {
                            let authorization = message
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown");
                            *state.wake_authorization.write().await = authorization.to_owned();
                            // `notDetermined` means the helper has just asked
                            // macOS for access. Keep it alive so the native
                            // permission sheet can complete its callback.
                            if matches!(authorization, "denied" | "restricted") {
                                state.wake_enabled.store(false, Ordering::SeqCst);
                            }
                        }
                        Some("ready") => {
                            state.wake_ready.store(true, Ordering::SeqCst);
                        }
                        Some("wake") => {
                            woke = true;
                            wake_speaker_access = message
                                .get("speakerAccess")
                                .or_else(|| message.get("speaker"))
                                .and_then(Value::as_str)
                                .map(|value| match value {
                                    "allen" => SpeakerAccess::Allen,
                                    "rejected" => SpeakerAccess::Rejected,
                                    _ => SpeakerAccess::Unknown,
                                })
                                .unwrap_or(SpeakerAccess::Unknown);
                            state.wake_enabled.store(false, Ordering::SeqCst);
                            state.wake_ready.store(false, Ordering::SeqCst);
                            raise_jarvis_window(&app);
                        }
                        Some("error") => {
                            *state.wake_authorization.write().await = message
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("wake listener error")
                                .to_owned();
                        }
                        _ => {}
                    }
                    let _ = app.emit("jarvis-wake-status", wake_status_value(&state).await);
                    if woke {
                        break;
                    }
                }
                processed = lines.len();
                if woke || !state.wake_enabled.load(Ordering::SeqCst) {
                    break;
                }
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }

            if woke || !state.wake_enabled.load(Ordering::SeqCst) {
                let _ = Command::new("/usr/bin/pkill")
                    .args(["-x", "JarvisWakeListener"])
                    .status()
                    .await;
            }
            let _ = child.wait().await;
            let _ = fs::remove_file(&event_file);
            state.wake_pid.store(0, Ordering::SeqCst);
            if woke || !state.wake_enabled.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        state.wake_ready.store(false, Ordering::SeqCst);
        state.wake_supervisor_running.store(false, Ordering::SeqCst);
        if woke {
            // The WebView owns the RTCPeerConnection, so wake only raises the
            // Jarvis surface and asks the renderer to begin the official Codex
            // app-server V3 Voice handshake. No keypress or UI automation.
            let speaker_access = match effective_speaker_access(wake_speaker_access) {
                SpeakerAccess::Allen => "allen",
                SpeakerAccess::Rejected => "rejected",
                SpeakerAccess::Unknown => "unknown",
            };
            let _ = app.emit(
                "jarvis-wake",
                json!({"ok": true, "speakerAccess": speaker_access}),
            );
        }
        let _ = app.emit("jarvis-wake-status", wake_status_value(&state).await);
    });
}

#[tauri::command]
async fn arm_wake_listener(app: AppHandle) -> Result<WakeStatus, String> {
    // Voice shutdown can race with the previous supervisor task finishing.
    // Set the flag before checking the supervisor guard so an in-flight task
    // continues its loop instead of leaving the listener stopped.
    app.state::<AppState>()
        .wake_enabled
        .store(true, Ordering::SeqCst);
    start_wake_supervisor(app.clone());
    tokio::time::sleep(Duration::from_millis(80)).await;
    Ok(wake_status_value(&app.state::<AppState>()).await)
}

#[tauri::command]
async fn disarm_wake_listener(app: AppHandle) -> Result<WakeStatus, String> {
    let state = app.state::<AppState>();
    state.wake_enabled.store(false, Ordering::SeqCst);
    state.wake_ready.store(false, Ordering::SeqCst);
    let pid = state.wake_pid.swap(0, Ordering::SeqCst);
    if pid > 0 {
        let _ = Command::new("/bin/kill")
            .arg(pid.to_string())
            .status()
            .await;
    }
    // The supervisor starts asynchronously and can cross this command in
    // flight. Keep terminating until it has observed wake_enabled=false.
    for _ in 0..15 {
        let _ = Command::new("/usr/bin/pkill")
            .args(["-x", "JarvisWakeListener"])
            .status()
            .await;
        if !state.wake_supervisor_running.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // AVAudioEngine releases the input device asynchronously after SIGTERM.
    // Starting WebRTC in the same tick can otherwise fail with NotAllowedError.
    tokio::time::sleep(Duration::from_millis(350)).await;
    Ok(wake_status_value(&state).await)
}

#[tauri::command]
async fn wake_listener_status(app: AppHandle) -> WakeStatus {
    wake_status_value(&app.state::<AppState>()).await
}

#[tauri::command]
async fn consume_cold_wake(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    if !state.cold_wake_pending.swap(false, Ordering::SeqCst) {
        return Ok(false);
    }
    // Replay a cold launch through the same external event path as a normal
    // warm wake, but only after the newly spawned listener fully releases mic.
    state.wake_enabled.store(false, Ordering::SeqCst);
    state.wake_ready.store(false, Ordering::SeqCst);
    let pid = state.wake_pid.swap(0, Ordering::SeqCst);
    if pid > 0 {
        let _ = Command::new("/bin/kill")
            .arg(pid.to_string())
            .status()
            .await;
    }
    for _ in 0..15 {
        let _ = Command::new("/usr/bin/pkill")
            .args(["-x", "JarvisWakeListener"])
            .status()
            .await;
        if !state.wake_supervisor_running.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(350)).await;
    raise_jarvis_window(&app);
    let _ = app.emit("jarvis-wake", json!({"ok": true, "cold": true}));
    Ok(true)
}

#[tauri::command]
fn default_workspace() -> Result<String, String> {
    if let Ok(configured) = std::env::var("JARVIS_WORKSPACE") {
        let path = PathBuf::from(configured);
        if path.is_dir() {
            return path
                .canonicalize()
                .map(|value| value.to_string_lossy().into_owned())
                .map_err(|error| format!("无法读取 JARVIS_WORKSPACE：{error}"));
        }
    }
    let project_workspace =
        PathBuf::from(config::project_root()).join(config::workspace_directory());
    if project_workspace.is_dir() {
        return project_workspace
            .canonicalize()
            .map(|value| value.to_string_lossy().into_owned())
            .map_err(|error| format!("无法读取 Jarvis 项目工作目录：{error}"));
    }
    if let Ok(home) = std::env::var("HOME") {
        let path = PathBuf::from(home);
        if path.is_dir() {
            return Ok(path.to_string_lossy().into_owned());
        }
    }
    std::env::current_dir()
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| format!("无法确定默认工作目录：{error}"))
}

fn validated_workspace(cwd: &str) -> Result<String, String> {
    let requested = cwd.trim();
    let path = if requested.is_empty() || requested.contains("/outputs/Jarvis/") {
        PathBuf::from(default_workspace()?)
    } else {
        PathBuf::from(requested)
    };
    // Migrate the pre-organization default without breaking a persisted UI
    // workspace from an older Jarvis build.
    if !path.is_dir() && path.file_name().and_then(|name| name.to_str()) == Some("agent-workspace")
    {
        return default_workspace();
    }
    if !path.is_dir() {
        return Err(format!("工作目录不存在或不是文件夹：{cwd}"));
    }
    path.canonicalize()
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| format!("无法读取工作目录：{error}"))
}

async fn terminate_runtime(state: &AppState) -> Result<(), String> {
    if let Some(runtime) = state.runtime.lock().await.take() {
        runtime
            .child
            .lock()
            .await
            .kill()
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn ensure_runtime(
    app: AppHandle,
    state: &State<'_, AppState>,
    cwd: &str,
    resume_thread_id: Option<&str>,
    permission_mode: PermissionMode,
    speaker_access: SpeakerAccess,
) -> Result<Arc<CodexRuntime>, String> {
    let speaker_access = effective_speaker_access(speaker_access);
    let cwd = validated_workspace(cwd)?;
    let existing = { state.runtime.lock().await.clone() };
    if let Some(existing) = existing {
        if existing.permission_mode == permission_mode
            && existing.speaker_access == speaker_access
            && existing.workspace == cwd
        {
            return Ok(existing);
        }
        terminate_runtime(state).await?;
    }
    let profile = permission_mode.profile();
    let runtime = CodexRuntime::spawn(app, permission_mode, speaker_access, cwd.clone()).await?;
    runtime.request("initialize", json!({
        "clientInfo": {"name": "jarvis-codex", "title": "Jarvis Codex", "version": env!("CARGO_PKG_VERSION")},
        "capabilities": {"experimentalApi": true}
    })).await?;
    runtime.notify("initialized", json!({})).await?;
    let memory_context = memory_store().initial_context(8_000);
    let skills_context = memory_store().skills_context(4_000);
    let foundation_context = [memory_context, skills_context]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let base_instructions = config::prompt("codexBaseInstructions")
        .replacen("{}", profile.instructions, 1)
        .replacen("{}", speaker_access.instructions(), 1)
        .replacen("{}", &foundation_context, 1);
    let base_instructions = format!(
        "{base_instructions}\n\n{}",
        config::prompt("speakerProfilePolicy")
    );
    let thread_options = json!({
        "cwd": cwd,
        "model": JARVIS_MODEL,
        "approvalPolicy": profile.approval_policy,
        "sandbox": profile.sandbox,
        "dynamicTools": tools::openai_schemas().into_iter().map(|schema| {
            let function = &schema["function"];
            json!({"type": "function", "name": function["name"], "description": function["description"], "inputSchema": function["parameters"]})
        }).collect::<Vec<_>>(),
        "baseInstructions": base_instructions
    });
    let started = if let Some(thread_id) = resume_thread_id.filter(|value| !value.trim().is_empty())
    {
        let mut resume_options = thread_options.clone();
        resume_options
            .as_object_mut()
            .expect("thread options must be a JSON object")
            .remove("dynamicTools");
        resume_options["threadId"] = Value::String(thread_id.to_owned());
        match runtime.request("thread/resume", resume_options).await {
            Ok(resumed) => resumed,
            Err(_) => {
                let mut start_options = thread_options;
                start_options["ephemeral"] = Value::Bool(false);
                runtime.request("thread/start", start_options).await?
            }
        }
    } else {
        let mut start_options = thread_options;
        start_options["ephemeral"] = Value::Bool(false);
        runtime.request("thread/start", start_options).await?
    };
    let thread_id = started
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .ok_or("Codex 未返回 threadId")?
        .to_owned();
    *runtime.thread_id.write().await = Some(thread_id.clone());
    *state.runtime.lock().await = Some(runtime.clone());
    Ok(runtime)
}

#[tauri::command]
async fn start_jarvis(
    app: AppHandle,
    state: State<'_, AppState>,
    cwd: String,
    thread_id: Option<String>,
    permission_mode: PermissionMode,
    speaker_access: SpeakerAccess,
) -> Result<SessionInfo, String> {
    let speaker_access = effective_speaker_access(speaker_access);
    if speaker_access == SpeakerAccess::Rejected {
        return Err("未识别的说话人".to_owned());
    }
    *state.speaker_access.write().await = speaker_access;
    let runtime = ensure_runtime(
        app,
        &state,
        &cwd,
        thread_id.as_deref(),
        permission_mode,
        speaker_access,
    )
    .await?;
    let thread_id = runtime.thread().await?;
    Ok(SessionInfo { thread_id, cwd })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn start_codex_voice(
    app: AppHandle,
    state: State<'_, AppState>,
    cwd: String,
    thread_id: Option<String>,
    permission_mode: PermissionMode,
    speaker_access: SpeakerAccess,
    sdp: String,
    voice: Option<String>,
) -> Result<DirectVoiceInfo, String> {
    if !sdp.starts_with("v=0") {
        return Err("WebRTC SDP offer 无效".to_owned());
    }
    let speaker_access = effective_speaker_access(speaker_access);
    if speaker_access == SpeakerAccess::Rejected {
        return Err("未识别的说话人".to_owned());
    }
    *state.speaker_access.write().await = speaker_access;
    let runtime = ensure_runtime(
        app,
        &state,
        &cwd,
        thread_id.as_deref(),
        permission_mode,
        speaker_access,
    )
    .await?;
    let thread_id = runtime.thread().await?;
    if runtime.voice_active.load(Ordering::SeqCst) {
        let _ = runtime
            .request("thread/realtime/stop", json!({"threadId": thread_id}))
            .await;
    }
    *runtime.voice_phase.write().await = "starting".to_owned();
    runtime.voice_active.store(false, Ordering::SeqCst);
    *runtime.realtime_session_id.write().await = None;

    let mut params = json!({
        "threadId": thread_id,
        "outputModality": "audio",
        "version": "v3",
        "includeStartupContext": true,
        "clientManagedHandoffs": false,
        // STOP must be final. Flushing the tail can create a new Codex turn
        // after the user has already stopped the session.
        "flushTranscriptTailOnSessionEnd": false,
        "codexResponsesAsItems": false,
        "codexResponseHandoffMode": "commentary",
        "transport": {"type": "webrtc", "sdp": sdp}
    });
    if let Some(voice) = voice {
        const SUPPORTED: &[&str] = &[
            "alloy", "arbor", "ash", "ballad", "breeze", "cedar", "coral", "cove", "echo", "ember",
            "juniper", "maple", "marin", "sage", "shimmer", "sol", "spruce", "vale", "verse",
        ];
        if SUPPORTED.contains(&voice.as_str()) {
            params["voice"] = Value::String(voice);
        }
    }
    if let Err(error) = runtime.request("thread/realtime/start", params).await {
        *runtime.voice_phase.write().await = "error".to_owned();
        return Err(format!("Codex Voice V3 启动失败：{error}"));
    }
    Ok(direct_voice_info(&state).await)
}

#[tauri::command]
async fn stop_codex_voice(state: State<'_, AppState>) -> Result<DirectVoiceInfo, String> {
    let Ok(runtime) = runtime(&state).await else {
        return Ok(direct_voice_info(&state).await);
    };
    let thread_id = runtime.thread().await?;
    *runtime.voice_phase.write().await = "stopping".to_owned();
    let result = runtime
        .request("thread/realtime/stop", json!({"threadId": thread_id}))
        .await;
    runtime.voice_active.store(false, Ordering::SeqCst);
    *runtime.voice_phase.write().await = "closed".to_owned();
    *runtime.realtime_session_id.write().await = None;
    result?;
    Ok(direct_voice_info(&state).await)
}

#[tauri::command]
async fn append_codex_voice_text(state: State<'_, AppState>, text: String) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Voice 文本不能为空".to_owned());
    }
    let runtime = runtime(&state).await?;
    if !runtime.voice_active.load(Ordering::SeqCst) {
        return Err("Codex Voice 尚未连接".to_owned());
    }
    let thread_id = runtime.thread().await?;
    let text = with_memory_context(text);
    runtime
        .request(
            "thread/realtime/appendText",
            json!({"threadId": thread_id, "role": "user", "text": text}),
        )
        .await?;
    Ok(())
}

#[tauri::command]
async fn send_text(
    _app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    let speaker_access = effective_speaker_access(*state.speaker_access.read().await);
    if speaker_access == SpeakerAccess::Rejected {
        return Err("未识别的说话人".to_owned());
    }
    let runtime = runtime(&state).await?;
    if runtime.speaker_access != speaker_access {
        return Err("说话人状态已变化，请重新建立安全会话".to_owned());
    }
    let thread_id = runtime.thread().await?;
    crate::logging::conversation("user", text.trim(), "codex");
    let text = with_memory_context(&text);
    runtime
        .request(
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{"type": "text", "text": text, "text_elements": []}]
            }),
        )
        .await?;
    Ok(())
}

#[tauri::command]
async fn cipherpipe_send(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    peer: Option<String>,
) -> Result<(), String> {
    state.cipherpipe.send(&app, &text, peer.as_deref()).await
}

fn with_memory_context(text: &str) -> String {
    let context = memory_store().recall(text, 4_000);
    let working = memory_store().read_working(2_000);
    let knowledge = knowledge::KnowledgeStore::default().context(text, 4_000);
    let context = [context, working, knowledge]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if context.is_empty() {
        return text.to_owned();
    }
    format!("{context}\n\n## Current user request\n{text}")
}

#[tauri::command]
fn memory_status() -> memory::MemoryStatus {
    memory_store().status()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WakeMemoryStatus {
    initial_context_chars: usize,
    working_context_chars: usize,
}

#[tauri::command]
fn prepare_wake_context() -> WakeMemoryStatus {
    let initial = memory_store().initial_context(8_000);
    let working = memory_store().read_working(2_000);
    let status = WakeMemoryStatus {
        initial_context_chars: initial.chars().count(),
        working_context_chars: working.chars().count(),
    };
    logging::text(
        "jarvis-runtime",
        &format!(
            "wake memory loaded: initial={} chars, working={} chars",
            status.initial_context_chars, status.working_context_chars
        ),
    );
    status
}

#[tauri::command]
fn memory_recall(query: String) -> String {
    memory_store().recall(&query, 4_000)
}

#[tauri::command]
fn memory_save_core(key: String, value: String, category: String) -> Result<String, String> {
    memory_store().save_core(&key, &value, &category)
}

#[tauri::command]
fn memory_save_episode(
    title: String,
    summary: String,
    tags: Vec<String>,
) -> Result<String, String> {
    memory_store().save_episode(&title, &summary, &tags)
}

#[tauri::command]
fn memory_working_append(role: String, content: String) -> Result<String, String> {
    memory_store().append_working(&role, &content)
}

#[tauri::command]
fn memory_working_context(max_chars: Option<usize>) -> String {
    memory_store().read_working(max_chars.unwrap_or(8_000))
}

#[tauri::command]
fn memory_working_compress(
    summary: Option<String>,
    keep_recent: Option<usize>,
) -> Result<String, String> {
    memory_store().compress_working(summary.as_deref(), keep_recent.unwrap_or(4))
}

#[tauri::command]
fn memory_save_procedure(
    name: String,
    description: String,
    trigger_pattern: String,
    steps: Vec<String>,
) -> Result<String, String> {
    memory_store().save_procedure(&name, &description, &trigger_pattern, &steps)
}

#[tauri::command]
fn memory_search_procedures(query: String, limit: Option<usize>) -> Vec<memory::ProcedureNote> {
    memory_store().search_procedures(&query, limit.unwrap_or(8))
}

#[tauri::command]
fn knowledge_status() -> knowledge::KnowledgeStatus {
    knowledge::KnowledgeStore::default().status()
}

#[tauri::command]
fn knowledge_scan() -> Result<knowledge::KnowledgeScanResult, String> {
    knowledge::KnowledgeStore::default().scan()
}

#[tauri::command]
fn knowledge_search(
    query: String,
    limit: Option<usize>,
) -> Result<Vec<knowledge::KnowledgeResult>, String> {
    knowledge::KnowledgeStore::default().search(&query, limit.unwrap_or(5))
}

#[tauri::command]
fn tool_list() -> Vec<tools::ToolSpec> {
    tools::list()
}

#[tauri::command]
fn bridge_status() -> bridge::BridgeStatus {
    bridge::status()
}

#[tauri::command]
async fn bridge_enable(
    app: AppHandle,
    state: State<'_, AppState>,
    pairing_token: Option<String>,
    bind_address: Option<String>,
) -> Result<bridge::BridgeEnableResult, String> {
    let workspace = state
        .runtime
        .lock()
        .await
        .as_ref()
        .map(|runtime| runtime.workspace.clone())
        .unwrap_or(default_workspace()?);
    let result = bridge::enable(app, PathBuf::from(workspace), pairing_token, bind_address)?;
    bridge::publish(
        "bridge.enabled",
        serde_json::to_value(&result.status).unwrap_or_else(|_| json!({})),
    );
    Ok(result)
}

#[tauri::command]
fn bridge_disable(pairing_token: Option<String>) -> Result<bridge::BridgeStatus, String> {
    let status = bridge::disable(pairing_token)?;
    bridge::publish(
        "bridge.disabled",
        serde_json::to_value(&status).unwrap_or_else(|_| json!({})),
    );
    Ok(status)
}

#[tauri::command]
fn workflow_list() -> Vec<workflow::WorkflowDefinition> {
    workflow::list()
}

#[tauri::command]
fn workflow_save(
    definition: workflow::WorkflowDefinition,
) -> Result<workflow::WorkflowDefinition, String> {
    workflow::save(definition)
}

#[tauri::command]
async fn workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    workflow_id: String,
    approved: Option<bool>,
) -> Result<workflow::WorkflowRunResult, String> {
    let runtime = runtime(&state).await?;
    let workspace = PathBuf::from(runtime.workspace.clone());
    let full_access = runtime.permission_mode == PermissionMode::Full;
    workflow::run(
        app,
        &workspace,
        &workflow_id,
        full_access,
        approved.unwrap_or(false),
    )
    .await
}

#[tauri::command]
fn task_list() -> Vec<tasks::TaskRecord> {
    tasks::list()
}

#[tauri::command]
fn task_schedule(task: tasks::TaskRecord) -> Result<tasks::TaskRecord, String> {
    tasks::schedule(task)
}

#[tauri::command]
fn task_cancel(task_id: String) -> Result<(), String> {
    tasks::cancel(&task_id)
}

#[tauri::command]
fn task_resume(task_id: String) -> Result<tasks::TaskRecord, String> {
    tasks::resume(&task_id)
}

#[tauri::command]
async fn task_run_due(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<tasks::TaskRecord>, String> {
    let runtime = runtime(&state).await?;
    let workspace = PathBuf::from(runtime.workspace.clone());
    let full_access = runtime.permission_mode == PermissionMode::Full;
    tasks::run_due(app, &workspace, full_access).await
}

#[tauri::command]
async fn tool_execute(
    app: AppHandle,
    state: State<'_, AppState>,
    tool_name: String,
    args: Value,
) -> Result<tools::ToolResult, String> {
    let runtime = runtime(&state).await?;
    let workspace = PathBuf::from(runtime.workspace.clone());
    let full_access = runtime.permission_mode == PermissionMode::Full;
    Ok(tools::execute(app, &workspace, &tool_name, args, full_access).await)
}

#[tauri::command]
async fn local_qwen_chat(
    app: AppHandle,
    text: String,
    workspace: Option<String>,
) -> Result<String, String> {
    let workspace = workspace
        .filter(|value| !value.trim().is_empty())
        .map(|value| validated_workspace(&value))
        .transpose()?
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default_workspace().unwrap_or_else(|_| ".".to_owned())));
    on_device_model::chat(app, text, workspace).await
}

#[tauri::command]
async fn on_device_model_status() -> Result<String, String> {
    match on_device_model::detect_model().await {
        Ok(model) => {
            logging::text(
                "jarvis-runtime",
                &format!("on-device model detected: {model}"),
            );
            Ok(model)
        }
        Err(error) => {
            logging::text(
                "jarvis-runtime",
                &format!("on-device model detection failed: {error}"),
            );
            Err(error)
        }
    }
}

async fn stop_speech(state: &AppState) {
    state.offline_speech.cancel().await;
    if let Some(mut child) = state.speech.lock().await.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

#[tauri::command]
async fn speak_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    stop_speech(&state).await;
    if let Ok(wav) = state.offline_speech.synthesize(&app, text).await {
        let path = std::env::temp_dir().join(format!("jarvis-speech-{}.wav", std::process::id()));
        fs::write(&path, wav).map_err(|e| e.to_string())?;
        let child = Command::new("/usr/bin/afplay")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
        *state.speech.lock().await = Some(child);
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    let child = {
        let resource_dir = app
            .path()
            .resource_dir()
            .map_err(|error| format!("无法定位本地语音资源：{error}"))?;
        let script = resource_dir.join("local_tts.py");
        if !script.is_file() {
            return Err(format!("本地语音脚本不存在：{}", script.display()));
        }
        let model_dir = std::env::var_os("JARVIS_TTS_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(config::project_root()).join("models/kokoro"));
        let python = offline_speech::python_path();
        Command::new(python)
            .args(["-u"])
            .arg(&script)
            .args(["--model"])
            .arg(model_dir)
            .args(["--text"])
            .arg(text)
            .spawn()
            .map_err(|error| format!("无法启动本地 Kokoro 语音：{error}"))?
    };
    #[cfg(not(target_os = "macos"))]
    let child = return Err("当前系统暂未接入本机语音".to_owned());
    *state.speech.lock().await = Some(child);
    loop {
        let finished = {
            let mut speech = state.speech.lock().await;
            let Some(child) = speech.as_mut() else {
                return Ok(());
            };
            match child.try_wait().map_err(|error| error.to_string())? {
                Some(status) => {
                    speech.take();
                    if status.success() {
                        return Ok(());
                    }
                    return Err(format!("本机语音退出：{status}"));
                }
                None => false,
            }
        };
        if finished {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

#[tauri::command]
async fn stop_all(state: State<'_, AppState>) -> Result<(), String> {
    stop_speech(&state).await;
    let Ok(runtime) = runtime(&state).await else {
        return Ok(());
    };
    let thread_id = runtime.thread().await?;
    // A realtime handoff and STOP can cross in flight. Re-check briefly so a
    // turn that starts just after realtime/stop is interrupted as well.
    let mut interrupted_turn: Option<String> = None;
    for _ in 0..6 {
        if let Some(turn_id) = runtime.active_turn.read().await.clone() {
            if interrupted_turn.as_deref() != Some(turn_id.as_str()) {
                let _ = runtime
                    .request(
                        "turn/interrupt",
                        json!({"threadId": thread_id, "turnId": turn_id}),
                    )
                    .await;
                interrupted_turn = Some(turn_id);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    if let Ok(background) = runtime
        .request(
            "thread/backgroundTerminals/list",
            json!({"threadId": thread_id, "limit": 100}),
        )
        .await
    {
        if let Some(terminals) = background.get("data").and_then(Value::as_array) {
            for terminal in terminals {
                if let Some(process_id) = terminal.get("processId").and_then(Value::as_str) {
                    let _ = runtime
                        .request(
                            "thread/backgroundTerminals/terminate",
                            json!({"threadId": thread_id, "processId": process_id}),
                        )
                        .await;
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn resolve_server_request(
    state: State<'_, AppState>,
    request_id: Value,
    approved: bool,
) -> Result<(), String> {
    let runtime = runtime(&state).await?;
    runtime.write(&json!({"id": request_id, "result": {"decision": if approved {"accept"} else {"decline"}}})).await
}

#[tauri::command]
async fn shutdown(state: State<'_, AppState>) -> Result<(), String> {
    stop_speech(&state).await;
    state.pdfspine.shutdown().await;
    state.cipherpipe.stop().await;
    terminate_runtime(&state).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();
    let arguments: Vec<String> = std::env::args().collect();
    let cold_wake_pending = arguments.iter().any(|argument| argument == "--jarvis-wake");
    let background_start = arguments.iter().any(|argument| argument == "--background");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            raise_jarvis_window(app);
        }))
        .manage(AppState {
            runtime: Mutex::new(None),
            speech: Mutex::new(None),
            offline_speech: offline_speech::OfflineSpeech::new(),
            pdfspine: pdfspine::PdfSpine::new(),
            cipherpipe: cipherpipe::CipherPipe::new(),
            speaker_access: RwLock::new(effective_speaker_access(SpeakerAccess::Unknown)),
            cold_wake_pending: AtomicBool::new(cold_wake_pending),
            background_start,
            wake_enabled: AtomicBool::new(false),
            wake_ready: AtomicBool::new(false),
            wake_supervisor_running: AtomicBool::new(false),
            wake_pid: AtomicU32::new(0),
            wake_authorization: RwLock::new("notDetermined".to_owned()),
        })
        .invoke_handler(tauri::generate_handler![
            direct_voice_status,
            arm_wake_listener,
            disarm_wake_listener,
            wake_listener_status,
            consume_cold_wake,
            default_workspace,
            startup_is_background,
            permission_status,
            request_microphone_permission,
            verify_speaker,
            speech_permission_status,
            request_location,
            request_capability,
            request_all_capabilities,
            start_jarvis,
            start_codex_voice,
            stop_codex_voice,
            append_codex_voice_text,
            send_text,
            cipherpipe_send,
            memory_status,
            prepare_wake_context,
            memory_recall,
            memory_save_core,
            memory_save_episode,
            memory_working_append,
            memory_working_context,
            memory_working_compress,
            memory_save_procedure,
            memory_search_procedures,
            skills_list,
            skills_match,
            skills_context,
            connector_list,
            knowledge_status,
            knowledge_scan,
            knowledge_search,
            tool_list,
            tool_execute,
            cancel_pdfspine,
            bridge_status,
            bridge_enable,
            bridge_disable,
            workflow_list,
            workflow_save,
            workflow_run,
            task_list,
            task_schedule,
            task_cancel,
            task_resume,
            task_run_due,
            local_qwen_chat,
            on_device_model_status,
            speak_text,
            stop_all,
            resolve_server_request,
            shutdown
        ])
        .setup(move |app| {
            use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
            app.handle().plugin(tauri_plugin_autostart::init(
                MacosLauncher::LaunchAgent,
                Some(vec!["--background"]),
            ))?;
            let _ = app.autolaunch().enable();
            tasks::start_scheduler(app.handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                if background_start {
                    let _ = window.hide();
                } else {
                    raise_jarvis_window(app.handle());
                }
            }
            if background_start {
                start_wake_supervisor(app.handle().clone());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Jarvis Codex");
}
