//! Provider trait and test doubles.

pub mod fake;

use std::pin::Pin;

use futures::Stream;
use thiserror::Error;

use crate::types::{ProviderEvent, Request};

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
