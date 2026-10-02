//! Deterministic actor/real blocking-task cancellation, without native inference.
use super::*;
use runtime_core::{CancellationHandle, ExecutionEvents, Executor, ExecutorCommand};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
struct NoInference(Arc<AtomicBool>);
impl Executor for NoInference {
    fn start(
        &mut self,
        _: ExecutorCommand,
        _: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        Err(RuntimeError::new(
            ErrorCode::UnsupportedModel,
            "no inference in this ownership test",
        ))
    }
    fn close(&mut self) -> Result<(), RuntimeError> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}
async fn fixture() -> (tempfile::TempDir, ApiState, Arc<AtomicBool>) {
    let root = tempfile::tempdir().unwrap();
    let store = ApiState::open_store(root.path().to_owned()).await.unwrap();
    let config = Config::default();
    let resolver = store.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let runtime = Runtime::spawn(
        config.runtime_config(),
        move |id: &ModelId| resolver.resolve(id),
        NoInference(closed.clone()),
    )
    .unwrap();
    let state = ApiState::new(runtime, store, config, None);
    state.initialize_registry().await.unwrap();
    (root, state, closed)
}
#[tokio::test]
async fn dropped_prepare_future_cancels_but_holds_registry_until_blocking_cleanup() {
    let (_root, state, closed) = fixture().await;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let owner = state.clone();
    let task = tokio::spawn(async move {
        owner
            .prepare_guarded(ModelId::new("fixture").unwrap(), move |control| {
                let _ = started_tx.send(control.clone());
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                control.check()
            })
            .await
    });
    let control = started_rx.await.unwrap();
    assert!(state.runtime.status().unwrap().registry_busy);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        control.check().unwrap_err().code,
        ErrorCode::ModelScanCancelled
    );
    assert!(state.runtime.status().unwrap().registry_busy);
    state.shutdown.begin();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), state.wait_shutdown())
            .await
            .is_err()
    );
    assert!(!closed.load(Ordering::SeqCst));
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), state.wait_shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(closed.load(Ordering::SeqCst));
}
#[tokio::test]
async fn shutdown_cancels_live_prepare_before_confirming_executor_close() {
    let (_root, state, closed) = fixture().await;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let owner = state.clone();
    let task = tokio::spawn(async move {
        owner
            .prepare_guarded(ModelId::new("fixture").unwrap(), move |control| {
                let _ = started_tx.send(control.clone());
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                control.check()
            })
            .await
    });
    let control = started_rx.await.unwrap();
    state.shutdown.begin();
    tokio::time::timeout(Duration::from_secs(1), async {
        while control.check().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(state.runtime.status().unwrap().registry_busy);
    assert!(!closed.load(Ordering::SeqCst));
    release_tx.send(()).unwrap();
    assert_eq!(
        task.await.unwrap().unwrap_err().error.code,
        "model_scan_cancelled"
    );
    state.wait_shutdown().await.unwrap();
    assert!(closed.load(Ordering::SeqCst));
}
#[test]
fn failed_preparation_changes_availability_and_page_generation() {
    let model = super::import_outcome_tests::model();
    let id = model.id.clone();
    let mut summary = ModelSummary::from(model);
    summary.available = true;
    let old = uuid::Uuid::new_v4();
    let registry = RwLock::new(RegistrySnapshot {
        models: vec![summary],
        generation: old,
    });
    publish_preparation_result(
        &registry,
        &id,
        &Err(RuntimeError::new(
            ErrorCode::ModelFileChanged,
            "private/path never serialized",
        )),
    )
    .unwrap();
    let observed = registry.read().unwrap();
    assert_ne!(observed.generation, old);
    assert!(!observed.models[0].available);
    assert_eq!(
        observed.models[0].availability_error.as_deref(),
        Some("model_file_changed")
    );
}
