use codex_app_server_sdk::{
    api::Codex,
    error::ClientError,
    protocol::{requests::ThreadListParams, requests::ThreadReadParams, responses::ThreadSummary},
};
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::state::{AppState, ThreadLifecycle, ThreadRecord};

const THREAD_LIST_PAGE_LIMIT: u32 = 100;
const MAX_THREAD_LIST_PAGES: usize = 100;

pub async fn sync_threads_from_codex(codex: &Codex, state: &AppState) -> Result<usize, ClientError> {
    let mut cursor: Option<String> = None;
    let mut pages = 0usize;
    let mut imported = 0usize;

    loop {
        pages += 1;
        if pages > MAX_THREAD_LIST_PAGES {
            warn!("stopped thread sync after {MAX_THREAD_LIST_PAGES} pages");
            break;
        }

        let page = codex
            .thread_list(ThreadListParams {
                limit: Some(THREAD_LIST_PAGE_LIMIT),
                cursor: cursor.clone(),
                ..Default::default()
            })
            .await?;

        for summary in page.data {
            let record = thread_record_from_summary(&summary);
            state.import_thread_if_absent(record).await;
            imported += 1;
        }

        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    info!(imported, "synced codex threads into mcp resources");
    Ok(imported)
}

pub async fn ensure_thread_known(codex: &Codex, state: &AppState, thread_id: &str) -> Result<(), ClientError> {
    if state.get_thread(thread_id).await.is_some() {
        return Ok(());
    }

    let remote = codex
        .thread_read(ThreadReadParams {
            thread_id: thread_id.to_string(),
            include_turns: Some(false),
            extra: Default::default(),
        })
        .await?;

    let Some(thread_value) = remote.extra.get("thread") else {
        return Err(ClientError::TransportSend(format!(
            "thread/read returned no thread payload for `{thread_id}`"
        )));
    };

    let record = thread_record_from_remote(thread_id, thread_value);
    state.upsert_thread(record).await;
    Ok(())
}

pub async fn read_remote_thread(
    codex: &Codex,
    thread_id: &str,
    include_turns: bool,
) -> Result<Value, ClientError> {
    let remote = codex
        .thread_read(ThreadReadParams {
            thread_id: thread_id.to_string(),
            include_turns: Some(include_turns),
            extra: Default::default(),
        })
        .await?;

    Ok(Value::Object(remote.extra))
}

pub fn thread_record_from_summary(summary: &ThreadSummary) -> ThreadRecord {
    let cwd = summary_string(summary, &["cwd"]).unwrap_or_else(|| ".".to_string());
    let description = summary
        .title
        .clone()
        .or_else(|| summary_string(summary, &["preview", "name"]))
        .unwrap_or_else(|| summary.id.clone());

    let mut record = ThreadRecord::new(summary.id.clone(), cwd, description);
    record.status = map_remote_status(summary);
    record.final_response = summary_string(summary, &["preview"]);
    record
}

fn thread_record_from_remote(thread_id: &str, thread: &Value) -> ThreadRecord {
    let cwd = json_string(thread, &["cwd"]).unwrap_or_else(|| ".".to_string());
    let description = json_string(thread, &["preview", "name"])
        .unwrap_or_else(|| thread_id.to_string());

    let mut record = ThreadRecord::new(thread_id.to_string(), cwd, description);
    record.status = map_json_status(thread.get("status"));
    record
}

fn map_remote_status(summary: &ThreadSummary) -> ThreadLifecycle {
    map_json_status(summary.extra.get("status"))
}

fn map_json_status(status: Option<&Value>) -> ThreadLifecycle {
    let Some(status) = status else {
        return ThreadLifecycle::Completed;
    };

    match status.get("type").and_then(Value::as_str) {
        Some("active") => ThreadLifecycle::Running,
        Some("idle") => ThreadLifecycle::Completed,
        Some("systemError") => ThreadLifecycle::Failed,
        _ => ThreadLifecycle::Completed,
    }
}

fn summary_string(summary: &ThreadSummary, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| summary.extra.get(*key).and_then(Value::as_str).map(str::to_string))
}

fn json_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_string))
}

pub fn merge_thread_resource(
    local: Option<ThreadRecord>,
    remote: Value,
) -> Value {
    match local {
        Some(record) => json!({
            "source": "local+remote",
            "local": record,
            "remote": remote,
        }),
        None => json!({
            "source": "remote",
            "remote": remote,
        }),
    }
}
