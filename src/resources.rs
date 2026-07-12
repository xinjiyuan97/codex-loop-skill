//! MCP resource URI helpers for Codex projects and threads.

use std::borrow::Cow;

pub const PROJECT_SCHEME: &str = "project://";
pub const THREAD_SCHEME: &str = "thread://";

const PROJECT_ID_BYTES: usize = 6;

pub fn project_uri(project_id: &str) -> String {
    format!("{PROJECT_SCHEME}{project_id}")
}

pub fn thread_uri(project_id: &str, thread_id: &str) -> String {
    format!("{THREAD_SCHEME}{project_id}/{thread_id}")
}

pub fn encode_project_id(cwd: &str) -> String {
    let normalized = normalize_cwd(cwd);
    let hash = fnv1a64(normalized.as_bytes());
    base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        &hash.to_le_bytes()[..PROJECT_ID_BYTES],
    )
}

pub fn decode_project_id(project_id: &str) -> Option<String> {
    if project_id.len() <= 12 {
        return None;
    }

    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        project_id,
    )
    .ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    if decoded.starts_with('/') || decoded.starts_with('.') {
        Some(decoded)
    } else {
        None
    }
}

fn normalize_cwd(cwd: &str) -> Cow<'_, str> {
    let trimmed = cwd.trim();
    if trimmed.len() > 1 && trimmed.ends_with('/') {
        Cow::Owned(trimmed.trim_end_matches('/').to_string())
    } else {
        Cow::Borrowed(trimmed)
    }
}

fn fnv1a64(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub fn parse_resource_uri(uri: &str) -> ResourceKind {
    if let Some(id) = uri.strip_prefix(PROJECT_SCHEME) {
        return ResourceKind::Project(id.to_string());
    }
    if let Some(rest) = uri.strip_prefix(THREAD_SCHEME) {
        if let Some((project_id, thread_id)) = rest.split_once('/') {
            if !project_id.is_empty() && !thread_id.is_empty() {
                return ResourceKind::Thread {
                    project_id: project_id.to_string(),
                    thread_id: thread_id.to_string(),
                };
            }
        }
    }
    ResourceKind::Unknown
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceKind {
    Project(String),
    Thread {
        project_id: String,
        thread_id: String,
    },
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_project_id_is_short_and_stable() {
        let cwd = "/Volumes/Data/work/src/github.com/xinjiyuan97/codex-skill";
        let id = encode_project_id(cwd);
        assert_eq!(id.len(), 8);
        assert_eq!(id, encode_project_id(cwd));
        assert_eq!(id, encode_project_id(&format!("{cwd}/")));
    }

    #[test]
    fn decode_project_id_supports_legacy_long_ids() {
        let cwd = "/tmp/example-project";
        let legacy = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            cwd.as_bytes(),
        );
        assert_eq!(decode_project_id(&legacy).as_deref(), Some(cwd));
        assert_eq!(decode_project_id("abcd1234"), None);
    }
}
