use model_store::ImportCancellation;
use runtime_core::Runtime;
use runtime_types::{ErrorCode, RuntimeError};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone)]
pub struct ServiceShutdown(Arc<ShutdownInner>);
struct ShutdownInner {
    runtime: Mutex<Option<Runtime>>,
    requested: watch::Sender<bool>,
    finished: watch::Sender<Option<Result<(), RuntimeError>>>,
    import_cancel: ImportCancellation,
}
impl ServiceShutdown {
    pub fn new(runtime: Runtime, import_cancel: ImportCancellation) -> Self {
        Self(Arc::new(ShutdownInner {
            runtime: Mutex::new(Some(runtime)),
            requested: watch::channel(false).0,
            finished: watch::channel(None).0,
            import_cancel,
        }))
    }
    /// Idempotent for HTTP shutdown, Ctrl+C, bind/startup errors, and serve errors.
    /// The one reserved cleanup task does not queue behind API/storage permits.
    pub fn begin(&self) {
        if self.0.requested.send_replace(true) {
            return;
        }
        self.0.import_cancel.cancel();
        let runtime = self
            .0
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let inner = self.0.clone();
        tokio::spawn(async move {
            let result = match runtime {
                Some(runtime) => tokio::task::spawn_blocking(move || runtime.shutdown())
                    .await
                    .unwrap_or_else(|_| {
                        Err(RuntimeError::new(
                            ErrorCode::RuntimeFaulted,
                            "shutdown task failed",
                        ))
                    }),
                None => Err(RuntimeError::new(
                    ErrorCode::RuntimeFaulted,
                    "shutdown owner missing",
                )),
            };
            inner.finished.send_replace(Some(result));
        });
    }
    pub fn is_stopping(&self) -> bool {
        *self.0.requested.borrow()
    }
    pub async fn requested(&self) {
        let mut receiver = self.0.requested.subscribe();
        let _ = receiver.wait_for(|requested| *requested).await;
    }
    /// Success means core shutdown and OS-confirmed worker cleanup completed.
    /// Failure is deliberately preserved for HTTP and the CLI process exit code.
    pub async fn wait(&self) -> Result<(), RuntimeError> {
        let mut receiver = self.0.finished.subscribe();
        let result = receiver.wait_for(Option::is_some).await.map_err(|_| {
            RuntimeError::new(ErrorCode::RuntimeFaulted, "shutdown notification failed")
        })?;
        result.clone().unwrap()
    }
}
