//! OS-specific child ownership. The supervisor alone spawns and reaps children.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;
#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("process-host supports Windows and the Linux development fallback only");
