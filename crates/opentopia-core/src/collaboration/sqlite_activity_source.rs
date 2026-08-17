use super::{
    ActivityQuery, AgentActivityReader, AgentActivitySource, AgentActivitySourceError,
    AgentActivityWindow, AgentThreadId, AgentTurnId, AgentTurnStatus, CollaborationRegistry,
    CollaborationSessionId, SqliteCollaborationRepository,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Lossy wake-ups over a durable activity log. The broadcast channel is only
/// a latency optimization; every read and every wait re-checks SQLite.
#[derive(Clone)]
pub struct SqliteAgentActivitySource {
    repository: Arc<SqliteCollaborationRepository>,
    notifications: broadcast::Sender<AgentThreadId>,
}

impl SqliteAgentActivitySource {
    pub fn new(repository: Arc<SqliteCollaborationRepository>) -> Self {
        let (notifications, _) = broadcast::channel(512);
        Self {
            repository,
            notifications,
        }
    }

    pub fn notify(&self, agent_thread_id: AgentThreadId) {
        let _ = self.notifications.send(agent_thread_id);
    }

    fn latest_cursor(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
    ) -> Result<i64, AgentActivitySourceError> {
        self.repository
            .list_activity_events(agent_thread_id, agent_turn_id, None)
            .map(|events| events.last().map_or(0, |event| event.seq))
            .map_err(|error| AgentActivitySourceError::Unavailable(error.to_string()))
    }

    async fn changed_thread(
        &self,
        session_id: CollaborationSessionId,
        target: Option<AgentThreadId>,
        after_cursor: i64,
    ) -> Result<Option<AgentThreadId>, AgentActivitySourceError> {
        let threads = match target {
            Some(target) => vec![self
                .repository
                .get_thread(target)
                .await
                .map_err(|error| AgentActivitySourceError::Unavailable(error.to_string()))?],
            None => self
                .repository
                .list_threads(session_id)
                .await
                .map_err(|error| AgentActivitySourceError::Unavailable(error.to_string()))?,
        };
        for thread in threads {
            if thread.session_id != session_id {
                continue;
            }
            let Some(turn) = self
                .repository
                .latest_turn(thread.id)
                .await
                .map_err(|error| AgentActivitySourceError::Unavailable(error.to_string()))?
            else {
                continue;
            };
            if self.latest_cursor(thread.id, turn.id)? > after_cursor {
                return Ok(Some(thread.id));
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl AgentActivitySource for SqliteAgentActivitySource {
    async fn read_activity(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        turn_status: AgentTurnStatus,
        query: ActivityQuery,
    ) -> Result<AgentActivityWindow, AgentActivitySourceError> {
        let events = self
            .repository
            .list_activity_events(agent_thread_id, agent_turn_id, None)
            .map_err(|error| AgentActivitySourceError::Unavailable(error.to_string()))?;
        Ok(AgentActivityReader.read(agent_thread_id, agent_turn_id, turn_status, &events, &query))
    }

    async fn wait_for_change(
        &self,
        session_id: CollaborationSessionId,
        target: Option<AgentThreadId>,
        after_cursor: Option<i64>,
        timeout: Duration,
    ) -> Result<Option<AgentThreadId>, AgentActivitySourceError> {
        let mut receiver = self.notifications.subscribe();
        if let Some(cursor) = after_cursor {
            if let Some(changed) = self.changed_thread(session_id, target, cursor).await? {
                return Ok(Some(changed));
            }
        }
        if timeout.is_zero() {
            return Ok(None);
        }
        let wait = async {
            loop {
                match receiver.recv().await {
                    Ok(changed) => {
                        if target.is_some_and(|target| target != changed) {
                            continue;
                        }
                        let thread =
                            self.repository.get_thread(changed).await.map_err(|error| {
                                AgentActivitySourceError::Unavailable(error.to_string())
                            })?;
                        if thread.session_id == session_id {
                            return Ok(Some(changed));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(cursor) = after_cursor {
                            if let Some(changed) =
                                self.changed_thread(session_id, target, cursor).await?
                            {
                                return Ok(Some(changed));
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(AgentActivitySourceError::Unavailable(
                            "activity notifier is closed".to_string(),
                        ));
                    }
                }
            }
        };
        match tokio::time::timeout(timeout, wait).await {
            Ok(result) => result,
            Err(_) => {
                if let Some(cursor) = after_cursor {
                    self.changed_thread(session_id, target, cursor).await
                } else {
                    Ok(None)
                }
            }
        }
    }
}
