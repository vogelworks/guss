//! Shared building blocks for the VHP admin tools workspace.
//!
//! Every task crate (build, deploy, vapi, net, docs, jump, setup, project)
//! depends on this crate so subprocess handling, prompting, paths, config,
//! login/session (`auth`), and the error type live in exactly one place.

pub mod auth;
pub mod clusters;
pub mod config;
pub mod db;
pub mod docker;
pub mod error;
pub mod git;
pub mod history;
pub mod paths;
pub mod proc;
pub mod ssh;
pub mod terminal;
pub mod ui;
pub mod version;
pub mod viewer;

pub use error::{Result, bail};
