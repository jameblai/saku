//! Discord adapter: Serenity gateway → Harness.

pub mod bot;
mod chunk;
mod commands;
mod progress;

pub use bot::run_bot;
pub use chunk::chunk_message;
