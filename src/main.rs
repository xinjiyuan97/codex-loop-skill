mod approval;
mod resources;
mod server;
mod state;
mod sync;

use anyhow::Result;
use codex_app_server_sdk::{Codex, StdioConfig};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::{EnvFilter, fmt};

use crate::server::CodexMcpServer;

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("starting codex mcp server");

    let codex = Codex::spawn_stdio(StdioConfig::default()).await?;
    let server = CodexMcpServer::new(codex);
    server.bootstrap().await?;

    let service = server
        .serve(stdio())
        .await
        .inspect_err(|error| tracing::error!(?error, "failed to start mcp server"))?;

    service.waiting().await?;
    Ok(())
}
