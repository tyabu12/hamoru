//! API Layer: OpenAI-Compatible Server.
//!
//! Provides `POST /v1/chat/completions` that appears as a single LLM from
//! the outside while internally running multi-model orchestration.
//!
//! This module contains framework-independent types and logic:
//! - [`types`]: OpenAI wire format request/response serde types
//! - [`translate`]: Conversion between OpenAI wire format and internal types

pub mod namespace;
pub mod translate;
pub mod types;
