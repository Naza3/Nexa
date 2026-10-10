//! Internal startup context, separate from the fixed desktop diagnostic wire format.
use std::{error::Error, fmt, io};

pub(crate) enum StartupError {
    LoopbackBind(io::Error),
    InstanceBusy,
    WorkerMissing(io::Error),
    WorkerNotRegular,
    SecurityConfiguration(runtime_api::ApiError),
    CredentialsNotIndependent,
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LoopbackBind(_) => "cannot bind configured loopback endpoint",
            Self::InstanceBusy => "another instance owns this data directory",
            Self::WorkerMissing(_) => "packaged worker is missing beside ai-runtime",
            Self::WorkerNotRegular => "packaged worker must be a regular file beside ai-runtime",
            Self::SecurityConfiguration(_) => "invalid server security configuration",
            Self::CredentialsNotIndependent => "LAN and management credentials must be independent",
        })
    }
}
impl fmt::Debug for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Sources remain inspectable, but default diagnostics never render them.
        f.debug_struct("StartupError")
            .field("context", &format_args!("{self}"))
            .finish_non_exhaustive()
    }
}
impl Error for StartupError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LoopbackBind(source) | Self::WorkerMissing(source) => Some(source),
            Self::SecurityConfiguration(source) => Some(source),
            Self::InstanceBusy | Self::WorkerNotRegular | Self::CredentialsNotIndependent => None,
        }
    }
}

pub(crate) fn report_line(error: &(dyn Error + 'static)) -> &'static [u8] {
    let mut current = Some(error);
    // Context wrappers may be added by application boundaries. Bound traversal
    // even if an unexpected Error implementation returns a cyclic source.
    for _ in 0..16 {
        let Some(error) = current else { break };
        if let Some(error) = error.downcast_ref::<StartupError>() {
            return match error {
                StartupError::LoopbackBind(_) => b"nexa-startup-v1:runtime_loopback_bind_failed\n",
                StartupError::InstanceBusy => b"nexa-startup-v1:runtime_instance_busy\n",
                StartupError::WorkerMissing(_) | StartupError::WorkerNotRegular => {
                    b"nexa-startup-v1:packaged_runtime_missing\n"
                }
                StartupError::SecurityConfiguration(_)
                | StartupError::CredentialsNotIndependent => {
                    b"nexa-startup-v1:runtime_security_invalid\n"
                }
            };
        }
        if let Some(error) = error.downcast_ref::<runtime_api::configuration::ConfigurationError>()
        {
            // This is a typed domain code, never Display or arbitrary error text.
            return match error.code {
                "configuration_unavailable" => b"nexa-startup-v1:configuration_unavailable\n",
                "configuration_invalid" => b"nexa-startup-v1:configuration_invalid\n",
                "configuration_busy" => b"nexa-startup-v1:configuration_busy\n",
                _ => b"nexa-startup-v1:runtime_start_failed\n",
            };
        }
        current = error.source();
    }
    b"nexa-startup-v1:runtime_start_failed\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Context(Box<dyn Error + Send + Sync>);
    impl fmt::Display for Context {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("startup classification must never format arbitrary errors")
        }
    }
    impl Error for Context {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self.0.as_ref())
        }
    }

    #[test]
    fn startup_context_retains_source_without_rendering_it() {
        let source = io::Error::other("secret-token /private/model/path");
        let error = StartupError::LoopbackBind(source);
        assert!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .is_some()
        );
        assert_eq!(
            error.to_string(),
            "cannot bind configured loopback endpoint"
        );
        assert!(!format!("{error:?} {error}").contains("secret-token"));
        assert!(!format!("{error:?} {error}").contains("/private"));
        let wrapped = Context(Box::new(Context(Box::new(error))));
        assert_eq!(
            report_line(&wrapped),
            b"nexa-startup-v1:runtime_loopback_bind_failed\n"
        );
    }

    #[test]
    fn typed_startup_variants_keep_original_messages_and_codes() {
        for (error, message, code) in [
            (
                StartupError::InstanceBusy,
                "another instance owns this data directory",
                "runtime_instance_busy",
            ),
            (
                StartupError::WorkerMissing(io::ErrorKind::NotFound.into()),
                "packaged worker is missing beside ai-runtime",
                "packaged_runtime_missing",
            ),
            (
                StartupError::WorkerNotRegular,
                "packaged worker must be a regular file beside ai-runtime",
                "packaged_runtime_missing",
            ),
            (
                StartupError::SecurityConfiguration(runtime_api::ApiError::internal()),
                "invalid server security configuration",
                "runtime_security_invalid",
            ),
            (
                StartupError::CredentialsNotIndependent,
                "LAN and management credentials must be independent",
                "runtime_security_invalid",
            ),
        ] {
            assert_eq!(error.to_string(), message);
            assert_eq!(
                report_line(&error),
                format!("nexa-startup-v1:{code}\n").as_bytes()
            );
        }
    }

    #[test]
    fn configuration_classification_survives_context_wrappers() {
        for code in [
            "configuration_unavailable",
            "configuration_invalid",
            "configuration_busy",
        ] {
            let error = Context(Box::new(runtime_api::configuration::ConfigurationError {
                code,
                param: None,
                reason: None,
            }));
            assert_eq!(
                report_line(&error),
                format!("nexa-startup-v1:{code}\n").as_bytes()
            );
        }
    }

    #[test]
    fn matching_display_is_not_authority_to_classify_an_error() {
        for message in [
            "cannot bind configured loopback endpoint",
            "configuration_unavailable",
            "configuration_invalid",
            "configuration_busy",
            "another instance owns this data directory",
            "packaged worker is missing beside ai-runtime",
            "packaged worker must be a regular file beside ai-runtime",
            "invalid server security configuration",
            "LAN and management credentials must be independent",
        ] {
            let error = Context(Box::new(io::Error::other(message)));
            assert_eq!(
                report_line(&error),
                b"nexa-startup-v1:runtime_start_failed\n"
            );
        }
    }

    #[test]
    fn cyclic_sources_are_bounded_without_formatting() {
        #[derive(Debug)]
        struct Cycle;
        impl fmt::Display for Cycle {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("must not render cyclic error")
            }
        }
        impl Error for Cycle {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(self)
            }
        }
        assert_eq!(
            report_line(&Cycle),
            b"nexa-startup-v1:runtime_start_failed\n"
        );
    }
}
