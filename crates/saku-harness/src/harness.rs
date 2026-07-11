//! Session-centric Harness: owns config, Provider, tools, Credentials, Session Store.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, watch};

use crate::background::BackgroundProcesses;
use crate::config::{Config, Effort};
use crate::credentials::CredentialStore;
use crate::index::{SharedIndex, WorkspaceIndex};
use crate::provider::Provider;
use crate::session::{
    RunControl, Session, SessionSearchIndex, SessionState, SessionStore, StoreError,
    new_run_control,
};
use crate::tools::{
    SubagentTool, Tool, ToolBatchPolicy, ToolContext, ToolError, ToolRegistry, ToolResult,
    background_tools, file_tools, memory_tools, register_web_tools, search_tools,
    session_search_tools, shell_tools,
};
use crate::types::ToolCall;

pub(crate) struct LiveSession {
    pub state: Arc<Mutex<SessionState>>,
    pub abort_tx: Arc<Mutex<watch::Sender<bool>>>,
    pub run_control: Arc<Mutex<RunControl>>,
    pub background: Arc<BackgroundProcesses>,
    pub goal_driver: Arc<std::sync::atomic::AtomicBool>,
}

/// Shared Harness state.
pub(crate) struct HarnessInner {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub global_skills_dir: PathBuf,
    pub default_model: String,
    pub default_effort: Effort,
    pub web_backend: String,
    pub provider: Arc<dyn Provider>,
    pub store: SessionStore,
    pub credentials: CredentialStore,
    pub index: SharedIndex,
    pub search_index: Arc<SessionSearchIndex>,
    pub tools: Mutex<ToolRegistry>,
    pub sessions: Mutex<HashMap<String, LiveSession>>,
}

impl HarnessInner {
    pub async fn tool_batch_policy(&self, calls: &[ToolCall]) -> ToolBatchPolicy {
        let Some(first) = calls.first() else {
            return ToolBatchPolicy::Sequential;
        };
        if calls.iter().any(|call| call.name != first.name) {
            return ToolBatchPolicy::Sequential;
        }
        let registry = self.tools.lock().await;
        let Some(tool) = registry.get(&first.name) else {
            return ToolBatchPolicy::Sequential;
        };
        let arguments = calls
            .iter()
            .map(|call| call.arguments.clone())
            .collect::<Vec<_>>();
        tool.batch_policy(&arguments)
    }

    pub async fn execute_tool(
        &self,
        session: &Session,
        call: &ToolCall,
        system_prompt: &str,
    ) -> Result<ToolResult, ToolError> {
        let tool = {
            let registry = self.tools.lock().await;
            registry
                .get(&call.name)
                .ok_or_else(|| ToolError::Unknown(call.name.clone()))?
        };
        let abort = session.abort_tx.lock().await.subscribe();
        let cwd = session.snapshot().await.cwd;
        let skill_roots = crate::skills::skill_base_dirs(
            &crate::skills::load_skills(crate::skills::LoadSkillsOptions {
                cwd: &cwd,
                workspace: &self.workspace,
                global_skills_dir: &self.global_skills_dir,
            })
            .skills,
        );
        let ctx = ToolContext {
            session,
            workspace: &self.workspace,
            cwd,
            data_dir: &self.data_dir,
            system_prompt,
            skill_roots,
            abort,
            progress: None,
        };
        tool.execute(&ctx, call.arguments.clone()).await
    }
}

/// Top-level Harness entry.
#[derive(Clone)]
pub struct Harness {
    inner: Arc<HarnessInner>,
}

impl Harness {
    pub fn new(config: Config, provider: Arc<dyn Provider>) -> Result<Self, HarnessError> {
        Self::with_global_skills_dir(config, provider, crate::skills::default_global_skills_dir())
    }

    /// Construct a Harness with an explicit global skills directory (tests / overrides).
    pub fn with_global_skills_dir(
        config: Config,
        provider: Arc<dyn Provider>,
        global_skills_dir: PathBuf,
    ) -> Result<Self, HarnessError> {
        let search_index = Arc::new(SessionSearchIndex::open(&config.data_dir)?);
        let store =
            SessionStore::open(&config.data_dir)?.with_search_index(Arc::clone(&search_index));
        let credentials = CredentialStore::open(&config.data_dir)?;
        let index = Arc::new(WorkspaceIndex::new(&config.workspace, &config.data_dir)?);
        Ok(Self {
            inner: Arc::new(HarnessInner {
                workspace: config.workspace,
                data_dir: config.data_dir,
                global_skills_dir,
                default_model: config.default_model,
                default_effort: config.default_effort,
                web_backend: config.web_backend,
                provider,
                store,
                credentials,
                index,
                search_index,
                tools: Mutex::new(ToolRegistry::new()),
                sessions: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn index(&self) -> &SharedIndex {
        &self.inner.index
    }

    pub fn search_index(&self) -> &Arc<SessionSearchIndex> {
        &self.inner.search_index
    }

    /// Rebuild the Session Search index from existing JSONL Sessions.
    ///
    /// Run at bot startup to migrate Sessions written before the index existed.
    /// Returns the number of Sessions indexed. Blocking work runs off the async
    /// runtime.
    pub async fn reindex_sessions(&self) -> Result<usize, crate::session::SearchError> {
        let search_index = Arc::clone(&self.inner.search_index);
        let sessions_dir = self.inner.store.sessions_dir().to_path_buf();
        tokio::task::spawn_blocking(move || search_index.reindex_all(&sessions_dir))
            .await
            .expect("reindex task panicked")
    }

    pub fn workspace(&self) -> &std::path::Path {
        &self.inner.workspace
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.inner.data_dir
    }

    pub fn credentials(&self) -> &CredentialStore {
        &self.inner.credentials
    }

    pub fn default_model(&self) -> &str {
        &self.inner.default_model
    }

    pub fn default_effort(&self) -> Effort {
        self.inner.default_effort
    }

    pub fn web_backend(&self) -> &str {
        &self.inner.web_backend
    }

    /// Live Codex Plan Usage + Reset Credits (best-effort; errors are for the caller).
    pub async fn fetch_codex_account_status(
        &self,
    ) -> Result<crate::status::CodexAccountStatus, crate::provider::ProviderError> {
        let client = reqwest::Client::new();
        crate::provider::fetch_codex_account_status(&self.inner.credentials, &client).await
    }

    pub async fn register_tool(&self, tool: Arc<dyn Tool>) {
        self.inner.tools.lock().await.register(tool);
    }

    /// Register the default Tool set: file, memory, shell, background, search, then
    /// credential-gated web Tools.
    pub async fn register_default_tools(&self) {
        for tool in file_tools() {
            self.register_tool(tool).await;
        }
        for tool in memory_tools() {
            self.register_tool(tool).await;
        }
        for tool in shell_tools() {
            self.register_tool(tool).await;
        }
        for tool in background_tools() {
            self.register_tool(tool).await;
        }
        for tool in search_tools(Arc::clone(self.index())) {
            self.register_tool(tool).await;
        }
        for tool in session_search_tools(Arc::clone(self.search_index())) {
            self.register_tool(tool).await;
        }
        register_web_tools(self).await;
        self.register_tool(Arc::new(SubagentTool::new())).await;
    }

    /// Load or create a Session for `thread_id`.
    pub async fn session(&self, thread_id: impl Into<String>) -> Result<Session, HarnessError> {
        let thread_id = thread_id.into();
        let mut sessions = self.inner.sessions.lock().await;
        if let Some(live) = sessions.get(&thread_id) {
            return Ok(Session {
                thread_id,
                inner: Arc::clone(&self.inner),
                state: Arc::clone(&live.state),
                abort_tx: Arc::clone(&live.abort_tx),
                run_control: Arc::clone(&live.run_control),
                background: Arc::clone(&live.background),
                goal_driver: Arc::clone(&live.goal_driver),
            });
        }
        let loaded = self.inner.store.load_or_create(
            &thread_id,
            &self.inner.workspace,
            &self.inner.default_model,
            self.inner.default_effort,
        )?;
        let state = Arc::new(Mutex::new(loaded));
        let (abort_tx, _) = watch::channel(false);
        let abort_tx = Arc::new(Mutex::new(abort_tx));
        let run_control = new_run_control();
        let background = Arc::new(BackgroundProcesses::new());
        let goal_driver = Arc::new(std::sync::atomic::AtomicBool::new(false));
        sessions.insert(
            thread_id.clone(),
            LiveSession {
                state: Arc::clone(&state),
                abort_tx: Arc::clone(&abort_tx),
                run_control: Arc::clone(&run_control),
                background: Arc::clone(&background),
                goal_driver: Arc::clone(&goal_driver),
            },
        );
        Ok(Session {
            thread_id,
            inner: Arc::clone(&self.inner),
            state,
            abort_tx,
            run_control,
            background,
            goal_driver,
        })
    }

    /// Thread ids of persisted Sessions that currently have an active Goal.
    ///
    /// Used on bot startup to resume outer Goal loops (issue #50).
    pub async fn sessions_with_active_goal(&self) -> Vec<String> {
        let mut out = Vec::new();
        for thread_id in self.inner.store.list_thread_ids() {
            match self.inner.store.replay(&thread_id) {
                Ok(state) if state.goal.is_some() => out.push(thread_id),
                _ => {}
            }
        }
        out
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Credentials(#[from] crate::credentials::CredentialError),
    #[error(transparent)]
    Index(#[from] crate::index::IndexError),
    #[error(transparent)]
    Search(#[from] crate::session::SearchError),
}
