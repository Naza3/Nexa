//! Native-free local management client and instance lifecycle.
pub mod client;
pub mod command;
pub mod instance;

/// Private desktop launch hint. It enables only a bounded diagnostic on stdout;
/// it changes no authentication, process containment, or service permissions.
pub const DESKTOP_STARTUP_REPORT_ENV: &str = "NEXA_DESKTOP_STARTUP_REPORT";

/// Only the final error of a desktop-launched `serve` invocation uses this
/// channel. stderr remains null, including stderr inherited by native workers.
/// A closed desktop/pipe is harmless: reporting cannot panic or change cleanup.
pub fn report_desktop_startup_error(
    error: &(dyn std::error::Error + Send + Sync),
    mut output: impl std::io::Write,
) {
    struct Text {
        bytes: [u8; 256],
        length: usize,
    }
    impl std::fmt::Write for Text {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            let end = self
                .length
                .checked_add(value.len())
                .ok_or(std::fmt::Error)?;
            if end > self.bytes.len() {
                return Err(std::fmt::Error);
            }
            self.bytes[self.length..end].copy_from_slice(value.as_bytes());
            self.length = end;
            Ok(())
        }
    }
    let mut text = Text {
        bytes: [0; 256],
        length: 0,
    };
    let valid = std::fmt::write(&mut text, format_args!("{error}")).is_ok();
    let line: &[u8] = match valid.then_some(&text.bytes[..text.length]) {
        Some(b"cannot bind configured loopback endpoint") => {
            b"nexa-startup-v1:runtime_loopback_bind_failed\n"
        }
        Some(b"configuration_unavailable") => b"nexa-startup-v1:configuration_unavailable\n",
        Some(b"configuration_invalid") => b"nexa-startup-v1:configuration_invalid\n",
        Some(b"another instance owns this data directory") => {
            b"nexa-startup-v1:runtime_instance_busy\n"
        }
        Some(b"configuration_busy") => b"nexa-startup-v1:configuration_busy\n",
        Some(
            b"packaged worker is missing beside ai-runtime"
            | b"packaged worker must be a regular file beside ai-runtime",
        ) => b"nexa-startup-v1:packaged_runtime_missing\n",
        Some(
            b"invalid server security configuration"
            | b"LAN and management credentials must be independent",
        ) => b"nexa-startup-v1:runtime_security_invalid\n",
        _ => b"nexa-startup-v1:runtime_start_failed\n",
    };
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
        let error = std::io::Error::other("cannot bind configured loopback endpoint");
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
