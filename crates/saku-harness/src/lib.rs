//! Saku agent harness: Tools, Provider, Session Store, Credentials, Compaction.
//!
//! No Discord / Serenity types live in this crate.

pub mod config;
pub mod credentials;
pub mod harness;
pub mod memory;
pub mod path;
pub mod prompt;
pub mod provider;
pub mod session;
pub mod tools;
pub mod types;

pub use config::{Config, ConfigError, Effort};
pub use credentials::{Credential, CredentialError, CredentialStore};
pub use harness::{Harness, HarnessError};
pub use memory::{MEMORY_CHAR_LIMIT, MemoryError, memory_path, read_memory, validate_memory_write};
pub use path::{PathError, resolve_in_workspace};
pub use provider::{FakeProvider, Provider, ProviderError, ScriptedResponse};
pub use session::{RunHandle, Session, SessionState, SessionStore};
pub use types::{ContentPart, Message, Request, RunEvent, UserTurn};
