use crate::execution_spec::{ExecutionFailure, ExecutionStage};
use std::time::Duration;
use tokio::process::Child;
use tokio::process::Command;

pub(crate) async fn spawn_process(
    mut command: Command,
    timeout: Duration,
    program: &str,
) -> Result<Child, ExecutionFailure> {
    // If CreateProcess/posix_spawn finishes after the startup timeout, the
    // detached blocking task drops the returned child and kills it.
    command.kill_on_drop(true);
    let task = tokio::task::spawn_blocking(move || command.spawn());
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(Ok(child))) => Ok(child),
        Ok(Ok(Err(error))) => Err(ExecutionFailure::from_io(
            ExecutionStage::Spawn,
            format!("failed to spawn {program}: {error}"),
            &error,
        )),
        Ok(Err(error)) => Err(ExecutionFailure::without_os_error(
            ExecutionStage::Spawn,
            format!("spawn worker failed for {program}: {error}"),
        )),
        Err(_) => Err(ExecutionFailure::without_os_error(
            ExecutionStage::Spawn,
            format!(
                "starting {program} exceeded the startup timeout of {}ms",
                timeout.as_millis()
            ),
        )),
    }
}

pub(crate) async fn terminate_process(
    child: &mut Child,
    timeout: Duration,
) -> Result<(), ExecutionFailure> {
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(error) => {
            return Err(ExecutionFailure::from_io(
                ExecutionStage::Terminate,
                format!("failed to inspect root process before termination: {error}"),
                &error,
            ));
        }
    }
    if let Err(error) = child.kill().await {
        if child.try_wait().ok().flatten().is_none() {
            return Err(ExecutionFailure::from_io(
                ExecutionStage::Terminate,
                format!("failed to terminate root process: {error}"),
                &error,
            ));
        }
        return Ok(());
    }
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(ExecutionFailure::from_io(
            ExecutionStage::Terminate,
            format!("failed while confirming process termination: {error}"),
            &error,
        )),
        Err(_) => Err(ExecutionFailure::without_os_error(
            ExecutionStage::Terminate,
            format!(
                "root process did not exit within {}ms after termination",
                timeout.as_millis()
            ),
        )),
    }
}
