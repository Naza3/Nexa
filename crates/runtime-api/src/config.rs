use runtime_types::{GenerationOptions, LoadOptions, RuntimeConfig, RuntimeError};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf, time::Duration};

pub const MAX_BODY_BYTES: usize = 1_048_576;
pub const MAX_NONSTREAM_RESPONSE_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub api: ApiConfig,
    pub lan_api: crate::lan::LanApiConfig,
    pub runtime: SchedulingConfig,
    pub inference: InferenceConfig,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            api: ApiConfig::default(),
            lan_api: crate::lan::LanApiConfig::default(),
            runtime: SchedulingConfig::default(),
            inference: InferenceConfig::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiConfig {
    pub listen: SocketAddr,
    pub max_body_bytes: usize,
    pub token_file: PathBuf,
    pub trusted_origins: Vec<String>,
}
impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:18080".parse().unwrap(),
            max_body_bytes: MAX_BODY_BYTES,
            token_file: "secrets/api-token".into(),
            trusted_origins: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulingConfig {
    pub max_active_models: usize,
    pub max_running_jobs: usize,
    pub max_queued_jobs: usize,
    pub queue_timeout_seconds: u64,
    pub execution_timeout_seconds: u64,
    pub load_timeout_seconds: u64,
    pub idle_unload_seconds: u64,
    pub cancel_grace_seconds: u64,
}
impl Default for SchedulingConfig {
    fn default() -> Self {
        Self {
            max_active_models: 1,
            max_running_jobs: 1,
            max_queued_jobs: 8,
            queue_timeout_seconds: 120,
            execution_timeout_seconds: 300,
            load_timeout_seconds: 300,
            idle_unload_seconds: 300,
            cancel_grace_seconds: 5,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InferenceConfig {
    pub backend: String,
    pub context_size: u32,
    pub max_output_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub gpu_layers: u32,
    /// None means min(4, available_parallelism), falling back to one.
    pub threads: Option<u32>,
    pub batch_size: u32,
}
impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            backend: "cpu".into(),
            context_size: 4096,
            max_output_tokens: 512,
            temperature: 0.7,
            top_p: 0.9,
            gpu_layers: 0,
            threads: None,
            batch_size: 512,
        }
    }
}
impl Config {
    pub fn from_toml(text: &str) -> Result<Self, RuntimeError> {
        let config: Self = toml::from_str(text).map_err(|_| {
            RuntimeError::invalid("invalid configuration or unknown/duplicate configuration field")
        })?;
        config.validate()?;
        Ok(config)
    }
    pub fn to_toml(&self) -> Result<String, RuntimeError> {
        toml::to_string_pretty(self)
            .map_err(|_| RuntimeError::invalid("cannot encode configuration"))
    }
    pub fn validate(&self) -> Result<(), RuntimeError> {
        self.lan_api.validate()?;
        if self.schema_version != 1
            || !self.api.listen.ip().is_loopback()
            || self.api.max_body_bytes == 0
            || self.api.max_body_bytes > MAX_BODY_BYTES
        {
            return Err(RuntimeError::invalid(
                "configuration requires schema 1, loopback listen, and body limit 1..=1048576",
            ));
        }
        if self.api.token_file != std::path::Path::new("secrets/api-token") {
            return Err(RuntimeError::invalid(
                "token_file must be secrets/api-token",
            ));
        }
        if self.runtime.max_active_models != 1
            || self.runtime.max_running_jobs != 1
            || self.runtime.cancel_grace_seconds != 5
        {
            return Err(RuntimeError::invalid(
                "this build requires one model, one running job, and five-second cancellation grace",
            ));
        }
        if self.inference.backend != "cpu" || self.inference.gpu_layers != 0 {
            return Err(RuntimeError::invalid(
                "this build supports only backend cpu and gpu_layers=0",
            ));
        }
        self.runtime_config().validate()?;
        self.generation_options().validate()
    }
    pub fn available_parallelism() -> u32 {
        std::thread::available_parallelism()
            .map_or(1, |value| value.get().min(u32::MAX as usize) as u32)
    }
    pub fn load_options(&self) -> LoadOptions {
        LoadOptions {
            context_size: self.inference.context_size,
            threads: self
                .inference
                .threads
                .unwrap_or_else(|| Self::available_parallelism().min(4)),
            batch_size: self.inference.batch_size,
        }
    }
    pub fn generation_options(&self) -> GenerationOptions {
        GenerationOptions {
            max_tokens: self.inference.max_output_tokens,
            temperature: self.inference.temperature,
            top_p: self.inference.top_p,
            ..GenerationOptions::default()
        }
    }
    pub fn runtime_config(&self) -> RuntimeConfig {
        RuntimeConfig {
            max_queued_jobs: self.runtime.max_queued_jobs,
            queue_timeout: Duration::from_secs(self.runtime.queue_timeout_seconds),
            load_timeout: Duration::from_secs(self.runtime.load_timeout_seconds),
            execution_timeout: Duration::from_secs(self.runtime.execution_timeout_seconds),
            idle_unload: Duration::from_secs(self.runtime.idle_unload_seconds),
            load_options: self.load_options(),
            ..RuntimeConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documented_defaults_and_explicit_threads_are_preserved() {
        let mut c = Config::default();
        assert_eq!(c.inference.context_size, 4096);
        assert_eq!(
            c.load_options().threads,
            Config::available_parallelism().min(4)
        );
        c.inference.threads = Some(32);
        assert_eq!(c.load_options().threads, 32);
        assert_eq!(
            Config::from_toml(&c.to_toml().unwrap())
                .unwrap()
                .load_options()
                .threads,
            32
        );
    }
    #[test]
    fn public_listen_unknown_fields_and_unsupported_runtime_are_rejected() {
        for text in [
            "[api]\nlisten='0.0.0.0:18080'",
            "[api]\nsecret='x'",
            "[runtime]\nmax_running_jobs=2",
            "[inference]\nthreads=0",
        ] {
            assert!(Config::from_toml(text).is_err());
        }
    }
}
