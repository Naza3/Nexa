use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct DownloadSpec {
    pub url: url::Url,
    pub expected_size: u64,
    /// Passed as aria2 checksum defense in depth. The staging owner MUST also
    /// independently verify the completed protected file before publication.
    pub sha256: [u8; 32],
}
impl fmt::Debug for DownloadSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloadSpec")
            .field("expected_size", &self.expected_size)
            .finish_non_exhaustive()
    }
}
/// The caller must verify AND pin the executable and dependency identities for
/// the entire transfer. A path here is not an engine-provided trust assertion.
#[derive(Clone)]
pub struct SidecarConfig {
    pub executable: PathBuf,
}
impl fmt::Debug for SidecarConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SidecarConfig { executable: [redacted] }")
    }
}
/// The caller pins the staging directory until writer_stopped is confirmed.
/// Only the fixed basename payload.part is accepted; no arbitrary aria2 path.
#[derive(Clone)]
pub struct StagingPaths {
    pub directory: PathBuf,
    pub file_name: String,
}
impl fmt::Debug for StagingPaths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StagingPaths { directory: [redacted], file_name: payload.part }")
    }
}
#[derive(Clone, Copy, Debug)]
pub struct TransferOptions {
    /// Wrapper process start, NOT an aria2 internal retry counter. Only 1 or 2.
    pub attempt: u8,
}
impl Default for TransferOptions {
    fn default() -> Self {
        Self { attempt: 1 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadPhase {
    Connecting,
    Downloading,
    Verifying,
    Committing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadProgress {
    pub phase: DownloadPhase,
    /// aria2's bounded numerical completed-length report, not proof of durable
    /// writes or hash verification. No synthetic 100% is emitted on exit zero.
    pub written_bytes: u64,
    pub total_bytes: u64,
    pub attempt: u8,
}
impl DownloadProgress {
    pub fn initial(total_bytes: u64) -> Self {
        Self {
            phase: DownloadPhase::Connecting,
            written_bytes: 0,
            total_bytes,
            attempt: 1,
        }
    }
}
struct ControlInner {
    state: AtomicU8,
    deadline: Instant,
}
#[derive(Clone)]
pub struct DownloadControl(Arc<ControlInner>);
impl Default for DownloadControl {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for DownloadControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloadControl")
            .field("cancelled", &self.is_cancelled())
            .field("expired", &self.is_expired())
            .finish()
    }
}
impl DownloadControl {
    /// One shared two-hour deadline covers both permitted wrapper attempts AND
    /// caller-side verification. Reuse the same control for a range fallback.
    pub fn new() -> Self {
        Self::with_deadline(Instant::now() + Duration::from_secs(7200))
    }
    pub fn with_deadline(deadline: Instant) -> Self {
        Self(Arc::new(ControlInner {
            state: AtomicU8::new(0),
            deadline,
        }))
    }
    pub fn deadline(&self) -> Instant {
        self.0.deadline
    }
    pub fn cancel(&self) {
        let _ = self.try_cancel();
    }
    /// Returns true only for the actor that wins the initial cancellation CAS.
    /// Secondary watchdog failures must not overwrite an earlier user cancel.
    pub fn try_cancel(&self) -> bool {
        self.0
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.state.load(Ordering::Acquire) == 1
    }
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.0.deadline
    }
    /// Caller invokes this only after independent protected-file verification.
    /// Successful CAS wins over late cancellation; saved facts stay saved.
    pub fn begin_publish(&self) -> bool {
        !self.is_expired()
            && self
                .0
                .state
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferReport {
    pub exit_code: u32,
    pub writer_stopped: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadError {
    InvalidSpec,
    InvalidOptions,
    UnsupportedPlatform,
    SpawnFailed,
    Cancelled,
    Timeout,
    SidecarExit {
        exit_code: u32,
        error_code: Option<u32>,
    },
    PipeFailed,
    /// A supervisor panic prevents a positive cleanup assertion. The staging
    /// owner must keep its active flag and must not adopt/delete the payload.
    CleanupUnconfirmed,
}
impl DownloadError {
    pub fn writer_stopped(&self) -> bool {
        !matches!(self, Self::CleanupUnconfirmed)
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSpec => "invalid_spec",
            Self::InvalidOptions => "invalid_options",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::SpawnFailed => "spawn_failed",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::SidecarExit { .. } => "sidecar_exit",
            Self::PipeFailed => "pipe_failed",
            Self::CleanupUnconfirmed => "cleanup_unconfirmed",
        }
    }
}
impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for DownloadError {}
