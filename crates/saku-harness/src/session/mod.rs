//! Session Store, Session, and RunHandle.

mod store;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::{Mutex, mpsc, watch};

use crate::background::BackgroundProcesses;
use crate::compaction::{
    DEFAULT_COMPACTION_TOKEN_LIMIT, DEFAULT_KEEP_RECENT, compact_messages, default_local_summarize,
    estimate_tokens,
};
use crate::config::Effort;
use crate::harness::HarnessInner;
use crate::memory::{MEMORY_CHAR_LIMIT, read_memory};
use crate::prompt::build_system_prompt;
use crate::provider::codex::models::{context_window_for, rates_for};
use crate::status::{CodexAccountStatus, RunState, StatusReport, estimate_cost_usd};
use crate::types::{
    Message, ProviderEvent, Request, Role, RunEvent, TokenUsage, ToolCall, UserTurn,
};

pub use store::{SessionEntry, SessionHeader, SessionStore, StoreError};

const MAX_TOOL_ROUNDS: usize = 40;

/// In-memory Session state rebuilt from the Session Store.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub thread_id: String,
    pub cwd: PathBuf,
    pub model: String,
    pub effort: Effort,
    pub messages: Vec<Message>,
    pub read_snapshots: Vec<ReadSnapshot>,
    pub run_count: u64,
    pub usage: TokenUsage,
    pub estimated_cost_usd: f64,
    /// Last Provider-reported prompt tokens (for context fill).
    pub last_prompt_tokens: Option<u64>,
}

/// Record of a file read for optimistic edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadSnapshot {
    pub path: PathBuf,
    pub hash: String,
    pub mtime_secs: i64,
}

struct QueuedRun {
    turn: UserTurn,
    tx: mpsc::UnboundedSender<RunEvent>,
}

pub(crate) struct RunControl {
    busy: bool,
    queue: VecDeque<QueuedRun>,
    pending_steer: Option<String>,
}

/// Handle to a Session bound to a Discord thread id.
#[derive(Clone)]
pub struct Session {
    pub(crate) thread_id: String,
    pub(crate) inner: Arc<HarnessInner>,
    pub(crate) state: Arc<Mutex<SessionState>>,
    pub(crate) abort_tx: Arc<Mutex<watch::Sender<bool>>>,
    pub(crate) run_control: Arc<Mutex<RunControl>>,
    pub(crate) background: Arc<BackgroundProcesses>,
}

impl Session {
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub async fn snapshot(&self) -> SessionState {
        self.state.lock().await.clone()
    }

    /// Idle / running / queued depth for `status`.
    pub async fn run_state(&self) -> RunState {
        let control = self.run_control.lock().await;
        if control.busy {
            RunState::Running {
                waiting: control.queue.len(),
            }
        } else if !control.queue.is_empty() {
            RunState::Queued {
                depth: control.queue.len(),
            }
        } else {
            RunState::Idle
        }
    }

    /// Full `saku status` text: Session snapshot + live Codex account (best-effort).
    pub async fn status_text(&self) -> String {
        let client = reqwest::Client::new();
        let (account, account_error) =
            match crate::provider::fetch_codex_account_status(&self.inner.credentials, &client)
                .await
            {
                Ok(account) => (Some(account), None),
                Err(err) => (None, Some(err.to_string())),
            };
        let report = self
            .status_report(
                &self.inner.default_model,
                self.inner.default_effort,
                crate::provider::CODEX_PROVIDER_ID,
                account,
                account_error,
            )
            .await;
        crate::status::format_status(&report)
    }

    /// Build a StatusReport for this Session (account fields filled by caller).
    pub async fn status_report(
        &self,
        default_model: &str,
        default_effort: Effort,
        provider: &str,
        account: Option<CodexAccountStatus>,
        account_error: Option<String>,
    ) -> StatusReport {
        let state = self.snapshot().await;
        let run_state = self.run_state().await;
        let window = context_window_for(&state.model);
        let mut memory = read_memory(&self.inner.data_dir).unwrap_or_default();
        if memory.chars().count() > MEMORY_CHAR_LIMIT {
            memory = memory.chars().take(MEMORY_CHAR_LIMIT).collect();
        }
        let system = build_system_prompt(&self.inner.workspace, &state.cwd, &memory);
        let estimated = estimate_tokens(&state.messages, &system) as u64;
        let (context_tokens, context_fill_percent) =
            context_fill_for(state.last_prompt_tokens, estimated, window);
        let (bg_running, bg_exited) = self.background.summary().await;
        StatusReport {
            model: state.model,
            effort: state.effort,
            cwd: state.cwd,
            run_state,
            run_count: state.run_count,
            usage: state.usage,
            estimated_cost_usd: state.estimated_cost_usd,
            context_fill_percent,
            context_tokens,
            context_window: window,
            default_model: default_model.into(),
            default_effort,
            provider: provider.into(),
            account,
            account_error,
            background_running: bg_running,
            background_exited: bg_exited,
        }
    }

    /// List this Session's Background Processes (running and exited).
    pub async fn bg_list_text(&self) -> String {
        self.background.list_text().await
    }

    /// Tail logs for a Background Process.
    pub async fn bg_logs_text(&self, pid: u32, lines: Option<usize>) -> Result<String, String> {
        self.background.logs_text(pid, lines).await
    }

    /// Stop one Background Process by pid, or all running when `pid` is `None`.
    /// Does not abort the active Run (`Session::stop` / `saku stop`).
    pub async fn bg_stop(&self, pid: Option<u32>) -> Result<String, String> {
        self.background.stop(pid).await
    }

    /// Abort the active Run and drain the Session Run Queue.
    pub async fn stop(&self) {
        // `send_replace` (not `send`): abort receivers only exist while a tool is
        // executing. `watch::Sender::send` no-ops with zero receivers, which left
        // the flag stuck true after stop and made every follow-up Run abort.
        let _ = self.abort_tx.lock().await.send_replace(true);
        let mut control = self.run_control.lock().await;
        while let Some(queued) = control.queue.pop_front() {
            let _ = queued.tx.send(RunEvent::RunAborted);
        }
        control.pending_steer = None;
    }

    /// Inject a mid-Run steer directive applied after the current tool batch.
    pub async fn steer(&self, message: impl Into<String>) {
        self.run_control.lock().await.pending_steer = Some(message.into());
    }

    pub async fn set_model(&self, model: impl Into<String>) -> Result<(), String> {
        let model = model.into();
        if !crate::provider::is_allowed_model(&model) {
            return Err(format!(
                "unsupported model `{model}`; allowed: {}",
                crate::provider::ALLOWED_MODELS.join(", ")
            ));
        }
        let effort = crate::provider::default_effort_for_model(&model);
        {
            let mut state = self.state.lock().await;
            state.model = model.clone();
            state.effort = effort;
        }
        self.inner
            .store
            .append_model(&self.thread_id, &model)
            .map_err(|e| e.to_string())?;
        self.inner
            .store
            .append_effort(&self.thread_id, effort)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn set_effort(&self, effort: Effort) -> Result<(), String> {
        let model = self.state.lock().await.model.clone();
        if !crate::provider::is_supported_effort(&model, effort) {
            return Err(format!("unsupported effort `{effort}` for model `{model}`"));
        }
        self.state.lock().await.effort = effort;
        self.inner
            .store
            .append_effort(&self.thread_id, effort)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(crate) async fn reset_abort(&self) {
        let _ = self.abort_tx.lock().await.send_replace(false);
    }

    async fn take_steer(&self) -> Option<String> {
        self.run_control.lock().await.pending_steer.take()
    }

    async fn is_aborted(&self) -> bool {
        *self.abort_tx.lock().await.borrow()
    }

    /// Start or queue a Run for this user turn.
    pub async fn run(&self, turn: UserTurn) -> RunHandle {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut control = self.run_control.lock().await;
        if control.busy {
            let _ = tx.send(RunEvent::Queued);
            control.queue.push_back(QueuedRun { turn, tx });
            return RunHandle { rx };
        }
        control.busy = true;
        drop(control);

        let session = self.clone();
        tokio::spawn(async move {
            session.run_pipeline(turn, tx).await;
        });
        RunHandle { rx }
    }

    async fn run_pipeline(self, turn: UserTurn, tx: mpsc::UnboundedSender<RunEvent>) {
        self.reset_abort().await;
        if let Err(err) = self.execute_run(turn, tx.clone()).await {
            let _ = tx.send(RunEvent::RunError {
                message: err.to_string(),
            });
        }
        self.pump_session_queue().await;
    }

    async fn pump_session_queue(&self) {
        loop {
            let next = {
                let mut control = self.run_control.lock().await;
                if let Some(queued) = control.queue.pop_front() {
                    Some(queued)
                } else {
                    control.busy = false;
                    None
                }
            };
            let Some(QueuedRun { turn, tx }) = next else {
                return;
            };
            let _ = tx.send(RunEvent::Dequeued);
            self.reset_abort().await;
            if let Err(err) = self.execute_run(turn, tx.clone()).await {
                let _ = tx.send(RunEvent::RunError {
                    message: err.to_string(),
                });
            }
        }
    }

    async fn execute_run(
        &self,
        turn: UserTurn,
        tx: mpsc::UnboundedSender<RunEvent>,
    ) -> Result<(), RunError> {
        let _ = tx.send(RunEvent::RunStarted);
        {
            let mut state = self.state.lock().await;
            state.run_count = state.run_count.saturating_add(1);
            self.inner
                .store
                .append_run_started(&self.thread_id)
                .map_err(|e| RunError::Store(e.to_string()))?;
        }

        {
            let mut state = self.state.lock().await;
            let user_msg = Message::from_user_turn(&turn);
            state.messages.push(user_msg.clone());
            self.inner
                .store
                .append_message(&self.thread_id, &user_msg)
                .map_err(|e| RunError::Store(e.to_string()))?;
        }

        for _round in 0..MAX_TOOL_ROUNDS {
            if self.is_aborted().await {
                let _ = tx.send(RunEvent::RunAborted);
                return Ok(());
            }

            let (model, effort, messages, cwd, workspace, data_dir, tools) = {
                let mut state = self.state.lock().await;
                let mut memory = read_memory(&self.inner.data_dir).unwrap_or_default();
                if memory.chars().count() > MEMORY_CHAR_LIMIT {
                    memory = memory.chars().take(MEMORY_CHAR_LIMIT).collect();
                }
                let system_probe = build_system_prompt(&self.inner.workspace, &state.cwd, &memory);
                if estimate_tokens(&state.messages, &system_probe) > DEFAULT_COMPACTION_TOKEN_LIMIT
                    && let Some(result) = compact_messages(
                        &state.messages,
                        DEFAULT_KEEP_RECENT,
                        default_local_summarize,
                    )
                {
                    self.inner
                        .store
                        .append_compaction(&self.thread_id, &result.summary)
                        .map_err(|e| RunError::Store(e.to_string()))?;
                    for msg in &result.kept_messages {
                        if msg
                            .content
                            .first()
                            .and_then(|c| match c {
                                crate::types::ContentPart::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .is_some_and(|t| t.starts_with("[compaction summary"))
                        {
                            continue;
                        }
                        self.inner
                            .store
                            .append_message(&self.thread_id, msg)
                            .map_err(|e| RunError::Store(e.to_string()))?;
                    }
                    state.messages = result.kept_messages;
                }

                let tools = self.inner.tools.lock().await.definitions();
                (
                    state.model.clone(),
                    state.effort,
                    state.messages.clone(),
                    state.cwd.clone(),
                    self.inner.workspace.clone(),
                    self.inner.data_dir.clone(),
                    tools,
                )
            };

            let mut memory = read_memory(&data_dir).unwrap_or_default();
            if memory.chars().count() > MEMORY_CHAR_LIMIT {
                memory = memory.chars().take(MEMORY_CHAR_LIMIT).collect();
            }
            let system = build_system_prompt(&workspace, &cwd, &memory);
            let model_for_usage = model.clone();
            let request = Request {
                system,
                messages,
                tools,
                model,
                effort,
            };

            let mut stream = self.inner.provider.complete(request);
            let mut assistant_text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut turn_usage: Option<TokenUsage> = None;

            while let Some(item) = stream.next().await {
                if self.is_aborted().await {
                    let _ = tx.send(RunEvent::RunAborted);
                    return Ok(());
                }
                match item {
                    Ok(ProviderEvent::TextDelta(text)) => {
                        assistant_text.push_str(&text);
                        let _ = tx.send(RunEvent::TextDelta { text });
                    }
                    Ok(ProviderEvent::ReasoningDelta(text)) => {
                        let _ = tx.send(RunEvent::ReasoningDelta { text });
                    }
                    Ok(ProviderEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    }) => {
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments,
                        });
                    }
                    Ok(ProviderEvent::Usage(usage)) => {
                        turn_usage = Some(usage);
                    }
                    Ok(ProviderEvent::MessageComplete) => break,
                    Ok(ProviderEvent::Error(message)) => {
                        let _ = tx.send(RunEvent::RunError { message });
                        return Ok(());
                    }
                    Err(err) => {
                        let _ = tx.send(RunEvent::RunError {
                            message: err.to_string(),
                        });
                        return Ok(());
                    }
                }
            }

            if let Some(usage) = turn_usage {
                self.record_usage(&model_for_usage, usage).await?;
            }

            if !assistant_text.is_empty() || !tool_calls.is_empty() {
                let assistant = Message {
                    role: Role::Assistant,
                    content: if assistant_text.is_empty() {
                        Vec::new()
                    } else {
                        vec![crate::types::ContentPart::text(&assistant_text)]
                    },
                    tool_call_id: None,
                    tool_calls: tool_calls.clone(),
                };
                {
                    let mut state = self.state.lock().await;
                    state.messages.push(assistant.clone());
                }
                self.inner
                    .store
                    .append_message(&self.thread_id, &assistant)
                    .map_err(|e| RunError::Store(e.to_string()))?;
            }

            if tool_calls.is_empty() {
                let _ = tx.send(RunEvent::AssistantFinished);
                let _ = tx.send(RunEvent::RunFinished);
                return Ok(());
            }

            self.execute_tools(&tool_calls, &tx).await?;
            if self.is_aborted().await {
                let _ = tx.send(RunEvent::RunAborted);
                return Ok(());
            }

            // Yield so concurrent steer() calls during the tool batch can land.
            tokio::task::yield_now().await;

            // Apply steer after the current tool batch, before the next Provider call.
            if let Some(steer) = self.take_steer().await {
                let steer_msg = Message::user_text(format!("[steer] {steer}"));
                {
                    let mut state = self.state.lock().await;
                    state.messages.push(steer_msg.clone());
                }
                self.inner
                    .store
                    .append_message(&self.thread_id, &steer_msg)
                    .map_err(|e| RunError::Store(e.to_string()))?;
            }
        }

        let _ = tx.send(RunEvent::RunError {
            message: format!("exceeded {MAX_TOOL_ROUNDS} tool rounds"),
        });
        Ok(())
    }

    async fn execute_tools(
        &self,
        calls: &[ToolCall],
        tx: &mpsc::UnboundedSender<RunEvent>,
    ) -> Result<(), RunError> {
        for call in calls {
            if self.is_aborted().await {
                return Ok(());
            }
            let _ = tx.send(RunEvent::ToolStarted {
                name: call.name.clone(),
                args: call.arguments.clone(),
            });
            tokio::task::yield_now().await;
            let result = self.inner.execute_tool(self, call).await;
            let ok = result.as_ref().map(|r| !r.is_error).unwrap_or(false);
            let tool_message = match result {
                Ok(r) => Message {
                    role: Role::Tool,
                    content: r.content,
                    tool_call_id: Some(call.id.clone()),
                    tool_calls: Vec::new(),
                },
                Err(err) => Message {
                    role: Role::Tool,
                    content: vec![crate::types::ContentPart::text(err.to_string())],
                    tool_call_id: Some(call.id.clone()),
                    tool_calls: Vec::new(),
                },
            };
            {
                let mut state = self.state.lock().await;
                state.messages.push(tool_message.clone());
            }
            self.inner
                .store
                .append_message(&self.thread_id, &tool_message)
                .map_err(|e| RunError::Store(e.to_string()))?;
            let _ = tx.send(RunEvent::ToolFinished {
                name: call.name.clone(),
                ok,
            });
        }
        Ok(())
    }

    async fn record_usage(&self, model: &str, usage: TokenUsage) -> Result<(), RunError> {
        let cost = rates_for(model)
            .map(|rates| estimate_cost_usd(&usage, &rates))
            .unwrap_or(0.0);
        self.inner
            .store
            .append_usage(&self.thread_id, &usage, cost)
            .map_err(|e| RunError::Store(e.to_string()))?;
        let mut state = self.state.lock().await;
        state.usage.add_assign(&usage);
        state.estimated_cost_usd += cost;
        state.last_prompt_tokens = Some(usage.prompt_tokens());
        Ok(())
    }
}

fn context_fill_for(
    last_prompt_tokens: Option<u64>,
    estimated_tokens: u64,
    window: Option<u64>,
) -> (Option<u64>, Option<f64>) {
    let Some(window) = window.filter(|w| *w > 0) else {
        return (None, None);
    };
    let tokens = last_prompt_tokens.unwrap_or(estimated_tokens);
    let percent = (tokens as f64 / window as f64) * 100.0;
    (Some(tokens), Some(percent))
}

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("session store: {0}")]
    Store(String),
}

/// Stream of [`RunEvent`]s for one Run.
pub struct RunHandle {
    rx: mpsc::UnboundedReceiver<RunEvent>,
}

impl RunHandle {
    pub async fn next_event(&mut self) -> Option<RunEvent> {
        self.rx.recv().await
    }

    pub async fn collect(mut self) -> Vec<RunEvent> {
        let mut events = Vec::new();
        while let Some(ev) = self.next_event().await {
            let done = matches!(
                ev,
                RunEvent::RunFinished | RunEvent::RunError { .. } | RunEvent::RunAborted
            );
            events.push(ev);
            if done {
                break;
            }
        }
        events
    }
}

pub(crate) fn new_run_control() -> Arc<Mutex<RunControl>> {
    Arc::new(Mutex::new(RunControl {
        busy: false,
        queue: VecDeque::new(),
        pending_steer: None,
    }))
}
