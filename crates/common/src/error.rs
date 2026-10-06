//! The single error/result type used across the workspace.
//!
//! Replaces the `type Result<T> = std::result::Result<T, Box<dyn Error>>` alias
//! that was re-declared in every `gust` module and needed in `cli`.

/// Workspace-wide result type. Backed by `anyhow` for ergonomic `?` and context.
pub type Result<T> = anyhow::Result<T>;

// Re-export so call sites can `use vhp_common::bail;` / `vhp_common::error::anyhow!(...)`.
pub use anyhow::{Context, anyhow, bail};
