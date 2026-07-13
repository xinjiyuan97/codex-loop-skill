//! In-memory thread and execution-flow state tracked by the MCP server.

use std::collections::HashMap;

use codex_app_server_sdk::api::{ThreadError, ThreadEvent, ThreadItem, UserMessageContentItem};
use schemars::JsonSchema;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::resources::{encode_project_id, project_uri, thread_uri};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadLifecycle {
    /// Thread is being created.
    Starting,
    /// A turn is currently running.
    Running,
    /// Waiting for user approval of a command or file change.
    WaitingApproval,
    /// The latest turn completed successfully.
    Completed,
    /// The latest turn failed.
    Failed,
    /// The thread was interrupted before completion.
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadItemKind {
    AgentMessage,
    UserMessage,
    CommandExecution,
    FileChange,
    McpToolCall,
    Reasoning,
    Plan,
    WebSearch,
    Error,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    CommandExecution,
    ExecCommand,
    ApplyPatch,
    FileChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessStepKind {
    ThreadStarted,
    TurnStarted,
    ItemStarted { item: ThreadItemKind },
    ItemUpdated { item: ThreadItemKind },
    ItemCompleted { item: ThreadItemKind },
    TurnCompleted,
    TurnFailed,
    Error,
    UserReply,
    Approval { kind: ApprovalKind },
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(description = "One step in the local thread execution trace.")]
pub struct ProcessStep {
    #[schemars(description = "Structured step kind")]
    pub kind: ProcessStepKind,
    #[schemars(description = "Human-readable step summary")]
    pub summary: String,
    #[schemars(description = "Unix timestamp in seconds")]
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(description = "Local execution state tracked for a Codex thread.")]
pub struct ThreadRecord {
    #[schemars(description = "Codex thread id")]
    pub thread_id: String,
    #[schemars(description = "Short stable project id derived from cwd")]
    pub project_id: String,
    #[schemars(description = "Absolute working directory for the thread")]
    pub cwd: String,
    #[schemars(description = "Brief task summary for calling-agent tracking and project resource listings")]
    pub description: String,
    #[schemars(description = "Current local lifecycle status")]
    pub status: ThreadLifecycle,
    #[schemars(description = "Ordered local execution trace")]
    pub process: Vec<ProcessStep>,
    #[schemars(description = "Latest final agent response, if available")]
    pub final_response: Option<String>,
    #[schemars(description = "Latest error message, if the thread failed")]
    pub error: Option<String>,
}

/// Splits the execution trace into rounds. Each round begins at
/// [`ProcessStepKind::ThreadStarted`] or [`ProcessStepKind::UserReply`].
pub fn split_process_into_rounds(process: &[ProcessStep]) -> Vec<Vec<ProcessStep>> {
    let mut rounds: Vec<Vec<ProcessStep>> = Vec::new();
    let mut current: Vec<ProcessStep> = Vec::new();

    for step in process {
        let starts_round = matches!(
            step.kind,
            ProcessStepKind::ThreadStarted | ProcessStepKind::UserReply
        );
        if starts_round && !current.is_empty() {
            rounds.push(current);
            current = Vec::new();
        }
        current.push(step.clone());
    }

    if !current.is_empty() {
        rounds.push(current);
    }

    rounds
}

/// Filters process steps based on thread status and a round limit.
///
/// Completed/failed/interrupted threads return only the last round.
/// In-progress threads return the most recent `round_limit` rounds (default 3).
pub fn filter_process_for_response(
    process: &[ProcessStep],
    status: ThreadLifecycle,
    round_limit: u32,
) -> (Vec<ProcessStep>, usize, usize) {
    let rounds = split_process_into_rounds(process);
    let total_rounds = rounds.len();
    if total_rounds == 0 {
        return (Vec::new(), 0, 0);
    }

    let take = match status {
        ThreadLifecycle::Completed | ThreadLifecycle::Failed | ThreadLifecycle::Interrupted => 1,
        ThreadLifecycle::Starting | ThreadLifecycle::Running | ThreadLifecycle::WaitingApproval => {
            round_limit.max(1) as usize
        }
    };

    let start = total_rounds.saturating_sub(take);
    let selected: Vec<ProcessStep> = rounds[start..]
        .iter()
        .flat_map(|round| round.iter().cloned())
        .collect();
    let rounds_included = total_rounds - start;

    (selected, total_rounds, rounds_included)
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(description = "Filtered thread execution trace returned by the process tool.")]
pub struct ProcessResult {
    #[schemars(description = "Codex thread id")]
    pub thread_id: String,
    #[schemars(description = "Short stable project id derived from cwd")]
    pub project_id: String,
    #[schemars(description = "Absolute working directory for the thread")]
    pub cwd: String,
    #[schemars(description = "Brief task summary for calling-agent tracking and project resource listings")]
    pub description: String,
    #[schemars(description = "Current local lifecycle status")]
    pub status: ThreadLifecycle,
    #[schemars(description = "Filtered local execution trace for the selected rounds")]
    pub process: Vec<ProcessStep>,
    #[schemars(description = "Latest final agent response, if available")]
    pub final_response: Option<String>,
    #[schemars(description = "Latest error message, if the thread failed")]
    pub error: Option<String>,
    #[schemars(description = "Total number of execution rounds recorded")]
    pub total_rounds: usize,
    #[schemars(description = "Number of rounds included in this response")]
    pub rounds_included: usize,
}

impl ThreadRecord {
    pub fn into_process_result(self, round_limit: u32) -> ProcessResult {
        let (process, total_rounds, rounds_included) =
            filter_process_for_response(&self.process, self.status, round_limit);

        ProcessResult {
            thread_id: self.thread_id,
            project_id: self.project_id,
            cwd: self.cwd,
            description: self.description,
            status: self.status,
            process,
            final_response: self.final_response,
            error: self.error,
            total_rounds,
            rounds_included,
        }
    }

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
        thread_uri(&self.project_id, &self.thread_id)
    }

    pub fn push_event(&mut self, event: &ThreadEvent) {
        let now = chrono_now();
        match event {
            ThreadEvent::ThreadStarted { thread_id } => {
                self.thread_id.clone_from(thread_id);
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::ThreadStarted,
                    summary: format!("Thread {thread_id} started"),
                    at: now,
                });
            }
            ThreadEvent::TurnStarted => {
                self.status = ThreadLifecycle::Running;
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::TurnStarted,
                    summary: "Turn started".into(),
                    at: now,
                });
            }
            ThreadEvent::ItemStarted { item } => {
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::ItemStarted {
                        item: map_item_kind(item),
                    },
                    summary: item_summary(item),
                    at: now,
                });
            }
            ThreadEvent::ItemUpdated { item } => {
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::ItemUpdated {
                        item: map_item_kind(item),
                    },
                    summary: item_summary(item),
                    at: now,
                });
            }
            ThreadEvent::ItemCompleted { item } => {
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::ItemCompleted {
                        item: map_item_kind(item),
                    },
                    summary: item_summary(item),
                    at: now,
                });
            }
            ThreadEvent::TurnCompleted { .. } => {
                self.status = ThreadLifecycle::Completed;
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::TurnCompleted,
                    summary: "Turn completed".into(),
                    at: now,
                });
            }
            ThreadEvent::TurnFailed { error } => {
                self.status = ThreadLifecycle::Failed;
                self.error = Some(error.message.clone());
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::TurnFailed,
                    summary: error.message.clone(),
                    at: now,
                });
            }
            ThreadEvent::Error { message } => {
                self.status = ThreadLifecycle::Failed;
                self.error = Some(message.clone());
                self.process.push(ProcessStep {
                    kind: ProcessStepKind::Error,
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

    pub async fn remove_thread(&self, thread_id: &str) -> Option<ThreadRecord> {
        let mut inner = self.inner.write().await;
        let record = inner.threads.remove(thread_id)?;
        if let Some(project) = inner.projects.get_mut(&record.project_id) {
            project.thread_ids.retain(|id| id != thread_id);
            if project.thread_ids.is_empty() {
                inner.projects.remove(&record.project_id);
            }
        }
        Some(record)
    }

    pub async fn list_threads_for_project(&self, project_id: &str) -> Vec<ThreadRecord> {
        let inner = self.inner.read().await;
        let Some(project) = inner.projects.get(project_id) else {
            return Vec::new();
        };

        project
            .thread_ids
            .iter()
            .filter_map(|thread_id| inner.threads.get(thread_id).cloned())
            .collect()
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn map_item_kind(item: &ThreadItem) -> ThreadItemKind {
    match item {
        ThreadItem::AgentMessage(_) => ThreadItemKind::AgentMessage,
        ThreadItem::UserMessage(_) => ThreadItemKind::UserMessage,
        ThreadItem::CommandExecution(_) => ThreadItemKind::CommandExecution,
        ThreadItem::FileChange(_) => ThreadItemKind::FileChange,
        ThreadItem::McpToolCall(_) => ThreadItemKind::McpToolCall,
        ThreadItem::Reasoning(_) => ThreadItemKind::Reasoning,
        ThreadItem::Plan(_) => ThreadItemKind::Plan,
        ThreadItem::WebSearch(_) => ThreadItemKind::WebSearch,
        ThreadItem::Error(_) => ThreadItemKind::Error,
        _ => ThreadItemKind::Other,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: ProcessStepKind, summary: &str) -> ProcessStep {
        ProcessStep {
            kind,
            summary: summary.to_string(),
            at: 0,
        }
    }

    fn sample_process() -> Vec<ProcessStep> {
        vec![
            step(ProcessStepKind::ThreadStarted, "thread"),
            step(ProcessStepKind::TurnStarted, "turn 1"),
            step(ProcessStepKind::TurnCompleted, "done 1"),
            step(ProcessStepKind::UserReply, "reply 2"),
            step(ProcessStepKind::TurnStarted, "turn 2"),
            step(ProcessStepKind::TurnCompleted, "done 2"),
            step(ProcessStepKind::UserReply, "reply 3"),
            step(ProcessStepKind::TurnStarted, "turn 3"),
            step(ProcessStepKind::ItemCompleted {
                item: ThreadItemKind::AgentMessage,
            }, "msg 3"),
        ]
    }

    #[test]
    fn split_process_into_rounds_groups_by_thread_start_and_user_reply() {
        let rounds = split_process_into_rounds(&sample_process());
        assert_eq!(rounds.len(), 3);
        assert!(matches!(
            rounds[0].first().map(|step| &step.kind),
            Some(ProcessStepKind::ThreadStarted)
        ));
        assert!(matches!(
            rounds[1].first().map(|step| &step.kind),
            Some(ProcessStepKind::UserReply)
        ));
        assert!(matches!(
            rounds[2].first().map(|step| &step.kind),
            Some(ProcessStepKind::UserReply)
        ));
    }

    #[test]
    fn completed_status_returns_only_last_round() {
        let process = sample_process();
        let (filtered, total_rounds, rounds_included) =
            filter_process_for_response(&process, ThreadLifecycle::Completed, 3);

        assert_eq!(total_rounds, 3);
        assert_eq!(rounds_included, 1);
        assert_eq!(filtered.len(), 3);
        assert!(matches!(
            filtered.first().map(|step| &step.kind),
            Some(ProcessStepKind::UserReply)
        ));
    }

    #[test]
    fn running_status_returns_recent_rounds_with_default_limit() {
        let process = sample_process();
        let (filtered, total_rounds, rounds_included) =
            filter_process_for_response(&process, ThreadLifecycle::Running, 3);

        assert_eq!(total_rounds, 3);
        assert_eq!(rounds_included, 3);
        assert_eq!(filtered.len(), process.len());
    }

    #[test]
    fn running_status_respects_custom_round_limit() {
        let process = sample_process();
        let (filtered, total_rounds, rounds_included) =
            filter_process_for_response(&process, ThreadLifecycle::Running, 2);

        assert_eq!(total_rounds, 3);
        assert_eq!(rounds_included, 2);
        assert_eq!(filtered.len(), 6);
        assert!(matches!(
            filtered.first().map(|step| &step.kind),
            Some(ProcessStepKind::UserReply)
        ));
        assert_eq!(filtered[0].summary, "reply 2");
    }
}
