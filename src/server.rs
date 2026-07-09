use std::{env, sync::Arc};

use codex_app_server_sdk::api::{Codex, SandboxMode, ThreadOptions, TurnOptions};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    service::{NotificationContext, RequestContext},
    tool, tool_handler, tool_router,
};
use serde::Serialize;
use serde_json::json;
use tokio::sync::RwLock;

use crate::{
    approval::{self, ApprovalPolicy},
    resources::{decode_project_id, parse_resource_uri, project_uri, ResourceKind},
    state::{AppState, ThreadLifecycle, ThreadRecord},
    sync::{self, ensure_thread_known, merge_thread_resource, read_remote_thread},
};

type McpPeer = rmcp::service::Peer<RoleServer>;

#[derive(Clone)]
pub struct CodexMcpServer {
    codex: Arc<Codex>,
    state: Arc<AppState>,
    peer: Arc<RwLock<Option<McpPeer>>>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct StartParams {
    /// Full business description / initial user prompt for the new thread.
    description: String,
    /// Working directory (project root). Defaults to the server process cwd.
    #[serde(default)]
    cwd: Option<String>,
    /// When true, wait for the turn to finish before returning. When false, return thread id immediately and notify on completion.
    #[serde(default = "default_block")]
    block: bool,
    /// Optional Codex model override.
    #[serde(default)]
    model: Option<String>,
    /// Sandbox mode: read-only, workspace-write, danger-full-access.
    #[serde(default)]
    sandbox: Option<String>,
}

fn default_block() -> bool {
    true
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ReplyParams {
    thread_id: String,
    prompt: String,
    #[serde(default = "default_block")]
    block: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ProcessParams {
    thread_id: String,
}

#[derive(Debug, Serialize)]
struct StartResult {
    thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    blocked: bool,
}

#[derive(Debug, Serialize)]
struct ReplyResult {
    thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    blocked: bool,
}

#[derive(Debug, Serialize)]
struct ThreadCompletedNotification {
    thread_id: String,
    project_id: String,
    status: ThreadLifecycle,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl CodexMcpServer {
    pub fn new(codex: Codex) -> Self {
        Self {
            codex: Arc::new(codex),
            state: Arc::new(AppState::new()),
            peer: Arc::new(RwLock::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    pub async fn bootstrap(&self) -> Result<(), McpError> {
        sync::sync_threads_from_codex(&self.codex, &self.state)
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        approval::register_approval_handlers(
            &self.codex,
            Arc::clone(&self.state),
            Arc::clone(&self.peer),
            ApprovalPolicy::from_env(),
        )
        .await;

        Ok(())
    }

    fn resolve_cwd(cwd: Option<String>) -> Result<String, McpError> {
        cwd.or_else(|| env::current_dir().ok().map(|p| p.display().to_string()))
            .ok_or_else(|| {
                McpError::invalid_params("cwd is required and could not be inferred", None)
            })
    }

    fn build_thread_options(
        cwd: &str,
        model: Option<String>,
        sandbox: Option<String>,
    ) -> ThreadOptions {
        let mut builder = ThreadOptions::builder().working_directory(cwd);
        if let Some(model) = model {
            builder = builder.model(model);
        }
        if let Some(mode) = sandbox {
            if let Some(parsed) = parse_sandbox_mode(&mode) {
                builder = builder.sandbox_mode(parsed);
            }
        }
        builder.build()
    }

    async fn track_streamed_turn(
        &self,
        thread_id: &str,
        mut streamed: codex_app_server_sdk::api::StreamedTurn,
    ) -> Result<String, String> {
        let mut final_response = None;

        while let Some(event) = streamed.next_event().await {
            match event {
                Ok(thread_event) => {
                    if let codex_app_server_sdk::api::ThreadEvent::ItemCompleted { item } =
                        &thread_event
                    {
                        if let codex_app_server_sdk::api::ThreadItem::AgentMessage(message) = item
                        {
                            final_response = Some(message.text.clone());
                        }
                    }
                    self.state
                        .update_thread(thread_id, |record| record.push_event(&thread_event))
                        .await;
                    if matches!(
                        thread_event,
                        codex_app_server_sdk::api::ThreadEvent::TurnCompleted { .. }
                            | codex_app_server_sdk::api::ThreadEvent::TurnFailed { .. }
                            | codex_app_server_sdk::api::ThreadEvent::Error { .. }
                    ) {
                        break;
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    self.state
                        .update_thread(thread_id, |record| {
                            record.status = ThreadLifecycle::Failed;
                            record.error = Some(message.clone());
                        })
                        .await;
                    return Err(message);
                }
            }
        }

        Ok(final_response.unwrap_or_default())
    }

    async fn run_turn(
        &self,
        thread_id: &str,
        prompt: String,
        peer: Option<McpPeer>,
    ) -> Result<String, String> {
        let codex = Arc::clone(&self.codex);
        let thread_id = thread_id.to_string();

        let result = async {
            let mut thread = codex.resume_thread_by_id(&thread_id, ThreadOptions::default());

            let streamed = thread
                .run_streamed(prompt, TurnOptions::default())
                .await
                .map_err(|error| error.to_string())?;

            let response = self.track_streamed_turn(&thread_id, streamed).await?;

            self.state
                .update_thread(&thread_id, |record| {
                    record.status = ThreadLifecycle::Completed;
                    if !response.is_empty() {
                        record.final_response = Some(response.clone());
                    }
                })
                .await;

            Ok(response)
        }
        .await;

        if let Some(peer) = peer {
            self.notify_thread_completed(&peer, &thread_id).await;
        }

        result
    }

    async fn notify_thread_completed(&self, peer: &McpPeer, thread_id: &str) {
        let Some(record) = self.state.get_thread(thread_id).await else {
            return;
        };

        let payload = ThreadCompletedNotification {
            thread_id: record.thread_id.clone(),
            project_id: record.project_id.clone(),
            status: record.status,
            content: record.final_response.clone(),
            error: record.error.clone(),
        };

        let _ = peer
            .send_notification(ServerNotification::CustomNotification(
                CustomNotification::new(
                    "notifications/codex/thread/completed",
                    serde_json::to_value(payload).ok(),
                ),
            ))
            .await;

        let _ = peer
            .notify_resource_updated(ResourceUpdatedNotificationParam::new(record.thread_uri()))
            .await;

        let _ = peer
            .notify_resource_updated(ResourceUpdatedNotificationParam::new(record.project_uri()))
            .await;
    }

    async fn start_thread_internal(
        &self,
        description: String,
        cwd: String,
        model: Option<String>,
        sandbox: Option<String>,
    ) -> Result<(String, String), McpError> {
        let thread_options = Self::build_thread_options(&cwd, model, sandbox);
        let mut thread = self.codex.start_thread(thread_options);
        let thread_id = thread
            .id()
            .ok_or_else(|| McpError::internal_error("thread id missing after start", None))?
            .to_string();

        let record = ThreadRecord::new(thread_id.clone(), cwd, description.clone());
        self.state.upsert_thread(record).await;

        let turn = thread
            .run(description, TurnOptions::default())
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        let effective_id = thread.id().unwrap_or(&thread_id).to_string();
        self.state
            .update_thread(&effective_id, |record| {
                record.status = ThreadLifecycle::Completed;
                record.final_response = Some(turn.final_response.clone());
                record.process.push(crate::state::ProcessStep {
                    kind: "turn_completed".into(),
                    summary: "Initial turn completed".into(),
                    at: 0,
                });
            })
            .await;

        Ok((effective_id, turn.final_response))
    }

    async fn start_thread_nonblocking(
        &self,
        description: String,
        cwd: String,
        model: Option<String>,
        sandbox: Option<String>,
        peer: McpPeer,
    ) -> Result<String, McpError> {
        let thread_options = Self::build_thread_options(&cwd, model, sandbox);
        let mut thread = self.codex.start_thread(thread_options);
        let thread_id = thread
            .id()
            .ok_or_else(|| McpError::internal_error("thread id missing after start", None))?
            .to_string();

        let record = ThreadRecord::new(thread_id.clone(), cwd, description.clone());
        self.state.upsert_thread(record).await;

        let server = self.clone();
        let thread_id_for_task = thread_id.clone();
        tokio::spawn(async move {
            let result = async {
                let streamed = thread
                    .run_streamed(description, TurnOptions::default())
                    .await
                    .map_err(|error| error.to_string())?;
                server
                    .track_streamed_turn(&thread_id_for_task, streamed)
                    .await
            }
            .await;

            if let Err(message) = result {
                server
                    .state
                    .update_thread(&thread_id_for_task, |record| {
                        record.status = ThreadLifecycle::Failed;
                        record.error = Some(message);
                    })
                    .await;
            } else {
                server
                    .state
                    .update_thread(&thread_id_for_task, |record| {
                        record.status = ThreadLifecycle::Completed;
                    })
                    .await;
            }

            server
                .notify_thread_completed(&peer, &thread_id_for_task)
                .await;
        });

        Ok(thread_id)
    }
}

#[tool_router]
impl CodexMcpServer {
    #[tool(description = "Create a new Codex thread with a full business description. Supports blocking and non-blocking modes.")]
    async fn start(
        &self,
        Parameters(params): Parameters<StartParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let cwd = Self::resolve_cwd(params.cwd)?;
        let block = params.block;

        if block {
            let (thread_id, content) = self
                .start_thread_internal(
                    params.description,
                    cwd,
                    params.model,
                    params.sandbox,
                )
                .await?;
            let payload = StartResult {
                thread_id,
                content: Some(content),
                blocked: true,
            };
            return tool_json_result(&payload);
        }

        let peer = ctx.peer.clone();
        let thread_id = self
            .start_thread_nonblocking(
                params.description,
                cwd,
                params.model,
                params.sandbox,
                peer,
            )
            .await?;
        let payload = StartResult {
            thread_id,
            content: None,
            blocked: false,
        };
        tool_json_result(&payload)
    }

    #[tool(description = "Reply to an existing Codex thread.")]
    async fn reply(
        &self,
        Parameters(params): Parameters<ReplyParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        ensure_thread_known(&self.codex, &self.state, &params.thread_id)
            .await
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;

        self.state
            .update_thread(&params.thread_id, |record| {
                record.status = ThreadLifecycle::Running;
                record.process.push(crate::state::ProcessStep {
                    kind: "user_reply".into(),
                    summary: truncate(&params.prompt, 120),
                    at: 0,
                });
            })
            .await;

        if params.block {
            let content = self
                .run_turn(&params.thread_id, params.prompt, None)
                .await
                .map_err(|error| McpError::internal_error(error, None))?;
            let payload = ReplyResult {
                thread_id: params.thread_id,
                content: Some(content),
                blocked: true,
            };
            return tool_json_result(&payload);
        }

        let peer = ctx.peer.clone();
        let server = self.clone();
        let thread_id = params.thread_id.clone();
        let prompt = params.prompt;
        tokio::spawn(async move {
            let _ = server.run_turn(&thread_id, prompt, Some(peer)).await;
        });

        let payload = ReplyResult {
            thread_id: params.thread_id,
            content: None,
            blocked: false,
        };
        tool_json_result(&payload)
    }

    #[tool(description = "Inspect the current execution flow for a thread.")]
    async fn process(
        &self,
        Parameters(params): Parameters<ProcessParams>,
    ) -> Result<CallToolResult, McpError> {
        let record = if let Some(record) = self.state.get_thread(&params.thread_id).await {
            record
        } else {
            ensure_thread_known(&self.codex, &self.state, &params.thread_id)
                .await
                .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
            self.state
                .get_thread(&params.thread_id)
                .await
                .ok_or_else(|| {
                    McpError::invalid_params(
                        "unknown thread_id",
                        Some(json!({ "thread_id": params.thread_id })),
                    )
                })?
        };
        tool_json_result(&record)
    }
}

#[tool_handler(
    name = "codex-mcp-server",
    version = "0.1.0",
    instructions = "Codex MCP server exposing project/thread resources and start/reply/process tools backed by codex app-server."
)]
impl ServerHandler for CodexMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "codex-mcp-server",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Use resources codex://project/{id} and codex://thread/{id}. Thread resources merge local execution state with codex thread/read payloads. \
             Tools: start, reply, process. Non-blocking start/reply emits notifications/codex/thread/completed. \
             Approval requests emit notifications/codex/approval/request; policy via CODEX_MCP_APPROVAL_POLICY=approve|session|deny.",
        )
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        *self.peer.write().await = Some(context.peer.clone());
        Ok(self.get_info())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        *self.peer.write().await = Some(context.peer.clone());

        if let Err(error) = sync::sync_threads_from_codex(&self.codex, &self.state).await {
            tracing::warn!(?error, "failed to refresh threads after mcp initialize");
            return;
        }

        let _ = context
            .peer
            .notify_resource_list_changed()
            .await;
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut resources = Vec::new();
        for project in self.state.list_projects().await {
            resources.push(
                Resource::new(project_uri(&project.project_id), project.cwd.clone())
                    .with_description("Codex project (grouped by working directory)"),
            );
        }
        for thread in self.state.list_threads().await {
            resources.push(
                Resource::new(thread.thread_uri(), thread.description.clone())
                    .with_description(format!("Codex thread ({:?})", thread.status)),
            );
        }

        Ok(ListResourcesResult {
            resources,
            ..Default::default()
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![
                ResourceTemplate::new(
                    "codex://project/{project_id}",
                    "Codex project by encoded cwd",
                )
                .with_description("Project id is URL-safe base64 of the absolute cwd"),
                ResourceTemplate::new("codex://thread/{thread_id}", "Codex thread by id"),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let body = match parse_resource_uri(&request.uri) {
            ResourceKind::Project(project_id) => {
                let project = self.state.get_project(&project_id).await;
                let threads = if let Some(project) = &project {
                    let mut items = Vec::new();
                    for thread_id in &project.thread_ids {
                        if let Some(thread) = self.state.get_thread(thread_id).await {
                            items.push(json!({
                                "thread_id": thread.thread_id,
                                "status": thread.status,
                                "description": thread.description,
                            }));
                        }
                    }
                    items
                } else {
                    Vec::new()
                };

                json!({
                    "project_id": project_id,
                    "cwd": project.as_ref().map(|p| p.cwd.clone()).or_else(|| decode_project_id(&project_id)),
                    "threads": threads,
                })
            }
            ResourceKind::Thread(thread_id) => {
                let local = self.state.get_thread(&thread_id).await;
                let remote = read_remote_thread(&self.codex, &thread_id, true)
                    .await
                    .map_err(|error| {
                        if local.is_some() {
                            McpError::internal_error(
                                format!("failed to read remote thread: {error}"),
                                None,
                            )
                        } else {
                            McpError::resource_not_found(
                                error.to_string(),
                                Some(json!({ "thread_id": thread_id })),
                            )
                        }
                    })?;
                merge_thread_resource(local, remote)
            }
            ResourceKind::Unknown => {
                return Err(McpError::resource_not_found(
                    "unsupported resource uri",
                    Some(json!({ "uri": request.uri })),
                ));
            }
        };

        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()),
            request.uri,
        )]))
    }
}

fn parse_sandbox_mode(value: &str) -> Option<SandboxMode> {
    match value.to_ascii_lowercase().as_str() {
        "read-only" | "read_only" | "readonly" => Some(SandboxMode::ReadOnly),
        "workspace-write" | "workspace_write" | "workspacewrite" => {
            Some(SandboxMode::WorkspaceWrite)
        }
        "danger-full-access" | "danger_full_access" | "dangerfullaccess" => {
            Some(SandboxMode::DangerFullAccess)
        }
        _ => None,
    }
}

fn tool_json_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}
