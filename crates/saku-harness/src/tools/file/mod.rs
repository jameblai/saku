//! File tools: read, edit, write + Read Snapshot helpers.

mod edit;
mod read;
mod write;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::memory::{MemoryError, memory_path, validate_memory_write};
use crate::path::{PathError, resolve_in_workspace};
use crate::session::{ReadSnapshot, Session};
use crate::tools::{Tool, ToolError, ToolResult};

pub use edit::EditTool;
pub use read::ReadTool;
pub use write::WriteTool;

pub fn file_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(ReadTool),
        std::sync::Arc::new(EditTool),
        std::sync::Arc::new(WriteTool),
    ]
}

pub(crate) async fn resolve_tool_path(
    session: &Session,
    candidate: &str,
) -> Result<PathBuf, ToolError> {
    let state = session.snapshot().await;
    let memory = memory_path(&session.inner.data_dir);
    resolve_in_workspace(
        &session.inner.workspace,
        &state.cwd,
        candidate,
        Some(&memory),
    )
    .map_err(path_err)
}

pub(crate) fn file_fingerprint(path: &Path) -> Result<(String, i64), ToolError> {
    let bytes = fs::read(path).map_err(|e| ToolError::Message(e.to_string()))?;
    let hash = hex_sha256(&bytes);
    let mtime_secs = mtime_secs(path)?;
    Ok((hash, mtime_secs))
}

pub(crate) fn mtime_secs(path: &Path) -> Result<i64, ToolError> {
    let meta = fs::metadata(path).map_err(|e| ToolError::Message(e.to_string()))?;
    let modified = meta
        .modified()
        .map_err(|e| ToolError::Message(e.to_string()))?;
    let secs = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(secs)
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) async fn record_snapshot(session: &Session, path: &Path) -> Result<(), ToolError> {
    let (hash, mtime_secs) = file_fingerprint(path)?;
    let snapshot = ReadSnapshot {
        path: path.to_path_buf(),
        hash,
        mtime_secs,
    };
    {
        let mut state = session.state.lock().await;
        if let Some(existing) = state
            .read_snapshots
            .iter_mut()
            .find(|s| s.path == snapshot.path)
        {
            *existing = snapshot.clone();
        } else {
            state.read_snapshots.push(snapshot.clone());
        }
    }
    session
        .inner
        .store
        .append_read_snapshot(&session.thread_id, &snapshot)
        .map_err(|e| ToolError::Message(e.to_string()))?;
    Ok(())
}

pub(crate) async fn require_fresh_snapshot(
    session: &Session,
    path: &Path,
) -> Result<(), ToolError> {
    let state = session.snapshot().await;
    let Some(snap) = state.read_snapshots.iter().find(|s| s.path == path) else {
        return Err(ToolError::Message(format!(
            "no Read Snapshot for {}; read the file before editing",
            path.display()
        )));
    };
    let (hash, mtime_secs) = file_fingerprint(path)?;
    if hash != snap.hash || mtime_secs != snap.mtime_secs {
        return Err(ToolError::Message(format!(
            "file changed since last read: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn maybe_enforce_memory_cap(
    path: &Path,
    data_dir: &Path,
    content: &str,
) -> Result<(), ToolError> {
    let mem = memory_path(data_dir);
    let same = path == mem
        || (path.canonicalize().ok().zip(mem.canonicalize().ok())).is_some_and(|(a, b)| a == b);
    if same {
        validate_memory_write(content).map_err(|e| match e {
            MemoryError::TooLong { actual } => ToolError::Message(format!(
                "Memory exceeds 2200 characters ({actual} characters)"
            )),
            other => ToolError::Message(other.to_string()),
        })?;
    }
    Ok(())
}

fn path_err(err: PathError) -> ToolError {
    ToolError::Message(err.to_string())
}

pub(crate) fn arg_string(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::Message(format!("missing string argument `{key}`")))
}

pub(crate) fn ok_text(text: impl Into<String>) -> ToolResult {
    ToolResult::text(text)
}
