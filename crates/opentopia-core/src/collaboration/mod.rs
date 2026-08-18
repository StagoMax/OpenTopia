//! Multi-agent collaboration domain.
//!
//! This module deliberately owns identities, tree policy, messaging metadata,
//! and activity projections only. Whole-agent execution is reached through the
//! [`AgentRunScheduler`] port; the Agent Core remains the sole owner of model
//! rounds, tool scheduling, safe points, and turn continuation state.

mod activity;
mod domain;
mod invocation;
mod mailbox;
mod registry;
mod run_scheduler;
mod runtime_snapshot;
mod service;
mod snapshot_deriver;
mod sqlite_activity_source;
mod sqlite_repository;

pub use activity::{
    ActivityEvent, ActivityEventDetails, ActivityQuery, AgentActivityReader, AgentActivitySource,
    AgentActivitySourceError, AgentActivityWindow, ToolResultKind, ToolResultProjection,
};
pub use domain::{
    AgentAvailability, AgentMailboxMessageId, AgentPath, AgentRuntimeSnapshotRecord,
    AgentSpawnPolicy, AgentThreadId, AgentThreadRecord, AgentTurnId, AgentTurnRecord,
    AgentTurnStatus, CollaborationDomainError, CollaborationSessionId, CollaborationSessionPolicy,
    CollaborationSessionRecord, RuntimeSnapshotId, RuntimeSnapshotSeed,
};
pub use invocation::{
    AgentCollaborationInvocation, AgentCollaborationInvocationError, AgentCompletionSnapshot,
    AgentInvocationIdentity, AgentListItem, AgentWorkspaceMode, ChildRuntimeSnapshotRequest,
    DerivedChildRuntime, ForkTurns, RuntimeSnapshotDerivationError, RuntimeSnapshotDeriver,
    SpawnChildAgentRequest, WaitAgentOutcome, WaitAgentRequest,
};
pub use mailbox::{
    AgentMailbox, AgentMailboxError, AgentMailboxMessage, AgentMailboxMessageKind,
    AgentMailboxNotifier, EnqueueAgentMessage, InMemoryAgentMailbox, NoopAgentMailboxNotifier,
};
pub use registry::{
    CollaborationRegistry, CreateCollaborationSession, FollowupAgentTurn,
    InMemoryCollaborationRegistry, SpawnAgentThread,
};
pub use run_scheduler::{
    AgentRunCommand, AgentRunResumeSignal, AgentRunScheduler, AgentRunSchedulerError,
};
#[cfg(test)]
pub(crate) use runtime_snapshot::test_runtime_snapshot;
pub use runtime_snapshot::{
    RuntimeForkTurnsLabelV1, RuntimeForkTurnsV1, RuntimeSnapshotV1, RuntimeWorkspaceAssignmentV1,
    RuntimeWorkspaceDeliveryStateV1, RuntimeWorkspaceModeV1, RUNTIME_SNAPSHOT_SCHEMA_VERSION,
};
pub use service::{AgentCollaborationRuntime, AgentCollaborationRuntimeError, SpawnAgentOutcome};
pub use snapshot_deriver::AttenuatingRuntimeSnapshotDeriver;
pub use sqlite_activity_source::SqliteAgentActivitySource;
pub use sqlite_repository::SqliteCollaborationRepository;
