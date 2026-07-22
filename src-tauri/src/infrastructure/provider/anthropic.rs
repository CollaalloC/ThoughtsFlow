use serde::Deserialize;

use crate::{
    infrastructure::provider::decoder::{
        ProviderStreamDecoder, append_stream_frame_fragment, checked_stream_frame_size,
    },
    ports::provider::{ProviderError, RunEvent, Usage},
};

#[derive(Default)]
pub struct AnthropicSseDecoder {
    buffer: Vec<u8>,
    data: String,
    has_data_line: bool,
    metadata_emitted: bool,
    finish_reason: Option<String>,
    usage: AnthropicUsageAccumulator,
    terminal: bool,
}

impl AnthropicSseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<RunEvent>,
    ) -> Result<(), ProviderError> {
        if self.terminal {
            return Ok(());
        }
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            return self.dispatch_event(events);
        }
        if line.starts_with(b":") {
            return Ok(());
        }
        let Some(data) = line.strip_prefix(b"data:") else {
            // Anthropic duplicates the event name inside the JSON `type`
            // field, so event/id/retry fields can be ignored safely here.
            return Ok(());
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        let data = std::str::from_utf8(data).map_err(|_| {
            ProviderError::InvalidResponse("Anthropic SSE data is not valid UTF-8".to_owned())
        })?;
        let separator_bytes = usize::from(self.has_data_line);
        let size_with_separator =
            checked_stream_frame_size(self.data.len(), separator_bytes, "Anthropic SSE event")?;
        checked_stream_frame_size(size_with_separator, data.len(), "Anthropic SSE event")?;
        if self.has_data_line {
            self.data.push('\n');
        }
        self.data.push_str(data);
        self.has_data_line = true;
        Ok(())
    }

    fn dispatch_event(&mut self, events: &mut Vec<RunEvent>) -> Result<(), ProviderError> {
        if !self.has_data_line || self.terminal {
            self.data.clear();
            self.has_data_line = false;
            return Ok(());
        }
        let payload = std::mem::take(&mut self.data);
        self.has_data_line = false;
        if payload.trim().is_empty() {
            return Ok(());
        }

        // Never include the provider-controlled body in a parse error. Besides
        // keeping diagnostics bounded, this prevents an echoed credential in
        // an error payload from reaching logs or the UI.
        let event: AnthropicStreamEvent = serde_json::from_str(&payload)
            .map_err(|_| ProviderError::InvalidResponse("invalid Anthropic SSE JSON".to_owned()))?;

        match event {
            AnthropicStreamEvent::MessageStart { message } => {
                if !self.metadata_emitted && (message.id.is_some() || message.model.is_some()) {
                    self.metadata_emitted = true;
                    events.push(RunEvent::ProviderMetadata {
                        request_id: message.id,
                        model: message.model,
                        created_at: None,
                    });
                }
                if let Some(reason) = message.stop_reason {
                    self.finish_reason = Some(reason);
                }
                if let Some(usage) = message.usage.and_then(|usage| self.usage.merge(usage)) {
                    events.push(RunEvent::UsageUpdated { usage });
                }
            }
            AnthropicStreamEvent::ContentBlockStart { content_block } => {
                emit_content_block_start(content_block, events);
            }
            AnthropicStreamEvent::ContentBlockDelta { delta } => {
                emit_content_block_delta(delta, events);
            }
            AnthropicStreamEvent::MessageDelta { delta, usage } => {
                if let Some(reason) = delta.stop_reason {
                    self.finish_reason = Some(reason);
                }
                if let Some(usage) = usage.and_then(|usage| self.usage.merge(usage)) {
                    events.push(RunEvent::UsageUpdated { usage });
                }
            }
            AnthropicStreamEvent::MessageStop => {
                self.terminal = true;
                events.push(RunEvent::RunCompleted {
                    finish_reason: self.finish_reason.take(),
                });
            }
            AnthropicStreamEvent::Error { error } => {
                self.terminal = true;
                events.push(normalize_stream_error(error.kind.as_deref()));
            }
            AnthropicStreamEvent::Ping
            | AnthropicStreamEvent::ContentBlockStop
            | AnthropicStreamEvent::Unknown => {}
        }
        Ok(())
    }
}

impl ProviderStreamDecoder for AnthropicSseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<RunEvent>, ProviderError> {
        if self.terminal {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        let mut remaining = chunk;
        while let Some(newline) = remaining.iter().position(|byte| *byte == b'\n') {
            append_stream_frame_fragment(
                &mut self.buffer,
                &remaining[..newline],
                "Anthropic SSE line",
            )?;
            let mut line = std::mem::take(&mut self.buffer);
            let result = self.process_line(&line, &mut events);
            line.clear();
            self.buffer = line;
            result?;
            if self.terminal {
                return Ok(events);
            }
            remaining = &remaining[newline + 1..];
        }
        append_stream_frame_fragment(&mut self.buffer, remaining, "Anthropic SSE line")?;
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<RunEvent>, ProviderError> {
        if self.terminal {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.process_line(&line, &mut events)?;
        }
        self.dispatch_event(&mut events)?;
        if !self.terminal {
            return Err(ProviderError::UnexpectedEof);
        }
        Ok(events)
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }
}

fn emit_content_block_start(content_block: AnthropicContentBlock, events: &mut Vec<RunEvent>) {
    match content_block {
        AnthropicContentBlock::Text { text } if !text.is_empty() => {
            events.push(RunEvent::TextDelta { text });
        }
        AnthropicContentBlock::Thinking { thinking } if !thinking.is_empty() => {
            events.push(RunEvent::ReasoningDelta { text: thinking });
        }
        AnthropicContentBlock::Text { .. }
        | AnthropicContentBlock::Thinking { .. }
        | AnthropicContentBlock::Other => {}
    }
}

fn emit_content_block_delta(delta: AnthropicContentDelta, events: &mut Vec<RunEvent>) {
    match delta {
        AnthropicContentDelta::Text { text } if !text.is_empty() => {
            events.push(RunEvent::TextDelta { text });
        }
        AnthropicContentDelta::Thinking { thinking } if !thinking.is_empty() => {
            events.push(RunEvent::ReasoningDelta { text: thinking });
        }
        AnthropicContentDelta::Text { .. }
        | AnthropicContentDelta::Thinking { .. }
        | AnthropicContentDelta::Other => {}
    }
}

fn normalize_stream_error(kind: Option<&str>) -> RunEvent {
    let (code, message, retryable) = match kind {
        Some("overloaded_error") => (
            "overloaded_error",
            "Anthropic is temporarily overloaded",
            true,
        ),
        Some("rate_limit_error") => ("rate_limited", "Anthropic rate limit exceeded", true),
        Some("api_error") | Some("internal_server_error") => (
            "api_error",
            "Anthropic reported an internal API error",
            true,
        ),
        Some("timeout_error") => ("timeout_error", "Anthropic stream timed out", true),
        Some("service_unavailable_error") => (
            "service_unavailable_error",
            "Anthropic service is temporarily unavailable",
            true,
        ),
        Some("authentication_error") => (
            "authentication_error",
            "Anthropic authentication failed",
            false,
        ),
        Some("permission_error") => (
            "permission_error",
            "Anthropic request was not permitted",
            false,
        ),
        Some("invalid_request_error") => (
            "invalid_request_error",
            "Anthropic rejected the request",
            false,
        ),
        Some("not_found_error") => ("not_found_error", "Anthropic resource was not found", false),
        Some("request_too_large") => (
            "request_too_large",
            "Anthropic request was too large",
            false,
        ),
        _ => (
            "anthropic_stream_error",
            "Anthropic stream reported an error",
            false,
        ),
    };
    RunEvent::RunFailed {
        code: code.to_owned(),
        message: message.to_owned(),
        retryable,
        // This is an error event delivered inside a successful HTTP SSE
        // response, so it must not fabricate an HTTP status.
        status: None,
    }
}

#[derive(Default)]
struct AnthropicUsageAccumulator {
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl AnthropicUsageAccumulator {
    fn merge(&mut self, update: AnthropicUsage) -> Option<Usage> {
        let has_update = update.input_tokens.is_some()
            || update.cache_creation_input_tokens.is_some()
            || update.cache_read_input_tokens.is_some()
            || update.output_tokens.is_some();
        if !has_update {
            return None;
        }
        if update.input_tokens.is_some() {
            self.input_tokens = update.input_tokens;
        }
        if update.cache_creation_input_tokens.is_some() {
            self.cache_creation_input_tokens = update.cache_creation_input_tokens;
        }
        if update.cache_read_input_tokens.is_some() {
            self.cache_read_input_tokens = update.cache_read_input_tokens;
        }
        if update.output_tokens.is_some() {
            self.output_tokens = update.output_tokens;
        }

        let has_prompt_usage = self.input_tokens.is_some()
            || self.cache_creation_input_tokens.is_some()
            || self.cache_read_input_tokens.is_some();
        let prompt_tokens = if has_prompt_usage {
            self.input_tokens
                .unwrap_or(0)
                .checked_add(self.cache_creation_input_tokens.unwrap_or(0))?
                .checked_add(self.cache_read_input_tokens.unwrap_or(0))
        } else {
            None
        };
        let total_tokens = match (prompt_tokens, self.output_tokens) {
            (Some(prompt), Some(completion)) => prompt.checked_add(completion),
            _ => None,
        };
        Some(Usage {
            prompt_tokens,
            completion_tokens: self.output_tokens,
            total_tokens,
        })
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicStreamEvent {
    MessageStart {
        message: AnthropicMessageStart,
    },
    ContentBlockStart {
        content_block: AnthropicContentBlock,
    },
    ContentBlockDelta {
        delta: AnthropicContentDelta,
    },
    ContentBlockStop,
    MessageDelta {
        #[serde(default)]
        delta: AnthropicMessageDelta,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
    },
    MessageStop,
    Ping,
    Error {
        #[serde(default)]
        error: AnthropicStreamError,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct AnthropicMessageStart {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentDelta {
    #[serde(rename = "text_delta")]
    Text {
        #[serde(default)]
        text: String,
    },
    #[serde(rename = "thinking_delta")]
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Default, Deserialize)]
struct AnthropicMessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[derive(Default, Deserialize)]
struct AnthropicStreamError {
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::{
        infrastructure::provider::{
            ProviderStreamDecoder, decoder::MAX_PROVIDER_STREAM_FRAME_BYTES,
        },
        ports::provider::{ProviderError, RunEvent, Usage},
    };

    use super::AnthropicSseDecoder;

    #[test]
    fn anthropic_sse_rejects_an_oversized_unterminated_line() {
        let mut decoder = AnthropicSseDecoder::new();
        assert!(
            decoder
                .push(&vec![b'x'; MAX_PROVIDER_STREAM_FRAME_BYTES])
                .unwrap()
                .is_empty()
        );

        assert!(matches!(
            decoder.push(b"x").unwrap_err(),
            ProviderError::InvalidResponse(message)
                if message == "Anthropic SSE line exceeds the 1048576-byte stream frame limit"
        ));
    }

    #[test]
    fn anthropic_sse_rejects_oversized_multiline_event_data() {
        let mut decoder = AnthropicSseDecoder::new();
        let half_limit = MAX_PROVIDER_STREAM_FRAME_BYTES / 2;
        let mut data_line = b"data: ".to_vec();
        data_line.extend(std::iter::repeat_n(b'x', half_limit));
        data_line.push(b'\n');

        assert!(decoder.push(&data_line).unwrap().is_empty());
        assert!(matches!(
            decoder.push(&data_line).unwrap_err(),
            ProviderError::InvalidResponse(message)
                if message == "Anthropic SSE event exceeds the 1048576-byte stream frame limit"
        ));
    }

    #[test]
    fn anthropic_sse_accepts_a_valid_frame_at_the_line_limit() {
        let mut frame = b"data: {\"type\":\"ping\",\"padding\":\"".to_vec();
        let suffix = b"\"}";
        frame.extend(std::iter::repeat_n(
            b'x',
            MAX_PROVIDER_STREAM_FRAME_BYTES - frame.len() - suffix.len(),
        ));
        frame.extend_from_slice(suffix);
        assert_eq!(frame.len(), MAX_PROVIDER_STREAM_FRAME_BYTES);
        frame.extend_from_slice(b"\n\n");

        let mut decoder = AnthropicSseDecoder::new();
        assert!(decoder.push(&frame).unwrap().is_empty());
    }

    #[test]
    fn anthropic_sse_accepts_many_small_events_in_one_large_network_chunk() {
        let ping = b"data: {\"type\":\"ping\"}\n\n";
        let chunk = ping.repeat(MAX_PROVIDER_STREAM_FRAME_BYTES / ping.len() + 1);
        assert!(chunk.len() > MAX_PROVIDER_STREAM_FRAME_BYTES);

        let mut decoder = AnthropicSseDecoder::new();
        assert!(decoder.push(&chunk).unwrap().is_empty());
    }

    #[test]
    fn anthropic_sse_decodes_arbitrary_chunks_crlf_blocks_cumulative_usage_and_stop() {
        let fixture = concat!(
            ": keep-alive\r\n\r\n",
            "event: message_start\r\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_fixture\",\"model\":\"claude-fixture\",\"usage\":{\"input_tokens\":5,\"cache_creation_input_tokens\":2,\"cache_read_input_tokens\":3,\"output_tokens\":1}}}\r\n\r\n",
            "event: future_event\n",
            "data: {\"type\":\"future_event\",\"future\":true}\n\n",
            "event: ping\n",
            "data: {\n",
            "data:   \"type\": \"ping\"\n",
            "data: }\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"先\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"分析\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"ignored\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"答\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"案\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"ignored\\\":\"}}\n\n",
            "event: message_delta\r\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":5}}\r\n\r\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let mut decoder = AnthropicSseDecoder::new();
        let mut events = Vec::new();

        // One-byte chunks exercise splits inside SSE field names, JSON tokens,
        // CRLF delimiters, and multi-byte UTF-8 text.
        for chunk in fixture.as_bytes().chunks(1) {
            events.extend(decoder.push(chunk).unwrap());
        }
        events.extend(decoder.finish().unwrap());

        assert_eq!(
            events,
            vec![
                RunEvent::ProviderMetadata {
                    request_id: Some("msg_fixture".to_owned()),
                    model: Some("claude-fixture".to_owned()),
                    created_at: None,
                },
                RunEvent::UsageUpdated {
                    usage: Usage {
                        prompt_tokens: Some(10),
                        completion_tokens: Some(1),
                        total_tokens: Some(11),
                    },
                },
                RunEvent::ReasoningDelta {
                    text: "先".to_owned(),
                },
                RunEvent::ReasoningDelta {
                    text: "分析".to_owned(),
                },
                RunEvent::TextDelta {
                    text: "答".to_owned(),
                },
                RunEvent::TextDelta {
                    text: "案".to_owned(),
                },
                RunEvent::UsageUpdated {
                    usage: Usage {
                        prompt_tokens: Some(10),
                        completion_tokens: Some(5),
                        total_tokens: Some(15),
                    },
                },
                RunEvent::RunCompleted {
                    finish_reason: Some("end_turn".to_owned()),
                },
            ]
        );
        assert!(decoder.is_terminal());
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn anthropic_sse_accepts_message_stop_without_a_trailing_blank_line() {
        let mut decoder = AnthropicSseDecoder::new();
        assert!(
            decoder
                .push(b"event: message_stop\ndata: {\"type\":\"message_stop\"}")
                .unwrap()
                .is_empty()
        );

        assert_eq!(
            decoder.finish().unwrap(),
            vec![RunEvent::RunCompleted {
                finish_reason: None,
            }]
        );
        assert!(decoder.is_terminal());
    }

    #[test]
    fn anthropic_sse_turns_overload_into_a_safe_retryable_terminal_event() {
        let mut decoder = AnthropicSseDecoder::new();
        let events = decoder
            .push(
                b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\nevent: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"credential sk-ant-secret was rejected\"}}\n\n",
            )
            .unwrap();

        assert_eq!(
            events,
            vec![
                RunEvent::TextDelta {
                    text: "partial".to_owned(),
                },
                RunEvent::RunFailed {
                    code: "overloaded_error".to_owned(),
                    message: "Anthropic is temporarily overloaded".to_owned(),
                    retryable: true,
                    status: None,
                },
            ]
        );
        assert!(!format!("{events:?}").contains("sk-ant-secret"));
        assert!(decoder.is_terminal());
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn anthropic_stream_quota_mapping_requires_the_exact_structured_error_type() {
        let cases = [
            (
                "rate_limit_error",
                "ignored",
                "rate_limited",
                "Anthropic rate limit exceeded",
                true,
            ),
            (
                "RATE_LIMIT_ERROR",
                "rate_limit_error",
                "anthropic_stream_error",
                "Anthropic stream reported an error",
                false,
            ),
            (
                "api_error",
                "rate_limit_error",
                "api_error",
                "Anthropic reported an internal API error",
                true,
            ),
            (
                "overloaded_error",
                "rate_limit_error",
                "overloaded_error",
                "Anthropic is temporarily overloaded",
                true,
            ),
        ];

        for (kind, provider_message, expected_code, expected_message, expected_retryable) in cases {
            let mut decoder = AnthropicSseDecoder::new();
            let frame = format!(
                "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"{kind}\",\"message\":\"{provider_message}\"}}}}\n\n"
            );

            assert_eq!(
                decoder.push(frame.as_bytes()).unwrap(),
                vec![RunEvent::RunFailed {
                    code: expected_code.to_owned(),
                    message: expected_message.to_owned(),
                    retryable: expected_retryable,
                    status: None,
                }],
                "kind={kind}"
            );
        }
    }

    #[test]
    fn anthropic_sse_does_not_reflect_unknown_error_fields() {
        let mut decoder = AnthropicSseDecoder::new();
        let events = decoder
            .push(
                b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"sk-ant-secret\",\"message\":\"sk-ant-secret\"}}\n\n",
            )
            .unwrap();

        assert_eq!(
            events,
            vec![RunEvent::RunFailed {
                code: "anthropic_stream_error".to_owned(),
                message: "Anthropic stream reported an error".to_owned(),
                retryable: false,
                status: None,
            }]
        );
        assert!(!format!("{events:?}").contains("sk-ant-secret"));
    }

    #[test]
    fn anthropic_sse_reports_eof_before_message_stop_even_after_a_stop_reason() {
        let mut decoder = AnthropicSseDecoder::new();
        decoder
            .push(
                b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            )
            .unwrap();

        assert_eq!(decoder.finish().unwrap_err(), ProviderError::UnexpectedEof);
    }

    #[test]
    fn anthropic_sse_malformed_json_error_does_not_echo_the_payload() {
        let mut decoder = AnthropicSseDecoder::new();
        let error = decoder
            .push(b"event: message_start\ndata: {\"type\":\"sk-ant-secret\" trailing}\n\n")
            .unwrap_err();

        assert_eq!(
            error,
            ProviderError::InvalidResponse("invalid Anthropic SSE JSON".to_owned())
        );
        assert!(!error.to_string().contains("sk-ant-secret"));
    }

    #[test]
    fn anthropic_sse_reports_a_truncated_final_json_event_without_echoing_it() {
        let mut decoder = AnthropicSseDecoder::new();
        assert!(
            decoder
                .push(b"event: message_start\ndata: {\"type\":\"sk-ant-secret")
                .unwrap()
                .is_empty()
        );

        let error = decoder.finish().unwrap_err();
        assert_eq!(
            error,
            ProviderError::InvalidResponse("invalid Anthropic SSE JSON".to_owned())
        );
        assert!(!error.to_string().contains("sk-ant-secret"));
    }
}
