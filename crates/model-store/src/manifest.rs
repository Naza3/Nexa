use std::collections::BTreeMap;

use runtime_types::{ErrorCode, ModelId, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Result, gguf::Metadata, invalid_manifest};

const LLAMA_COMMIT: &str = "2149c00f4442dc59302e134a02e4c99d5f7ed9fc";
const MODEL_SHA256: &str = "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031";
const TEMPLATE_SHA256: &str = "57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361";
const MODEL_SIZE: u64 = 639_446_688;
const VALIDATED_CONTEXT: u32 = 2048;
const RESERVED: &[&str] = &[
    "schema_version",
    "id",
    "display_name",
    "relative_file",
    "size_bytes",
    "sha256",
    "source",
    "architecture",
    "quantization",
    "gguf_file_type",
    "template_sha256",
    "context_limit",
    "default_context",
    "validated_llama_commit",
    "capabilities",
    "validated",
    "validation",
];

/// Caller-supplied provenance. It is descriptive, never proof of verification.
/// Use a label or URI rather than unnecessarily persisting a full private path.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelSource {
    pub uri: String,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl ModelSource {
    pub fn local(label: impl Into<String>) -> Self {
        Self {
            uri: label.into(),
            revision: None,
            file_name: None,
            license: None,
            extra: BTreeMap::new(),
        }
    }
    fn validate(&self) -> Result<()> {
        if self.uri.trim().is_empty()
            || self.uri.len() > 4096
            || [&self.revision, &self.file_name, &self.license]
                .into_iter()
                .flatten()
                .any(|v| v.len() > 4096)
            || self
                .extra
                .keys()
                .any(|key| ["uri", "revision", "file_name", "license"].contains(&key.as_str()))
        {
            return Err(invalid_manifest("invalid or oversized model provenance"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ImportRequest {
    pub id: ModelId,
    pub display_name: String,
    pub source: ModelSource,
    pub default_context: u32,
    pub expected_sha256: Option<String>,
    /// Forward-compatible application metadata, excluding reserved manifest keys.
    pub extra: BTreeMap<String, Value>,
}
impl ImportRequest {
    pub fn new(id: ModelId, display_name: impl Into<String>, source: ModelSource) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            source,
            default_context: VALIDATED_CONTEXT,
            expected_sha256: None,
            extra: BTreeMap::new(),
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        validate_portable_id(&self.id)?;
        if self.display_name.trim().is_empty() || self.display_name.len() > 1024 {
            return Err(invalid_manifest("display name must contain 1..=1024 bytes"));
        }
        if !(32..=131_072).contains(&self.default_context) {
            return Err(invalid_manifest("default context outside 32..=131072"));
        }
        if self
            .expected_sha256
            .as_deref()
            .is_some_and(|hash| !is_hash(hash))
        {
            return Err(invalid_manifest(
                "expected SHA-256 must be 64 lowercase hexadecimal characters",
            ));
        }
        validate_extra(&self.extra)?;
        self.source.validate()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct Capabilities {
    pub chat: bool,
    pub streaming: bool,
    pub cancellation: bool,
}

/// Records the tested configuration, not an assertion about every platform or
/// thread count. Runtime composition must also enforce its backend/device scope.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ValidationEvidence {
    pub matrix_id: String,
    pub backend: String,
    pub platform: String,
    pub context_size: u32,
    pub threads: u32,
    pub evidence_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub id: ModelId,
    pub display_name: String,
    pub relative_file: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub source: ModelSource,
    pub architecture: String,
    pub quantization: String,
    pub gguf_file_type: u32,
    pub template_sha256: String,
    pub context_limit: u32,
    pub default_context: u32,
    #[serde(default)]
    pub validated_llama_commit: Option<String>,
    pub capabilities: Capabilities,
    #[serde(default)]
    pub validated: bool,
    #[serde(default)]
    pub validation: Option<ValidationEvidence>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl ModelManifest {
    pub(crate) fn build(
        request: ImportRequest,
        size: u64,
        hash: String,
        metadata: Metadata,
    ) -> Result<Self> {
        if request
            .expected_sha256
            .as_ref()
            .is_some_and(|expected| *expected != hash)
        {
            return Err(RuntimeError::new(
                ErrorCode::IntegrityFailure,
                "model SHA-256 does not match expected value",
            ));
        }
        if request.default_context > metadata.context_length {
            return Err(invalid_manifest(
                "default context exceeds GGUF context length",
            ));
        }
        let mut manifest = Self {
            schema_version: 1,
            id: request.id,
            display_name: request.display_name,
            relative_file: "model.gguf".into(),
            size_bytes: size,
            sha256: hash,
            source: request.source,
            architecture: metadata.architecture,
            quantization: quantization(metadata.file_type),
            gguf_file_type: metadata.file_type,
            template_sha256: format!("{:x}", Sha256::digest(metadata.template.as_bytes())),
            context_limit: metadata.context_length,
            default_context: request.default_context,
            validated_llama_commit: None,
            capabilities: Capabilities::default(),
            validated: false,
            validation: None,
            extra: request.extra,
        };
        if manifest.matches_matrix() {
            manifest.validated = true;
            manifest.validated_llama_commit = Some(LLAMA_COMMIT.into());
            manifest.capabilities = Capabilities {
                chat: true,
                streaming: true,
                cancellation: true,
            };
            manifest.validation = Some(known_evidence());
        }
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates the manifest's internal claims. File hash verification is a
    /// separate operation performed by ModelStore::resolve before execution.
    pub fn validate(&self) -> Result<()> {
        validate_portable_id(&self.id)?;
        if self.schema_version != 1
            || self.relative_file != "model.gguf"
            || self.size_bytes == 0
            || !is_hash(&self.sha256)
            || !is_hash(&self.template_sha256)
            || self.display_name.trim().is_empty()
            || self.display_name.len() > 1024
            || self.architecture.is_empty()
            || self.architecture.len() > 128
            || self.quantization != quantization(self.gguf_file_type)
            || self.context_limit < 32
            || !(32..=131_072).contains(&self.default_context)
            || self.default_context > self.context_limit
        {
            return Err(invalid_manifest(
                "manifest fields are missing, inconsistent, or unsupported",
            ));
        }
        self.source.validate()?;
        validate_extra(&self.extra)?;
        let has_claim = self.validated
            || self.validated_llama_commit.is_some()
            || self.validation.is_some()
            || self.capabilities != Capabilities::default();
        if has_claim
            && (!self.validated
                || !self.matches_matrix()
                || self.validated_llama_commit.as_deref() != Some(LLAMA_COMMIT)
                || self.validation != Some(known_evidence())
                || self.capabilities
                    != (Capabilities {
                        chat: true,
                        streaming: true,
                        cancellation: true,
                    }))
        {
            return Err(invalid_manifest(
                "manifest validation claim does not match the exact known model matrix",
            ));
        }
        Ok(())
    }

    pub(crate) fn matches_metadata(&self, metadata: &Metadata) -> bool {
        self.architecture == metadata.architecture
            && self.gguf_file_type == metadata.file_type
            && self.context_limit == metadata.context_length
            && self.template_sha256 == format!("{:x}", Sha256::digest(metadata.template.as_bytes()))
    }

    pub(crate) fn executable_context_limit(&self) -> u32 {
        if self.validated {
            VALIDATED_CONTEXT
        } else {
            self.context_limit.min(131_072)
        }
    }

    fn matches_matrix(&self) -> bool {
        self.sha256 == MODEL_SHA256
            && self.size_bytes == MODEL_SIZE
            && self.architecture == "qwen3"
            && self.gguf_file_type == 7
            && self.quantization == "Q8_0"
            && self.template_sha256 == TEMPLATE_SHA256
            && self.context_limit == 40_960
            && self.default_context == VALIDATED_CONTEXT
    }
}

fn known_evidence() -> ValidationEvidence {
    ValidationEvidence {
        matrix_id: "qwen3-0.6b-q8_0".into(),
        backend: "cpu".into(),
        platform: "windows-x86_64-ci-server2022".into(),
        context_size: VALIDATED_CONTEXT,
        threads: 2,
        evidence_url: "https://github.com/Naza3/Nexa/actions/runs/36791679663".into(),
    }
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate_extra(extra: &BTreeMap<String, Value>) -> Result<()> {
    if extra.keys().any(|key| RESERVED.contains(&key.as_str())) {
        return Err(invalid_manifest(
            "extension metadata shadows a reserved manifest field",
        ));
    }
    if serde_json::to_vec(extra)
        .map_err(|_| invalid_manifest("invalid extension metadata"))?
        .len()
        > 64 * 1024
    {
        return Err(invalid_manifest("extension metadata exceeds 64 KiB"));
    }
    Ok(())
}

/// IDs meet the shared protocol grammar. Reject Windows device names and trailing
/// dots too, so a registry copied between Windows and Android remains unambiguous.
pub(crate) fn validate_portable_id(id: &ModelId) -> Result<()> {
    let name = id.as_str();
    let stem = name.split('.').next().unwrap_or(name);
    let device = matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if name.ends_with('.') || device {
        return Err(RuntimeError::invalid(
            "model ID is not a portable file name",
        ));
    }
    Ok(())
}
fn quantization(file_type: u32) -> String {
    match file_type {
        0 => "F32".into(),
        1 => "F16".into(),
        2 => "Q4_0".into(),
        3 => "Q4_1".into(),
        7 => "Q8_0".into(),
        8 => "Q5_0".into(),
        9 => "Q5_1".into(),
        _ => format!("GGUF_FTYPE_{file_type}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn known() -> ModelManifest {
        ModelManifest {
            schema_version: 1,
            id: ModelId::new("renamed-known-artifact").unwrap(),
            display_name: "Known artifact".into(),
            relative_file: "model.gguf".into(),
            size_bytes: MODEL_SIZE,
            sha256: MODEL_SHA256.into(),
            source: ModelSource::local("user-selected local copy"),
            architecture: "qwen3".into(),
            quantization: "Q8_0".into(),
            gguf_file_type: 7,
            template_sha256: TEMPLATE_SHA256.into(),
            context_limit: 40960,
            default_context: 2048,
            validated_llama_commit: Some(LLAMA_COMMIT.into()),
            capabilities: Capabilities {
                chat: true,
                streaming: true,
                cancellation: true,
            },
            validated: true,
            validation: Some(known_evidence()),
            extra: BTreeMap::new(),
        }
    }
    #[test]
    fn exact_matrix_claim_requires_all_identity_and_configuration_fields() {
        // Manifest-only tests do not claim the model bytes have been loaded.
        let manifest = known();
        assert!(manifest.validate().is_ok());
        assert_eq!(manifest.executable_context_limit(), 2048);
        let mut alternatives = Vec::new();
        let mut m = known();
        m.sha256 = "0".repeat(64);
        alternatives.push(m);
        let mut m = known();
        m.size_bytes -= 1;
        alternatives.push(m);
        let mut m = known();
        m.template_sha256 = "0".repeat(64);
        alternatives.push(m);
        let mut m = known();
        m.default_context = 4096;
        alternatives.push(m);
        let mut m = known();
        m.context_limit = 131072;
        alternatives.push(m);
        let mut m = known();
        m.validated_llama_commit = Some("0".repeat(40));
        alternatives.push(m);
        let mut m = known();
        m.validation.as_mut().unwrap().threads = 4;
        alternatives.push(m);
        let mut m = known();
        m.validation.as_mut().unwrap().platform = "android-arm64".into();
        alternatives.push(m);
        let mut m = known();
        m.validated = false;
        alternatives.push(m);
        for mutated in alternatives {
            assert!(mutated.validate().is_err());
        }
    }
    #[test]
    fn missing_verification_fields_cannot_retain_validated_claim() {
        let mut json = serde_json::to_value(known()).unwrap();
        json.as_object_mut()
            .unwrap()
            .remove("validated_llama_commit");
        let parsed: ModelManifest = serde_json::from_value(json).unwrap();
        assert!(parsed.validate().is_err());
        let mut request = ImportRequest::new(
            ModelId::new("test").unwrap(),
            "Test",
            ModelSource::local("local"),
        );
        request.extra.insert("validated".into(), Value::Bool(true));
        assert!(request.validate().is_err());
    }
}
