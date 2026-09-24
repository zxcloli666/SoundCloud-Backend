use std::future::Future;
use std::time::Duration;

use tokio::task::{JoinError, JoinSet};
use tokio_util::sync::CancellationToken;

pub struct Supervisor {
    cancellation: CancellationToken,
    shutdown_grace: Duration,
    tasks: JoinSet<TaskExit>,
}

struct TaskExit {
    name: String,
    result: anyhow::Result<()>,
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("failed to install shutdown signal handler: {0}")]
    Signal(#[source] std::io::Error),

    #[error("shutdown signal stream closed")]
    SignalClosed,

    #[error("no supervised tasks are running")]
    NoTasks,

    #[error("supervised task {task} exited unexpectedly")]
    TaskExited { task: String },

    #[error("supervised task {task} failed: {source}")]
    TaskFailed {
        task: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("supervised task panicked or was cancelled: {0}")]
    TaskJoin(#[source] JoinError),
}

impl Supervisor {
    pub fn new(cancellation: CancellationToken, shutdown_grace: Duration) -> Self {
        Self {
            cancellation,
            shutdown_grace,
            tasks: JoinSet::new(),
        }
    }

    pub fn spawn<F, E>(&mut self, name: impl Into<String>, task: F)
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Into<anyhow::Error> + Send + 'static,
    {
        let name = name.into();
        self.tasks.spawn(async move {
            let result = task.await.map_err(Into::into);
            TaskExit { name, result }
        });
    }

    pub async fn run_until_signal(self) -> Result<(), SupervisorError> {
        self.run_until(shutdown_signal()).await
    }

    async fn run_until<F>(mut self, shutdown: F) -> Result<(), SupervisorError>
    where
        F: Future<Output = Result<(), SupervisorError>>,
    {
        tokio::pin!(shutdown);

        let failure = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => None,
            signal = &mut shutdown => signal.err(),
            task = self.tasks.join_next() => Some(unexpected_exit(task)),
        };

        self.cancellation.cancel();
        let drain_failure = self.drain().await;

        match failure.or(drain_failure) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn drain(&mut self) -> Option<SupervisorError> {
        let graceful = async {
            let mut failure = None;
            while let Some(task) = self.tasks.join_next().await {
                failure = failure.or_else(|| shutdown_failure(task));
            }
            failure
        };

        match tokio::time::timeout(self.shutdown_grace, graceful).await {
            Ok(failure) => failure,
            Err(_) => {
                tracing::warn!(
                    grace_seconds = self.shutdown_grace.as_secs_f64(),
                    remaining_tasks = self.tasks.len(),
                    "jobs shutdown grace expired"
                );
                self.tasks.abort_all();

                let mut failure = None;
                while let Some(task) = self.tasks.join_next().await {
                    failure = failure.or_else(|| shutdown_failure(task));
                }
                failure
            }
        }
    }
}

fn unexpected_exit(task: Option<Result<TaskExit, JoinError>>) -> SupervisorError {
    match task {
        Some(Ok(TaskExit {
            name,
            result: Ok(()),
        })) => SupervisorError::TaskExited { task: name },
        Some(Ok(TaskExit {
            name,
            result: Err(source),
        })) => SupervisorError::TaskFailed { task: name, source },
        Some(Err(error)) => SupervisorError::TaskJoin(error),
        None => SupervisorError::NoTasks,
    }
}

fn shutdown_failure(task: Result<TaskExit, JoinError>) -> Option<SupervisorError> {
    match task {
        Ok(TaskExit { result: Ok(()), .. }) => None,
        Ok(TaskExit {
            name,
            result: Err(source),
        }) => Some(SupervisorError::TaskFailed { task: name, source }),
        Err(error) if error.is_cancelled() => None,
        Err(error) => Some(SupervisorError::TaskJoin(error)),
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<(), SupervisorError> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).map_err(SupervisorError::Signal)?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.map_err(SupervisorError::Signal),
        received = terminate.recv() => match received {
            Some(()) => Ok(()),
            None => Err(SupervisorError::SignalClosed),
        },
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), SupervisorError> {
    tokio::signal::ctrl_c()
        .await
        .map_err(SupervisorError::Signal)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct DropMarker(Arc<AtomicBool>);

    impl Drop for DropMarker {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn unexpected_task_exit_is_fatal() {
        let cancellation = CancellationToken::new();
        let mut supervisor = Supervisor::new(cancellation, Duration::from_millis(50));
        supervisor.spawn("short-lived", async { Ok::<(), anyhow::Error>(()) });

        let result = supervisor
            .run_until(std::future::pending::<Result<(), SupervisorError>>())
            .await;

        assert!(matches!(
            result,
            Err(SupervisorError::TaskExited { task }) if task == "short-lived"
        ));
    }

    #[tokio::test]
    async fn cancellation_drains_cooperative_tasks() {
        let cancellation = CancellationToken::new();
        let child = cancellation.clone();
        let mut supervisor = Supervisor::new(cancellation, Duration::from_millis(50));
        supervisor.spawn("cooperative", async move {
            child.cancelled().await;
            Ok::<(), anyhow::Error>(())
        });

        assert!(supervisor.run_until(async { Ok(()) }).await.is_ok());
    }

    #[tokio::test]
    async fn task_panic_is_fatal() {
        let cancellation = CancellationToken::new();
        let mut supervisor = Supervisor::new(cancellation, Duration::from_millis(50));
        supervisor.spawn("panicking", async {
            if std::hint::black_box(true) {
                panic!("boom");
            }
            Ok::<(), anyhow::Error>(())
        });

        let result = supervisor
            .run_until(std::future::pending::<Result<(), SupervisorError>>())
            .await;

        assert!(matches!(result, Err(SupervisorError::TaskJoin(_))));
    }

    #[tokio::test]
    async fn shutdown_aborts_tasks_after_grace() {
        let dropped = Arc::new(AtomicBool::new(false));
        let marker = DropMarker(dropped.clone());
        let cancellation = CancellationToken::new();
        let mut supervisor = Supervisor::new(cancellation, Duration::from_millis(1));
        supervisor.spawn("stuck", async move {
            let _marker = marker;
            std::future::pending::<()>().await;
            Ok::<(), anyhow::Error>(())
        });

        let result = supervisor.run_until(async { Ok(()) }).await;

        assert!(result.is_ok());
        assert!(dropped.load(Ordering::Acquire));
    }
}
