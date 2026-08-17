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
mod service;
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
    AgentCollaborationInvocation, AgentCollaborationInvocationError, AgentInvocationIdentity,
    AgentListItem, AgentWorkspaceMode, ChildRuntimeSnapshotRequest, DerivedChildRuntime, ForkTurns,
    RuntimeSnapshotDerivationError, RuntimeSnapshotDeriver, SpawnChildAgentRequest,
    WaitAgentOutcome, WaitAgentRequest,
};
pub use mailbox::{
    AgentMailbox, AgentMailboxError, AgentMailboxMessage, AgentMailboxMessageKind,
    EnqueueAgentMessage, InMemoryAgentMailbox,
};
pub use registry::{
    CollaborationRegistry, CreateCollaborationSession, FollowupAgentTurn,
    InMemoryCollaborationRegistry, SpawnAgentThread,
};
pub use run_scheduler::{AgentRunCommand, AgentRunScheduler, AgentRunSchedulerError};
pub use service::{AgentCollaborationRuntime, AgentCollaborationRuntimeError, SpawnAgentOutcome};
pub use sqlite_repository::SqliteCollaborationRepository;
