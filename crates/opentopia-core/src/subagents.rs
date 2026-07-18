use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, watch, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

impl SubagentRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            other => anyhow::bail!("unknown subagent run status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRun {
    pub id: Uuid,
    pub parent_thread_id: Uuid,
    pub parent_turn_id: Uuid,
    pub name: String,
    pub input: String,
    pub depth: u8,
    pub status: SubagentRunStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct SpawnSubagentRequest {
    pub parent_thread_id: Uuid,
    pub parent_turn_id: Uuid,
    pub name: String,
    pub input: String,
    pub depth: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentScope {
    pub thread_id: Uuid,
    pub parent_turn_id: Uuid,
    pub depth: u8,
}

#[derive(Debug, Clone)]
pub struct SubagentSchedulerConfig {
    pub max_concurrency_per_parent: usize,
    pub max_depth: u8,
    pub timeout: Option<Duration>,
}

impl Default for SubagentSchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrency_per_parent: 4,
            max_depth: 2,
            timeout: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    pub run: SubagentRun,
}

#[derive(Debug, Error)]
pub enum SubagentError {
    #[error("subagent name cannot be empty")]
    EmptyName,
    #[error("subagent input cannot be empty")]
    EmptyInput,
    #[error("subagent depth {actual} exceeds maximum {maximum}")]
    MaximumDepth { actual: u8, maximum: u8 },
    #[error("subagent run not found: {0}")]
    NotFound(Uuid),
    #[error("subagent run is already terminal: {0}")]
    AlreadyTerminal(Uuid),
    #[error("subagent input channel is closed: {0}")]
    InputClosed(Uuid),
    #[error("timed out waiting for subagent run: {0}")]
    WaitTimedOut(Uuid),
}

#[async_trait]
pub trait SubagentExecutor: Send + Sync + 'static {
    async fn execute(
        &self,
        run: SubagentRun,
        input: mpsc::UnboundedReceiver<String>,
        cancellation: CancellationToken,
    ) -> anyhow::Result<String>;
}

pub trait SubagentObserver: Send + Sync + 'static {
    fn on_update(&self, run: &SubagentRun);
}

#[derive(Debug, Default)]
pub struct NoopSubagentObserver;

impl SubagentObserver for NoopSubagentObserver {
    fn on_update(&self, _run: &SubagentRun) {}
}

struct RunControl {
    run: Mutex<SubagentRun>,
    cancellation: CancellationToken,
    input: mpsc::UnboundedSender<String>,
    updates: watch::Sender<SubagentRun>,
}

struct SchedulerInner {
    config: SubagentSchedulerConfig,
    executor: Arc<dyn SubagentExecutor>,
    observer: Arc<dyn SubagentObserver>,
    runs: Mutex<HashMap<Uuid, Arc<RunControl>>>,
    groups: Mutex<HashMap<Uuid, Arc<Semaphore>>>,
    events: broadcast::Sender<SubagentEvent>,
}

#[derive(Clone)]
pub struct SubagentScheduler {
    inner: Arc<SchedulerInner>,
}

impl SubagentScheduler {
    pub fn new(
        config: SubagentSchedulerConfig,
        executor: Arc<dyn SubagentExecutor>,
        observer: Arc<dyn SubagentObserver>,
    ) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(SchedulerInner {
                config,
                executor,
                observer,
                runs: Mutex::new(HashMap::new()),
                groups: Mutex::new(HashMap::new()),
                events,
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SubagentEvent> {
        self.inner.events.subscribe()
    }

    pub fn spawn(&self, request: SpawnSubagentRequest) -> Result<SubagentRun, SubagentError> {
        let name = request.name.trim().to_string();
        if name.is_empty() {
            return Err(SubagentError::EmptyName);
        }
        let input_text = request.input.trim().to_string();
        if input_text.is_empty() {
            return Err(SubagentError::EmptyInput);
        }
        if request.depth > self.inner.config.max_depth {
            return Err(SubagentError::MaximumDepth {
                actual: request.depth,
                maximum: self.inner.config.max_depth,
            });
        }

        let run = SubagentRun {
            id: Uuid::new_v4(),
            parent_thread_id: request.parent_thread_id,
            parent_turn_id: request.parent_turn_id,
            name,
            input: input_text,
            depth: request.depth,
            status: SubagentRunStatus::Queued,
            result: None,
            error: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (updates, _) = watch::channel(run.clone());
        let control = Arc::new(RunControl {
            run: Mutex::new(run.clone()),
            cancellation: CancellationToken::new(),
            input: input_tx,
            updates,
        });
        self.inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .insert(run.id, control.clone());
        self.publish(&run);

        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.execute(control, input_rx).await;
        });
        Ok(run)
    }

    pub fn get(&self, run_id: Uuid) -> Option<SubagentRun> {
        self.control(run_id).map(|control| {
            control
                .run
                .lock()
                .expect("subagent run mutex poisoned")
                .clone()
        })
    }

    pub fn list_for_thread(&self, thread_id: Uuid) -> Vec<SubagentRun> {
        let controls = self
            .inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut runs = controls
            .into_iter()
            .filter_map(|control| {
                let run = control
                    .run
                    .lock()
                    .expect("subagent run mutex poisoned")
                    .clone();
                (run.parent_thread_id == thread_id).then_some(run)
            })
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
        runs
    }

    pub fn send_input(&self, run_id: Uuid, input: String) -> Result<(), SubagentError> {
        let control = self
            .control(run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        if control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .status
            .is_terminal()
        {
            return Err(SubagentError::AlreadyTerminal(run_id));
        }
        if input.trim().is_empty() {
            return Err(SubagentError::EmptyInput);
        }
        control
            .input
            .send(input)
            .map_err(|_| SubagentError::InputClosed(run_id))
    }

    pub fn send_input_scoped(
        &self,
        scope: SubagentScope,
        run_id: Uuid,
        input: String,
    ) -> Result<(), SubagentError> {
        self.ensure_visible(scope, run_id)?;
        self.send_input(run_id, input)
    }

    pub fn cancel(&self, run_id: Uuid) -> Result<(), SubagentError> {
        let control = self
            .control(run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        if control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .status
            .is_terminal()
        {
            return Err(SubagentError::AlreadyTerminal(run_id));
        }
        control.cancellation.cancel();
        self.cancel_parent(run_id);
        Ok(())
    }

    pub fn cancel_scoped(&self, scope: SubagentScope, run_id: Uuid) -> Result<(), SubagentError> {
        self.ensure_visible(scope, run_id)?;
        self.cancel(run_id)
    }

    pub fn cancel_parent(&self, parent_turn_id: Uuid) -> usize {
        let controls = self
            .inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut cancelled = 0usize;
        let mut parents = vec![parent_turn_id];
        let mut visited = std::collections::HashSet::new();
        while let Some(parent) = parents.pop() {
            if !visited.insert(parent) {
                continue;
            }
            for control in &controls {
                let run = control
                    .run
                    .lock()
                    .expect("subagent run mutex poisoned")
                    .clone();
                if run.parent_turn_id == parent && !run.status.is_terminal() {
                    control.cancellation.cancel();
                    cancelled += 1;
                    parents.push(run.id);
                }
            }
        }
        cancelled
    }

    pub async fn wait(
        &self,
        run_id: Uuid,
        timeout: Duration,
    ) -> Result<SubagentRun, SubagentError> {
        let control = self
            .control(run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        let mut updates = control.updates.subscribe();
        let future = async {
            loop {
                let run = updates.borrow().clone();
                if run.status.is_terminal() {
                    return run;
                }
                if updates.changed().await.is_err() {
                    return updates.borrow().clone();
                }
            }
        };
        tokio::time::timeout(timeout, future)
            .await
            .map_err(|_| SubagentError::WaitTimedOut(run_id))
    }

    pub async fn wait_scoped(
        &self,
        scope: SubagentScope,
        run_id: Uuid,
        timeout: Duration,
    ) -> Result<SubagentRun, SubagentError> {
        self.ensure_visible(scope, run_id)?;
        self.wait(run_id, timeout).await
    }

    fn ensure_visible(&self, scope: SubagentScope, run_id: Uuid) -> Result<(), SubagentError> {
        let run = self.get(run_id).ok_or(SubagentError::NotFound(run_id))?;
        let is_direct_child = run.parent_thread_id == scope.thread_id
            && run.parent_turn_id == scope.parent_turn_id
            && run.depth == scope.depth.saturating_add(1);
        if !is_direct_child {
            // Do not disclose whether a UUID belongs to another thread or parent turn.
            return Err(SubagentError::NotFound(run_id));
        }
        Ok(())
    }

    async fn execute(&self, control: Arc<RunControl>, input: mpsc::UnboundedReceiver<String>) {
        let parent_turn_id = control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .parent_turn_id;
        let semaphore = self
            .inner
            .groups
            .lock()
            .expect("subagent groups mutex poisoned")
            .entry(parent_turn_id)
            .or_insert_with(|| {
                Arc::new(Semaphore::new(
                    self.inner.config.max_concurrency_per_parent.max(1),
                ))
            })
            .clone();
        let permit = tokio::select! {
            _ = control.cancellation.cancelled() => {
                self.finish(&control, SubagentRunStatus::Cancelled, None, None);
                return;
            }
            permit = semaphore.acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => {
                    self.finish(&control, SubagentRunStatus::Failed, None, Some("subagent queue closed".to_string()));
                    return;
                }
            }
        };
        self.transition_running(&control);
        let run = control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .clone();
        let execution = self
            .inner
            .executor
            .execute(run, input, control.cancellation.child_token());
        tokio::pin!(execution);
        if let Some(timeout_duration) = self.inner.config.timeout {
            let timeout = tokio::time::sleep(timeout_duration);
            tokio::pin!(timeout);
            tokio::select! {
                _ = control.cancellation.cancelled() => {
                    self.finish(&control, SubagentRunStatus::Cancelled, None, None);
                }
                _ = &mut timeout => {
                    control.cancellation.cancel();
                    let run_id = control.run.lock().expect("subagent run mutex poisoned").id;
                    self.cancel_parent(run_id);
                    self.finish(&control, SubagentRunStatus::TimedOut, None, Some("subagent execution timed out".to_string()));
                }
                result = &mut execution => self.finish_execution_result(&control, result),
            }
        } else {
            tokio::select! {
                _ = control.cancellation.cancelled() => {
                    self.finish(&control, SubagentRunStatus::Cancelled, None, None);
                }
                result = &mut execution => self.finish_execution_result(&control, result),
            }
        }
        drop(permit);
    }

    fn finish_execution_result(&self, control: &RunControl, result: anyhow::Result<String>) {
        match result {
            Ok(result) => self.finish(control, SubagentRunStatus::Completed, Some(result), None),
            Err(_) if control.cancellation.is_cancelled() => {
                self.finish(control, SubagentRunStatus::Cancelled, None, None)
            }
            Err(error) => self.finish(
                control,
                SubagentRunStatus::Failed,
                None,
                Some(error.to_string()),
            ),
        }
    }

    fn transition_running(&self, control: &RunControl) {
        let run = {
            let mut run = control.run.lock().expect("subagent run mutex poisoned");
            if run.status != SubagentRunStatus::Queued {
                return;
            }
            run.status = SubagentRunStatus::Running;
            run.started_at = Some(Utc::now());
            run.clone()
        };
        control.updates.send_replace(run.clone());
        self.publish(&run);
    }

    fn finish(
        &self,
        control: &RunControl,
        status: SubagentRunStatus,
        result: Option<String>,
        error: Option<String>,
    ) {
        let run = {
            let mut run = control.run.lock().expect("subagent run mutex poisoned");
            if run.status.is_terminal() {
                return;
            }
            run.status = status;
            run.result = result;
            run.error = error;
            run.completed_at = Some(Utc::now());
            run.clone()
        };
        control.updates.send_replace(run.clone());
        self.publish(&run);
    }

    fn publish(&self, run: &SubagentRun) {
        self.inner.observer.on_update(run);
        let _ = self.inner.events.send(SubagentEvent { run: run.clone() });
    }

    fn control(&self, run_id: Uuid) -> Option<Arc<RunControl>> {
        self.inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .get(&run_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestExecutor {
        active: AtomicUsize,
        peak: AtomicUsize,
        delay: Duration,
        fail: bool,
    }

    #[async_trait]
    impl SubagentExecutor for TestExecutor {
        async fn execute(
            &self,
            run: SubagentRun,
            mut input: mpsc::UnboundedReceiver<String>,
            cancellation: CancellationToken,
        ) -> anyhow::Result<String> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let result = tokio::select! {
                _ = cancellation.cancelled() => anyhow::bail!("cancelled"),
                _ = tokio::time::sleep(self.delay) => {
                    if self.fail { anyhow::bail!("failed intentionally") }
                    let mut messages = Vec::new();
                    while let Ok(message) = input.try_recv() { messages.push(message); }
                    Ok(format!("{}:{}", run.name, messages.join(",")))
                }
            };
            self.active.fetch_sub(1, Ordering::SeqCst);
            result
        }
    }

    fn scheduler(
        max: usize,
        delay: Duration,
        timeout: Duration,
    ) -> (SubagentScheduler, Arc<TestExecutor>) {
        let executor = Arc::new(TestExecutor {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay,
            fail: false,
        });
        let scheduler = SubagentScheduler::new(
            SubagentSchedulerConfig {
                max_concurrency_per_parent: max,
                max_depth: 2,
                timeout: Some(timeout),
            },
            executor.clone(),
            Arc::new(NoopSubagentObserver),
        );
        (scheduler, executor)
    }

    fn request(parent: Uuid, name: &str) -> SpawnSubagentRequest {
        SpawnSubagentRequest {
            parent_thread_id: Uuid::new_v4(),
            parent_turn_id: parent,
            name: name.to_string(),
            input: "work".to_string(),
            depth: 1,
        }
    }

    #[test]
    fn default_scheduler_has_no_execution_timeout() {
        assert_eq!(SubagentSchedulerConfig::default().timeout, None);
    }

    fn owning_scope(run: &SubagentRun) -> SubagentScope {
        SubagentScope {
            thread_id: run.parent_thread_id,
            parent_turn_id: run.parent_turn_id,
            depth: run.depth - 1,
        }
    }

    #[tokio::test]
    async fn queues_fairly_and_enforces_parent_concurrency() {
        let (scheduler, executor) = scheduler(2, Duration::from_millis(40), Duration::from_secs(2));
        let parent = Uuid::new_v4();
        let runs = (0..5)
            .map(|index| {
                scheduler
                    .spawn(request(parent, &format!("run-{index}")))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for run in runs {
            assert_eq!(
                scheduler
                    .wait(run.id, Duration::from_secs(2))
                    .await
                    .unwrap()
                    .status,
                SubagentRunStatus::Completed
            );
        }
        assert_eq!(executor.peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn send_input_is_delivered_and_wait_returns_result() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(40), Duration::from_secs(2));
        let run = scheduler.spawn(request(Uuid::new_v4(), "reader")).unwrap();
        scheduler
            .send_input(run.id, "follow-up".to_string())
            .unwrap();
        let run = scheduler
            .wait(run.id, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(run.status, SubagentRunStatus::Completed);
        assert_eq!(run.result.as_deref(), Some("reader:follow-up"));
    }

    #[tokio::test]
    async fn scoped_operations_hide_runs_from_other_threads() {
        let (scheduler, _) = scheduler(1, Duration::from_secs(5), Duration::from_secs(10));
        let run = scheduler.spawn(request(Uuid::new_v4(), "private")).unwrap();
        let cross_thread_scope = SubagentScope {
            thread_id: Uuid::new_v4(),
            ..owning_scope(&run)
        };

        assert!(matches!(
            scheduler.send_input_scoped(cross_thread_scope, run.id, "intrude".to_string()),
            Err(SubagentError::NotFound(id)) if id == run.id
        ));
        assert!(matches!(
            scheduler.cancel_scoped(cross_thread_scope, run.id),
            Err(SubagentError::NotFound(id)) if id == run.id
        ));
        assert!(matches!(
            scheduler
                .wait_scoped(cross_thread_scope, run.id, Duration::from_millis(5))
                .await,
            Err(SubagentError::NotFound(id)) if id == run.id
        ));

        scheduler.cancel(run.id).unwrap();
    }

    #[tokio::test]
    async fn scoped_operations_require_direct_parent_and_depth() {
        let (scheduler, _) = scheduler(1, Duration::from_secs(5), Duration::from_secs(10));
        let run = scheduler
            .spawn(request(Uuid::new_v4(), "direct-child"))
            .unwrap();
        let wrong_parent_scope = SubagentScope {
            parent_turn_id: Uuid::new_v4(),
            ..owning_scope(&run)
        };
        let wrong_depth_scope = SubagentScope {
            depth: run.depth,
            ..owning_scope(&run)
        };

        assert!(matches!(
            scheduler.send_input_scoped(wrong_parent_scope, run.id, "intrude".to_string()),
            Err(SubagentError::NotFound(id)) if id == run.id
        ));
        assert!(matches!(
            scheduler.cancel_scoped(wrong_depth_scope, run.id),
            Err(SubagentError::NotFound(id)) if id == run.id
        ));
        assert!(matches!(
            scheduler
                .wait_scoped(wrong_parent_scope, run.id, Duration::from_millis(5))
                .await,
            Err(SubagentError::NotFound(id)) if id == run.id
        ));

        scheduler
            .send_input_scoped(owning_scope(&run), run.id, "allowed".to_string())
            .unwrap();
        scheduler.cancel_scoped(owning_scope(&run), run.id).unwrap();
    }

    #[tokio::test]
    async fn supports_single_and_parent_cancellation() {
        let (scheduler, _) = scheduler(2, Duration::from_secs(5), Duration::from_secs(10));
        let parent = Uuid::new_v4();
        let one = scheduler.spawn(request(parent, "one")).unwrap();
        let two = scheduler.spawn(request(parent, "two")).unwrap();
        let mut nested_request = request(one.id, "nested");
        nested_request.depth = 2;
        let nested = scheduler.spawn(nested_request).unwrap();
        scheduler.cancel(one.id).unwrap();
        assert_eq!(scheduler.cancel_parent(parent), 3);
        assert_eq!(
            scheduler
                .wait(one.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::Cancelled
        );
        assert_eq!(
            scheduler
                .wait(two.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::Cancelled
        );
        assert_eq!(
            scheduler
                .wait(nested.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn times_out_and_rejects_excessive_depth() {
        let (scheduler, _) = scheduler(1, Duration::from_secs(5), Duration::from_millis(25));
        let run = scheduler.spawn(request(Uuid::new_v4(), "slow")).unwrap();
        assert_eq!(
            scheduler
                .wait(run.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::TimedOut
        );
        let mut deep = request(Uuid::new_v4(), "deep");
        deep.depth = 3;
        assert!(matches!(
            scheduler.spawn(deep),
            Err(SubagentError::MaximumDepth { .. })
        ));
    }

    #[tokio::test]
    async fn wait_has_its_own_timeout_and_terminal_runs_reject_input() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(100), Duration::from_secs(2));
        let run = scheduler.spawn(request(Uuid::new_v4(), "wait")).unwrap();
        assert!(matches!(
            scheduler.wait(run.id, Duration::from_millis(5)).await,
            Err(SubagentError::WaitTimedOut(_))
        ));
        scheduler.cancel(run.id).unwrap();
        scheduler
            .wait(run.id, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(matches!(
            scheduler.send_input(run.id, "late".to_string()),
            Err(SubagentError::AlreadyTerminal(_))
        ));
    }
}
