//! Discord adapter: Serenity gateway → Harness.

pub mod bot;
mod chunk;
mod commands;
mod progress;
mod typing;

pub use bot::run_bot;
pub use chunk::chunk_message;
