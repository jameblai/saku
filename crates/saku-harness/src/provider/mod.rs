//! Provider trait and implementations.

pub mod codex;
pub mod fake;

use std::pin::Pin;

use futures::Stream;
use thiserror::Error;

use crate::types::{ProviderEvent, Request};

pub use codex::models::{
    ALLOWED_MODELS, PROVIDER_ID as CODEX_PROVIDER_ID, default_effort_for_model, is_allowed_model,
    is_supported_effort, supported_efforts,
};
pub use codex::{
    CodexProvider, DeviceCodeInfo, LoginError, LoginNotify, create_codex_provider,
    login_device_code,
};
pub use fake::{FakeProvider, ScriptedResponse};

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("{0}")]
    Message(String),
    #[error("provider aborted")]
    Aborted,
}

pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send + 'static>>;

/// Pluggable LLM backend.
pub trait Provider: Send + Sync {
    fn complete(&self, request: Request) -> ProviderStream;
}
