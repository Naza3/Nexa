use crate::*;
use runtime_cli::instance::InstanceObservation;

impl DesktopBridge {
    pub async fn unregister_model(
        self: &std::sync::Arc<Self>,
        request: UnregisterModelRequest,
    ) -> Result<UnregisterModelResult> {
        self.open()?;
        let work = self
            .work
            .clone()
            .try_lock_owned()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        if request.generation.is_nil() {
            return Err(BridgeError::new("invalid_request"));
        }
        match InstanceLock::observe(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            InstanceObservation::Running => {
                let upstream = self
                    .list_generation
                    .lock()
                    .unwrap()
                    .filter(|(public, _)| *public == request.generation)
                    .ok_or_else(|| BridgeError::new("model_list_changed"))?
                    .1;
                let body = json!({"model_id": request.model_id, "generation": upstream});
                let result: UnregisterModelResult = self
                    .json(Method::POST, "/runtime/models/unregister", Some(&body))
                    .await?;
                if result.model_id != request.model_id || !result.removed || !result.files_preserved
                {
                    return Err(BridgeError::new("response_invalid"));
                }
                *self.list_generation.lock().unwrap() = None;
                Ok(result)
            }
            InstanceObservation::Stopped(observation) => {
                // Acquire a real mutation lock even when observe found no prior
                // lock file. Neither a failed proof nor stale discovery permits it.
                drop(observation);
                let bridge = self.clone();
                tokio::task::spawn_blocking(move || {
                    let _work = work;
                    let instance = InstanceLock::try_acquire(&bridge.root)
                        .map_err(|_| BridgeError::new("instance_unavailable"))?
                        .ok_or_else(|| BridgeError::new("runtime_running"))?;
                    if instance.has_discovery() {
                        return Err(BridgeError::new("runtime_stop_unconfirmed"));
                    }
                    let _catalog = model_store::unregister::CatalogLock::acquire(&bridge.root)
                        .map_err(|e| BridgeError::new(e.code.as_str()))?;
                    bridge.local_models_page(None, Some(request.generation))?;
                    let (library, _) =
                        model_store::unregister::prepare(&bridge.root, &request.model_id)
                            .map_err(|e| BridgeError::new(e.code.as_str()))?;
                    let mut committed = false;
                    let outcome = model_store::unregister::publish_with_commit(
                        &bridge.root,
                        &library,
                        || committed = true,
                    );
                    *bridge.list_generation.lock().unwrap() = None;
                    if let Err(error) = outcome {
                        return Err(BridgeError::new(if committed {
                            "model_unregister_durability_unconfirmed"
                        } else {
                            error.code.as_str()
                        }));
                    }
                    Ok(UnregisterModelResult {
                        model_id: request.model_id,
                        removed: true,
                        files_preserved: true,
                    })
                })
                .await
                .map_err(|_| BridgeError::new("model_library_write_failed"))?
            }
        }
    }
}
