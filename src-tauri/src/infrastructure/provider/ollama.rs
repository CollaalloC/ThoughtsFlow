use serde::Deserialize;

use crate::{
    infrastructure::provider::decoder::{ProviderStreamDecoder, append_stream_frame_fragment},
    ports::provider::{ProviderError, RunEvent, Usage},
};

#[derive(Default)]
pub struct OllamaNdjsonDecoder {
    buffer: Vec<u8>,
    metadata_emitted: bool,
    terminal: bool,
}

impl OllamaNdjsonDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<RunEvent>,
    ) -> Result<(), ProviderError> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) || self.terminal {
            return Ok(());
        }
        let item: OllamaStreamItem = serde_json::from_slice(line).map_err(|error| {
            ProviderError::InvalidResponse(format!("invalid Ollama NDJSON: {error}"))
        })?;
        if let Some(error) = item.error {
            return Err(ProviderError::InvalidResponse(error));
        }

        if !self.metadata_emitted && (item.model.is_some() || item.created_at.is_some()) {
            self.metadata_emitted = true;
            events.push(RunEvent::ProviderMetadata {
                request_id: None,
                model: item.model,
                created_at: item.created_at,
            });
        }
        if let Some(message) = item.message {
            if let Some(reasoning) = message.thinking.filter(|text| !text.is_empty()) {
                events.push(RunEvent::ReasoningDelta { text: reasoning });
            }
            if let Some(text) = message.content.filter(|text| !text.is_empty()) {
                events.push(RunEvent::TextDelta { text });
            }
        }

        if item.done {
            if item.prompt_eval_count.is_some() || item.eval_count.is_some() {
                events.push(RunEvent::UsageUpdated {
                    usage: Usage {
                        prompt_tokens: item.prompt_eval_count,
                        completion_tokens: item.eval_count,
                        total_tokens: match (item.prompt_eval_count, item.eval_count) {
                            (Some(prompt), Some(completion)) => Some(prompt + completion),
                            _ => None,
                        },
                    },
                });
            }
            self.terminal = true;
            events.push(RunEvent::RunCompleted {
                finish_reason: item.done_reason,
            });
        }
        Ok(())
    }
}

impl ProviderStreamDecoder for OllamaNdjsonDecoder {
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
                "Ollama NDJSON record",
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
        append_stream_frame_fragment(&mut self.buffer, remaining, "Ollama NDJSON record")?;
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
        if !self.terminal {
            return Err(ProviderError::UnexpectedEof);
        }
        Ok(events)
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }
}

#[derive(Deserialize)]
struct OllamaStreamItem {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    message: Option<OllamaMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct OllamaMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::{
        infrastructure::provider::{
            ProviderStreamDecoder, decoder::MAX_PROVIDER_STREAM_FRAME_BYTES,
        },
        ports::provider::{ProviderError, RunEvent, Usage},
    };

    use super::OllamaNdjsonDecoder;

    #[test]
    fn ollama_ndjson_rejects_an_oversized_unterminated_record() {
        let mut decoder = OllamaNdjsonDecoder::new();
        assert!(
            decoder
                .push(&vec![b'x'; MAX_PROVIDER_STREAM_FRAME_BYTES])
                .unwrap()
                .is_empty()
        );

        assert!(matches!(
            decoder.push(b"x").unwrap_err(),
            ProviderError::InvalidResponse(message)
                if message == "Ollama NDJSON record exceeds the 1048576-byte stream frame limit"
        ));
    }

    #[test]
    fn ollama_ndjson_accepts_a_valid_record_at_the_limit() {
        let mut record = b"{\"message\":{\"content\":\"".to_vec();
        let suffix = b"\"},\"done\":false}";
        let content_len = MAX_PROVIDER_STREAM_FRAME_BYTES - record.len() - suffix.len();
        record.extend(std::iter::repeat_n(b'x', content_len));
        record.extend_from_slice(suffix);
        assert_eq!(record.len(), MAX_PROVIDER_STREAM_FRAME_BYTES);
        record.push(b'\n');

        let mut decoder = OllamaNdjsonDecoder::new();
        let events = decoder.push(&record).unwrap();
        assert!(matches!(
            events.as_slice(),
            [RunEvent::TextDelta { text }] if text.len() == content_len
        ));
    }

    #[test]
    fn ollama_ndjson_accepts_many_small_records_in_one_large_network_chunk() {
        let record = b"{\"message\":{},\"done\":false}\n";
        let chunk = record.repeat(MAX_PROVIDER_STREAM_FRAME_BYTES / record.len() + 1);
        assert!(chunk.len() > MAX_PROVIDER_STREAM_FRAME_BYTES);

        let mut decoder = OllamaNdjsonDecoder::new();
        assert!(decoder.push(&chunk).unwrap().is_empty());
    }

    #[test]
    fn ollama_ndjson_decodes_arbitrary_chunks_empty_lines_usage_and_done() {
        let fixture = concat!(
            "\n",
            "{\"model\":\"qwen3\",\"created_at\":\"2026-07-22T10:00:00Z\",\"message\":{\"role\":\"assistant\",\"thinking\":\"分析\",\"content\":\"答案\"},\"done\":false}\r\n",
            "\r\n",
            "{\"model\":\"qwen3\",\"created_at\":\"2026-07-22T10:00:01Z\",\"message\":{\"role\":\"assistant\",\"content\":\"。\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":11,\"eval_count\":3}\n",
        );
        let mut decoder = OllamaNdjsonDecoder::new();
        let mut events = Vec::new();

        for chunk in fixture.as_bytes().chunks(5) {
            events.extend(decoder.push(chunk).unwrap());
        }
        events.extend(decoder.finish().unwrap());

        assert_eq!(
            events,
            vec![
                RunEvent::ProviderMetadata {
                    request_id: None,
                    model: Some("qwen3".to_owned()),
                    created_at: Some("2026-07-22T10:00:00Z".to_owned()),
                },
                RunEvent::ReasoningDelta {
                    text: "分析".to_owned(),
                },
                RunEvent::TextDelta {
                    text: "答案".to_owned(),
                },
                RunEvent::TextDelta {
                    text: "。".to_owned(),
                },
                RunEvent::UsageUpdated {
                    usage: Usage {
                        prompt_tokens: Some(11),
                        completion_tokens: Some(3),
                        total_tokens: Some(14),
                    },
                },
                RunEvent::RunCompleted {
                    finish_reason: Some("stop".to_owned()),
                },
            ]
        );
        assert!(decoder.is_terminal());
    }

    #[test]
    fn ollama_ndjson_reports_a_stream_error_object() {
        let mut decoder = OllamaNdjsonDecoder::new();
        let error = decoder
            .push(b"{\"error\":\"model requires more memory\"}\n")
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid provider response: model requires more memory"
        );
    }

    #[test]
    fn ollama_ndjson_reports_a_stream_that_ends_before_done() {
        let mut decoder = OllamaNdjsonDecoder::new();
        decoder
            .push(b"{\"message\":{\"content\":\"partial\"},\"done\":false}\n")
            .unwrap();

        assert_eq!(
            decoder.finish().unwrap_err(),
            crate::ports::provider::ProviderError::UnexpectedEof
        );
    }
}
