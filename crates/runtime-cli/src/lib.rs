//! Native-free local management client and instance lifecycle.
pub mod client;
pub mod command;
pub mod instance;
mod startup_error;

/// Private desktop launch hint. It enables only a bounded diagnostic on stdout;
/// it changes no authentication, process containment, or service permissions.
pub const DESKTOP_STARTUP_REPORT_ENV: &str = "NEXA_DESKTOP_STARTUP_REPORT";

/// Only the final error of a desktop-launched `serve` invocation uses this
/// channel. stderr remains null, including stderr inherited by native workers.
/// A closed desktop/pipe is harmless: reporting cannot panic or change cleanup.
pub fn report_desktop_startup_error(
    error: &(dyn std::error::Error + Send + Sync + 'static),
    mut output: impl std::io::Write,
) {
    let line = startup_error::report_line(error);
    let _ = output.write_all(line);
}

#[cfg(test)]
mod startup_report_tests {
    use super::*;
    #[test]
    fn startup_report_is_fixed_bounded_and_never_contains_source_error() {
        for message in ["token=secret /private/path".to_owned(), "x".repeat(100_000)] {
            let error = std::io::Error::other(message);
            let mut output = Vec::new();
            report_desktop_startup_error(&error, &mut output);
            assert_eq!(output, b"nexa-startup-v1:runtime_start_failed\n");
        }
        let error = startup_error::StartupError::LoopbackBind(std::io::ErrorKind::AddrInUse.into());
        let mut output = Vec::new();
        report_desktop_startup_error(&error, &mut output);
        assert_eq!(output, b"nexa-startup-v1:runtime_loopback_bind_failed\n");
    }
    #[test]
    fn a_closed_desktop_diagnostic_reader_does_not_panic_or_require_retry() {
        struct Closed;
        impl std::io::Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        report_desktop_startup_error(&std::io::Error::other("configuration_invalid"), Closed);
    }
}
