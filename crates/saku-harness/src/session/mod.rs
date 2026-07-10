//! Session Store, Session, and RunHandle.

mod store;

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::{Mutex, mpsc, watch};

use crate::config::Effort;
use crate::harness::HarnessInner;
use crate::memory::read_memory;
use crate::prompt::build_system_prompt;
use crate::types::{Message, ProviderEvent, Request, Role, RunEvent, ToolCall, UserTurn};

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
}

/// Record of a file read for optimistic edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadSnapshot {
    pub path: PathBuf,
    pub hash: String,
    pub mtime_secs: i64,
}

/// Handle to a Session bound to a Discord thread id.
#[derive(Clone)]
pub struct Session {
    pub(crate) thread_id: String,
    pub(crate) inner: Arc<HarnessInner>,
    pub(crate) state: Arc<Mutex<SessionState>>,
    pub(crate) abort_tx: Arc<Mutex<watch::Sender<bool>>>,
}

impl Session {
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub async fn snapshot(&self) -> SessionState {
        self.state.lock().await.clone()
    }

    /// Abort the active Run (and running bash) for later `stop` Bot Command wiring.
    pub async fn stop(&self) {
        let _ = self.abort_tx.lock().await.send(true);
    }

    pub(crate) async fn reset_abort(&self) {
        let _ = self.abort_tx.lock().await.send(false);
    }

    /// Start (or queue) a Run for this user turn.
    pub async fn run(&self, turn: UserTurn) -> RunHandle {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = self.clone();
        tokio::spawn(async move {
            session.reset_abort().await;
            if let Err(err) = session.execute_run(turn, tx.clone()).await {
                let _ = tx.send(RunEvent::RunError {
                    message: err.to_string(),
                });
            }
        });
        RunHandle { rx }
    }

    async fn execute_run(
        &self,
        turn: UserTurn,
        tx: mpsc::UnboundedSender<RunEvent>,
    ) -> Result<(), RunError> {
        let _ = tx.send(RunEvent::RunStarted);

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
            let (model, effort, messages, cwd, workspace, data_dir, tools) = {
                let state = self.state.lock().await;
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

            let memory = read_memory(&data_dir).unwrap_or_default();
            let system = build_system_prompt(&workspace, &cwd, &memory);
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

            while let Some(item) = stream.next().await {
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
            let _ = tx.send(RunEvent::ToolStarted {
                name: call.name.clone(),
                args: call.arguments.clone(),
            });
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
