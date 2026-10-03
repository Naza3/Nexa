#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::Child;
#[cfg(all(unix, test))]
mod unix;
#[cfg(all(unix, test))]
pub(crate) use unix::Child;
