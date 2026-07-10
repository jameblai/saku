//! Append-only linear JSONL Session Store.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::Effort;
use crate::session::{ReadSnapshot, SessionState};
use crate::types::Message;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("session store io: {0}")]
    Io(#[from] std::io::Error),
    #[error("session store parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("invalid session file: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeader {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub version: u32,
    pub thread_id: String,
    pub created_at: String,
    pub cwd: String,
    pub model: String,
    pub effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Message {
        role: crate::types::Role,
        content: Vec<crate::types::ContentPart>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<crate::types::ToolCall>,
    },
    ModelChange {
        model: String,
    },
    EffortChange {
        effort: String,
    },
    CwdChange {
        cwd: String,
    },
    ReadSnapshot {
        path: String,
        hash: String,
        mtime_secs: i64,
    },
    Compaction {
        summary: String,
    },
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions_dir: PathBuf,
}

impl SessionStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let sessions_dir = data_dir.as_ref().join("sessions");
        fs::create_dir_all(&sessions_dir)?;
        Ok(Self { sessions_dir })
    }

    pub fn path_for(&self, thread_id: &str) -> PathBuf {
        // Discord snowflakes are safe filenames; still sanitize path separators.
        let safe: String = thread_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.sessions_dir.join(format!("{safe}.jsonl"))
    }

    pub fn load_or_create(
        &self,
        thread_id: &str,
        default_cwd: &Path,
        default_model: &str,
        default_effort: Effort,
    ) -> Result<SessionState, StoreError> {
        let path = self.path_for(thread_id);
        if !path.exists() {
            let header = SessionHeader {
                entry_type: "session".into(),
                version: 1,
                thread_id: thread_id.into(),
                created_at: now_rfc3339(),
                cwd: default_cwd.display().to_string(),
                model: default_model.into(),
                effort: default_effort.as_str().into(),
            };
            let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
            writeln!(file, "{}", serde_json::to_string(&header)?)?;
            return Ok(SessionState {
                thread_id: thread_id.into(),
                cwd: default_cwd.to_path_buf(),
                model: default_model.into(),
                effort: default_effort,
                messages: Vec::new(),
                read_snapshots: Vec::new(),
            });
        }
        self.replay(thread_id)
    }

    pub fn replay(&self, thread_id: &str) -> Result<SessionState, StoreError> {
        let path = self.path_for(thread_id);
        let file = fs::File::open(&path)?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let header_line = lines
            .next()
            .ok_or_else(|| StoreError::Invalid("empty session file".into()))??;
        let header: SessionHeader = serde_json::from_str(&header_line)?;
        if header.entry_type != "session" {
            return Err(StoreError::Invalid(
                "first line must be session header".into(),
            ));
        }

        let mut state = SessionState {
            thread_id: header.thread_id,
            cwd: PathBuf::from(&header.cwd),
            model: header.model,
            effort: parse_effort(&header.effort).unwrap_or(Effort::Medium),
            messages: Vec::new(),
            read_snapshots: Vec::new(),
        };

        for line in lines {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: SessionEntry = serde_json::from_str(&line)?;
            match entry {
                SessionEntry::Message {
                    role,
                    content,
                    tool_call_id,
                    tool_calls,
                } => {
                    state.messages.push(Message {
                        role,
                        content,
                        tool_call_id,
                        tool_calls,
                    });
                }
                SessionEntry::ModelChange { model } => state.model = model,
                SessionEntry::EffortChange { effort } => {
                    if let Some(e) = parse_effort(&effort) {
                        state.effort = e;
                    }
                }
                SessionEntry::CwdChange { cwd } => state.cwd = PathBuf::from(cwd),
                SessionEntry::ReadSnapshot {
                    path,
                    hash,
                    mtime_secs,
                } => {
                    state.read_snapshots.push(ReadSnapshot {
                        path: PathBuf::from(path),
                        hash,
                        mtime_secs,
                    });
                }
                SessionEntry::Compaction { summary } => {
                    // Replace transcript with summary + keep nothing from before;
                    // subsequent message entries after this line are the live tail.
                    state.messages = vec![crate::types::Message {
                        role: crate::types::Role::User,
                        content: vec![crate::types::ContentPart::text(format!(
                            "[compaction summary of earlier turns]\n{summary}"
                        ))],
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    }];
                }
            }
        }
        Ok(state)
    }

    pub fn append_message(&self, thread_id: &str, message: &Message) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::Message {
                role: message.role,
                content: message.content.clone(),
                tool_call_id: message.tool_call_id.clone(),
                tool_calls: message.tool_calls.clone(),
            },
        )
    }

    pub fn append_cwd(&self, thread_id: &str, cwd: &Path) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::CwdChange {
                cwd: cwd.display().to_string(),
            },
        )
    }

    pub fn append_read_snapshot(
        &self,
        thread_id: &str,
        snapshot: &ReadSnapshot,
    ) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::ReadSnapshot {
                path: snapshot.path.display().to_string(),
                hash: snapshot.hash.clone(),
                mtime_secs: snapshot.mtime_secs,
            },
        )
    }

    pub fn append_model(&self, thread_id: &str, model: &str) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::ModelChange {
                model: model.into(),
            },
        )
    }

    pub fn append_effort(&self, thread_id: &str, effort: Effort) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::EffortChange {
                effort: effort.as_str().into(),
            },
        )
    }

    pub fn append_compaction(&self, thread_id: &str, summary: &str) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::Compaction {
                summary: summary.into(),
            },
        )
    }

    fn append(&self, thread_id: &str, entry: &SessionEntry) -> Result<(), StoreError> {
        let path = self.path_for(thread_id);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
        Ok(())
    }
}

fn parse_effort(s: &str) -> Option<Effort> {
    match s {
        "minimal" => Some(Effort::Minimal),
        "low" => Some(Effort::Low),
        "medium" => Some(Effort::Medium),
        "high" => Some(Effort::High),
        "xhigh" => Some(Effort::Xhigh),
        "max" => Some(Effort::Max),
        _ => None,
    }
}

fn now_rfc3339() -> String {
    // Avoid a chrono dependency for a timestamp label; unix secs is enough for v1 headers.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
