use std::sync::Arc;

use codex_app_server_sdk::{
    api::Codex,
    error::ClientError,
    protocol::server_requests::{
        ApplyPatchApprovalParams, ApplyPatchApprovalResponse, CommandExecutionRequestApprovalParams,
        CommandExecutionRequestApprovalResponse, ExecCommandApprovalParams,
        ExecCommandApprovalResponse, FileChangeRequestApprovalParams,
        FileChangeRequestApprovalResponse,
    },
};
use rmcp::{RoleServer, model::*};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::state::{AppState, ApprovalKind, ProcessStepKind, ThreadLifecycle};

type McpPeer = rmcp::service::Peer<RoleServer>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Approve,
    ApproveForSession,
    Deny,
}

impl ApprovalPolicy {
    pub fn from_env() -> Self {
        match std::env::var("CODEX_MCP_APPROVAL_POLICY")
            .unwrap_or_else(|_| "approve".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "deny" | "decline" | "reject" => Self::Deny,
            "session" | "approve_for_session" | "approve-for-session" => Self::ApproveForSession,
            _ => Self::Approve,
        }
    }
}

pub async fn register_approval_handlers(
    codex: &Codex,
    state: Arc<AppState>,
    peer: Arc<RwLock<Option<McpPeer>>>,
    policy: ApprovalPolicy,
) {
    let client = codex.client();

    {
        let state = Arc::clone(&state);
        let peer = Arc::clone(&peer);
        client
            .set_command_execution_request_approval_handler(move |params| {
                let state = Arc::clone(&state);
                let peer = Arc::clone(&peer);
                async move {
                    handle_command_execution_approval(state, peer, policy, params).await
                }
            })
            .await;
    }

    {
        let state = Arc::clone(&state);
        let peer = Arc::clone(&peer);
        client
            .set_exec_command_approval_handler(move |params| {
                let state = Arc::clone(&state);
                let peer = Arc::clone(&peer);
                async move { handle_exec_command_approval(state, peer, policy, params).await }
            })
            .await;
    }

    {
        let state = Arc::clone(&state);
        let peer = Arc::clone(&peer);
        client
            .set_apply_patch_approval_handler(move |params| {
                let state = Arc::clone(&state);
                let peer = Arc::clone(&peer);
                async move { handle_apply_patch_approval(state, peer, policy, params).await }
            })
            .await;
    }

    {
        let state = Arc::clone(&state);
        let peer = Arc::clone(&peer);
        client
            .set_file_change_request_approval_handler(move |params| {
                let state = Arc::clone(&state);
                let peer = Arc::clone(&peer);
                async move {
                    handle_file_change_approval(state, peer, policy, params).await
                }
            })
            .await;
    }

    info!(?policy, "registered codex app-server approval handlers");
}

async fn handle_command_execution_approval(
    state: Arc<AppState>,
    peer: Arc<RwLock<Option<McpPeer>>>,
    policy: ApprovalPolicy,
    params: CommandExecutionRequestApprovalParams,
) -> Result<CommandExecutionRequestApprovalResponse, ClientError> {
    notify_approval_request(
        &peer,
        ApprovalKind::CommandExecution,
        &params.extra,
        policy,
    )
    .await;
    track_approval(&state, &params.extra, ApprovalKind::CommandExecution).await;

    let decision = match policy {
        ApprovalPolicy::Approve => json!("accept"),
        ApprovalPolicy::ApproveForSession => json!("acceptForSession"),
        ApprovalPolicy::Deny => json!("decline"),
    };

    let mut response = CommandExecutionRequestApprovalResponse::default();
    response.extra.insert("decision".to_string(), decision);
    Ok(response)
}

async fn handle_exec_command_approval(
    state: Arc<AppState>,
    peer: Arc<RwLock<Option<McpPeer>>>,
    policy: ApprovalPolicy,
    params: ExecCommandApprovalParams,
) -> Result<ExecCommandApprovalResponse, ClientError> {
    notify_approval_request(&peer, ApprovalKind::ExecCommand, &params.extra, policy).await;
    track_approval(&state, &params.extra, ApprovalKind::ExecCommand).await;

    let decision = match policy {
        ApprovalPolicy::Approve => json!("approved"),
        ApprovalPolicy::ApproveForSession => json!("approved_for_session"),
        ApprovalPolicy::Deny => json!("denied"),
    };

    let mut response = ExecCommandApprovalResponse::default();
    response.extra.insert("decision".to_string(), decision);
    Ok(response)
}

async fn handle_apply_patch_approval(
    state: Arc<AppState>,
    peer: Arc<RwLock<Option<McpPeer>>>,
    policy: ApprovalPolicy,
    params: ApplyPatchApprovalParams,
) -> Result<ApplyPatchApprovalResponse, ClientError> {
    notify_approval_request(&peer, ApprovalKind::ApplyPatch, &params.extra, policy).await;
    track_approval(&state, &params.extra, ApprovalKind::ApplyPatch).await;

    let decision = match policy {
        ApprovalPolicy::Approve => json!("approved"),
        ApprovalPolicy::ApproveForSession => json!("approved_for_session"),
        ApprovalPolicy::Deny => json!("denied"),
    };

    let mut response = ApplyPatchApprovalResponse::default();
    response.extra.insert("decision".to_string(), decision);
    Ok(response)
}

async fn handle_file_change_approval(
    state: Arc<AppState>,
    peer: Arc<RwLock<Option<McpPeer>>>,
    policy: ApprovalPolicy,
    params: FileChangeRequestApprovalParams,
) -> Result<FileChangeRequestApprovalResponse, ClientError> {
    notify_approval_request(&peer, ApprovalKind::FileChange, &params.extra, policy).await;
    track_approval(&state, &params.extra, ApprovalKind::FileChange).await;

    let decision = match policy {
        ApprovalPolicy::Approve => json!("accept"),
        ApprovalPolicy::ApproveForSession => json!("acceptForSession"),
        ApprovalPolicy::Deny => json!("decline"),
    };

    let mut response = FileChangeRequestApprovalResponse::default();
    response.extra.insert("decision".to_string(), decision);
    Ok(response)
}

async fn notify_approval_request(
    peer: &Arc<RwLock<Option<McpPeer>>>,
    kind: ApprovalKind,
    params: &serde_json::Map<String, Value>,
    policy: ApprovalPolicy,
) {
    let Some(peer) = peer.read().await.clone() else {
        return;
    };

    let payload = json!({
        "kind": kind,
        "policy": policy,
        "params": params,
    });

    if let Err(error) = peer
        .send_notification(ServerNotification::CustomNotification(
            CustomNotification::new(
                "notifications/codex/approval/request",
                Some(payload),
            ),
        ))
        .await
    {
        warn!(?error, ?kind, "failed to send approval notification");
    }
}

async fn track_approval(
    state: &AppState,
    params: &serde_json::Map<String, Value>,
    kind: ApprovalKind,
) {
    let Some(thread_id) = params
        .get("threadId")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };

    let summary = params
        .get("command")
        .or_else(|| params.get("reason"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(&kind).unwrap_or_default());

    state
        .update_thread(&thread_id, |record| {
            record.status = ThreadLifecycle::WaitingApproval;
            record.process.push(crate::state::ProcessStep {
                kind: ProcessStepKind::Approval { kind },
                summary,
                at: 0,
            });
        })
        .await;
}
