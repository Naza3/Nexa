//! One bounded validator reused by the actor and the private IPC receiver.
//! It never executes tools and never exposes argument text in an error.
use runtime_types::{
    ErrorCode, FinishReason, GenerationRequest, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_CALLS,
    MAX_TOOL_OUTPUT_BYTES, RuntimeError, ToolCall, ToolCallDelta, ToolConfig,
};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct ToolStreamValidator {
    enabled: bool,
    config: ToolConfig,
    history_ids: BTreeSet<String>,
    calls: Vec<ToolCall>,
    bytes: usize,
}
fn invalid() -> RuntimeError {
    RuntimeError::new(ErrorCode::NativeProtocol, "invalid tool output sequence")
}
impl ToolStreamValidator {
    pub fn new(request: &GenerationRequest) -> Self {
        Self {
            enabled: request.uses_tools(),
            config: request.tools.clone(),
            history_ids: request
                .messages
                .iter()
                .flat_map(|m| m.tool_calls.iter().map(|c| c.id.clone()))
                .collect(),
            calls: Vec::new(),
            bytes: 0,
        }
    }
    pub fn text(&mut self, text: &str) -> Result<(), RuntimeError> {
        if !self.enabled {
            return Ok(());
        }
        if !self.calls.is_empty() {
            return Err(invalid());
        }
        self.add(text.len())
    }
    fn add(&mut self, bytes: usize) -> Result<(), RuntimeError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|v| *v <= MAX_TOOL_OUTPUT_BYTES)
            .ok_or_else(invalid)?;
        Ok(())
    }
    pub fn delta(&mut self, delta: &ToolCallDelta) -> Result<(), RuntimeError> {
        if !self.enabled {
            return Err(invalid());
        }
        delta.validate_piece()?;
        self.add(delta.payload_bytes())?;
        match delta {
            ToolCallDelta::Start { index, id, name } => {
                if *index as usize != self.calls.len()
                    || self.calls.len() >= MAX_TOOL_CALLS
                    || self.history_ids.contains(id)
                    || self.calls.iter().any(|c| c.id == *id)
                    || self.calls.last().is_some_and(|c| {
                        runtime_types::validate_tool_arguments(&c.arguments).is_err()
                    })
                    || !self
                        .config
                        .definitions
                        .iter()
                        .any(|tool| tool.name == *name)
                {
                    return Err(invalid());
                }
                self.calls.push(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: String::new(),
                });
            }
            ToolCallDelta::Arguments { index, arguments } => {
                if self.calls.len().checked_sub(1) != Some(*index as usize) {
                    return Err(invalid());
                }
                let call = self.calls.last_mut().ok_or_else(invalid)?;
                if call
                    .arguments
                    .len()
                    .checked_add(arguments.len())
                    .is_none_or(|v| v > MAX_TOOL_ARGUMENT_BYTES)
                {
                    return Err(invalid());
                }
                let required = call.arguments.len() + arguments.len();
                if required > call.arguments.capacity() {
                    call.arguments
                        .try_reserve_exact(required - call.arguments.len())
                        .map_err(|_| invalid())?;
                }
                call.arguments.push_str(arguments);
                let allocated = self
                    .calls
                    .iter()
                    .map(|call| {
                        call.id.capacity() + call.name.capacity() + call.arguments.capacity()
                    })
                    .sum::<usize>();
                if allocated > MAX_TOOL_OUTPUT_BYTES {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
    pub fn complete(&self, reason: FinishReason) -> Result<(), RuntimeError> {
        if !self.enabled {
            return if reason == FinishReason::ToolCalls {
                Err(invalid())
            } else {
                Ok(())
            };
        }
        if reason == FinishReason::Length
            || (reason == FinishReason::ToolCalls) == self.calls.is_empty()
        {
            return Err(invalid());
        }
        self.config
            .validate_output(&self.calls)
            .map_err(|_| invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_types::{
        GenerationOptions, Message, ModelId, RequestId, Role, ToolChoice, ToolDefinition,
    };
    fn request() -> GenerationRequest {
        GenerationRequest {
            request_id: RequestId::new(),
            model: ModelId::new("qa-small").unwrap(),
            messages: vec![Message::new(Role::User, "lookup")],
            options: GenerationOptions::default(),
            tools: ToolConfig {
                definitions: vec![ToolDefinition {
                    name: "lookup".into(),
                    description: None,
                    parameters: serde_json::json!({"type":"object","properties":{"key":{"type":"string"}}}),
                }],
                choice: ToolChoice::Auto,
                parallel_tool_calls: true,
            },
        }
    }
    fn start(index: u32, id: &str) -> ToolCallDelta {
        ToolCallDelta::Start {
            index,
            id: id.into(),
            name: "lookup".into(),
        }
    }
    fn args(index: u32, value: &str) -> ToolCallDelta {
        ToolCallDelta::Arguments {
            index,
            arguments: value.into(),
        }
    }
    #[test]
    fn complete_calls_and_text_have_distinct_terminal_contracts() {
        let mut state = ToolStreamValidator::new(&request());
        state.text("checking").unwrap();
        state.delta(&start(0, "call_a")).unwrap();
        state.delta(&args(0, "{\"key\":")).unwrap();
        state.delta(&args(0, "\"蓝色\"}")).unwrap();
        assert!(state.complete(FinishReason::Stop).is_err());
        state.complete(FinishReason::ToolCalls).unwrap();
        assert!(state.text("late text").is_err());
        let mut text = ToolStreamValidator::new(&request());
        text.text("plain text").unwrap();
        text.complete(FinishReason::Stop).unwrap();
        assert!(text.complete(FinishReason::ToolCalls).is_err());
        assert!(text.complete(FinishReason::Length).is_err());
    }
    #[test]
    fn malformed_order_duplicate_ids_and_incomplete_json_fail_closed() {
        for invalid in [start(1, "call_a"), args(0, "{}")] {
            assert!(
                ToolStreamValidator::new(&request())
                    .delta(&invalid)
                    .is_err()
            );
        }
        let mut state = ToolStreamValidator::new(&request());
        state.delta(&start(0, "call_a")).unwrap();
        state.delta(&args(0, "{\"key\":")).unwrap();
        assert!(state.complete(FinishReason::ToolCalls).is_err());
        assert!(state.delta(&start(1, "call_b")).is_err());
        let mut state = ToolStreamValidator::new(&request());
        state.delta(&start(0, "call_a")).unwrap();
        state.delta(&args(0, "{}")).unwrap();
        assert!(state.delta(&start(1, "call_a")).is_err());
        let mut state = ToolStreamValidator::new(&request());
        state.delta(&start(0, "call_a")).unwrap();
        state.delta(&args(0, "{\"key\":1,\"key\":2}")).unwrap();
        assert!(state.complete(FinishReason::ToolCalls).is_err());
    }
    #[test]
    fn uneven_argument_chunks_do_not_double_retained_capacity() {
        let mut state = ToolStreamValidator::new(&request());
        for index in 0..4 {
            state
                .delta(&start(index, &format!("call_{index}")))
                .unwrap();
            state.delta(&args(index, "{\"key\":\"")).unwrap();
            for _ in 0..4 {
                state.delta(&args(index, &"x".repeat(3000))).unwrap();
            }
            state.delta(&args(index, "\"}")).unwrap();
        }
        state.complete(FinishReason::ToolCalls).unwrap();
        let allocated = state
            .calls
            .iter()
            .map(|call| call.id.capacity() + call.name.capacity() + call.arguments.capacity())
            .sum::<usize>();
        assert!(allocated <= MAX_TOOL_OUTPUT_BYTES);
        assert!(
            state
                .calls
                .iter()
                .all(|call| call.arguments.capacity() == call.arguments.len())
        );
    }
    #[test]
    fn selection_and_text_only_rules_are_preserved() {
        let mut req = request();
        req.tools.choice = ToolChoice::Required;
        assert!(
            ToolStreamValidator::new(&req)
                .complete(FinishReason::Stop)
                .is_err()
        );
        req.tools.parallel_tool_calls = false;
        let mut state = ToolStreamValidator::new(&req);
        for (index, id) in [(0, "call_a"), (1, "call_b")] {
            state.delta(&start(index, id)).unwrap();
            state.delta(&args(index, "{}")).unwrap();
        }
        assert!(state.complete(FinishReason::ToolCalls).is_err());
        let mut text = ToolStreamValidator::default();
        assert!(text.delta(&start(0, "call_a")).is_err());
        text.complete(FinishReason::Length).unwrap();
    }
}
