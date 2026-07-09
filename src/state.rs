//! In-memory thread and execution-flow state tracked by the MCP server.

use std::collections::HashMap;

use codex_app_server_sdk::api::{ThreadError, ThreadEvent, ThreadItem, UserMessageContentItem};
use serde::Serialize;
use tokio::sync::RwLock;

use crate::resources::{encode_project_id, project_uri, thread_uri};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadLifecycle {
    Starting,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessStep {
    pub kind: String,
    pub summary: String,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadRecord {
    pub thread_id: String,
    pub project_id: String,
    pub cwd: String,
    pub description: String,
    pub status: ThreadLifecycle,
    pub process: Vec<ProcessStep>,
    pub final_response: Option<String>,
    pub error: Option<String>,
}

impl ThreadRecord {
    pub fn new(thread_id: String, cwd: String, description: String) -> Self {
        let project_id = encode_project_id(&cwd);
        Self {
            thread_id,
            project_id,
            cwd,
            description,
            status: ThreadLifecycle::Starting,
            process: Vec::new(),
            final_response: None,
            error: None,
        }
    }

    pub fn project_uri(&self) -> String {
        project_uri(&self.project_id)
    }

    pub fn thread_uri(&self) -> String {
        thread_uri(&self.thread_id)
    }

    pub fn push_event(&mut self, event: &ThreadEvent) {
        let now = chrono_now();
        match event {
            ThreadEvent::ThreadStarted { thread_id } => {
                self.thread_id.clone_from(thread_id);
                self.process.push(ProcessStep {
                    kind: "thread_started".into(),
                    summary: format!("Thread {thread_id} started"),
                    at: now,
                });
            }
            ThreadEvent::TurnStarted => {
                self.status = ThreadLifecycle::Running;
                self.process.push(ProcessStep {
                    kind: "turn_started".into(),
                    summary: "Turn started".into(),
                    at: now,
                });
            }
            ThreadEvent::ItemStarted { item } => {
                self.process.push(ProcessStep {
                    kind: format!("item_started:{}", item_kind(item)),
                    summary: item_summary(item),
                    at: now,
                });
            }
            ThreadEvent::ItemUpdated { item } => {
                self.process.push(ProcessStep {
                    kind: format!("item_updated:{}", item_kind(item)),
                    summary: item_summary(item),
                    at: now,
                });
            }
            ThreadEvent::ItemCompleted { item } => {
                self.process.push(ProcessStep {
                    kind: format!("item_completed:{}", item_kind(item)),
                    summary: item_summary(item),
                    at: now,
                });
            }
            ThreadEvent::TurnCompleted { .. } => {
                self.status = ThreadLifecycle::Completed;
                self.process.push(ProcessStep {
                    kind: "turn_completed".into(),
                    summary: "Turn completed".into(),
                    at: now,
                });
            }
            ThreadEvent::TurnFailed { error } => {
                self.status = ThreadLifecycle::Failed;
                self.error = Some(error.message.clone());
                self.process.push(ProcessStep {
                    kind: "turn_failed".into(),
                    summary: error.message.clone(),
                    at: now,
                });
            }
            ThreadEvent::Error { message } => {
                self.status = ThreadLifecycle::Failed;
                self.error = Some(message.clone());
                self.process.push(ProcessStep {
                    kind: "error".into(),
                    summary: message.clone(),
                    at: now,
                });
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectRecord {
    pub project_id: String,
    pub cwd: String,
    pub thread_ids: Vec<String>,
}

#[derive(Default)]
pub struct AppState {
    inner: RwLock<StateInner>,
}

#[derive(Default)]
struct StateInner {
    threads: HashMap<String, ThreadRecord>,
    projects: HashMap<String, ProjectRecord>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn import_thread_if_absent(&self, record: ThreadRecord) {
        let mut inner = self.inner.write().await;
        if inner.threads.contains_key(&record.thread_id) {
            return;
        }
        let project = inner
            .projects
            .entry(record.project_id.clone())
            .or_insert_with(|| ProjectRecord {
                project_id: record.project_id.clone(),
                cwd: record.cwd.clone(),
                thread_ids: Vec::new(),
            });
        if !project.thread_ids.contains(&record.thread_id) {
            project.thread_ids.push(record.thread_id.clone());
        }
        inner.threads.insert(record.thread_id.clone(), record);
    }

    pub async fn upsert_thread(&self, record: ThreadRecord) {
        let mut inner = self.inner.write().await;
        let project = inner
            .projects
            .entry(record.project_id.clone())
            .or_insert_with(|| ProjectRecord {
                project_id: record.project_id.clone(),
                cwd: record.cwd.clone(),
                thread_ids: Vec::new(),
            });
        if !project.thread_ids.contains(&record.thread_id) {
            project.thread_ids.push(record.thread_id.clone());
        }
        inner.threads.insert(record.thread_id.clone(), record);
    }

    pub async fn update_thread<F>(&self, thread_id: &str, update: F)
    where
        F: FnOnce(&mut ThreadRecord),
    {
        let mut inner = self.inner.write().await;
        if let Some(record) = inner.threads.get_mut(thread_id) {
            update(record);
        }
    }

    pub async fn get_thread(&self, thread_id: &str) -> Option<ThreadRecord> {
        self.inner.read().await.threads.get(thread_id).cloned()
    }

    pub async fn list_projects(&self) -> Vec<ProjectRecord> {
        self.inner.read().await.projects.values().cloned().collect()
    }

    pub async fn get_project(&self, project_id: &str) -> Option<ProjectRecord> {
        self.inner.read().await.projects.get(project_id).cloned()
    }

    pub async fn list_threads(&self) -> Vec<ThreadRecord> {
        self.inner.read().await.threads.values().cloned().collect()
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn item_kind(item: &ThreadItem) -> &'static str {
    match item {
        ThreadItem::AgentMessage(_) => "agent_message",
        ThreadItem::UserMessage(_) => "user_message",
        ThreadItem::CommandExecution(_) => "command_execution",
        ThreadItem::FileChange(_) => "file_change",
        ThreadItem::McpToolCall(_) => "mcp_tool_call",
        ThreadItem::Reasoning(_) => "reasoning",
        ThreadItem::Plan(_) => "plan",
        ThreadItem::WebSearch(_) => "web_search",
        ThreadItem::Error(_) => "error",
        _ => "other",
    }
}

fn item_summary(item: &ThreadItem) -> String {
    match item {
        ThreadItem::AgentMessage(item) => truncate(&item.text, 120),
        ThreadItem::UserMessage(item) => item
            .content
            .iter()
            .map(user_message_part)
            .collect::<Vec<_>>()
            .join(", "),
        ThreadItem::CommandExecution(item) => item.command.clone(),
        ThreadItem::FileChange(item) => format!("{} file(s)", item.changes.len()),
        ThreadItem::McpToolCall(item) => format!("{}::{}", item.server, item.tool),
        ThreadItem::Reasoning(item) => truncate(&item.text, 120),
        ThreadItem::Plan(item) => truncate(&item.text, 120),
        ThreadItem::WebSearch(item) => item.query.clone(),
        ThreadItem::Error(item) => item.message.clone(),
        other => format!("{other:?}"),
    }
}

fn user_message_part(item: &UserMessageContentItem) -> String {
    match item {
        UserMessageContentItem::Text { text } => text.clone(),
        UserMessageContentItem::Image { url } => format!("image:{url}"),
        UserMessageContentItem::LocalImage { path } => format!("local_image:{path}"),
        UserMessageContentItem::Unknown(value) => value.to_string(),
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}

#[allow(dead_code)]
fn format_thread_error(error: &ThreadError) -> String {
    error.message.clone()
}
