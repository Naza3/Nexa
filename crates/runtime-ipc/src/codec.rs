use crate::{Frame, MAX_TEXT_BYTES, Message, PROTOCOL_VERSION, protocol_error};
use runtime_core::ExecutorEvent;
use runtime_types::{ErrorCode, RuntimeError};
use std::io::{BufRead, Write};

/// 4 KiB text may expand to 24 KiB of JSON escapes, plus at most 1 KiB of
/// envelope. This tighter bound is part of the conservative text-credit proof.
pub const MAX_TEXT_FRAME_BYTES: usize = 25 * 1024;

pub fn encode_frame(frame: &Frame, limit: usize) -> Result<Vec<u8>, RuntimeError> {
    validate(frame)?;
    let mut bytes =
        serde_json::to_vec(frame).map_err(|_| protocol_error("frame serialization failed"))?;
    bytes.push(b'\n');
    check_size(frame, bytes.len(), limit)?;
    Ok(bytes)
}
pub fn write_frame<W: Write>(
    writer: &mut W,
    frame: &Frame,
    limit: usize,
) -> Result<(), RuntimeError> {
    // Validate the ENTIRE encoded frame before writing its first byte.
    let bytes = encode_frame(frame, limit)?;
    writer
        .write_all(&bytes)
        .and_then(|()| writer.flush())
        .map_err(|_| RuntimeError::new(ErrorCode::Io, "worker pipe write failed"))
}
pub fn read_frame<R: BufRead>(reader: &mut R, limit: usize) -> Result<Option<Frame>, RuntimeError> {
    let mut bytes = Vec::with_capacity(limit.min(4096));
    loop {
        let available = reader
            .fill_buf()
            .map_err(|_| RuntimeError::new(ErrorCode::Io, "worker pipe read failed"))?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            return Err(protocol_error("truncated frame at EOF"));
        }
        let end = available.iter().position(|b| *b == b'\n');
        let take = end.map_or(available.len(), |i| i + 1);
        if take > limit.saturating_sub(bytes.len()) {
            return Err(protocol_error("encoded frame exceeds byte limit"));
        }
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if end.is_some() {
            if bytes.len() == 1 {
                return Err(protocol_error("empty NDJSON frame"));
            }
            // serde_json rejects malformed UTF-8, unknown kinds and malformed JSON.
            let frame: Frame = serde_json::from_slice(&bytes)
                .map_err(|_| protocol_error("malformed worker frame"))?;
            validate(&frame)?;
            check_size(&frame, bytes.len(), limit)?;
            return Ok(Some(frame));
        }
    }
}
fn check_size(frame: &Frame, size: usize, limit: usize) -> Result<(), RuntimeError> {
    if size > limit {
        return Err(protocol_error("encoded frame exceeds byte limit"));
    }
    if matches!(&frame.message, Message::Generate { request }
        if request.messages.iter().all(|message| message.image.is_none()))
        && size > 2 * 1024 * 1024
    {
        return Err(protocol_error("encoded text request exceeds byte limit"));
    }
    if matches!(
        frame.message,
        Message::Event {
            event: ExecutorEvent::TextDelta(_) | ExecutorEvent::ToolCallDelta(_),
            ..
        }
    ) && size > MAX_TEXT_FRAME_BYTES
    {
        return Err(protocol_error(
            "encoded payload frame exceeds credit accounting bound",
        ));
    }
    Ok(())
}
fn validate(frame: &Frame) -> Result<(), RuntimeError> {
    if frame.protocol_version != PROTOCOL_VERSION || frame.session_id.is_nil() {
        return Err(protocol_error("invalid protocol version or session"));
    }
    match &frame.message {
        Message::Hello(hello) => {
            if frame.operation_id != 0 || frame.request_id.is_some() || frame.seq.is_some() {
                return Err(protocol_error("invalid Hello identity"));
            }
            hello.validate()?;
        }
        Message::Shutdown {} => {
            if frame.operation_id != 0 || frame.request_id.is_some() || frame.seq.is_some() {
                return Err(protocol_error("invalid Shutdown identity"));
            }
        }
        Message::Event { event, credit_id } => {
            if matches!(event, ExecutorEvent::CleanupUnconfirmed(_))
                || matches!(event, ExecutorEvent::Failed(error) | ExecutorEvent::Faulted(error) | ExecutorEvent::GenerationFailed { error, .. } if error.code == ErrorCode::ExecutorCleanupUnconfirmed)
            {
                return Err(protocol_error(
                    "parent-only cleanup event is forbidden on wire",
                ));
            }
            if frame.operation_id == 0 || frame.seq.is_none_or(|seq| seq == 0) {
                return Err(protocol_error("invalid event identity"));
            }
            if let ExecutorEvent::TextDelta(text) = event {
                if text.is_empty()
                    || text.len() > MAX_TEXT_BYTES
                    || credit_id.is_none_or(|id| id == 0)
                {
                    return Err(protocol_error("invalid text delta or credit"));
                }
            } else if let ExecutorEvent::ToolCallDelta(delta) = event {
                if delta.validate_piece().is_err() || credit_id.is_none_or(|id| id == 0) {
                    return Err(protocol_error("invalid tool delta or credit"));
                }
            } else if credit_id.is_some() {
                return Err(protocol_error("credit attached to non-payload event"));
            }
        }
        message => {
            if frame.operation_id == 0 || frame.seq.is_some() {
                return Err(protocol_error("invalid command identity"));
            }
            match message {
                Message::Load { options, .. } => {
                    if frame.request_id.is_some() {
                        return Err(protocol_error("Load has request identity"));
                    }
                    options
                        .validate()
                        .map_err(|_| protocol_error("invalid Load options"))?;
                }
                Message::Generate { request } => {
                    if frame.request_id != Some(request.request_id) {
                        return Err(protocol_error("Generate request identity mismatch"));
                    }
                    request
                        .validate()
                        .map_err(|_| protocol_error("invalid Generate payload"))?;
                }
                Message::Unload {} => {
                    if frame.request_id.is_some() {
                        return Err(protocol_error("Unload has request identity"));
                    }
                }
                Message::Credit { credit_id } => {
                    if *credit_id == 0 || frame.request_id.is_none() {
                        return Err(protocol_error("invalid credit identity"));
                    }
                }
                Message::Cancel {} => {}
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}
