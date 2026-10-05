use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
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
            description: "Read a UTF-8 text file within the active workspace.",
            requires_full_access: false,
        },
        ToolSpec {
            name: "write_file",
            description: "Create or replace a UTF-8 text file within the active workspace.",
            requires_full_access: false,
        },
        ToolSpec {
            name: "append_file",
            description: "Append UTF-8 text to a file within the active workspace.",
            requires_full_access: false,
        },
        ToolSpec {
            name: "list_files",
            description: "List files below a workspace directory.",
            requires_full_access: false,
        },
        ToolSpec {
            name: "search_files",
            description: "Search text files below a workspace directory.",
            requires_full_access: false,
        },
        ToolSpec {
            name: "run_command",
            description: "Run a shell command from the active workspace with a timeout.",
            requires_full_access: true,
        },
        ToolSpec {
            name: "current_time",
            description: "Return the current UTC time.",
            requires_full_access: false,
        },
    ]
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
    if is_dangerous(command) && !full_access {
        return Err("命令被工具网关拦截：需要完全访问权限".to_owned());
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
    if path.is_file() {
        output.push(path.to_owned());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_files(&entry.path(), output, limit);
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
    let mut words = command.split_whitespace();
    let first = words.next().unwrap_or_default();
    let command = if first == "sudo" {
        words.next().unwrap_or_default()
    } else {
        first
    };
    matches!(
        command,
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
    )
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
    if let Ok(payload) = serde_json::to_value(&event) {
        crate::bridge::publish("tool", payload);
    }
    let _ = app.emit("jarvis-event", event);
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
        assert!(!is_dangerous("git status"));
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
}
