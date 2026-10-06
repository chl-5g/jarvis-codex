use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;
use tokio::{process::Command, time::timeout};

const MAX_OUTPUT: usize = 12_000;
const MAX_SEARCH_RESULTS: usize = 40;
const MAX_LIST_RESULTS: usize = 200;
const DEFAULT_TIMEOUT_SECONDS: u64 = 60;
static NEXT_CALL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub requires_full_access: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: String,
    pub tool_name: String,
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolEvent {
    pub kind: &'static str,
    pub phase: &'static str,
    pub call_id: String,
    pub tool_name: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

pub fn list() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file",
            description: crate::config::tool_description("read_file"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "write_file",
            description: crate::config::tool_description("write_file"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "append_file",
            description: crate::config::tool_description("append_file"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "list_files",
            description: crate::config::tool_description("list_files"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "search_files",
            description: crate::config::tool_description("search_files"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "run_command",
            description: crate::config::tool_description("run_command"),
            requires_full_access: true,
        },
        ToolSpec {
            name: "current_time",
            description: crate::config::tool_description("current_time"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "current_location",
            description: crate::config::tool_description("current_location"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "current_weather",
            description: crate::config::tool_description("current_weather"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "open_camera",
            description: crate::config::tool_description("open_camera"),
            requires_full_access: false,
        },
        ToolSpec {
            name: "request_capability",
            description: crate::config::tool_description("request_capability"),
            requires_full_access: false,
        },
    ]
}

/// OpenAI-compatible function schemas for local Qwen/Xinference providers.
/// Keep this registry in one place so workflows, the UI and model routing all
/// observe the same bounded tool surface.
pub fn openai_schemas() -> Vec<Value> {
    vec![
        schema(
            "read_file",
            crate::config::tool_description("read_file"),
            json!({
                "path": {"type": "string", "description": "Workspace-relative or explicitly permitted path"}
            }),
            &["path"],
        ),
        schema(
            "write_file",
            crate::config::tool_description("write_file"),
            json!({
                "path": {"type": "string"}, "content": {"type": "string"}
            }),
            &["path", "content"],
        ),
        schema(
            "append_file",
            crate::config::tool_description("append_file"),
            json!({
                "path": {"type": "string"}, "content": {"type": "string"}
            }),
            &["path", "content"],
        ),
        schema(
            "list_files",
            crate::config::tool_description("list_files"),
            json!({
                "path": {"type": "string", "description": "Directory, default ."}
            }),
            &[],
        ),
        schema(
            "search_files",
            crate::config::tool_description("search_files"),
            json!({
                "query": {"type": "string"}, "path": {"type": "string", "description": "Directory, default ."}
            }),
            &["query"],
        ),
        schema(
            "run_command",
            crate::config::tool_description("run_command"),
            json!({
                "command": {"type": "string"}, "timeout": {"type": "integer", "minimum": 1, "maximum": DEFAULT_TIMEOUT_SECONDS}
            }),
            &["command"],
        ),
        schema(
            "current_time",
            crate::config::tool_description("current_time"),
            json!({}),
            &[],
        ),
        schema(
            "current_location",
            crate::config::tool_description("current_location"),
            json!({}),
            &[],
        ),
        schema(
            "current_weather",
            crate::config::tool_description("current_weather"),
            json!({}),
            &[],
        ),
        schema(
            "open_camera",
            crate::config::tool_description("open_camera"),
            json!({}),
            &[],
        ),
        schema(
            "request_capability",
            crate::config::tool_description("request_capability"),
            json!({"capability": {"type": "string"}}),
            &["capability"],
        ),
    ]
}

fn schema(
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &[&str],
) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {"type": "object", "properties": properties, "required": required, "additionalProperties": false}
        }
    })
}

pub async fn execute(
    app: AppHandle,
    workspace: &Path,
    tool_name: &str,
    args: Value,
    full_access: bool,
) -> ToolResult {
    let call_id = next_call_id();
    emit_event(
        &app,
        ToolEvent {
            kind: "tool",
            phase: "started",
            call_id: call_id.clone(),
            tool_name: tool_name.to_owned(),
            output: None,
            error: None,
        },
    );

    let result = match tool_name {
        "read_file" => read_file(workspace, &args, full_access),
        "write_file" => write_file(workspace, &args, full_access),
        "append_file" => append_file(workspace, &args, full_access),
        "list_files" => list_files(workspace, &args, full_access),
        "search_files" => search_files(workspace, &args, full_access),
        "run_command" => run_command(workspace, &args, full_access).await,
        "current_time" => Ok(chrono_like_now()),
        "current_location" => crate::request_current_location(app.clone()).await,
        "current_weather" => network_weather(app.clone()).await,
        "open_camera" => open_camera().await,
        "request_capability" => match string_arg(&args, "capability") {
            Ok(capability) => crate::open_capability_settings(capability.trim()).await,
            Err(error) => Err(error),
        },
        _ => Err(format!("未知工具：{tool_name}")),
    };

    let response = match result {
        Ok(output) => ToolResult {
            call_id: call_id.clone(),
            tool_name: tool_name.to_owned(),
            success: true,
            output: truncate(output),
            error: None,
        },
        Err(error) => ToolResult {
            call_id: call_id.clone(),
            tool_name: tool_name.to_owned(),
            success: false,
            output: String::new(),
            error: Some(error),
        },
    };
    emit_event(
        &app,
        ToolEvent {
            kind: "tool",
            phase: if response.success {
                "completed"
            } else {
                "error"
            },
            call_id,
            tool_name: response.tool_name.clone(),
            output: response.success.then(|| response.output.clone()),
            error: response.error.clone(),
        },
    );
    response
}

fn read_file(root: &Path, args: &Value, full_access: bool) -> Result<String, String> {
    let path = resolve_path(root, string_arg(args, "path")?, full_access)?;
    let content = fs::read_to_string(&path).map_err(|error| format!("读取文件失败：{error}"))?;
    Ok(truncate(content))
}

fn write_file(root: &Path, args: &Value, full_access: bool) -> Result<String, String> {
    let path = resolve_path(root, string_arg(args, "path")?, full_access)?;
    let content = string_arg(args, "content")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
    }
    fs::write(&path, content).map_err(|error| format!("写入文件失败：{error}"))?;
    Ok(format!(
        "已写入 {} 个字符：{}",
        content.chars().count(),
        path.display()
    ))
}

fn append_file(root: &Path, args: &Value, full_access: bool) -> Result<String, String> {
    let path = resolve_path(root, string_arg(args, "path")?, full_access)?;
    let content = string_arg(args, "content")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
    }
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("打开文件失败：{error}"))?;
    file.write_all(content.as_bytes())
        .map_err(|error| format!("追加文件失败：{error}"))?;
    Ok(format!(
        "已追加 {} 个字符：{}",
        content.chars().count(),
        path.display()
    ))
}

fn list_files(root: &Path, args: &Value, full_access: bool) -> Result<String, String> {
    let base = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let path = resolve_path(root, base, full_access)?;
    let mut files = Vec::new();
    collect_files(&path, &mut files, MAX_LIST_RESULTS);
    files.sort();
    Ok(files
        .into_iter()
        .map(|file| file.display().to_string())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn search_files(root: &Path, args: &Value, full_access: bool) -> Result<String, String> {
    let query = string_arg(args, "query")?.to_lowercase();
    if query.trim().is_empty() {
        return Err("search_files 需要 query".to_owned());
    }
    let base = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let path = resolve_path(root, base, full_access)?;
    let mut files = Vec::new();
    collect_files(&path, &mut files, MAX_SEARCH_RESULTS * 4);
    let mut matches = Vec::new();
    for file in files {
        if let Ok(content) = fs::read_to_string(&file) {
            if content.to_lowercase().contains(&query) {
                let excerpt: String = content.chars().take(400).collect();
                matches.push(format!("{}\n{}", file.display(), excerpt));
                if matches.len() >= MAX_SEARCH_RESULTS {
                    break;
                }
            }
        }
    }
    Ok(matches.join("\n---\n"))
}

async fn run_command(root: &Path, args: &Value, full_access: bool) -> Result<String, String> {
    let command = string_arg(args, "command")?;
    if command.trim().is_empty() {
        return Err("run_command 需要 command".to_owned());
    }
    if !full_access {
        if is_dangerous(command) {
            return Err("命令被工具网关拦截：需要完全访问权限".to_owned());
        }
        if !restricted_command_allowed(command) {
            return Err("命令被工具网关拦截：本地 Agent 只允许只读白名单命令".to_owned());
        }
    }
    let requested_timeout = args
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .clamp(1, DEFAULT_TIMEOUT_SECONDS);
    let child = Command::new("/bin/zsh")
        .args(["-lc", command])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("启动命令失败：{error}"))?;
    let output = timeout(
        std::time::Duration::from_secs(requested_timeout),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| format!("命令超时（{} 秒）", requested_timeout))?
    .map_err(|error| format!("等待命令失败：{error}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let status = output.status.code().unwrap_or(-1);
    Ok(format!("Exit code: {status}\n{}", truncate(text)))
}

async fn network_location() -> Result<String, String> {
    let output = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "8",
            "https://ipapi.co/json/",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("位置服务启动失败：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "位置服务不可用：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("位置服务返回无效数据：{error}"))?;
    let city = value.get("city").and_then(Value::as_str).unwrap_or("");
    let region = value.get("region").and_then(Value::as_str).unwrap_or("");
    let country = value
        .get("country_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    if city.is_empty() && region.is_empty() {
        return Err("位置服务没有返回城市".to_owned());
    }
    Ok(
        json!({"city": city, "region": region, "country": country, "source": "network"})
            .to_string(),
    )
}

async fn network_weather(app: AppHandle) -> Result<String, String> {
    let location = match crate::request_current_location(app).await {
        Ok(location) => location,
        Err(_) => network_location().await?,
    };
    let value: Value =
        serde_json::from_str(&location).map_err(|error| format!("位置数据解析失败：{error}"))?;
    let city = value
        .get("city")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or("无法确定当前城市")?;
    let encoded = city.replace(' ', "%20");
    let url = format!("https://wttr.in/{encoded}?format=j1");
    let output = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "12",
            &url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("天气服务启动失败：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "天气服务不可用：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let weather: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("天气服务返回无效数据：{error}"))?;
    Ok(json!({"location": value, "weather": weather, "source": "wttr.in"}).to_string())
}

async fn open_camera() -> Result<String, String> {
    let status = Command::new("/usr/bin/open")
        .args(["-a", "Photo Booth"])
        .status()
        .await;
    let status = match status {
        Ok(status) if status.success() => {
            return Ok(crate::config::prompt("cameraOpenSuccess").to_owned())
        }
        Ok(status) => status,
        Err(error) => {
            return Err(format!(
                "{}：{error}",
                crate::config::prompt("cameraOpenFailure")
            ))
        }
    };
    if !status.success() {
        let _ = crate::open_capability_settings("camera").await;
        return Err(crate::config::prompt("cameraOpenFailure").to_owned());
    }
    Err(crate::config::prompt("cameraOpenFailure").to_owned())
}

fn resolve_path(root: &Path, raw: &str, full_access: bool) -> Result<PathBuf, String> {
    if raw.trim().is_empty() {
        return Err("文件路径不能为空".to_owned());
    }
    let expanded = if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var_os("HOME").ok_or("无法定位 HOME")?;
        PathBuf::from(home).join(raw.trim_start_matches("~/"))
    } else {
        PathBuf::from(raw)
    };
    let root = root
        .canonicalize()
        .map_err(|error| format!("工作目录不可用：{error}"))?;
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        root.join(expanded)
    };
    let canonical = canonicalize_with_missing(&candidate)?;
    if !full_access && !canonical.starts_with(&root) {
        return Err("路径超出当前工作目录，工具网关已拒绝".to_owned());
    }
    Ok(canonical)
}

fn canonicalize_with_missing(candidate: &Path) -> Result<PathBuf, String> {
    let mut missing = Vec::new();
    let mut cursor = candidate.to_path_buf();
    while !cursor.exists() {
        let name = cursor.file_name().ok_or("路径没有文件名")?.to_owned();
        missing.push(name);
        cursor = cursor.parent().ok_or("路径没有父目录")?.to_path_buf();
    }
    let mut resolved = cursor
        .canonicalize()
        .map_err(|error| format!("路径不可用：{error}"))?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn collect_files(path: &Path, output: &mut Vec<PathBuf>, limit: usize) {
    if output.len() >= limit {
        return;
    }
    // Never follow symlinked files or directories during recursive traversal.
    // `Path::is_file`/`is_dir` follow links, which would let a workspace-local
    // link expose an arbitrary tree to list/search tools.
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        output.push(path.to_owned());
        return;
    }
    if !metadata.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        let Ok(child_metadata) = entry.file_type() else {
            continue;
        };
        if child_metadata.is_symlink() {
            continue;
        }
        collect_files(&child, output, limit);
        if output.len() >= limit {
            break;
        }
    }
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("缺少参数：{key}"))
}

fn truncate(mut text: String) -> String {
    if text.chars().count() > MAX_OUTPUT {
        text = text.chars().take(MAX_OUTPUT).collect();
        text.push_str("\n… [output truncated]");
    }
    text
}

fn is_dangerous(command: &str) -> bool {
    // The command is executed through `zsh -lc`, so shell composition and
    // redirection are unsafe even when the first word looks harmless. Reject
    // these controls conservatively in restricted mode, rather than trying to
    // implement a shell parser here.
    if command
        .chars()
        .any(|character| matches!(character, ';' | '&' | '|' | '`' | '>' | '<'))
        || command.contains("$(")
    {
        return true;
    }

    // Inspect every shell word. This catches a dangerous command after an
    // environment assignment, `sudo`, or a benign-looking command prefix.
    let words = command
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| matches!(character, '\'' | '"' | '(' | ')' | '\\'))
        })
        .map(|word| word.rsplit('/').next().unwrap_or(word).to_owned())
        .collect::<Vec<_>>();
    words.iter().enumerate().any(|(index, word)| {
        let code_runner = matches!(
            word.as_str(),
            "python"
                | "python2"
                | "python3"
                | "node"
                | "nodejs"
                | "ruby"
                | "perl"
                | "osascript"
                | "bash"
                | "sh"
                | "zsh"
        );
        let code_flag = words
            .get(index + 1)
            .is_some_and(|argument| matches!(argument.as_str(), "-c" | "-e" | "--eval"));
        matches!(
            word.as_str(),
            "rm" | "rmdir"
                | "dd"
                | "mkfs"
                | "fdisk"
                | "shutdown"
                | "reboot"
                | "kill"
                | "killall"
                | "chmod"
                | "chown"
                | "curl"
                | "wget"
        ) || (code_runner && code_flag)
    })
}

/// Commands exposed to the local Qwen path are deliberately read-only and
/// small in scope. Codex's explicit full-access profile uses the existing
/// permission boundary instead of this local allowlist.
fn restricted_command_allowed(command: &str) -> bool {
    if command.trim().is_empty() || is_dangerous(command) {
        return false;
    }
    let words = command.split_whitespace().collect::<Vec<_>>();
    let Some(first) = words.first() else {
        return false;
    };
    let first = Path::new(first.trim_matches(['\'', '"', '(', ')', '\\']))
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match first {
        "pwd" | "ls" | "find" | "du" | "df" | "file" | "stat" | "wc" | "head" | "tail" | "cat"
        | "rg" | "grep" | "date" | "uname" | "whoami" => true,
        "diskutil" => words.get(1).is_some_and(|word| *word == "list"),
        "git" => matches!(
            words.get(1).copied(),
            Some("status" | "diff" | "log" | "show")
        ),
        _ => false,
    }
}

fn next_call_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    format!(
        "tool-{now}-{}",
        NEXT_CALL_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn chrono_like_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    format!("UTC unix seconds: {seconds}")
}

fn emit_event(app: &AppHandle, event: ToolEvent) {
    // The shared envelope publishes this on the renderer's `jarvis-event`
    // channel and mirrors it to the localhost bridge.
    if let Ok(payload) = serde_json::to_value(event) {
        crate::events::emit(app, "tool", payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn resolve_path_stays_inside_workspace() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("jarvis-tools-{}-{stamp}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        let safe = resolve_path(&root, "notes/a.md", false).unwrap();
        assert!(safe.starts_with(root.canonicalize().unwrap()));
        assert!(resolve_path(&root, "../../outside.txt", false).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dangerous_commands_require_full_access() {
        assert!(is_dangerous("rm -rf /tmp/example"));
        assert!(is_dangerous("sudo reboot"));
        assert!(is_dangerous("echo done; rm -rf /tmp/example"));
        assert!(is_dangerous("printf '%s' \"$(rm -rf /tmp/example)\""));
        assert!(is_dangerous("cat input.txt | rm -f output.txt"));
        assert!(is_dangerous("echo secret > /tmp/output"));
        assert!(is_dangerous("/bin/rm -rf /tmp/example"));
        assert!(is_dangerous("env SAFE=1 /usr/bin/curl https://example.com"));
        assert!(is_dangerous("python3 -c 'print(1)'"));
        assert!(is_dangerous("node --eval 'console.log(1)'"));
        assert!(!is_dangerous("git status"));
    }

    #[test]
    fn restricted_commands_use_a_small_read_only_allowlist() {
        assert!(restricted_command_allowed("df -h /"));
        assert!(restricted_command_allowed("git status --short"));
        assert!(restricted_command_allowed("diskutil list"));
        assert!(!restricted_command_allowed("python -c 'print(1)'"));
        assert!(!restricted_command_allowed("git reset --hard"));
        assert!(!restricted_command_allowed("echo ok; df -h"));
    }

    #[test]
    fn output_is_bounded() {
        let output = truncate("x".repeat(MAX_OUTPUT + 100));
        assert!(output.chars().count() <= MAX_OUTPUT + 32);
        assert!(output.contains("output truncated"));
    }

    #[test]
    fn search_reads_text_files_without_executing_content() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "jarvis-tools-search-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&root);
        let path = root.join("note.md");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "Jarvis local knowledge").unwrap();
        let result =
            search_files(&root, &serde_json::json!({"query": "knowledge"}), false).unwrap();
        assert!(result.contains("Jarvis local knowledge"));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn file_traversal_skips_symlinked_files_and_directories() {
        use std::os::unix::fs::symlink;

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "jarvis-tools-symlink-{}-{stamp}",
            std::process::id()
        ));
        let outside = root.with_extension("outside");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("nested/real.md"), "inside").unwrap();
        fs::write(outside.join("secret.md"), "must not leak").unwrap();
        symlink(outside.join("secret.md"), root.join("file-link.md")).unwrap();
        symlink(&outside, root.join("dir-link")).unwrap();

        let mut files = Vec::new();
        collect_files(&root, &mut files, MAX_LIST_RESULTS);
        assert!(files.iter().any(|path| path.ends_with("nested/real.md")));
        assert!(!files.iter().any(|path| path.ends_with("file-link.md")));
        assert!(!files.iter().any(|path| path.ends_with("secret.md")));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
}
