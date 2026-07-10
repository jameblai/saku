//! Session-centric Harness: owns config, Provider, tools, Credentials, Session Store.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, watch};

use crate::config::{Config, Effort};
use crate::credentials::CredentialStore;
use crate::provider::Provider;
use crate::session::{Session, SessionState, SessionStore, StoreError};
use crate::tools::{Tool, ToolContext, ToolError, ToolRegistry, ToolResult};
use crate::types::ToolCall;

/// Shared Harness state.
pub struct HarnessInner {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub default_model: String,
    pub default_effort: Effort,
    pub provider: Arc<dyn Provider>,
    pub store: SessionStore,
    pub credentials: CredentialStore,
    pub tools: Mutex<ToolRegistry>,
    pub sessions: Mutex<HashMap<String, Arc<Mutex<SessionState>>>>,
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
        let (_abort_tx, abort_rx) = watch::channel(false);
        let ctx = ToolContext {
            session,
            abort: abort_rx,
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
        Ok(Self {
            inner: Arc::new(HarnessInner {
                workspace: config.workspace,
                data_dir: config.data_dir,
                default_model: config.default_model,
                default_effort: config.default_effort,
                provider,
                store,
                credentials,
                tools: Mutex::new(ToolRegistry::new()),
                sessions: Mutex::new(HashMap::new()),
            }),
        })
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

    pub async fn register_tool(&self, tool: Arc<dyn Tool>) {
        self.inner.tools.lock().await.register(tool);
    }

    /// Load or create a Session for `thread_id`.
    pub async fn session(&self, thread_id: impl Into<String>) -> Result<Session, HarnessError> {
        let thread_id = thread_id.into();
        let mut sessions = self.inner.sessions.lock().await;
        if let Some(state) = sessions.get(&thread_id) {
            return Ok(Session {
                thread_id,
                inner: Arc::clone(&self.inner),
                state: Arc::clone(state),
            });
        }
        let loaded = self.inner.store.load_or_create(
            &thread_id,
            &self.inner.workspace,
            &self.inner.default_model,
            self.inner.default_effort,
        )?;
        let state = Arc::new(Mutex::new(loaded));
        sessions.insert(thread_id.clone(), Arc::clone(&state));
        Ok(Session {
            thread_id,
            inner: Arc::clone(&self.inner),
            state,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Credentials(#[from] crate::credentials::CredentialError),
}
