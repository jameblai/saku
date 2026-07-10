//! Saku agent harness: Tools, Provider, Session Store, Credentials, Compaction.
//!
//! No Discord / Serenity types live in this crate.

pub mod background;
pub mod compaction;
pub mod config;
pub mod credentials;
pub mod harness;
pub mod index;
pub mod memory;
pub mod path;
pub mod prompt;
pub mod provider;
pub mod session;
pub mod status;
pub mod tools;
pub mod types;
pub mod vision;
pub mod web_backend;

pub use background::{BackgroundProcesses, BgStatus, MAX_RUNNING};
pub use config::{Config, ConfigError, Effort};
pub use credentials::{Credential, CredentialError, CredentialStore};
pub use harness::{Harness, HarnessError};
pub use memory::{MEMORY_CHAR_LIMIT, MemoryError, memory_path, read_memory, validate_memory_write};
pub use path::{PathError, resolve_in_workspace};
pub use provider::{
    ALLOWED_MODELS, CODEX_PROVIDER_ID, CodexProvider, DeviceCodeInfo, FakeProvider, LoginError,
    LoginNotify, Provider, ProviderError, ScriptedResponse, create_codex_provider,
    default_effort_for_model, fetch_codex_account_status, is_allowed_model, is_supported_effort,
    login_device_code, supported_efforts,
};
pub use session::{RunHandle, Session, SessionState, SessionStore};
pub use status::{
    CodexAccountStatus, ModelRates, PlanWindow, RunState, StatusReport, WebBackendStatus,
    estimate_cost_usd, format_status,
};
pub use tools::{
    Tool, ToolError, ToolResult, background_tools, file_tools, register_web_tools, search_tools,
    shell_tools, web_tools, web_tools_from_store,
};
pub use types::{ContentPart, Message, Request, RunEvent, TokenUsage, UserTurn};
pub use vision::{is_image_mime, is_image_path, resize_for_provider};
pub use web_backend::{
    EXA_WEB_BACKEND_ID, ExaClient, ExaError, ExaTeamInfo, fetch_web_backend_status,
    login_exa_api_key,
};
