use crate::DownloadProgress;
use std::{
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::sync::watch;
const MAX_LINE: usize = 2048;
const NO_ERROR: u32 = u32::MAX;
#[derive(Clone)]
pub(crate) struct OutputState {
    pub error: Arc<AtomicU32>,
}
impl Default for OutputState {
    fn default() -> Self {
        Self {
            error: Arc::new(AtomicU32::new(NO_ERROR)),
        }
    }
}
impl OutputState {
    pub fn error_code(&self) -> Option<u32> {
        let code = self.error.load(Ordering::Acquire);
        (code != NO_ERROR).then_some(code)
    }
}
/// Arbitrary bytes (including signed URLs and paths) are never stored beyond a
/// bounded current line, formatted, logged, or passed through to observers.
struct Parser {
    line: Vec<u8>,
    overflow: bool,
}
impl Parser {
    fn new() -> Self {
        Self {
            line: Vec::with_capacity(MAX_LINE),
            overflow: false,
        }
    }
    fn push(
        &mut self,
        bytes: &[u8],
        state: &OutputState,
        progress: &watch::Sender<DownloadProgress>,
    ) {
        for &byte in bytes {
            if byte == b'\r' || byte == b'\n' {
                if !self.overflow {
                    parse_line(&self.line, state, progress);
                }
                self.line.clear();
                self.overflow = false;
            } else if !self.overflow {
                if self.line.len() == MAX_LINE {
                    self.line.clear();
                    self.overflow = true;
                } else {
                    self.line.push(byte);
                }
            }
        }
    }
    fn finish(&mut self, state: &OutputState, progress: &watch::Sender<DownloadProgress>) {
        if !self.overflow {
            parse_line(&self.line, state, progress);
        }
        self.line.clear();
    }
}
fn number(value: &[u8]) -> Option<u64> {
    if value.is_empty() || value.len() > 20 || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    value.iter().try_fold(0u64, |n, b| {
        n.checked_mul(10)?.checked_add((b - b'0') as u64)
    })
}
fn progress_bytes(line: &[u8]) -> Option<(u64, u64)> {
    let rest = line.strip_prefix(b"[#")?;
    let end = rest.iter().position(|b| *b == b' ')?;
    if !(6..=16).contains(&end) || !rest[..end].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let rest = &rest[end + 1..];
    let slash = rest.iter().position(|b| *b == b'/')?;
    let received = number(rest[..slash].strip_suffix(b"B")?)?;
    let rest = &rest[slash + 1..];
    let end = rest.iter().position(|b| *b == b'(')?;
    let total = number(rest[..end].strip_suffix(b"B")?)?;
    Some((received, total))
}
fn parse_line(line: &[u8], state: &OutputState, progress: &watch::Sender<DownloadProgress>) {
    if let Some((bytes, total)) = progress_bytes(line) {
        progress.send_if_modified(|p| {
            if total != p.total_bytes || bytes > total || bytes < p.written_bytes {
                return false;
            }
            p.written_bytes = bytes;
            p.phase = crate::DownloadPhase::Downloading;
            true
        });
    }
    // Numeric aria2 diagnostic only. Never used for retry eligibility: that is
    // decided from the OS exit code, not potentially reflected log content.
    for index in 0..line.len().saturating_sub(10) {
        if !line[index..].starts_with(b"errorCode=") {
            continue;
        }
        let rest = &line[index + 10..];
        let end = rest
            .iter()
            .position(|b| !b.is_ascii_digit())
            .unwrap_or(rest.len());
        if let Some(code) = number(&rest[..end]).filter(|n| *n <= 32) {
            state.error.store(code as u32, Ordering::Release);
        }
    }
}
pub(crate) fn drain(
    mut input: impl Read,
    state: OutputState,
    progress: watch::Sender<DownloadProgress>,
) -> io::Result<()> {
    let mut parser = Parser::new();
    let mut buffer = [0u8; 4096];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => {
                parser.finish(&state, &progress);
                return Ok(());
            }
            Ok(n) => parser.push(&buffer[..n], &state, &progress),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_cr_lf_incremental_parser_never_exports_secrets() {
        let state = OutputState::default();
        let (tx, rx) = watch::channel(DownloadProgress::initial(100));
        let mut p = Parser::new();
        for b in b"CANARY https://secret.invalid/?token=secret\r\n[#abcdef 42B/100B(42%) CN:1]\r\nException: [file:1] errorCode=8 secret\n" {p.push(&[*b],&state,&tx);}
        assert_eq!(rx.borrow().written_bytes, 42);
        assert_eq!(state.error_code(), Some(8));
        assert!(!format!("{:?}", *rx.borrow()).contains("secret"));
    }
    #[test]
    fn oversized_utf8_or_malformed_lines_are_bounded_and_ignored() {
        let state = OutputState::default();
        let (tx, rx) = watch::channel(DownloadProgress::initial(100));
        let mut p = Parser::new();
        p.push(&vec![b'x'; MAX_LINE * 32], &state, &tx);
        assert!(p.line.len() <= MAX_LINE);
        p.push(b"[#abcdef 99B/100B(99%)]\r[#abcdef 25B/100B(25%)]\r[#abcdef 1B/100B(1%)]\n[#abcdef 101B/100B(101%)]\n[#abcdef 80B/999B(8%)]\n",&state,&tx);
        assert_eq!(rx.borrow().written_bytes, 25);
        p.push(&[0xff, 0xfe, b'\r'], &state, &tx);
        assert_eq!(rx.borrow().written_bytes, 25);
    }
    #[test]
    fn error_codes_are_bounded_and_overflow_rejected() {
        let state = OutputState::default();
        let (tx, _) = watch::channel(DownloadProgress::initial(100));
        parse_line(b"errorCode=9999999999999999999999999999999999", &state, &tx);
        assert_eq!(state.error_code(), None);
        parse_line(b"errorCode=32", &state, &tx);
        assert_eq!(state.error_code(), Some(32));
    }
}
