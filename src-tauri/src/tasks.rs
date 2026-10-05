//! Durable, file-backed task scheduler for local workflows.

use crate::workflow::{self, WorkflowRunResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

const MAX_TASKS: usize = 500;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub name: String,
    pub due_at_unix: u64,
    pub workflow_id: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default = "scheduled")]
    pub status: String,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TaskFile {
    version: u32,
    tasks: Vec<TaskRecord>,
}

fn scheduled() -> String {
    "scheduled".to_owned()
}

pub fn list() -> Vec<TaskRecord> {
    load().tasks
}

pub fn schedule(mut task: TaskRecord) -> Result<TaskRecord, String> {
    validate(&task)?;
    task.status = "scheduled".to_owned();
    task.last_error = None;
    let mut file = load();
    if let Some(existing) = file.tasks.iter_mut().find(|item| item.id == task.id) {
        *existing = task.clone();
    } else {
        if file.tasks.len() >= MAX_TASKS {
            return Err("任务数量已达到上限".to_owned());
        }
        file.tasks.push(task.clone());
    }
    persist(&file)?;
    Ok(task)
}

pub fn cancel(id: &str) -> Result<(), String> {
    let mut file = load();
    let task = file
        .tasks
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("找不到任务：{id}"))?;
    if task.status == "completed" {
        return Err("已完成任务不能取消".to_owned());
    }
    task.status = "cancelled".to_owned();
    persist(&file)
}

/// Start the bounded five-second local scheduler. It has no network listener,
/// uses the workspace selected at app startup, and treats all scheduled work
/// as non-full-access unless the explicit command path supplies full access.
pub fn start_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let state = app.state::<crate::AppState>();
            let Ok(runtime) = crate::runtime(&state).await else {
                continue;
            };
            let workspace = PathBuf::from(runtime.workspace.clone());
            let full_access = runtime.permission_mode == crate::PermissionMode::Full;
            let _ = run_due(app.clone(), &workspace, full_access).await;
        }
    });
}

/// Resume a cancelled or failed task without changing its durable due time.
pub fn resume(id: &str) -> Result<TaskRecord, String> {
    let mut file = load();
    let task = file
        .tasks
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("找不到任务：{id}"))?;
    if task.status == "completed" || task.status == "running" {
        return Err("当前任务状态不能恢复".to_owned());
    }
    task.status = "scheduled".to_owned();
    task.last_error = None;
    let value = task.clone();
    persist(&file)?;
    Ok(value)
}

pub async fn run_due(
    app: AppHandle,
    workspace: &Path,
    full_access: bool,
) -> Result<Vec<TaskRecord>, String> {
    let now = now();
    let due_ids: Vec<String> = load()
        .tasks
        .into_iter()
        .filter(|task| task.status == "scheduled" && task.due_at_unix <= now)
        .map(|task| task.id)
        .collect();
    let mut completed = Vec::new();
    for id in due_ids {
        let task = mark_running(&id)?;
        emit(&app, "started", &task, None);
        let result = workflow::run(
            app.clone(),
            workspace,
            &task.workflow_id,
            full_access,
            false,
        )
        .await;
        let updated = finish(&id, result);
        emit(
            &app,
            if updated.status == "completed" {
                "completed"
            } else {
                "error"
            },
            &updated,
            updated.last_error.as_deref(),
        );
        completed.push(updated);
    }
    Ok(completed)
}

fn mark_running(id: &str) -> Result<TaskRecord, String> {
    let mut file = load();
    let task = file
        .tasks
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("找不到任务：{id}"))?;
    task.status = "running".to_owned();
    let value = task.clone();
    persist(&file)?;
    Ok(value)
}

fn finish(id: &str, result: Result<WorkflowRunResult, String>) -> TaskRecord {
    let mut file = load();
    let Some(task) = file.tasks.iter_mut().find(|item| item.id == id) else {
        return TaskRecord {
            id: id.to_owned(),
            name: id.to_owned(),
            due_at_unix: now(),
            workflow_id: String::new(),
            payload: Value::Null,
            status: "error".to_owned(),
            last_error: Some("任务记录消失".to_owned()),
            completed_at_unix: None,
        };
    };
    match result {
        Ok(value) if value.success => {
            task.status = "completed".to_owned();
            task.last_error = None;
            task.completed_at_unix = Some(now());
        }
        Ok(value) => {
            task.status = "error".to_owned();
            task.last_error = value.error;
        }
        Err(error) => {
            task.status = "error".to_owned();
            task.last_error = Some(error);
        }
    }
    let value = task.clone();
    let _ = persist(&file);
    value
}

fn validate(task: &TaskRecord) -> Result<(), String> {
    if task.id.trim().is_empty()
        || task.name.trim().is_empty()
        || task.workflow_id.trim().is_empty()
    {
        return Err("任务 id、name、workflowId 不能为空".to_owned());
    }
    if task.id.len() > 96
        || !task
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("任务 id 格式无效".to_owned());
    }
    Ok(())
}

fn file_path() -> std::path::PathBuf {
    env::var_os("JARVIS_TASKS_FILE")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".jarvis/tasks.json"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".jarvis/tasks.json"))
}
fn load() -> TaskFile {
    fs::read_to_string(file_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(TaskFile {
            version: 1,
            tasks: Vec::new(),
        })
}
fn persist(file: &TaskFile) -> Result<(), String> {
    let path = file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(
        path,
        serde_json::to_string_pretty(file).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("保存任务失败：{error}"))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default()
}
fn emit(app: &AppHandle, phase: &'static str, task: &TaskRecord, error: Option<&str>) {
    let event = serde_json::json!({"kind":"task", "phase":phase, "taskId":task.id, "workflowId":task.workflow_id, "message":error});
    let _ = app.emit("jarvis-event", &event);
    crate::bridge::publish("task", event);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_rejects_empty_task_fields() {
        assert!(validate(&TaskRecord {
            id: String::new(),
            name: "n".into(),
            due_at_unix: 0,
            workflow_id: "w".into(),
            payload: Value::Null,
            status: "scheduled".into(),
            last_error: None,
            completed_at_unix: None
        })
        .is_err());
    }
}
