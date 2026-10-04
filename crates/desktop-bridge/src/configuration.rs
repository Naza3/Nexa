use crate::*;
use runtime_api::configuration as shared;
use runtime_cli::instance::InstanceObservation;

impl DesktopBridge {
    pub fn ui_preferences_get(&self) -> Result<UiPreferencesSnapshot> {
        shared::ui_preferences_get(&self.root).map_err(BridgeError::from)
    }
    pub async fn ui_preferences_save(
        &self,
        request: UiPreferencesSaveRequest,
    ) -> Result<UiPreferencesSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        shared::ui_preferences_save(&self.root, request).map_err(BridgeError::from)
    }
    pub(crate) fn configuration_offline(&self) -> Result<ConfigurationSnapshot> {
        if !self.root.join("config.toml").exists() && !self.root.join("secrets/api-token").exists()
        {
            return Ok(shared::preview());
        }
        let document = shared::read(&self.root).map_err(BridgeError::from)?;
        shared::snapshot(&self.root, &document, None).map_err(BridgeError::from)
    }
    pub async fn configuration_get(&self) -> Result<ConfigurationSnapshot> {
        match InstanceLock::observe(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            InstanceObservation::Stopped(_guard) => self.configuration_offline(),
            InstanceObservation::Running => {
                self.json(Method::GET, "/runtime/configuration", None).await
            }
        }
    }
    pub async fn initialize(&self) -> Result<DesktopSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
            .ok_or_else(|| BridgeError::new("runtime_running"))?;
        if lock.has_discovery() {
            return Err(BridgeError::new("runtime_stop_unconfirmed"));
        }
        shared::initialize(&self.root).map_err(BridgeError::from)?;
        drop(lock);
        self.snapshot_inner().await
    }
    pub async fn configuration_save(
        &self,
        request: ConfigurationSaveRequest,
    ) -> Result<ConfigurationSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        settings::require_initialized(&self.root)?;
        match InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            None => {
                self.json(
                    Method::PUT,
                    "/runtime/configuration",
                    Some(
                        &serde_json::to_value(request)
                            .map_err(|_| BridgeError::new("settings_invalid"))?,
                    ),
                )
                .await
            }
            Some(lock) => {
                if lock.has_discovery() {
                    return Err(BridgeError::new("runtime_stop_unconfirmed"));
                }
                let limits = model_store::inventory::read(&self.root)
                    .map_err(|e| BridgeError::new(e.code.as_str()))?
                    .entries
                    .into_iter()
                    .map(|e| (e.manifest.id, e.manifest.context_limit))
                    .collect();
                let document =
                    shared::save(&self.root, request, None, &limits).map_err(BridgeError::from)?;
                shared::snapshot(&self.root, &document, None).map_err(BridgeError::from)
            }
        }
    }
    pub async fn configuration_migrate(
        &self,
        request: ConfigurationMigrateRequest,
    ) -> Result<ConfigurationSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
            .ok_or_else(|| BridgeError::new("runtime_running"))?;
        if lock.has_discovery() {
            return Err(BridgeError::new("runtime_stop_unconfirmed"));
        }
        settings::require_initialized(&self.root)?;
        let document = shared::migrate(&self.root, request).map_err(BridgeError::from)?;
        shared::snapshot(&self.root, &document, None).map_err(BridgeError::from)
    }
    pub async fn configuration_model_get(&self, model_id: String) -> Result<ModelConfiguration> {
        let id = runtime_types::ModelId::new(model_id)
            .map_err(|_| BridgeError::new("invalid_request"))?;
        match InstanceLock::observe(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            InstanceObservation::Running => {
                self.json(
                    Method::GET,
                    &format!("/runtime/configuration/models/{id}"),
                    None,
                )
                .await
            }
            InstanceObservation::Stopped(_guard) => {
                let document = shared::read(&self.root).map_err(BridgeError::from)?;
                let limit = model_store::inventory::read(&self.root)
                    .map_err(|e| BridgeError::new(e.code.as_str()))?
                    .entries
                    .iter()
                    .find(|e| e.manifest.id == id)
                    .map(|e| e.manifest.context_limit);
                shared::model_configuration(&document, id, None, limit).map_err(BridgeError::from)
            }
        }
    }
    pub async fn model_load_profile(
        &self,
        request: ModelLoadProfileRequest,
    ) -> Result<RuntimeStatus> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        let config: ConfigurationSnapshot = self
            .json(Method::GET, "/runtime/configuration", None)
            .await?;
        if config.schema_version != 2 {
            return Err(BridgeError::new("configuration_migration_required"));
        }
        if config.pending_restart {
            return Err(BridgeError::new("configuration_restart_required"));
        }
        let id = runtime_types::ModelId::new(request.model_id)
            .map_err(|_| BridgeError::new("invalid_request"))?;
        let mut body = serde_json::to_value(request.load_overrides)
            .map_err(|_| BridgeError::new("settings_invalid"))?;
        body["model"] = json!(id);
        body["backend"] = json!("cpu");
        body["gpu_layers"] = json!(0);
        tokio::select! {
            biased;
            _=self.closing_requested()=>{self.load_disconnected.store(true,Ordering::Release);Err(BridgeError::new("model_load_interrupted"))},
            result=self.json::<Value>(Method::POST,"/runtime/load-and-test",Some(&body))=>{
                let value=result?;onboarding::check_load_observation(&value)?;
                serde_json::from_value(value).map_err(|_|BridgeError::new("response_invalid"))
            }
        }
    }
}
