use crate::*;
use model_store::{
    inventory,
    local_validation::{self, LocalValidation, Receipts, ValidationState},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

impl DesktopBridge {
    fn inventory_with_observations(&self) -> Result<(inventory::Inventory, Vec<LocalValidation>)> {
        let inventory =
            inventory::read(&self.root).map_err(|e| BridgeError::new(e.code.as_str()))?;
        let receipts = Receipts::read(&self.root).unwrap_or_default();
        let build = if receipts.entries.is_empty() {
            None
        } else {
            let stamps = local_validation::engine_file_stamps(&self.executable).ok();
            let mut cached = self.validation_build.lock().unwrap();
            if let Some(stamps) = stamps {
                if cached.as_ref().is_none_or(|(old, _)| old != &stamps) {
                    *cached = local_validation::engine_build(&self.executable)
                        .ok()
                        .map(|build| (stamps, build));
                }
                cached.as_ref().map(|(_, build)| build.clone())
            } else {
                None
            }
        };
        let preferences = settings::preferences(&self.root)?;
        let options = runtime_types::LoadOptions {
            context_size: preferences.context_size,
            threads: preferences.threads,
            batch_size: preferences.batch_size,
        };
        let observations = inventory
            .entries
            .iter()
            .map(|entry| {
                let scope = build.as_ref().and_then(|build| {
                    local_validation::scope(&self.root, entry, options, build).ok()
                });
                if let Some(scope) = scope {
                    receipts.observation(&scope, entry.availability_error.is_none())
                } else {
                    receipts
                        .entries
                        .iter()
                        .rev()
                        .find(|r| r.scope.model_id == entry.manifest.id)
                        .map(|r| LocalValidation {
                            state: ValidationState::Stale,
                            ..r.observation.clone()
                        })
                        .unwrap_or_default()
                }
            })
            .collect();
        Ok((inventory, observations))
    }
    pub(crate) fn local_observation(&self, id: &runtime_types::ModelId) -> Result<LocalValidation> {
        let (inventory, observations) = self.inventory_with_observations()?;
        Ok(inventory
            .entries
            .iter()
            .position(|entry| &entry.manifest.id == id)
            .map(|index| observations[index].clone())
            .unwrap_or_default())
    }
    pub(crate) fn local_models_page(
        &self,
        after: Option<&str>,
        generation: Option<Uuid>,
    ) -> Result<ModelsPage> {
        let (inventory, observations) = self.inventory_with_observations()?;
        let generation_now =
            overlay_generation(inventory.generation, &observations, ModelsSource::Local);
        if generation.is_some_and(|g| g != generation_now) {
            return Err(BridgeError::new("model_list_changed"));
        }
        let mut models = Vec::new();
        for (entry, observation) in inventory.entries.into_iter().zip(observations) {
            let mut summary: ModelSummary = serde_json::from_value(
                serde_json::to_value(runtime_api::dto::ModelSummary::from(entry.manifest))
                    .map_err(|_| BridgeError::new("response_invalid"))?,
            )
            .map_err(|_| BridgeError::new("response_invalid"))?;
            if let Some(error) = entry.availability_error {
                summary.available = false;
                summary.availability_error = Some(error.as_str().into());
            }
            summary.local_validation = Some(observation);
            if after.is_none_or(|after| summary.id.as_str() > after) {
                models.push(summary);
            }
        }
        let has_more = models.len() > 64;
        models.truncate(64);
        let next_after = if has_more {
            models.last().map(|m| m.id.to_string())
        } else {
            None
        };
        Ok(ModelsPage {
            source: ModelsSource::Local,
            generation: generation_now,
            data: models,
            next_after,
        })
    }
    pub(crate) fn runtime_observations(&self, page: &mut ModelsPage) -> Result<()> {
        let (inventory, observations) = self.inventory_with_observations()?;
        // Include the complete bounded local overlay so all pages share a
        // generation, including evidence/options changes between page reads.
        page.generation = overlay_generation(page.generation, &observations, ModelsSource::Runtime);
        for model in &mut page.data {
            model.local_validation = inventory
                .entries
                .iter()
                .position(|entry| {
                    entry.manifest.id == model.id && entry.manifest.sha256 == model.sha256
                })
                .map(|index| observations[index].clone());
        }
        page.source = ModelsSource::Runtime;
        Ok(())
    }
    pub async fn model_test(&self, request: LoadModelRequest) -> Result<LocalValidation> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        let body = load_body(request)?;
        tokio::select! {
            biased;
            _=self.closing_requested()=>{self.load_disconnected.store(true,Ordering::Release);Err(BridgeError::new("model_load_interrupted"))},
            result=self.json(Method::POST,"/runtime/model-test",Some(&body))=>result,
        }
    }
    /// Two stable, spaced observations; a refresh does no model hashing unless
    /// previously unseen completed files really need the existing transaction.
    pub fn models_reconcile(self: &Arc<Self>) -> Result<ModelsReconcile> {
        self.open()?;
        if self.library_active().is_some() {
            return Ok(reconcile("unchanged", None));
        }
        let Some(library) = model_store::library::ModelLibrary::read(&self.root)
            .map_err(|e| BridgeError::new(e.code.as_str()))?
        else {
            return Ok(reconcile("unchanged", None));
        };
        let observation = model_store::library::observe_candidates(&library)
            .map_err(|e| BridgeError::new(e.code.as_str()))?;
        if observation.is_empty() {
            return Ok(reconcile("unchanged", None));
        }
        if matches!(
            runtime_cli::instance::InstanceLock::observe(&self.root)
                .map_err(|_| BridgeError::new("instance_unavailable"))?,
            runtime_cli::instance::InstanceObservation::Running
        ) {
            return Ok(reconcile("pending", None));
        }
        let mut previous = self.candidate_observation.lock().unwrap();
        let now = std::time::Instant::now();
        match previous.as_ref() {
            Some((old, _, true)) if old == &observation => return Ok(reconcile("unchanged", None)),
            Some((old, since, false))
                if old == &observation && since.elapsed() >= Duration::from_secs(2) => {}
            Some((old, _, _)) if old == &observation => return Ok(reconcile("observing", None)),
            _ => {
                *previous = Some((observation, now, false));
                return Ok(reconcile("observing", None));
            }
        }
        // Failed scans are not repeated automatically until directory metadata
        // changes. Explicit scan remains the retry control.
        let handle = self.models_scan()?;
        *previous = Some((observation, now, true));
        drop(previous);
        Ok(reconcile("started", Some(handle.operation_id)))
    }
}
pub(crate) fn check_load_observation(value: &Value) -> Result<()> {
    let observation = value
        .get("local_validation")
        .ok_or_else(|| BridgeError::new("response_invalid"))?;
    let observation: LocalValidation = serde_json::from_value(observation.clone())
        .map_err(|_| BridgeError::new("response_invalid"))?;
    if observation.error_code.as_deref() == Some("validation_record_unavailable") {
        return Err(BridgeError::new("validation_record_unavailable"));
    }
    Ok(())
}
fn reconcile(status: &str, operation_id: Option<Uuid>) -> ModelsReconcile {
    ModelsReconcile {
        status: status.into(),
        operation_id,
    }
}
pub(crate) fn load_body(request: LoadModelRequest) -> Result<Value> {
    let id = runtime_types::ModelId::new(request.model_id)
        .map_err(|_| BridgeError::new("invalid_request"))?;
    runtime_types::LoadOptions {
        context_size: request.context_size,
        threads: request.threads,
        batch_size: request.batch_size,
    }
    .validate()
    .map_err(|_| BridgeError::new("settings_invalid"))?;
    Ok(
        json!({"model":id,"backend":"cpu","gpu_layers":0,"context_size":request.context_size,"threads":request.threads,"batch_size":request.batch_size}),
    )
}
fn overlay_generation(
    upstream: Uuid,
    observations: &[LocalValidation],
    source: ModelsSource,
) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(upstream.as_bytes());
    hash.update(match source {
        ModelsSource::Local => b"local".as_slice(),
        ModelsSource::Runtime => b"runtime".as_slice(),
    });
    hash.update(serde_json::to_vec(observations).unwrap_or_default());
    Uuid::from_bytes(hash.finalize()[..16].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bridge(root: &Path) -> DesktopBridge {
        DesktopBridge::new(
            root.to_owned(),
            root.parent().unwrap().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn stopped_inventory_does_not_initialize_spawn_recover_or_create_lock() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let b = bridge(&root);
        let page = b.models_page(None, None).await.unwrap();
        assert_eq!(page.source, ModelsSource::Local);
        assert!(page.data.is_empty());
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("imports")).unwrap();
        fs::write(root.join("imports/leave.partial"), b"leave").unwrap();
        let before = fs::read_dir(&root).unwrap().count();
        b.models_page(None, None).await.unwrap();
        assert_eq!(fs::read_dir(&root).unwrap().count(), before);
        assert!(!root.join("runtime").exists());
        assert!(!root.join("config.toml").exists());
    }
    #[tokio::test]
    async fn live_unproved_instance_never_falls_back_to_local_inventory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let b = bridge(&root);
        let _lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
        assert_eq!(
            b.models_page(None, None).await.unwrap_err().code,
            "connection_failed"
        );
    }
    #[tokio::test]
    async fn stale_discovery_never_becomes_stopped_inventory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let b = bridge(&root);
        drop(InstanceLock::try_acquire(&root).unwrap().unwrap());
        fs::write(root.join("runtime/instance.json"), b"stale").unwrap();
        assert_eq!(
            b.models_page(None, None).await.unwrap_err().code,
            "instance_unavailable"
        );
    }
    #[test]
    fn completed_candidate_observations_exclude_partial_control_and_symlink_files() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let scan = model_store::library::scan_directory(
            root.path(),
            source.path(),
            None,
            &model_store::library::ScanControl::default(),
        )
        .unwrap();
        let library = scan.library().unwrap().clone();
        drop(scan);
        for name in [
            "good.gguf",
            "partial.gguf.part",
            "browser.gguf.crdownload",
            "controlled.gguf",
            "controlled.gguf.aria2",
        ] {
            fs::write(source.path().join(name), b"bytes").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            source.path().join("good.gguf"),
            source.path().join("linked.gguf"),
        )
        .unwrap();
        let observed = model_store::library::observe_candidates(&library).unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].0, "good.gguf");
    }
    #[tokio::test]
    async fn reconcile_is_stable_bounded_and_failed_admission_can_retry() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let scan = model_store::library::scan_directory(
            root.path(),
            source.path(),
            None,
            &model_store::library::ScanControl::default(),
        )
        .unwrap();
        let library = scan.library().unwrap().clone();
        drop(scan);
        fs::write(
            root.path().join(model_store::library::LIBRARY_FILE),
            library.encode().unwrap(),
        )
        .unwrap();
        fs::write(source.path().join("new.gguf"), b"not-a-valid-model").unwrap();
        let b = Arc::new(bridge(root.path()));
        assert_eq!(b.models_reconcile().unwrap().status, "observing");
        assert_eq!(b.models_reconcile().unwrap().status, "observing");
        b.candidate_observation.lock().unwrap().as_mut().unwrap().1 =
            std::time::Instant::now() - Duration::from_secs(3);
        let work = b.work.lock().await;
        assert_eq!(b.models_reconcile().unwrap_err().code, "desktop_busy");
        drop(work);
        let started = b.models_reconcile().unwrap();
        assert_eq!(started.status, "started");
        loop {
            let state = b.library_next(started.operation_id.unwrap()).await.unwrap();
            if state.terminal {
                break;
            }
        }
        assert_eq!(b.models_reconcile().unwrap().status, "unchanged");
        assert!(!root.path().join("config.toml").exists());
    }
    #[tokio::test]
    async fn owned_scan_lock_keeps_last_published_inventory_readable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let source = tempfile::tempdir().unwrap();
        let (started, wait_started) = std::sync::mpsc::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        let wait_release = Mutex::new(wait_release);
        let b = Arc::new(bridge(&root).with_directory_validator(move |_| {
            started.send(()).unwrap();
            wait_release.lock().unwrap().recv().unwrap();
            Ok(())
        }));
        let handle = b.directory_apply(source.path().to_owned()).unwrap();
        tokio::task::spawn_blocking(move || {
            wait_started.recv_timeout(Duration::from_secs(2)).unwrap()
        })
        .await
        .unwrap();
        let page = b.models_page(None, None).await.unwrap();
        assert_eq!(page.source, ModelsSource::Local);
        assert!(page.data.is_empty());
        release.send(()).unwrap();
        loop {
            if b.library_next(handle.operation_id).await.unwrap().terminal {
                break;
            }
        }
        assert!(!root.join("config.toml").exists());
    }
    #[tokio::test]
    async fn evidence_beyond_first_page_and_pagination_scope_changes_are_consistent() {
        use model_store::local_validation::Receipt;
        use model_store::{ModelManifest, ModelSource, ModelStorage};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        fs::create_dir(&root).unwrap();
        for index in 0..65 {
            let id = runtime_types::ModelId::new(format!("fixture-{index:03}")).unwrap();
            let directory = root.join("models").join(id.as_str());
            fs::create_dir_all(&directory).unwrap();
            let manifest = ModelManifest {
                schema_version: 1,
                storage: ModelStorage::Managed,
                id,
                display_name: "Synthetic inventory".into(),
                relative_file: "model.gguf".into(),
                size_bytes: 64,
                sha256: "0".repeat(64),
                source: ModelSource::local("synthetic"),
                architecture: "qwen3".into(),
                quantization: "Q8_0".into(),
                gguf_file_type: 7,
                template_sha256: "1".repeat(64),
                context_limit: 40960,
                default_context: 2048,
                validated_llama_commit: None,
                capabilities: Default::default(),
                validated: false,
                validation: None,
                extra: Default::default(),
            };
            fs::write(
                directory.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fs::write(directory.join("model.gguf"), [0; 64]).unwrap();
        }
        let b = bridge(&root);
        fs::write(&b.executable, b"synthetic-runtime").unwrap();
        fs::write(
            b.executable.with_file_name(if cfg!(windows) {
                "ai-runtime-worker.exe"
            } else {
                "ai-runtime-worker"
            }),
            b"synthetic-worker",
        )
        .unwrap();
        let inventory = inventory::read(&root).unwrap();
        let last = inventory.entries.last().unwrap();
        let prefs = DesktopPreferences::default();
        let options = runtime_types::LoadOptions {
            context_size: prefs.context_size,
            threads: prefs.threads,
            batch_size: prefs.batch_size,
        };
        let scope = local_validation::scope(
            &root,
            last,
            options,
            &local_validation::engine_build(&b.executable).unwrap(),
        )
        .unwrap();
        let mut receipts = Receipts::default();
        receipts
            .record(
                &root,
                Receipt {
                    scope,
                    observation: LocalValidation {
                        state: ValidationState::Passed,
                        checked_at_unix_ms: Some(local_validation::now_ms()),
                        error_code: None,
                        load_success: true,
                        generation_pass: true,
                    },
                },
            )
            .unwrap();
        let first = b.models_page(None, None).await.unwrap();
        assert_eq!(first.data.len(), 64);
        let second = b
            .models_page(first.next_after.clone(), Some(first.generation))
            .await
            .unwrap();
        assert_eq!(second.data.len(), 1);
        assert_eq!(second.generation, first.generation);
        assert_eq!(
            second.data[0].local_validation.as_ref().unwrap().state,
            ValidationState::Passed
        );
        assert_eq!(
            b.local_observation(&last.manifest.id).unwrap().state,
            ValidationState::Passed
        );
        fs::write(&b.executable, b"replaced-runtime").unwrap();
        assert_eq!(
            b.models_page(first.next_after, Some(first.generation))
                .await
                .unwrap_err()
                .code,
            "model_list_changed"
        );
        assert_eq!(
            b.local_observation(&last.manifest.id).unwrap().state,
            ValidationState::Stale
        );
    }
    #[test]
    fn failed_receipt_publication_cannot_reuse_an_older_passing_load_badge() {
        let observation = LocalValidation {
            state: ValidationState::Loaded,
            error_code: Some("validation_record_unavailable".into()),
            load_success: true,
            ..LocalValidation::default()
        };
        assert_eq!(
            check_load_observation(&json!({"local_validation":observation}))
                .unwrap_err()
                .code,
            "validation_record_unavailable"
        );
        let deferred = LocalValidation {
            state: ValidationState::Deferred,
            ..observation
        };
        assert_eq!(
            check_load_observation(&json!({"local_validation":deferred}))
                .unwrap_err()
                .code,
            "validation_record_unavailable"
        );
        assert!(check_load_observation(&json!({})).is_err());
        assert!(check_load_observation(&json!({"local_validation":{"state":"made_up"}})).is_err());
    }
}
