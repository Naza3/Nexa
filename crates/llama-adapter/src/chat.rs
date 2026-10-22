use super::*;
use runtime_types::{
    GenerationRequest, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_CALLS, MAX_TOOL_CONTENT_BYTES,
    MAX_TOOL_OUTPUT_BYTES, ToolCall, ToolCallDelta, ToolChoice, ToolConfig,
};
use std::collections::BTreeSet;

pub enum GeneratedDelta<'a> {
    Text(&'a str),
    ToolCall(ToolCallDelta),
}

pub(super) struct ChatContext {
    tools: ToolConfig,
    id_prefix: String,
    history_ids: BTreeSet<String>,
}

impl Model<'_> {
    pub fn prepare_request<'model>(
        &'model mut self,
        request: &GenerationRequest,
        cancel: &CancelHandle,
    ) -> Result<Prepared<'model>, RuntimeError> {
        request.validate()?;
        if !request.uses_tools() {
            return self.prepare(&request.messages, &request.options, cancel);
        }
        if !request.options.stops.is_empty() {
            return Err(RuntimeError::invalid(
                "custom stops are unsupported with tools",
            ));
        }
        let calls: Vec<Vec<_>> = request
            .messages
            .iter()
            .map(|message| {
                message
                    .tool_calls
                    .iter()
                    .map(|call| ffi::AirToolCallV3 {
                        id: ffi::AirString::borrowed(&call.id),
                        name: ffi::AirString::borrowed(&call.name),
                        arguments: ffi::AirString::borrowed(&call.arguments),
                    })
                    .collect()
            })
            .collect();
        let messages: Vec<_> = request
            .messages
            .iter()
            .zip(&calls)
            .map(|(message, calls)| ffi::AirMessageV3 {
                role: ffi::AirString::borrowed(message.role.as_str()),
                has_content: u32::from(message.content.is_some()),
                content: ffi::AirString::borrowed(message.content.as_deref().unwrap_or_default()),
                calls: calls.as_ptr(),
                call_count: calls.len() as u64,
                tool_call_id: ffi::AirString::borrowed(
                    message.tool_call_id.as_deref().unwrap_or_default(),
                ),
            })
            .collect();
        let schemas: Vec<_> = request
            .tools
            .definitions
            .iter()
            .map(|tool| tool.parameters.to_string())
            .collect();
        let tools: Vec<_> = request
            .tools
            .definitions
            .iter()
            .zip(&schemas)
            .map(|(tool, schema)| ffi::AirToolV3 {
                name: ffi::AirString::borrowed(&tool.name),
                description: ffi::AirString::borrowed(
                    tool.description.as_deref().unwrap_or_default(),
                ),
                parameters_json: ffi::AirString::borrowed(schema),
            })
            .collect();
        let (choice, choice_name) = match &request.tools.choice {
            ToolChoice::None => (0, ""),
            ToolChoice::Auto => (1, ""),
            ToolChoice::Required => (2, ""),
            ToolChoice::Function(name) => (3, name.as_str()),
        };
        let mut raw = std::ptr::null_mut();
        let mut prompt_tokens = 0;
        let mut error = ffi::AirError::default();
        let options = ffi::AirGenerateOptions {
            max_tokens: request.options.max_tokens,
            temperature: request.options.temperature,
            top_p: request.options.top_p,
            seed: request.options.seed,
        };
        // SAFETY: All arrays borrow live request data for this synchronous call;
        // native copies the complete prompt and parser into the returned handle.
        let status = unsafe {
            ffi::air_prepare_chat_v3(
                self.raw.as_ptr(),
                messages.as_ptr(),
                messages.len() as u64,
                tools.as_ptr(),
                tools.len() as u64,
                choice,
                ffi::AirString::borrowed(choice_name),
                u32::from(request.tools.parallel_tool_calls),
                options,
                cancel.raw(),
                &mut raw,
                &mut prompt_tokens,
                &mut error,
            )
        };
        check_status(status, error).map_err(|mut error| {
            if error.code == ErrorCode::UnsupportedChatTemplate {
                error.code = ErrorCode::UnsupportedToolCalling;
            }
            error
        })?;
        Ok(Prepared {
            raw: Some(nonnull(raw)?),
            prompt_tokens,
            chat: Some(ChatContext {
                tools: request.tools.clone(),
                id_prefix: format!("call_{}_", request.request_id.to_string().replace('-', "")),
                history_ids: request
                    .messages
                    .iter()
                    .flat_map(|message| message.tool_calls.iter().map(|call| call.id.clone()))
                    .collect(),
            }),
            _model: PhantomData,
            _thread: PhantomData,
        })
    }
}

#[derive(Default)]
struct ChatBuffer {
    text: String,
    calls: Vec<ToolCall>,
    bytes: usize,
    error: Option<RuntimeError>,
    panic: Option<Box<dyn Any + Send>>,
}

unsafe fn borrowed<'a>(value: ffi::AirString, limit: usize) -> Result<&'a str, RuntimeError> {
    if value.len as u128 > limit as u128 || (value.len > 0 && value.data.is_null()) {
        return Err(protocol("invalid native chat callback buffer"));
    }
    if value.len == 0 {
        return Ok("");
    }
    // SAFETY: The ABI keeps the indicated bytes live until this callback returns.
    let bytes = unsafe { std::slice::from_raw_parts(value.data, value.len as usize) };
    std::str::from_utf8(bytes).map_err(|_| protocol("native chat output is not UTF-8"))
}
unsafe extern "C" fn chat_callback(
    user: *mut c_void,
    kind: u32,
    index: u32,
    name: ffi::AirString,
    payload: ffi::AirString,
) -> i32 {
    // SAFETY: Native calls synchronously with the unique stack buffer below.
    let state = unsafe { &mut *user.cast::<ChatBuffer>() };
    if state.error.is_some() || state.panic.is_some() {
        return 1;
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<(), RuntimeError> {
        // SAFETY: The borrowed bytes are consumed and copied only within this callback.
        let name = unsafe { borrowed(name, 64)? };
        let payload = unsafe {
            borrowed(
                payload,
                if kind == 0 {
                    MAX_TOOL_CONTENT_BYTES
                } else {
                    MAX_TOOL_ARGUMENT_BYTES
                },
            )?
        };
        let total = state
            .bytes
            .checked_add(name.len())
            .and_then(|n| n.checked_add(payload.len()))
            .filter(|n| *n <= MAX_TOOL_OUTPUT_BYTES)
            .ok_or_else(|| protocol("native chat aggregate exceeds limit"))?;
        match kind {
            0 if index == 0
                && name.is_empty()
                && state.text.is_empty()
                && state.calls.is_empty()
                && !payload.is_empty() =>
            {
                state.text = payload.to_owned();
            }
            1 if index as usize == state.calls.len()
                && state.calls.len() < MAX_TOOL_CALLS
                && !name.is_empty() =>
            {
                state.calls.push(ToolCall {
                    id: String::new(),
                    name: name.to_owned(),
                    arguments: payload.to_owned(),
                });
            }
            _ => return Err(protocol("invalid native chat event order")),
        }
        let retained = state
            .calls
            .iter()
            .try_fold(state.text.capacity(), |n, call| {
                n.checked_add(call.id.capacity() + call.name.capacity() + call.arguments.capacity())
            });
        if retained.is_none_or(|n| n > MAX_TOOL_OUTPUT_BYTES) {
            return Err(protocol("native chat retained capacity exceeds limit"));
        }
        state.bytes = total;
        Ok(())
    }));
    match outcome {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            state.error = Some(error);
            1
        }
        Err(panic) => {
            state.panic = Some(panic);
            1
        }
    }
}

impl Prepared<'_> {
    /// Tool-mode output is released only after native completion and whole-message
    /// validation. Deltas are bounded transport chunks, not live token streaming.
    pub fn generate_chat_observed<F, P>(
        mut self,
        cancel: &CancelHandle,
        mut on_delta: F,
        on_progress: P,
    ) -> Result<GenerationResult, GenerationError>
    where
        F: FnMut(GeneratedDelta<'_>) -> StreamControl,
        P: FnMut(GenerationProgress) -> StreamControl,
    {
        let Some(context) = self.chat.take() else {
            return self.generate_observed(
                cancel,
                |text| on_delta(GeneratedDelta::Text(text)),
                on_progress,
            );
        };
        let mut buffer = ChatBuffer::default();
        let mut progress = ProgressCallbackState {
            on_progress,
            panic: None,
            protocol_error: None,
            stopped: false,
        };
        let mut usage = ffi::AirUsage::default();
        let mut error = ffi::AirError::default();
        let raw = self.raw.take().expect("prepared handle is consumed once");
        // SAFETY: Native consumes the handle and all callbacks are synchronous,
        // panic-isolated, and copy their borrowed payload before returning.
        let status = unsafe {
            ffi::air_generate_chat_v3(
                raw.as_ptr(),
                cancel.raw(),
                chat_callback,
                (&mut buffer as *mut ChatBuffer).cast(),
                progress_callback::<P>,
                (&mut progress as *mut ProgressCallbackState<P>).cast(),
                &mut usage,
                &mut error,
            )
        };
        let result = check_status(status, error);
        if let Some(panic) = buffer.panic.take().or_else(|| progress.panic.take()) {
            resume_unwind(panic);
        }
        let native_finish = usage.finish_reason;
        let usage = Usage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        };
        let failure = |error| GenerationError { error, usage };
        if let Some(error) = buffer.error.take().or(progress.protocol_error) {
            return Err(failure(error));
        }
        result.map_err(failure)?;
        if progress.stopped {
            return Err(failure(protocol("native ignored progress consumer stop")));
        }
        for (index, call) in buffer.calls.iter_mut().enumerate() {
            call.id = format!("{}{index}", context.id_prefix);
            if context.history_ids.contains(&call.id) {
                return Err(failure(RuntimeError::new(
                    ErrorCode::InvalidToolOutput,
                    "generated tool ID collides with history",
                )));
            }
        }
        context
            .tools
            .validate_output(&buffer.calls)
            .map_err(failure)?;
        let total = buffer
            .calls
            .iter()
            .try_fold(buffer.text.capacity(), |n, call| {
                n.checked_add(call.id.capacity() + call.name.capacity() + call.arguments.capacity())
            });
        if total.is_none_or(|n| n > MAX_TOOL_OUTPUT_BYTES) {
            return Err(failure(RuntimeError::new(
                ErrorCode::ToolOutputLimitExceeded,
                "normalized output exceeds limit",
            )));
        }
        let finish_reason = match (native_finish, buffer.calls.is_empty()) {
            (0, true) => FinishReason::Stop,
            (4, false) => FinishReason::ToolCalls,
            _ => {
                return Err(failure(protocol(
                    "native chat finish reason disagrees with its output",
                )));
            }
        };
        let mut deliver = |delta| {
            if cancel.is_cancelled() {
                return Err(failure(RuntimeError::new(
                    ErrorCode::RequestCancelled,
                    "request cancelled",
                )));
            }
            if on_delta(delta) == StreamControl::Stop {
                return Err(failure(RuntimeError::new(
                    ErrorCode::ConsumerStopped,
                    "chat consumer stopped",
                )));
            }
            Ok(())
        };
        for text in utf8_chunks(&buffer.text) {
            deliver(GeneratedDelta::Text(text))?;
        }
        for (index, call) in buffer.calls.iter().enumerate() {
            deliver(GeneratedDelta::ToolCall(ToolCallDelta::Start {
                index: index as u32,
                id: call.id.clone(),
                name: call.name.clone(),
            }))?;
            for arguments in utf8_chunks(&call.arguments) {
                deliver(GeneratedDelta::ToolCall(ToolCallDelta::Arguments {
                    index: index as u32,
                    arguments: arguments.to_owned(),
                }))?;
            }
        }
        if cancel.is_cancelled() {
            return Err(failure(RuntimeError::new(
                ErrorCode::RequestCancelled,
                "request cancelled",
            )));
        }
        Ok(GenerationResult {
            usage,
            finish_reason,
        })
    }
}
fn utf8_chunks(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() {
            return None;
        }
        let mut end = text.len().min(4096);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let (chunk, rest) = text.split_at(end);
        text = rest;
        Some(chunk)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn send(state: &mut ChatBuffer, kind: u32, index: u32, name: &str, payload: &str) -> i32 {
        // SAFETY: Valid synchronous test buffers matching the native callback contract.
        unsafe {
            chat_callback(
                (state as *mut ChatBuffer).cast(),
                kind,
                index,
                ffi::AirString::borrowed(name),
                ffi::AirString::borrowed(payload),
            )
        }
    }
    #[test]
    fn complete_batch_is_retained_without_publishing_or_ids() {
        let mut state = ChatBuffer::default();
        assert_eq!(send(&mut state, 0, 0, "", "Hello 蓝色"), 0);
        assert_eq!(send(&mut state, 1, 0, "lookup", "{\"code\":\"B7\"}"), 0);
        assert_eq!(send(&mut state, 1, 1, "lookup", "{}"), 0);
        assert!(state.calls.iter().all(|call| call.id.is_empty()));
        assert_eq!(state.calls.len(), 2);
        assert_eq!(
            state.bytes,
            state.text.len()
                + state
                    .calls
                    .iter()
                    .map(|call| call.name.len() + call.arguments.len())
                    .sum::<usize>()
        );
    }
    #[test]
    fn invalid_order_or_oversized_call_fails_closed() {
        let mut state = ChatBuffer::default();
        assert_eq!(send(&mut state, 1, 1, "lookup", "{}"), 1);
        assert!(state.calls.is_empty());
        let mut state = ChatBuffer::default();
        assert_eq!(send(&mut state, 1, 0, "lookup", "{}"), 0);
        assert_eq!(send(&mut state, 0, 0, "", "late text"), 1);
        let mut state = ChatBuffer::default();
        assert_eq!(
            send(
                &mut state,
                1,
                0,
                "lookup",
                &"x".repeat(MAX_TOOL_ARGUMENT_BYTES + 1)
            ),
            1
        );
        assert!(state.calls.is_empty());
    }
    #[test]
    fn chunks_preserve_utf8_and_transport_limit() {
        let source = "蓝色🌈".repeat(2500);
        let chunks: Vec<_> = utf8_chunks(&source).collect();
        assert!(chunks.iter().all(|chunk| chunk.len() <= 4096));
        assert_eq!(chunks.concat(), source);
    }
}
