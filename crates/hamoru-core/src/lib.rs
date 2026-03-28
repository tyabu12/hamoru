//! hamoru-core: LLM Orchestration Infrastructure as Code.
//!
//! Core library providing traits, types, and error definitions for all layers
//! of the hamoru orchestration engine.
#![deny(missing_docs)]

pub mod agents;
pub mod config;
pub mod error;
pub mod orchestrator;
pub mod policy;
pub mod provider;
pub mod server;
pub mod telemetry;

/// Convenience type alias for `Result<T, HamoruError>`.
pub type Result<T> = std::result::Result<T, error::HamoruError>;
