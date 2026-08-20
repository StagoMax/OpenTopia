use super::*;
use crate::model::{
    ContextCheckpoint, ContextCheckpointCoverage, ContextSummary, MessagePart, TurnRecord,
};
use crate::policy::ApprovalRequired;
use crate::settings::ProviderHealthCheck;
use crate::store::SqliteSessionStore;
use crate::tools::{Tool, ToolExecutionPolicy};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

include!("tests/foundation_catalog.rs");
include!("tests/effects_and_execution_policy.rs");
include!("tests/concurrency_and_provider_loop.rs");
include!("tests/rollout_and_background.rs");
include!("tests/rollout_context_pressure.rs");
include!("tests/approval_and_guardian.rs");
include!("tests/guardian_reviews.rs");
include!("tests/long_turn_and_context.rs");
