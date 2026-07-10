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
    RunControl, Session, SessionState, SessionStore, StoreError, new_run_control,
};
use crate::tools::{
    Tool, ToolContext, ToolError, ToolRegistry, ToolResult, background_tools, file_tools,
    memory_tools, register_web_tools, search_tools, shell_tools,
};
use crate::types::ToolCall;

pub(crate) struct LiveSession {
    pub state: Arc<Mutex<SessionState>>,
    pub abort_tx: Arc<Mutex<watch::Sender<bool>>>,
    pub run_control: Arc<Mutex<RunControl>>,
    pub background: Arc<BackgroundProcesses>,
}

/// Shared Harness state.
pub(crate) struct HarnessInner {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub default_model: String,
    pub default_effort: Effort,
    pub web_backend: String,
    pub provider: Arc<dyn Provider>,
    pub store: SessionStore,
    pub credentials: CredentialStore,
    pub index: SharedIndex,
    pub tools: Mutex<ToolRegistry>,
    pub sessions: Mutex<HashMap<String, LiveSession>>,
}

impl HarnessInner {
    pub async fn execute_tool(
        &self,
        session: &Session,
        call: &ToolCall,
    ) -> Result<ToolResult, ToolError> {
        let tool = {
            let registry = self.tools.lock().await;
            registry
                .get(&call.name)
                .ok_or_else(|| ToolError::Unknown(call.name.clone()))?
        };
        let abort = session.abort_tx.lock().await.subscribe();
        let cwd = session.snapshot().await.cwd;
        let ctx = ToolContext {
            session,
            workspace: &self.workspace,
            cwd,
            data_dir: &self.data_dir,
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
        let store = SessionStore::open(&config.data_dir)?;
        let credentials = CredentialStore::open(&config.data_dir)?;
        let index = Arc::new(WorkspaceIndex::new(&config.workspace, &config.data_dir)?);
        Ok(Self {
            inner: Arc::new(HarnessInner {
                workspace: config.workspace,
                data_dir: config.data_dir,
                default_model: config.default_model,
                default_effort: config.default_effort,
                web_backend: config.web_backend,
                provider,
                store,
                credentials,
                index,
                tools: Mutex::new(ToolRegistry::new()),
                sessions: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn index(&self) -> &SharedIndex {
        &self.inner.index
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
        register_web_tools(self).await;
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
        sessions.insert(
            thread_id.clone(),
            LiveSession {
                state: Arc::clone(&state),
                abort_tx: Arc::clone(&abort_tx),
                run_control: Arc::clone(&run_control),
                background: Arc::clone(&background),
            },
        );
        Ok(Session {
            thread_id,
            inner: Arc::clone(&self.inner),
            state,
            abort_tx,
            run_control,
            background,
        })
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
}
