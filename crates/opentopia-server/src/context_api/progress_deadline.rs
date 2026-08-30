use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::{sleep_until, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgressDeadlineExceeded {
    Idle { timeout: Duration },
    Absolute { timeout: Duration },
}

pub(super) fn record_progress(progress: &watch::Sender<u64>) {
    progress.send_modify(|revision| *revision = revision.wrapping_add(1));
}

pub(super) async fn await_with_progress_deadlines<F, T>(
    future: F,
    mut progress: watch::Receiver<u64>,
    idle_timeout: Duration,
    absolute_timeout: Duration,
) -> Result<T, ProgressDeadlineExceeded>
where
    F: Future<Output = T>,
{
    let absolute_deadline = Instant::now() + absolute_timeout;
    let mut progress_open = true;
    tokio::pin!(future);

    loop {
        let idle_deadline = Instant::now() + idle_timeout;
        tokio::select! {
            biased;
            output = &mut future => return Ok(output),
            _ = sleep_until(absolute_deadline) => {
                return Err(ProgressDeadlineExceeded::Absolute {
                    timeout: absolute_timeout,
                });
            }
            changed = progress.changed(), if progress_open => {
                progress_open = changed.is_ok();
            }
            _ = sleep_until(idle_deadline) => {
                return Err(ProgressDeadlineExceeded::Idle {
                    timeout: idle_timeout,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{await_with_progress_deadlines, record_progress, ProgressDeadlineExceeded};
    use std::time::Duration;

    #[tokio::test]
    async fn progress_renews_the_idle_timeout() {
        let (progress_sender, progress_receiver) = tokio::sync::watch::channel(0_u64);
        let result = await_with_progress_deadlines(
            async move {
                for _ in 0..5 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    record_progress(&progress_sender);
                }
                "complete"
            },
            progress_receiver,
            Duration::from_millis(80),
            Duration::from_secs(1),
        )
        .await;

        assert_eq!(result, Ok("complete"));
    }

    #[tokio::test]
    async fn silence_reaches_the_idle_timeout() {
        let (progress_sender, progress_receiver) = tokio::sync::watch::channel(0_u64);
        let result = await_with_progress_deadlines(
            async move {
                let _progress_sender = progress_sender;
                std::future::pending::<()>().await;
            },
            progress_receiver,
            Duration::from_millis(30),
            Duration::from_millis(500),
        )
        .await;

        assert_eq!(
            result,
            Err(ProgressDeadlineExceeded::Idle {
                timeout: Duration::from_millis(30),
            })
        );
    }

    #[tokio::test]
    async fn the_absolute_timeout_caps_an_active_stream() {
        let (progress_sender, progress_receiver) = tokio::sync::watch::channel(0_u64);
        let result = await_with_progress_deadlines(
            async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    record_progress(&progress_sender);
                }
            },
            progress_receiver,
            Duration::from_millis(80),
            Duration::from_millis(100),
        )
        .await;

        assert_eq!(
            result,
            Err(ProgressDeadlineExceeded::Absolute {
                timeout: Duration::from_millis(100),
            })
        );
    }
}
