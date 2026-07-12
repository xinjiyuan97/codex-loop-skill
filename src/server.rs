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
    resources::{decode_project_id, parse_resource_uri, project_uri, thread_uri, ResourceKind},
    state::{AppState, ProcessStepKind, ThreadLifecycle, ThreadRecord},
    sync::{self, archive_remote_thread, ensure_thread_known, merge_thread_resource, read_remote_thread},
};

type McpPeer = rmcp::service::Peer<RoleServer>;

#[derive(Clone)]
pub struct CodexMcpServer {
    codex: Arc<Codex>,
    state: Arc<AppState>,
    peer: Arc<RwLock<Option<McpPeer>>>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxModeParam {
    /// Read-only filesystem access.
    #[serde(alias = "read_only", alias = "readonly")]
    #[schemars(description = "Read-only filesystem access")]
    ReadOnly,
    /// Allow writes within the workspace.
    #[serde(alias = "workspace_write", alias = "workspacewrite")]
    #[schemars(description = "Allow writes within the workspace")]
    WorkspaceWrite,
    /// Disable sandbox restrictions.
    #[serde(alias = "danger_full_access", alias = "dangerfullaccess")]
    #[schemars(description = "Disable sandbox restrictions")]
    DangerFullAccess,
}

impl SandboxModeParam {
    fn into_sdk(self) -> SandboxMode {
        match self {
            Self::ReadOnly => SandboxMode::ReadOnly,
            Self::WorkspaceWrite => SandboxMode::WorkspaceWrite,
            Self::DangerFullAccess => SandboxMode::DangerFullAccess,
        }
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[schemars(description = "Arguments for creating a new Codex thread.")]
struct StartParams {
    /// Brief summary for the calling agent to identify and track this thread.
    #[schemars(description = "Brief task summary for the calling agent to judge scope and track the thread in resource listings")]
    description: String,
    /// Full task requirement sent to Codex as the initial user message.
    #[schemars(description = "Full task requirement in markdown (supports mermaid). Sent to Codex as the initial user prompt")]
    prompt: String,
    /// Working directory (project root). Defaults to the server process cwd.
    #[serde(default)]
    #[schemars(default)]
    #[schemars(description = "Project working directory. Defaults to the MCP server process cwd")]
    cwd: Option<String>,
    /// When true, wait for the turn to finish before returning. When false, return thread id immediately and notify on completion.
    #[serde(default = "default_block")]
    #[schemars(default = "default_block")]
    #[schemars(description = "Wait for completion when true; return thread_id immediately and notify later when false")]
    block: bool,
    /// Optional Codex model override.
    #[serde(default)]
    #[schemars(default)]
    #[schemars(description = "Optional Codex model override")]
    model: Option<String>,
    /// Sandbox mode. Defaults to danger-full-access for unrestricted execution.
    #[serde(default = "default_sandbox")]
    #[schemars(default = "default_sandbox")]
    #[schemars(description = "Sandbox mode for command execution and file changes. Defaults to danger-full-access.")]
    sandbox: SandboxModeParam,
}

fn default_block() -> bool {
    true
}

fn default_sandbox() -> SandboxModeParam {
    SandboxModeParam::DangerFullAccess
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[schemars(description = "Arguments for replying to an existing Codex thread.")]
struct ReplyParams {
    #[schemars(description = "Target thread id returned by start or listed under a project resource")]
    thread_id: String,
    #[schemars(description = "Follow-up user prompt to send to the thread")]
    prompt: String,
    #[serde(default = "default_block")]
    #[schemars(default = "default_block")]
    #[schemars(description = "Wait for completion when true; return immediately and notify later when false")]
    block: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[schemars(description = "Arguments for inspecting a thread execution trace.")]
struct ProcessParams {
    #[schemars(description = "Target thread id to inspect")]
    thread_id: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(description = "Result of the start tool.")]
struct StartResult {
    #[schemars(description = "Created or resumed Codex thread id")]
    thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Final agent response when block=true")]
    content: Option<String>,
    #[schemars(description = "Whether the call waited for turn completion")]
    blocked: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(description = "Result of the reply tool.")]
struct ReplyResult {
    #[schemars(description = "Target thread id")]
    thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Final agent response when block=true")]
    content: Option<String>,
    #[schemars(description = "Whether the call waited for turn completion")]
    blocked: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[schemars(description = "Arguments for archiving a Codex thread.")]
struct ArchiveParams {
    #[schemars(description = "Target thread id to archive and remove from project listings")]
    thread_id: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(description = "Result of the archive tool.")]
struct ArchiveResult {
    #[schemars(description = "Archived thread id")]
    thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Project id the thread belonged to before archival")]
    project_id: Option<String>,
    #[schemars(description = "Whether the thread was archived successfully")]
    archived: bool,
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
        sandbox: SandboxModeParam,
    ) -> ThreadOptions {
        let mut builder = ThreadOptions::builder()
            .working_directory(cwd)
            .sandbox_mode(sandbox.into_sdk());
        if let Some(model) = model {
            builder = builder.model(model);
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

    async fn notify_thread_archived(
        &self,
        peer: &McpPeer,
        thread_id: &str,
        project_id: Option<&str>,
    ) {
        let _ = peer.notify_resource_list_changed().await;
        if let Some(project_id) = project_id {
            let _ = peer
                .notify_resource_updated(ResourceUpdatedNotificationParam::new(
                    project_uri(project_id),
                ))
                .await;
        } else {
            tracing::debug!(thread_id, "archived thread had no local project mapping");
        }
    }

    async fn start_thread_internal(
        &self,
        description: String,
        prompt: String,
        cwd: String,
        model: Option<String>,
        sandbox: SandboxModeParam,
    ) -> Result<(String, String), McpError> {
        let thread_options = Self::build_thread_options(&cwd, model, sandbox);
        let mut thread = self.codex.start_thread(thread_options);
        let streamed = thread
            .run_streamed(prompt, TurnOptions::default())
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        let thread_id = thread
            .id()
            .ok_or_else(|| McpError::internal_error("thread id missing after start", None))?
            .to_string();

        let record = ThreadRecord::new(thread_id.clone(), cwd, description);
        self.state.upsert_thread(record).await;

        let final_response = self
            .track_streamed_turn(&thread_id, streamed)
            .await
            .map_err(|error| McpError::internal_error(error, None))?;

        self.state
            .update_thread(&thread_id, |record| {
                record.status = ThreadLifecycle::Completed;
                if !final_response.is_empty() {
                    record.final_response = Some(final_response.clone());
                }
            })
            .await;

        Ok((thread_id, final_response))
    }

    async fn start_thread_nonblocking(
        &self,
        description: String,
        prompt: String,
        cwd: String,
        model: Option<String>,
        sandbox: SandboxModeParam,
        peer: McpPeer,
    ) -> Result<String, McpError> {
        let thread_options = Self::build_thread_options(&cwd, model, sandbox);
        let mut thread = self.codex.start_thread(thread_options);
        let streamed = thread
            .run_streamed(prompt, TurnOptions::default())
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        let thread_id = thread
            .id()
            .ok_or_else(|| McpError::internal_error("thread id missing after start", None))?
            .to_string();

        let record = ThreadRecord::new(thread_id.clone(), cwd, description);
        self.state.upsert_thread(record).await;

        let server = self.clone();
        let thread_id_for_task = thread_id.clone();
        tokio::spawn(async move {
            let result = server
                .track_streamed_turn(&thread_id_for_task, streamed)
                .await;

            match result {
                Err(message) => {
                    server
                        .state
                        .update_thread(&thread_id_for_task, |record| {
                            record.status = ThreadLifecycle::Failed;
                            record.error = Some(message);
                        })
                        .await;
                }
                Ok(response) => {
                    server
                        .state
                        .update_thread(&thread_id_for_task, |record| {
                            record.status = ThreadLifecycle::Completed;
                            if !response.is_empty() {
                                record.final_response = Some(response);
                            }
                        })
                        .await;
                }
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
    #[tool(
        title = "Start Thread",
        description = "Create a new Codex thread in a project working directory. \
            Provide a brief description for tracking and a full prompt for Codex to execute. \
            Use block=true to wait for the agent response, or block=false to return thread_id immediately and receive a completion notification later.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<StartResult>()
    )]
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
                    params.prompt,
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
                params.prompt,
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

    #[tool(
        title = "Reply Thread",
        description = "Send a follow-up prompt to an existing Codex thread. \
            Requires thread_id from start or from project:// resource listing. \
            Use block=true to wait for the response, or block=false for async completion notification.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<ReplyResult>()
    )]
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
                    kind: ProcessStepKind::UserReply,
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

    #[tool(
        title = "Process Trace",
        description = "Inspect the local execution trace for a thread, including lifecycle status, process steps, final response, and errors.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<ThreadRecord>()
    )]
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

    #[tool(
        title = "Archive Thread",
        description = "Archive a Codex thread and remove it from local project listings. \
            The archived thread will no longer appear when reading project:// resources.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<ArchiveResult>()
    )]
    async fn archive(
        &self,
        Parameters(params): Parameters<ArchiveParams>,
    ) -> Result<CallToolResult, McpError> {
        archive_remote_thread(&self.codex, &params.thread_id)
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        let removed = self.state.remove_thread(&params.thread_id).await;
        let project_id = removed.map(|record| record.project_id);

        if let Some(peer) = self.peer.read().await.clone() {
            self.notify_thread_archived(&peer, &params.thread_id, project_id.as_deref())
                .await;
        }

        let payload = ArchiveResult {
            thread_id: params.thread_id,
            project_id,
            archived: true,
        };
        tool_json_result(&payload)
    }
}

#[tool_handler(
    name = "codex-mcp-server",
    version = "0.1.0",
    instructions = "Codex MCP server exposing project/thread resources and start/reply/process/archive tools backed by codex app-server."
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
            "Use resources/list for projects only; read project://{project_id} to list thread ids for that project. \
             Individual threads use thread://{project_id}/{thread_id}. Thread resources merge local execution state with codex thread/read payloads. \
             Tools: start, reply, process, archive. Use archive to close a thread and remove it from project listings. \
             Non-blocking start/reply emits notifications/codex/thread/completed. \
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
                    "project://{project_id}",
                    "Codex project by encoded cwd",
                )
                .with_description("Project id is a short stable hash of the absolute cwd"),
                ResourceTemplate::new(
                    "thread://{project_id}/{thread_id}",
                    "Codex thread by project and id",
                ),
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
                let cwd = self
                    .state
                    .get_project(&project_id)
                    .await
                    .map(|project| project.cwd)
                    .or_else(|| decode_project_id(&project_id));

                if let Some(cwd) = cwd.as_deref() {
                    if let Err(error) =
                        sync::sync_threads_for_project(&self.codex, &self.state, cwd).await
                    {
                        tracing::warn!(
                            ?error,
                            project_id = %project_id,
                            "failed to sync threads for project"
                        );
                    }
                }

                let threads = self
                    .state
                    .list_threads_for_project(&project_id)
                    .await
                    .into_iter()
                    .map(|thread| {
                        json!({
                            "thread_id": thread.thread_id,
                            "thread_uri": thread_uri(&thread.project_id, &thread.thread_id),
                            "status": thread.status,
                            "description": thread.description,
                        })
                    })
                    .collect::<Vec<_>>();

                json!({
                    "project_id": project_id,
                    "cwd": cwd,
                    "threads": threads,
                })
            }
            ResourceKind::Thread { thread_id, .. } => {
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

fn tool_json_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let value = serde_json::to_value(value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::structured(value))
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tool_schema_tests {
    use super::*;
    use rmcp::handler::server::wrapper::Parameters;

    fn assert_input_schema<T: schemars::JsonSchema + 'static>(label: &str) {
        let schema = rmcp::handler::server::common::schema_for_input::<T>()
            .unwrap_or_else(|error| panic!("{label} input schema invalid: {error}"));
        assert_eq!(
            schema.get("type").and_then(|value| value.as_str()),
            Some("object"),
            "{label} inputSchema root type must be object: {schema:?}"
        );
    }

    #[test]
    fn list_tools_result_uses_input_schema_camel_case() {
        let tools = CodexMcpServer::tool_router().list_all();
        let result = rmcp::model::ListToolsResult::with_all_items(tools);
        let json = serde_json::to_value(rmcp::model::ServerResult::ListToolsResult(result))
            .expect("serialize list tools result");
        let tools_json = json
            .get("tools")
            .and_then(|value| value.as_array())
            .expect("tools array");
        for tool in tools_json {
            assert!(
                tool.get("inputSchema").is_some(),
                "list_tools result missing inputSchema: {tool}"
            );
        }
    }

    #[test]
    fn tool_attr_functions_expose_input_schema() {
        assert_input_schema::<Parameters<StartParams>>("start");
        assert_input_schema::<Parameters<ReplyParams>>("reply");
        assert_input_schema::<Parameters<ProcessParams>>("process");
        assert_input_schema::<Parameters<ArchiveParams>>("archive");

        let tools = [
            CodexMcpServer::start_tool_attr(),
            CodexMcpServer::reply_tool_attr(),
            CodexMcpServer::process_tool_attr(),
            CodexMcpServer::archive_tool_attr(),
        ];

        for tool in tools {
            let json = serde_json::to_value(&tool).expect("serialize tool");
            assert!(
                json.get("inputSchema").is_some(),
                "tool {} missing inputSchema: {json}",
                tool.name
            );
            let input_schema = json.get("inputSchema").unwrap();
            assert_eq!(
                input_schema.get("type").and_then(|value| value.as_str()),
                Some("object"),
                "tool {} inputSchema must be object: {input_schema}",
                tool.name
            );
        }
    }
}
