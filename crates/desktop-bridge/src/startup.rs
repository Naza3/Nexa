//! The private startup channel is untrusted. Retain only fixed CLI codes;
//! never retain or return console text, paths, tokens, or configuration values.
use crate::BridgeError;
use std::io::{self, Read};
use tokio::sync::oneshot;

const MAX_LINE: usize = 256;
const PARSE_BUDGET: usize = 64 * 1024;

pub(crate) struct Capture(oneshot::Receiver<Option<&'static str>>);
impl Capture {
    pub(crate) fn start(pipe: impl Read + Send + 'static) -> io::Result<Self> {
        let (send, receive) = oneshot::channel();
        std::thread::Builder::new()
            .name("runtime-startup-report".into())
            .spawn(move || {
                let _ = send.send(drain(pipe));
            })?;
        Ok(Self(receive))
    }

    pub(crate) async fn exited(self) -> BridgeError {
        // A descendant could still own the pipe after the root exits. Never let
        // such a pipe block start/close, or stop draining a running service.
        let code = tokio::time::timeout(std::time::Duration::from_millis(200), self.0)
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .unwrap_or("runtime_start_failed");
        BridgeError::new(code)
    }
}

fn classify(line: &[u8]) -> Option<&'static str> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    Some(match line {
        b"nexa-startup-v1:runtime_loopback_bind_failed" => "runtime_loopback_bind_failed",
        b"nexa-startup-v1:configuration_unavailable" => "configuration_unavailable",
        b"nexa-startup-v1:configuration_invalid" => "configuration_invalid",
        b"nexa-startup-v1:runtime_instance_busy" => "runtime_instance_busy",
        b"nexa-startup-v1:configuration_busy" => "configuration_busy",
        b"nexa-startup-v1:packaged_runtime_missing" => "packaged_runtime_missing",
        b"nexa-startup-v1:runtime_security_invalid" => "runtime_security_invalid",
        b"nexa-startup-v1:runtime_start_failed" => "runtime_start_failed",
        _ => return None,
    })
}

fn drain(mut pipe: impl Read) -> Option<&'static str> {
    let mut buffer = [0; 4096];
    let mut line = [0; MAX_LINE];
    let mut length = 0;
    let mut overlong = false;
    let mut parsed = 0;
    let mut reason = None;
    loop {
        let count = match pipe.read(&mut buffer) {
            Ok(0) => {
                if !overlong && parsed < PARSE_BUDGET && reason.is_none() {
                    reason = classify(&line[..length]);
                }
                return reason;
            }
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return reason,
        };
        if parsed >= PARSE_BUDGET || reason.is_some() {
            continue;
        }
        for &byte in &buffer[..count] {
            if parsed >= PARSE_BUDGET || reason.is_some() {
                // Continue draining without allocating or parsing, including
                // after startup succeeded and the Capture receiver was dropped.
                continue;
            }
            parsed += 1;
            if byte == b'\n' {
                if !overlong {
                    reason = classify(&line[..length]);
                }
                length = 0;
                overlong = false;
            } else if length < MAX_LINE {
                line[length] = byte;
                length += 1;
            } else {
                overlong = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_lines_only_and_no_console_text_is_retained() {
        for suffix in ["\n", "\r\n", ""] {
            let text = format!("nexa-startup-v1:configuration_invalid{suffix}");
            assert_eq!(drain(text.as_bytes()), Some("configuration_invalid"));
        }
        for text in [
            "token=secret nexa-startup-v1:configuration_invalid\n",
            "nexa-startup-v1:configuration_invalid C:\\Users\\secret\n",
            "nexa-startup-v1:https://example.invalid/?token=secret\n",
            "configuration_invalid\n",
            "nexa-startup-v1:configuration_invalid\0\n",
        ] {
            assert_eq!(drain(text.as_bytes()), None);
        }
        for code in [
            "runtime_loopback_bind_failed",
            "configuration_unavailable",
            "configuration_busy",
            "runtime_instance_busy",
            "packaged_runtime_missing",
            "runtime_security_invalid",
            "runtime_start_failed",
        ] {
            let text = format!("unrecognized token=secret\nnexa-startup-v1:{code}\n");
            assert_eq!(drain(text.as_bytes()), Some(code));
        }
    }
    #[test]
    fn floods_are_drained_but_do_not_expand_or_bypass_parse_budget() {
        let valid = b"nexa-startup-v1:configuration_invalid\n";
        let mut text = vec![b'x'; MAX_LINE + 1];
        text.extend(valid);
        assert_eq!(drain(text.as_slice()), None);
        let mut text = vec![b'\n'; PARSE_BUDGET];
        text.extend(valid);
        let mut cursor = io::Cursor::new(text);
        assert_eq!(drain(&mut cursor), None);
        assert_eq!(cursor.position(), cursor.get_ref().len() as u64);
        let mut text = valid.to_vec();
        text.extend(vec![b'x'; PARSE_BUDGET * 2]);
        let mut cursor = io::Cursor::new(text);
        assert_eq!(drain(&mut cursor), Some("configuration_invalid"));
        assert_eq!(cursor.position(), cursor.get_ref().len() as u64);
    }
    #[tokio::test]
    async fn delayed_eof_does_not_block_startup_result() {
        let (send, receive) = oneshot::channel();
        let capture = Capture(receive);
        assert_eq!(capture.exited().await.code, "runtime_start_failed");
        assert!(send.send(Some("configuration_invalid")).is_err());
    }
    #[tokio::test]
    async fn dropping_successful_capture_keeps_draining_until_child_closes_pipe() {
        struct Reader {
            bytes: io::Cursor<Vec<u8>>,
            finished: Option<oneshot::Sender<u64>>,
        }
        impl Read for Reader {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.bytes.read(bytes)
            }
        }
        impl Drop for Reader {
            fn drop(&mut self) {
                let _ = self.finished.take().unwrap().send(self.bytes.position());
            }
        }
        let (send, receive) = oneshot::channel();
        let length = PARSE_BUDGET * 4;
        let capture = Capture::start(Reader {
            bytes: io::Cursor::new(vec![b'x'; length]),
            finished: Some(send),
        })
        .unwrap();
        drop(capture);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), receive)
                .await
                .unwrap()
                .unwrap(),
            length as u64,
        );
    }
}
