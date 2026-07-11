//! Append-only linear JSONL Session Store.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::Effort;
use crate::provider::rates_for;
use crate::session::search::extract_text;
use crate::session::{ReadSnapshot, SessionSearchIndex, SessionState};
use crate::status::estimate_cost_usd;
use crate::types::{Message, Role, TokenUsage, UsageBySource, UsageRecord, UsageSource};

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
    /// A Run started (including ones later aborted).
    RunStarted,
    /// Token usage (+ estimated USD) accumulated for one Provider turn.
    Usage {
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        /// Legacy persisted estimate; accepted on replay but never written.
        #[serde(default, skip_serializing)]
        cost_usd: Option<f64>,
        #[serde(default)]
        source: UsageSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// A Goal was set (or replaced); resets the outer loop run count.
    GoalSet {
        condition: String,
    },
    /// One Goal Evaluator verdict after a working Run.
    GoalEvaluated {
        met: bool,
        reason: String,
    },
    /// Goal cleared explicitly (e.g. `saku stop`).
    GoalCleared,
}

#[derive(Clone)]
pub struct SessionStore {
    sessions_dir: PathBuf,
    append_lock: Arc<std::sync::Mutex<()>>,
    /// Session Search index kept incrementally in sync on every append.
    search_index: Option<Arc<SessionSearchIndex>>,
}

impl SessionStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let sessions_dir = data_dir.as_ref().join("sessions");
        fs::create_dir_all(&sessions_dir)?;
        Ok(Self {
            sessions_dir,
            append_lock: Arc::new(std::sync::Mutex::new(())),
            search_index: None,
        })
    }

    /// Attach a Session Search index; future appends keep it up to date.
    pub fn with_search_index(mut self, index: Arc<SessionSearchIndex>) -> Self {
        self.search_index = Some(index);
        self
    }

    /// Directory holding per-thread `<thread_id>.jsonl` files.
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// Thread ids of all persisted Sessions (one `<thread_id>.jsonl` per Session).
    ///
    /// Discord snowflakes survive filename sanitization unchanged, so the file
    /// stem is the thread id for Sessions created by the bot.
    pub fn list_thread_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        let Ok(entries) = fs::read_dir(&self.sessions_dir) else {
            return ids;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                ids.push(stem.to_string());
            }
        }
        ids
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
                run_count: 0,
                usage: TokenUsage::default(),
                estimated_cost_usd: 0.0,
                usage_by_source: UsageBySource::default(),
                usage_records: Vec::new(),
                last_prompt_tokens: None,
                goal: None,
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
            run_count: 0,
            usage: TokenUsage::default(),
            estimated_cost_usd: 0.0,
            usage_by_source: UsageBySource::default(),
            usage_records: Vec::new(),
            last_prompt_tokens: None,
            goal: None,
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
                SessionEntry::RunStarted => {
                    state.run_count = state.run_count.saturating_add(1);
                }
                SessionEntry::Usage {
                    input,
                    output,
                    cache_read,
                    cache_write,
                    cost_usd: _,
                    source,
                    model,
                } => {
                    let delta = TokenUsage {
                        input,
                        output,
                        cache_read,
                        cache_write,
                    };
                    let model = model.unwrap_or_else(|| state.model.clone());
                    let cost_usd = rates_for(&model)
                        .map(|rates| estimate_cost_usd(&delta, &rates))
                        .unwrap_or(0.0);
                    state.usage.add_assign(&delta);
                    state.estimated_cost_usd += cost_usd;
                    let source_usage = state.usage_by_source.get_mut(source);
                    source_usage.tokens.add_assign(&delta);
                    source_usage.estimated_cost_usd += cost_usd;
                    state.usage_records.push(UsageRecord {
                        source,
                        model,
                        tokens: delta,
                    });
                    if source == UsageSource::Run {
                        state.last_prompt_tokens = Some(delta.prompt_tokens());
                    }
                }
                SessionEntry::GoalSet { condition } => {
                    state.goal = Some(crate::session::Goal {
                        condition,
                        run_count: 0,
                        last_evaluator_reason: None,
                    });
                }
                SessionEntry::GoalEvaluated { met, reason } => {
                    crate::session::apply_goal_evaluated(&mut state.goal, met, &reason);
                }
                SessionEntry::GoalCleared => {
                    state.goal = None;
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
        )?;
        // Keep the Session Search index in sync: user/assistant text only.
        if let Some(index) = &self.search_index
            && matches!(message.role, Role::User | Role::Assistant)
        {
            let text = extract_text(&message.content);
            if let Err(err) =
                index.index_message(&self.path_for(thread_id), thread_id, message.role, &text)
            {
                eprintln!("session search index (message): {err}");
            }
        }
        Ok(())
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
        )?;
        if let Some(index) = &self.search_index
            && let Err(err) = index.index_compaction(&self.path_for(thread_id), thread_id, summary)
        {
            eprintln!("session search index (compaction): {err}");
        }
        Ok(())
    }

    pub fn append_run_started(&self, thread_id: &str) -> Result<(), StoreError> {
        self.append(thread_id, &SessionEntry::RunStarted)
    }

    pub fn append_usage(
        &self,
        thread_id: &str,
        source: UsageSource,
        model: &str,
        usage: &TokenUsage,
    ) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::Usage {
                input: usage.input,
                output: usage.output,
                cache_read: usage.cache_read,
                cache_write: usage.cache_write,
                cost_usd: None,
                source,
                model: Some(model.into()),
            },
        )
    }

    pub fn append_goal_set(&self, thread_id: &str, condition: &str) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::GoalSet {
                condition: condition.into(),
            },
        )
    }

    pub fn append_goal_evaluated(
        &self,
        thread_id: &str,
        met: bool,
        reason: &str,
    ) -> Result<(), StoreError> {
        self.append(
            thread_id,
            &SessionEntry::GoalEvaluated {
                met,
                reason: reason.into(),
            },
        )
    }

    pub fn append_goal_cleared(&self, thread_id: &str) -> Result<(), StoreError> {
        self.append(thread_id, &SessionEntry::GoalCleared)
    }

    fn append(&self, thread_id: &str, entry: &SessionEntry) -> Result<(), StoreError> {
        let _guard = self
            .append_lock
            .lock()
            .expect("session append lock poisoned");
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
