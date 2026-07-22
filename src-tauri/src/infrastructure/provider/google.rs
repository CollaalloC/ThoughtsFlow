use serde::Deserialize;

use crate::{
    infrastructure::provider::decoder::{
        ProviderStreamDecoder, append_stream_frame_fragment, checked_stream_frame_size,
    },
    ports::provider::{ProviderError, RunEvent, Usage},
};

/// Decodes the sequence of `GenerateContentResponse` values returned by
/// Google's `streamGenerateContent?alt=sse` endpoint.
///
/// Google does not define a `[DONE]` sentinel for this protocol. A candidate's
/// `finishReason` therefore records the terminal reason while the HTTP EOF is
/// what closes the stream. Waiting for EOF also preserves a trailing cumulative
/// `usageMetadata` response.
#[derive(Default)]
pub struct GoogleSseDecoder {
    buffer: Vec<u8>,
    data: String,
    has_data_line: bool,
    metadata_emitted: bool,
    finish_reason: Option<String>,
    usage: Usage,
    terminal: bool,
}

impl GoogleSseDecoder {
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
            return Ok(());
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        let data = std::str::from_utf8(data).map_err(|error| {
            ProviderError::InvalidResponse(format!("Google SSE data is not valid UTF-8: {error}"))
        })?;
        let separator_bytes = usize::from(self.has_data_line);
        let size_with_separator =
            checked_stream_frame_size(self.data.len(), separator_bytes, "Google SSE event")?;
        checked_stream_frame_size(size_with_separator, data.len(), "Google SSE event")?;
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
        let payload = payload.trim();
        if payload.is_empty() {
            return Ok(());
        }

        let response: GoogleGenerateContentResponse =
            serde_json::from_str(payload).map_err(|error| {
                ProviderError::InvalidResponse(format!("invalid Google SSE JSON: {error}"))
            })?;

        if let Some(error) = response.error {
            validate_protocol_text(&error.message, "Google stream error message", true)?;
            if let Some(status) = error.status.as_deref() {
                validate_protocol_text(status, "Google stream error status", false)?;
            }
            let status = error.code.and_then(|code| u16::try_from(code).ok());
            let provider_code = error
                .status
                .filter(|status| !status.trim().is_empty())
                .unwrap_or_else(|| "google_stream_error".to_owned());
            let retryable = google_error_is_retryable(status, &provider_code);
            self.terminal = true;
            events.push(RunEvent::RunFailed {
                code: provider_code,
                message: if error.message.trim().is_empty() {
                    "Google stream failed without an error message".to_owned()
                } else {
                    error.message
                },
                retryable,
                status,
            });
            return Ok(());
        }

        if !self.metadata_emitted
            && (response.response_id.is_some() || response.model_version.is_some())
        {
            if let Some(request_id) = response.response_id.as_deref() {
                validate_protocol_text(request_id, "Google responseId", false)?;
            }
            if let Some(model) = response.model_version.as_deref() {
                validate_protocol_text(model, "Google modelVersion", false)?;
            }
            self.metadata_emitted = true;
            events.push(RunEvent::ProviderMetadata {
                request_id: response.response_id,
                model: response.model_version,
                created_at: None,
            });
        }

        if let Some(candidate) = response
            .candidates
            .into_iter()
            .find(|candidate| candidate.index.unwrap_or(0) == 0)
        {
            if let Some(content) = candidate.content {
                for part in content.parts {
                    let Some(text) = part.text.filter(|text| !text.is_empty()) else {
                        continue;
                    };
                    validate_protocol_text(&text, "Google content part", true)?;
                    if part.thought {
                        events.push(RunEvent::ReasoningDelta { text });
                    } else {
                        events.push(RunEvent::TextDelta { text });
                    }
                }
            }
            if let Some(finish_reason) = candidate.finish_reason {
                validate_protocol_text(&finish_reason, "Google finishReason", false)?;
                if !finish_reason.trim().is_empty() {
                    self.finish_reason = Some(finish_reason);
                }
            }
        }

        if let Some(usage) = response.usage_metadata {
            if usage.prompt_token_count.is_some() {
                self.usage.prompt_tokens = usage.prompt_token_count;
            }
            if usage.candidates_token_count.is_some() {
                self.usage.completion_tokens = usage.candidates_token_count;
            }
            if usage.total_token_count.is_some() {
                self.usage.total_tokens = usage.total_token_count;
            }
            events.push(RunEvent::UsageUpdated {
                usage: self.usage.clone(),
            });
        }

        if let Some(block_reason) = response
            .prompt_feedback
            .and_then(|feedback| feedback.block_reason)
        {
            validate_protocol_text(&block_reason, "Google prompt blockReason", false)?;
            if !block_reason.trim().is_empty() {
                self.terminal = true;
                events.push(RunEvent::RunFailed {
                    code: block_reason.clone(),
                    message: format!("Google blocked the prompt: {block_reason}"),
                    retryable: false,
                    status: None,
                });
            }
        }

        Ok(())
    }
}

impl ProviderStreamDecoder for GoogleSseDecoder {
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
                "Google SSE line",
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
        append_stream_frame_fragment(&mut self.buffer, remaining, "Google SSE line")?;
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
        if self.terminal {
            return Ok(events);
        }

        let Some(finish_reason) = self.finish_reason.take() else {
            return Err(ProviderError::UnexpectedEof);
        };
        self.terminal = true;
        events.push(RunEvent::RunCompleted {
            finish_reason: Some(finish_reason),
        });
        Ok(events)
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }
}

fn google_error_is_retryable(status: Option<u16>, provider_code: &str) -> bool {
    status.is_some_and(|status| matches!(status, 408 | 409 | 425 | 429 | 500..=599))
        || matches!(
            provider_code,
            "ABORTED"
                | "DEADLINE_EXCEEDED"
                | "INTERNAL"
                | "RESOURCE_EXHAUSTED"
                | "UNAVAILABLE"
                | "UNKNOWN"
        )
}

fn validate_protocol_text(
    value: &str,
    field: &str,
    allow_layout_controls: bool,
) -> Result<(), ProviderError> {
    let contains_unsafe_control = value.chars().any(|character| {
        (character.is_control()
            && !(allow_layout_controls && matches!(character, '\n' | '\r' | '\t')))
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
    });
    if contains_unsafe_control {
        return Err(ProviderError::InvalidResponse(format!(
            "{field} contains unsafe control characters"
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleGenerateContentResponse {
    #[serde(default)]
    candidates: Vec<GoogleCandidate>,
    #[serde(default)]
    prompt_feedback: Option<GooglePromptFeedback>,
    #[serde(default)]
    usage_metadata: Option<GoogleUsageMetadata>,
    #[serde(default)]
    model_version: Option<String>,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    error: Option<GoogleStreamError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCandidate {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    content: Option<GoogleContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GoogleContent {
    #[serde(default)]
    parts: Vec<GooglePart>,
}

#[derive(Deserialize)]
struct GooglePart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thought: bool,
    // `thoughtSignature` is intentionally not deserialized: it is opaque
    // continuation metadata, not human-readable reasoning output.
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleUsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<u64>,
    #[serde(default)]
    candidates_token_count: Option<u64>,
    #[serde(default)]
    total_token_count: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GooglePromptFeedback {
    #[serde(default)]
    block_reason: Option<String>,
}

#[derive(Deserialize)]
struct GoogleStreamError {
    #[serde(default)]
    code: Option<u64>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    status: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::{
        infrastructure::provider::{
            ProviderStreamDecoder, decoder::MAX_PROVIDER_STREAM_FRAME_BYTES,
        },
        ports::provider::{ProviderError, RunEvent, Usage},
    };

    use super::GoogleSseDecoder;

    #[test]
    fn google_sse_rejects_an_oversized_unterminated_line() {
        let mut decoder = GoogleSseDecoder::new();
        assert!(
            decoder
                .push(&vec![b'x'; MAX_PROVIDER_STREAM_FRAME_BYTES])
                .unwrap()
                .is_empty()
        );

        assert!(matches!(
            decoder.push(b"x").unwrap_err(),
            ProviderError::InvalidResponse(message)
                if message == "Google SSE line exceeds the 1048576-byte stream frame limit"
        ));
    }

    #[test]
    fn google_sse_rejects_oversized_multiline_event_data() {
        let mut decoder = GoogleSseDecoder::new();
        let half_limit = MAX_PROVIDER_STREAM_FRAME_BYTES / 2;
        let mut data_line = b"data: ".to_vec();
        data_line.extend(std::iter::repeat_n(b'x', half_limit));
        data_line.push(b'\n');

        assert!(decoder.push(&data_line).unwrap().is_empty());
        assert!(matches!(
            decoder.push(&data_line).unwrap_err(),
            ProviderError::InvalidResponse(message)
                if message == "Google SSE event exceeds the 1048576-byte stream frame limit"
        ));
    }

    #[test]
    fn google_sse_accepts_many_small_events_in_one_large_network_chunk() {
        let event = b"data: {\"candidates\":[]}\n\n";
        let chunk = event.repeat(MAX_PROVIDER_STREAM_FRAME_BYTES / event.len() + 1);
        assert!(chunk.len() > MAX_PROVIDER_STREAM_FRAME_BYTES);

        let mut decoder = GoogleSseDecoder::new();
        assert!(decoder.push(&chunk).unwrap().is_empty());
    }

    #[test]
    fn google_sse_decodes_arbitrary_chunks_multiline_data_thoughts_usage_and_finish() {
        let fixture = concat!(
            ": keep-alive\r\n\r\n",
            "event: response\r\n",
            "data: {\"responseId\":\"resp-1\",\r\n",
            "data: \"modelVersion\":\"gemini-fixture\",\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"plan\",\"thought\":true,\"thoughtSignature\":\"opaque-signature\"},{\"text\":\"你\"}]}}]}\r\n\r\n",
            "\r\n",
            "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"好\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"index\":0,\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2,\"thoughtsTokenCount\":3,\"totalTokenCount\":12}}\n\n",
        );
        let mut decoder = GoogleSseDecoder::new();
        let mut events = Vec::new();

        // One-byte chunks exercise framing prefixes, JSON escapes, CRLF, and
        // UTF-8 code points split at every possible TCP boundary.
        for chunk in fixture.as_bytes().chunks(1) {
            events.extend(decoder.push(chunk).unwrap());
        }

        assert!(!decoder.is_terminal());
        events.extend(decoder.finish().unwrap());
        assert_eq!(
            events,
            vec![
                RunEvent::ProviderMetadata {
                    request_id: Some("resp-1".to_owned()),
                    model: Some("gemini-fixture".to_owned()),
                    created_at: None,
                },
                RunEvent::ReasoningDelta {
                    text: "plan".to_owned(),
                },
                RunEvent::TextDelta {
                    text: "你".to_owned(),
                },
                RunEvent::TextDelta {
                    text: "好".to_owned(),
                },
                RunEvent::UsageUpdated {
                    usage: Usage {
                        prompt_tokens: Some(7),
                        completion_tokens: Some(2),
                        total_tokens: Some(12),
                    },
                },
                RunEvent::RunCompleted {
                    finish_reason: Some("STOP".to_owned()),
                },
            ]
        );
        assert!(decoder.is_terminal());
    }

    #[test]
    fn google_sse_waits_for_eof_after_finish_reason_and_keeps_trailing_usage() {
        let mut decoder = GoogleSseDecoder::new();
        assert!(
            decoder
                .push(b"data: {\"candidates\":[{\"finishReason\":\"MAX_TOKENS\"}]}\n\n")
                .unwrap()
                .is_empty()
        );
        assert!(!decoder.is_terminal());
        assert_eq!(
            decoder
                .push(b"data: {\"usageMetadata\":{\"promptTokenCount\":3}}\n\n")
                .unwrap(),
            vec![RunEvent::UsageUpdated {
                usage: Usage {
                    prompt_tokens: Some(3),
                    completion_tokens: None,
                    total_tokens: None,
                },
            }]
        );
        assert_eq!(
            decoder
                .push(b"data: {\"usageMetadata\":{\"candidatesTokenCount\":4,\"totalTokenCount\":7}}\n\n")
                .unwrap(),
            vec![RunEvent::UsageUpdated {
                usage: Usage {
                    prompt_tokens: Some(3),
                    completion_tokens: Some(4),
                    total_tokens: Some(7),
                },
            }]
        );
        assert_eq!(
            decoder.finish().unwrap(),
            vec![RunEvent::RunCompleted {
                finish_reason: Some("MAX_TOKENS".to_owned()),
            }]
        );
        assert!(decoder.push(b"ignored").unwrap().is_empty());
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn google_sse_maps_stream_error_to_one_retryable_terminal_failure() {
        let mut decoder = GoogleSseDecoder::new();
        let events = decoder
            .push(
                concat!(
                    "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"partial\"}]}}]}\r\n\r\n",
                    "data: {\"error\":{\"code\":429,\"message\":\"quota exhausted\",\"status\":\"RESOURCE_EXHAUSTED\"}}\r\n\r\n",
                )
                .as_bytes(),
            )
            .unwrap();

        assert_eq!(
            events,
            vec![
                RunEvent::TextDelta {
                    text: "partial".to_owned(),
                },
                RunEvent::RunFailed {
                    code: "RESOURCE_EXHAUSTED".to_owned(),
                    message: "quota exhausted".to_owned(),
                    retryable: true,
                    status: Some(429),
                },
            ]
        );
        assert!(decoder.is_terminal());
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn google_sse_maps_prompt_feedback_without_candidates_to_terminal_failure() {
        let mut decoder = GoogleSseDecoder::new();
        let events = decoder
            .push(
                b"data: {\"promptFeedback\":{\"blockReason\":\"SAFETY\"},\"usageMetadata\":{\"promptTokenCount\":5,\"totalTokenCount\":5}}\n\n",
            )
            .unwrap();

        assert_eq!(
            events,
            vec![
                RunEvent::UsageUpdated {
                    usage: Usage {
                        prompt_tokens: Some(5),
                        completion_tokens: None,
                        total_tokens: Some(5),
                    },
                },
                RunEvent::RunFailed {
                    code: "SAFETY".to_owned(),
                    message: "Google blocked the prompt: SAFETY".to_owned(),
                    retryable: false,
                    status: None,
                },
            ]
        );
        assert!(decoder.is_terminal());
    }

    #[test]
    fn google_sse_reports_eof_without_a_finish_reason_as_interrupted() {
        let mut decoder = GoogleSseDecoder::new();
        assert_eq!(
            decoder
                .push(b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"partial\"}]}}]}\n\n")
                .unwrap(),
            vec![RunEvent::TextDelta {
                text: "partial".to_owned(),
            }]
        );
        assert_eq!(decoder.finish().unwrap_err(), ProviderError::UnexpectedEof);
    }

    #[test]
    fn google_sse_accepts_a_final_event_without_a_blank_line() {
        let mut decoder = GoogleSseDecoder::new();
        assert!(
            decoder
                .push(b"data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            decoder.finish().unwrap(),
            vec![RunEvent::RunCompleted {
                finish_reason: Some("STOP".to_owned()),
            }]
        );
    }

    #[test]
    fn google_sse_rejects_unsafe_escaped_controls_in_model_text() {
        let mut decoder = GoogleSseDecoder::new();
        let error = decoder
            .push(
                br#"data: {"candidates":[{"content":{"parts":[{"text":"bad\u0000text"}]}}]}

"#,
            )
            .unwrap_err();

        assert_eq!(
            error,
            ProviderError::InvalidResponse(
                "Google content part contains unsafe control characters".to_owned()
            )
        );
    }

    #[test]
    fn google_sse_rejects_invalid_utf8_and_unescaped_json_controls() {
        let mut invalid_utf8 = GoogleSseDecoder::new();
        assert!(matches!(
            invalid_utf8.push(b"data: \xff\n\n").unwrap_err(),
            ProviderError::InvalidResponse(message)
                if message.starts_with("Google SSE data is not valid UTF-8:")
        ));

        let mut raw_control = GoogleSseDecoder::new();
        assert!(matches!(
            raw_control
                .push(b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"bad\x00text\"}]}}]}\n\n")
                .unwrap_err(),
            ProviderError::InvalidResponse(message)
                if message.starts_with("invalid Google SSE JSON:")
        ));
    }
}
