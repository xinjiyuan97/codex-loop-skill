//! MCP resource URI helpers for Codex projects and threads.

pub const PROJECT_SCHEME: &str = "codex://project/";
pub const THREAD_SCHEME: &str = "codex://thread/";

pub fn project_uri(project_id: &str) -> String {
    format!("{PROJECT_SCHEME}{project_id}")
}

pub fn thread_uri(thread_id: &str) -> String {
    format!("{THREAD_SCHEME}{thread_id}")
}

pub fn encode_project_id(cwd: &str) -> String {
    base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        cwd.as_bytes(),
    )
}

pub fn decode_project_id(project_id: &str) -> Option<String> {
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        project_id,
    )
    .ok()?;
    String::from_utf8(bytes).ok()
}

pub fn parse_resource_uri(uri: &str) -> ResourceKind {
    if let Some(id) = uri.strip_prefix(PROJECT_SCHEME) {
        return ResourceKind::Project(id.to_string());
    }
    if let Some(id) = uri.strip_prefix(THREAD_SCHEME) {
        return ResourceKind::Thread(id.to_string());
    }
    ResourceKind::Unknown
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceKind {
    Project(String),
    Thread(String),
    Unknown,
}
