//! Session Store, Session, and RunHandle.

mod store;

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, mpsc, watch};

use crate::background::BackgroundProcesses;
use crate::compaction::{
    DEFAULT_COMPACTION_TOKEN_LIMIT, DEFAULT_KEEP_RECENT, compact_messages, default_local_summarize,
    estimate_tokens,
};
use crate::config::Effort;
use crate::harness::HarnessInner;
use crate::memory::{MEMORY_CHAR_LIMIT, read_memory};
use crate::prompt::build_system_prompt_with_skills;
use crate::provider::codex::models::{context_window_for, rates_for};
use crate::skills::{LoadSkillsOptions, expand_skill_invocations, load_skills};
use crate::status::{
    CodexAccountStatus, RunState, StatusReport, WebBackendStatus, estimate_cost_usd,
};
use crate::types::{
    Message, ProviderEvent, Request, Role, RunEvent, TokenUsage, ToolCall, ToolDefinition, UserTurn,
};

pub use store::{SessionEntry, SessionHeader, SessionStore, StoreError};

/// Replay-time application of a Goal Store entry to `goal` state.
///
/// Shared by [`SessionStore::replay`] and the live evaluation path so persisted
/// and in-memory Goal state stay identical.
pub(crate) fn apply_goal_evaluated(goal: &mut Option<Goal>, met: bool, reason: &str) {
    let Some(g) = goal.as_mut() else { return };
    g.run_count = g.run_count.saturating_add(1);
    if met || g.run_count >= MAX_GOAL_RUNS {
        *goal = None;
    } else {
        g.last_evaluator_reason = Some(reason.to_string());
    }
}

const MAX_TOOL_ROUNDS: usize = 90;

/// Fixed cap on Goal-driven Runs (not user-configurable in v1).
pub const MAX_GOAL_RUNS: u32 = 20;

/// Model + Effort for the Goal Evaluator (issue #50).
const GOAL_EVALUATOR_MODEL: &str = "gpt-5.4-mini";
const GOAL_EVALUATOR_EFFORT: Effort = Effort::Low;

/// Active Session Goal: keep chaining Runs until the condition is met or
/// `MAX_GOAL_RUNS` is exhausted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goal {
    /// The completion condition the user set via `saku goal <condition>`.
    pub condition: String,
    /// Number of working Runs completed and evaluated under this Goal.
    pub run_count: u32,
    /// Most recent Goal Evaluator reason (why the condition was not yet met).
    pub last_evaluator_reason: Option<String>,
}

/// Outcome of one Goal Evaluator pass, driving the outer Goal loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalDecision {
    /// No Goal is active (nothing to evaluate).
    Inactive,
    /// Condition met — the Goal is cleared and achievement should be reported.
    Achieved { condition: String, run: u32 },
    /// Not met yet — run again with `message` as the continuation user turn.
    Continue { message: String, run: u32 },
    /// `MAX_GOAL_RUNS` exhausted without meeting the condition; Goal cleared.
    Exhausted { condition: String, run: u32 },
}

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
    /// Active Session Goal, if any (issue #50).
    pub goal: Option<Goal>,
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
    /// True while a Goal-driver loop owns this Session (single driver at a time).
    pub(crate) goal_driver: Arc<AtomicBool>,
}

/// RAII marker that a Goal-driver loop owns a Session; releases on drop.
pub struct GoalDriverGuard {
    flag: Arc<AtomicBool>,
}

impl Drop for GoalDriverGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

impl Session {
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub async fn snapshot(&self) -> SessionState {
        self.state.lock().await.clone()
    }

    /// Persist a new Session Working Directory (in-memory + Session Store).
    pub async fn set_cwd(&self, cwd: PathBuf) -> Result<(), String> {
        {
            let mut state = self.state.lock().await;
            state.cwd = cwd.clone();
        }
        self.inner
            .store
            .append_cwd(&self.thread_id, &cwd)
            .map_err(|e| e.to_string())
    }

    /// Record a Read Snapshot for `path` (fingerprint + Session Store append).
    pub async fn record_read_snapshot(&self, path: &Path) -> Result<(), String> {
        let (hash, mtime_secs) = file_fingerprint(path)?;
        let snapshot = ReadSnapshot {
            path: path.to_path_buf(),
            hash,
            mtime_secs,
        };
        {
            let mut state = self.state.lock().await;
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
        self.inner
            .store
            .append_read_snapshot(&self.thread_id, &snapshot)
            .map_err(|e| e.to_string())
    }

    /// Assert the on-disk file still matches the Session's Read Snapshot for `path`.
    pub async fn assert_fresh_snapshot(&self, path: &Path) -> Result<(), String> {
        let state = self.snapshot().await;
        let Some(snap) = state.read_snapshots.iter().find(|s| s.path == path) else {
            return Err(format!(
                "no Read Snapshot for {}; read the file before editing",
                path.display()
            ));
        };
        let (hash, mtime_secs) = file_fingerprint(path)?;
        if hash != snap.hash || mtime_secs != snap.mtime_secs {
            return Err(format!("file changed since last read: {}", path.display()));
        }
        Ok(())
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

    /// Full `saku status` text: Session snapshot + live Codex / Web Backend (best-effort).
    pub async fn status_text(&self) -> String {
        let client = reqwest::Client::new();
        let (account, account_error) =
            match crate::provider::fetch_codex_account_status(&self.inner.credentials, &client)
                .await
            {
                Ok(account) => (Some(account), None),
                Err(err) => (None, Some(err.to_string())),
            };
        let web_status = crate::web_backend::fetch_web_backend_status(
            &self.inner.credentials,
            &self.inner.web_backend,
        );
        let report = self
            .status_report(
                &self.inner.default_model,
                self.inner.default_effort,
                crate::provider::CODEX_PROVIDER_ID,
                account,
                account_error,
                web_status,
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
        web_status: WebBackendStatus,
    ) -> StatusReport {
        let state = self.snapshot().await;
        let run_state = self.run_state().await;
        let window = context_window_for(&state.model);
        let mut memory = read_memory(&self.inner.data_dir).unwrap_or_default();
        if memory.chars().count() > MEMORY_CHAR_LIMIT {
            memory = memory.chars().take(MEMORY_CHAR_LIMIT).collect();
        }
        let skills = load_skills(LoadSkillsOptions {
            cwd: &state.cwd,
            workspace: &self.inner.workspace,
            global_skills_dir: &self.inner.global_skills_dir,
        });
        let system = build_system_prompt_with_skills(
            &self.inner.workspace,
            &state.cwd,
            &memory,
            state.goal.as_ref().map(|g| g.condition.as_str()),
            &skills.skills,
        );
        let estimated = estimate_tokens(&state.messages, &system) as u64;
        let (context_tokens, context_fill_percent) =
            context_fill_for(state.last_prompt_tokens, estimated, window);
        let (bg_running, bg_exited) = self.background.summary().await;
        let tool_names = self
            .inner
            .tools
            .lock()
            .await
            .definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();
        let mut skill_names: Vec<String> = skills.skills.iter().map(|s| s.name.clone()).collect();
        skill_names.sort();
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
            tool_names,
            skill_names,
            web_backend: self.inner.web_backend.clone(),
            web_status,
            goal: state.goal,
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
    ///
    /// Also clears any active Goal: `saku stop` ends the outer Goal loop so no
    /// auto-continuation Run is scheduled (issue #50).
    pub async fn stop(&self) {
        // `send_replace` (not `send`): abort receivers only exist while a tool is
        // executing. `watch::Sender::send` no-ops with zero receivers, which left
        // the flag stuck true after stop and made every follow-up Run abort.
        let _ = self.abort_tx.lock().await.send_replace(true);
        {
            let mut control = self.run_control.lock().await;
            while let Some(queued) = control.queue.pop_front() {
                let _ = queued.tx.send(RunEvent::RunAborted);
            }
            control.pending_steer = None;
        }
        self.clear_goal().await;
    }

    /// Currently active Goal, if any.
    pub async fn goal(&self) -> Option<Goal> {
        self.state.lock().await.goal.clone()
    }

    /// Try to claim the single Goal-driver slot for this Session.
    ///
    /// Returns `None` if another driver loop already owns it, preventing two
    /// concurrent outer loops (e.g. resume + replace) from chaining Runs at once.
    pub fn try_become_goal_driver(&self) -> Option<GoalDriverGuard> {
        if self
            .goal_driver
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(GoalDriverGuard {
                flag: Arc::clone(&self.goal_driver),
            })
        } else {
            None
        }
    }

    /// Set (or replace) the active Goal. Replacing resets the run count.
    pub async fn set_goal(&self, condition: &str) -> Result<(), String> {
        {
            let mut state = self.state.lock().await;
            state.goal = Some(Goal {
                condition: condition.to_string(),
                run_count: 0,
                last_evaluator_reason: None,
            });
        }
        self.inner
            .store
            .append_goal_set(&self.thread_id, condition)
            .map_err(|e| e.to_string())
    }

    /// Clear the active Goal (persisted). No-op if no Goal is active.
    pub async fn clear_goal(&self) {
        let had_goal = {
            let mut state = self.state.lock().await;
            state.goal.take().is_some()
        };
        if had_goal {
            let _ = self.inner.store.append_goal_cleared(&self.thread_id);
        }
    }

    /// Run the Goal Evaluator over the current transcript and advance the outer
    /// Goal loop. Records the verdict, updates run count / last reason, and
    /// clears the Goal on achievement or exhaustion.
    pub async fn evaluate_goal(&self) -> Result<GoalDecision, String> {
        let condition = match self.state.lock().await.goal.as_ref() {
            Some(g) => g.condition.clone(),
            None => return Ok(GoalDecision::Inactive),
        };

        let (met, reason) = self
            .run_goal_evaluator(&condition)
            .await
            .map_err(|e| e.to_string())?;

        self.inner
            .store
            .append_goal_evaluated(&self.thread_id, met, &reason)
            .map_err(|e| e.to_string())?;

        let mut state = self.state.lock().await;
        // A concurrent `saku stop` may have cleared the Goal during evaluation.
        if state.goal.is_none() {
            return Ok(GoalDecision::Inactive);
        }
        // The Run number this verdict completes (1-based), captured before the
        // Goal may be cleared by achievement/exhaustion.
        let run = state.goal.as_ref().map(|g| g.run_count).unwrap_or(0) + 1;
        apply_goal_evaluated(&mut state.goal, met, &reason);

        if met {
            Ok(GoalDecision::Achieved { condition, run })
        } else if state.goal.is_none() {
            Ok(GoalDecision::Exhausted { condition, run })
        } else {
            let message = continuation_message(&condition, &reason);
            Ok(GoalDecision::Continue { message, run })
        }
    }

    /// One Goal Evaluator Provider pass: judge the condition against the full
    /// transcript via the `goal_check` Tool. Uses `gpt-5.4-mini` at low Effort.
    async fn run_goal_evaluator(&self, condition: &str) -> Result<(bool, String), RunError> {
        let mut messages = { self.state.lock().await.messages.clone() };
        messages.push(Message::user_text(format!(
            "Evaluate whether the following Goal has been met based on the conversation above.\n\n\
             Goal: {condition}\n\n\
             Call the `goal_check` tool with your verdict: `met` true only if the Goal is fully \
             satisfied, and a short `reason`.",
        )));

        let request = Request {
            system: GOAL_EVALUATOR_SYSTEM.to_string(),
            messages,
            tools: vec![goal_check_tool_def()],
            model: GOAL_EVALUATOR_MODEL.to_string(),
            effort: GOAL_EVALUATOR_EFFORT,
        };

        let mut stream = self.inner.provider.complete(request);
        let mut verdict: Option<(bool, String)> = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(ProviderEvent::ToolCall {
                    name, arguments, ..
                }) if name == "goal_check" => {
                    let met = arguments
                        .get("met")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let reason = arguments
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    verdict = Some((met, reason));
                }
                Ok(ProviderEvent::MessageComplete) => break,
                Ok(ProviderEvent::Error(message)) => return Err(RunError::Evaluator(message)),
                Err(err) => return Err(RunError::Evaluator(err.to_string())),
                _ => {}
            }
        }
        verdict.ok_or_else(|| RunError::Evaluator("evaluator did not call goal_check".into()))
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
        mut turn: UserTurn,
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

            // Expand `$skill-name` against skills for the current Working Directory.
            let discovered = load_skills(LoadSkillsOptions {
                cwd: &state.cwd,
                workspace: &self.inner.workspace,
                global_skills_dir: &self.inner.global_skills_dir,
            });
            turn.text = expand_skill_invocations(&turn.text, &discovered.skills);
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

            let (model, effort, messages, cwd, workspace, data_dir, tools, goal_condition) = {
                let mut state = self.state.lock().await;
                let mut memory = read_memory(&self.inner.data_dir).unwrap_or_default();
                if memory.chars().count() > MEMORY_CHAR_LIMIT {
                    memory = memory.chars().take(MEMORY_CHAR_LIMIT).collect();
                }
                let goal_condition = state.goal.as_ref().map(|g| g.condition.clone());
                let round_skills = load_skills(LoadSkillsOptions {
                    cwd: &state.cwd,
                    workspace: &self.inner.workspace,
                    global_skills_dir: &self.inner.global_skills_dir,
                });
                let system_probe = build_system_prompt_with_skills(
                    &self.inner.workspace,
                    &state.cwd,
                    &memory,
                    goal_condition.as_deref(),
                    &round_skills.skills,
                );
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
                    goal_condition,
                )
            };

            let mut memory = read_memory(&data_dir).unwrap_or_default();
            if memory.chars().count() > MEMORY_CHAR_LIMIT {
                memory = memory.chars().take(MEMORY_CHAR_LIMIT).collect();
            }
            let round_skills = load_skills(LoadSkillsOptions {
                cwd: &cwd,
                workspace: &workspace,
                global_skills_dir: &self.inner.global_skills_dir,
            });
            let system = build_system_prompt_with_skills(
                &workspace,
                &cwd,
                &memory,
                goal_condition.as_deref(),
                &round_skills.skills,
            );            let model_for_usage = model.clone();
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

fn file_fingerprint(path: &Path) -> Result<(String, i64), String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let hash = hex_sha256(&bytes);
    let mtime_secs = mtime_secs(path)?;
    Ok((hash, mtime_secs))
}

fn mtime_secs(path: &Path) -> Result<i64, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = meta.modified().map_err(|e| e.to_string())?;
    let secs = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(secs)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("session store: {0}")]
    Store(String),
    #[error("goal evaluator: {0}")]
    Evaluator(String),
}

const GOAL_EVALUATOR_SYSTEM: &str = "You are the Goal Evaluator for Saku, a coding agent. \
     Given a Session transcript and a Goal condition, judge whether the Goal is fully met. \
     Be strict: only report `met: true` when the transcript shows the condition is actually \
     satisfied (e.g. tests actually pass), not merely attempted. Always respond by calling the \
     `goal_check` tool with `met` and a short `reason`.";

/// The `goal_check` Tool definition offered to the Goal Evaluator.
fn goal_check_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "goal_check".to_string(),
        description: "Report whether the Goal condition is met, with a short reason.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "met": {
                    "type": "boolean",
                    "description": "True only if the Goal condition is fully satisfied."
                },
                "reason": {
                    "type": "string",
                    "description": "Short explanation of the verdict (what remains if not met)."
                }
            },
            "required": ["met", "reason"]
        }),
    }
}

/// Continuation user message for the next Goal Run: original condition + last reason.
fn continuation_message(condition: &str, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        format!("Goal (not yet met): {condition}\n\nKeep working toward the Goal.")
    } else {
        format!(
            "Goal (not yet met): {condition}\n\nGoal Evaluator reason: {reason}\n\nKeep working toward the Goal.",
        )
    }
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
