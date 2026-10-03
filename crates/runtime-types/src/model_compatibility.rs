//! Historical model validation labels, separate from controlled load eligibility.
//! This observation is neither current file integrity nor permission to execute.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCompatibility {
    Admitted,
    ArchitectureUnsupported,
    QuantizationUnvalidated,
    TemplateUnvalidated,
    ContextUnvalidated,
    ArtifactUnvalidated,
    Unvalidated,
    /// Older services omit details; future statuses must not imply support.
    #[default]
    #[serde(other)]
    Unknown,
}
