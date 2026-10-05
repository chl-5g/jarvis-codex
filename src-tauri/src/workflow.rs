//! File-backed workflows for the local Jarvis/OpenAgentic bridge.
//!
//! A workflow is deliberately small and auditable: it is a sequence of local
//! tool or knowledge steps persisted as JSON.  Steps marked `requiresApproval`
//! stop before execution unless the caller explicitly supplies approval.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

const MAX_WORKFLOWS: usize = 200;
const MAX_STEPS: usize = 64;
const MAX_OUTPUT: usize = 12_000;
static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub requires_approval: bool,
    /// Optional static condition. `false` skips this step and emits an event.
    #[serde(default)]
    pub condition: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunResult {
    pub run_id: String,
    pub workflow_id: String,
    pub success: bool,
    pub output: String,
    pub completed_steps: usize,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct WorkflowFile {
    version: u32,
    workflows: Vec<WorkflowDefinition>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowEvent {
    kind: &'static str,
    phase: &'static str,
    run_id: String,
    workflow_id: String,
    step_id: Option<String>,
    message: Option<String>,
}

fn default_true() -> bool {
    true
}

pub fn list() -> Vec<WorkflowDefinition> {
    load().workflows
}

pub fn save(mut workflow: WorkflowDefinition) -> Result<WorkflowDefinition, String> {
    validate(&workflow)?;
    workflow.id = safe_id(&workflow.id)?;
    workflow.name = workflow.name.trim().to_owned();
    let mut file = load();
    if let Some(existing) = file
        .workflows
        .iter_mut()
        .find(|item| item.id == workflow.id)
    {
        *existing = workflow.clone();
    } else {
        if file.workflows.len() >= MAX_WORKFLOWS {
            return Err("工作流数量已达到上限".to_owned());
        }
        file.workflows.push(workflow.clone());
    }
    persist(&file)?;
    Ok(workflow)
}

pub async fn run(
    app: AppHandle,
    workspace: &Path,
    workflow_id: &str,
    full_access: bool,
    approved: bool,
) -> Result<WorkflowRunResult, String> {
    let workflow = load()
        .workflows
        .into_iter()
        .find(|item| item.id == workflow_id)
        .ok_or_else(|| format!("找不到工作流：{workflow_id}"))?;
    run_definition(app, workspace, &workflow, full_access, approved).await
}

/// Execute one already loaded workflow. Tasks use this function so a task run
/// cannot observe a different definition midway through its lifecycle.
pub async fn run_definition(
    app: AppHandle,
    workspace: &Path,
    workflow: &WorkflowDefinition,
    full_access: bool,
    approved: bool,
) -> Result<WorkflowRunResult, String> {
    validate(workflow)?;
    if !workflow.enabled {
        return Err("工作流已禁用".to_owned());
    }
    let run_id = next_run_id();
    emit(&app, &run_id, workflow, "started", None, Some("工作流开始"));
    let mut output = Vec::new();
    for step in &workflow.steps {
        if step.condition == Some(false) {
            emit(
                &app,
                &run_id,
                workflow,
                "step-skipped",
                Some(step),
                Some("条件为 false"),
            );
            continue;
        }
        if (step.requires_approval || step.kind == "approval") && !approved {
            let message = format!("步骤 {} 需要明确批准", step.id);
            emit(
                &app,
                &run_id,
                workflow,
                "approval-required",
                Some(step),
                Some(&message),
            );
            return Ok(WorkflowRunResult {
                run_id,
                workflow_id: workflow.id.clone(),
                success: false,
                output: output.join("\n"),
                completed_steps: output.len(),
                error: Some(message),
            });
        }
        emit(&app, &run_id, workflow, "step-started", Some(step), None);
        let result = match step.kind.as_str() {
            "tool" => {
                let tool_name = step
                    .tool_name
                    .as_deref()
                    .ok_or_else(|| format!("步骤 {} 缺少 toolName", step.id))?;
                let result = crate::tools::execute(
                    app.clone(),
                    workspace,
                    tool_name,
                    step.args.clone(),
                    full_access,
                )
                .await;
                if result.success {
                    Ok(result.output)
                } else {
                    Err(result.error.unwrap_or_else(|| "工具执行失败".to_owned()))
                }
            }
            "knowledge_search" => {
                let query = step
                    .args
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if query.trim().is_empty() {
                    Err("knowledge_search 需要 query".to_owned())
                } else {
                    crate::knowledge::KnowledgeStore::default()
                        .search(
                            query,
                            step.args.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize,
                        )
                        .map(|items| serde_json::to_string(&items).unwrap_or_default())
                }
            }
            "approval" => Ok("已批准".to_owned()),
            "memory_save_episode" => {
                let title = step
                    .args
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Jarvis workflow");
                let summary = step
                    .args
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let tags = step
                    .args
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                crate::memory::MemoryStore::default().save_episode(title, summary, &tags)
            }
            other => Err(format!("不支持的工作流步骤类型：{other}")),
        };
        match result {
            Ok(value) => {
                output.push(truncate(value));
                emit(&app, &run_id, workflow, "step-completed", Some(step), None);
            }
            Err(error) => {
                emit(&app, &run_id, workflow, "error", Some(step), Some(&error));
                return Ok(WorkflowRunResult {
                    run_id,
                    workflow_id: workflow.id.clone(),
                    success: false,
                    output: truncate(output.join("\n")),
                    completed_steps: output.len(),
                    error: Some(error),
                });
            }
        }
    }
    emit(
        &app,
        &run_id,
        workflow,
        "completed",
        None,
        Some("工作流完成"),
    );
    Ok(WorkflowRunResult {
        run_id,
        workflow_id: workflow.id.clone(),
        success: true,
        output: truncate(output.join("\n")),
        completed_steps: output.len(),
        error: None,
    })
}

fn validate(workflow: &WorkflowDefinition) -> Result<(), String> {
    if workflow.id.trim().is_empty() || workflow.name.trim().is_empty() {
        return Err("工作流 id 和 name 不能为空".to_owned());
    }
    safe_id(&workflow.id)?;
    if workflow.steps.is_empty() || workflow.steps.len() > MAX_STEPS {
        return Err(format!("工作流步骤数量必须为 1-{MAX_STEPS}"));
    }
    for step in &workflow.steps {
        safe_id(&step.id)?;
        match step.kind.as_str() {
            "tool" | "knowledge_search" | "memory_save_episode" | "approval" => {}
            other => return Err(format!("不支持的工作流步骤类型：{other}")),
        }
        if step.kind == "tool"
            && step
                .tool_name
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(format!("步骤 {} 缺少 toolName", step.id));
        }
    }
    Ok(())
}

fn safe_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 96
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("id 只能包含字母、数字、-、_、. 且长度不超过 96".to_owned());
    }
    Ok(value.to_owned())
}

fn load() -> WorkflowFile {
    let path = file_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(WorkflowFile {
            version: 1,
            workflows: Vec::new(),
        })
}

fn persist(file: &WorkflowFile) -> Result<(), String> {
    let path = file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建工作流目录失败：{error}"))?;
    }
    let text = serde_json::to_string_pretty(file).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| format!("保存工作流失败：{error}"))
}

fn file_path() -> PathBuf {
    env::var_os("JARVIS_WORKFLOW_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME").map(|home| PathBuf::from(home).join(".jarvis/workflows.json"))
        })
        .unwrap_or_else(|| PathBuf::from(".jarvis/workflows.json"))
}

fn next_run_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    format!(
        "workflow-{now}-{}",
        NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn truncate(value: String) -> String {
    value.chars().take(MAX_OUTPUT).collect()
}

fn emit(
    app: &AppHandle,
    run_id: &str,
    workflow: &WorkflowDefinition,
    phase: &'static str,
    step: Option<&WorkflowStep>,
    message: Option<&str>,
) {
    let event = WorkflowEvent {
        kind: "workflow",
        phase,
        run_id: run_id.to_owned(),
        workflow_id: workflow.id.clone(),
        step_id: step.map(|item| item.id.clone()),
        message: message.map(str::to_owned),
    };
    let _ = app.emit("jarvis-event", &event);
    crate::bridge::publish(
        "workflow",
        serde_json::to_value(event).unwrap_or(Value::Null),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validation_rejects_path_like_ids_and_unknown_steps() {
        let base = WorkflowDefinition {
            id: "ok".into(),
            name: "demo".into(),
            enabled: true,
            steps: vec![WorkflowStep {
                id: "step".into(),
                kind: "tool".into(),
                tool_name: Some("read_file".into()),
                args: json!({"path":"a.md"}),
                requires_approval: false,
                condition: None,
            }],
        };
        assert!(validate(&base).is_ok());
        assert!(validate(&WorkflowDefinition {
            id: "../bad".into(),
            ..base.clone()
        })
        .is_err());
        assert!(validate(&WorkflowDefinition {
            steps: vec![WorkflowStep {
                kind: "exec".into(),
                ..base.steps[0].clone()
            }],
            ..base
        })
        .is_err());
    }
}
