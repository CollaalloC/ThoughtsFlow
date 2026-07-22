use serde::Deserialize;

use crate::{
    infrastructure::provider::decoder::ProviderStreamDecoder,
    ports::provider::{ProviderError, RunEvent, Usage},
};

#[derive(Default)]
pub struct OpenAiSseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<String>,
    metadata_emitted: bool,
    finish_reason: Option<String>,
    terminal: bool,
}

impl OpenAiSseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<RunEvent>,
    ) -> Result<(), ProviderError> {
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
            ProviderError::InvalidResponse(format!("SSE data is not valid UTF-8: {error}"))
        })?;
        self.data_lines.push(data.to_owned());
        Ok(())
    }

    fn dispatch_event(&mut self, events: &mut Vec<RunEvent>) -> Result<(), ProviderError> {
        if self.data_lines.is_empty() || self.terminal {
            self.data_lines.clear();
            return Ok(());
        }
        let payload = self.data_lines.join("\n");
        self.data_lines.clear();
        let payload = payload.trim();
        if payload.is_empty() {
            return Ok(());
        }
        if payload == "[DONE]" {
            self.terminal = true;
            events.push(RunEvent::RunCompleted {
                finish_reason: self.finish_reason.take(),
            });
            return Ok(());
        }

        let chunk: OpenAiChunk = serde_json::from_str(payload).map_err(|error| {
            ProviderError::InvalidResponse(format!("invalid OpenAI SSE JSON: {error}"))
        })?;
        if let Some(error) = chunk.error {
            return Err(ProviderError::InvalidResponse(error.message));
        }

        if !self.metadata_emitted
            && (chunk.id.is_some() || chunk.model.is_some() || chunk.created.is_some())
        {
            self.metadata_emitted = true;
            events.push(RunEvent::ProviderMetadata {
                request_id: chunk.id,
                model: chunk.model,
                created_at: chunk.created.map(|created| created.to_string()),
            });
        }

        if let Some(choice) = chunk.choices.into_iter().find(|choice| choice.index == 0) {
            if let Some(reasoning) = choice
                .delta
                .reasoning_content
                .or(choice.delta.reasoning)
                .filter(|text| !text.is_empty())
            {
                events.push(RunEvent::ReasoningDelta { text: reasoning });
            }
            if let Some(text) = choice.delta.content.filter(|text| !text.is_empty()) {
                events.push(RunEvent::TextDelta { text });
            }
            if choice.finish_reason.is_some() {
                self.finish_reason = choice.finish_reason;
            }
        }

        if let Some(usage) = chunk.usage {
            events.push(RunEvent::UsageUpdated {
                usage: Usage {
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                    total_tokens: usage.total_tokens,
                },
            });
        }
        Ok(())
    }
}

impl ProviderStreamDecoder for OpenAiSseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<RunEvent>, ProviderError> {
        if self.terminal {
            return Ok(Vec::new());
        }
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            self.process_line(&line, &mut events)?;
        }
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
            if self.finish_reason.is_some() {
                self.terminal = true;
                events.push(RunEvent::RunCompleted {
                    finish_reason: self.finish_reason.take(),
                });
            } else {
                return Err(ProviderError::UnexpectedEof);
            }
        }
        Ok(events)
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }
}

#[derive(Deserialize)]
struct OpenAiChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
    #[serde(default)]
    error: Option<OpenAiStreamError>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct OpenAiStreamError {
    message: String,
}

#[cfg(test)]
mod tests {
    use crate::{
        infrastructure::provider::ProviderStreamDecoder,
        ports::provider::{RunEvent, Usage},
    };

    use super::OpenAiSseDecoder;

    #[test]
    fn openai_sse_decodes_arbitrary_chunks_heartbeats_reasoning_usage_and_done() {
        let fixture = concat!(
            ": keep-alive\r\n\r\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-local\",\"choices\":[{\"delta\":{\"content\":\"你\"},\"finish_reason\":null}]}\r\n\r\n",
            "\r\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-local\",\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-local\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2,\"total_tokens\":9}}\n\n",
            "data: [DONE]\n\n",
        );
        let mut decoder = OpenAiSseDecoder::new();
        let mut events = Vec::new();

        for chunk in fixture.as_bytes().chunks(7) {
            events.extend(decoder.push(chunk).unwrap());
        }
        events.extend(decoder.finish().unwrap());

        assert_eq!(
            events,
            vec![
                RunEvent::ProviderMetadata {
                    request_id: Some("chatcmpl-1".to_owned()),
                    model: Some("gpt-local".to_owned()),
                    created_at: None,
                },
                RunEvent::TextDelta {
                    text: "你".to_owned(),
                },
                RunEvent::ReasoningDelta {
                    text: "thinking".to_owned(),
                },
                RunEvent::UsageUpdated {
                    usage: Usage {
                        prompt_tokens: Some(7),
                        completion_tokens: Some(2),
                        total_tokens: Some(9),
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
    fn openai_sse_accepts_done_without_a_final_blank_line_and_emits_one_terminal_event() {
        let mut decoder = OpenAiSseDecoder::new();
        assert!(decoder.push(b"data: [DO").unwrap().is_empty());
        assert!(decoder.push(b"NE]").unwrap().is_empty());

        assert_eq!(
            decoder.finish().unwrap(),
            vec![RunEvent::RunCompleted {
                finish_reason: None,
            }]
        );
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn openai_sse_reports_a_stream_that_ends_without_done_or_finish_reason() {
        let mut decoder = OpenAiSseDecoder::new();
        decoder
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n")
            .unwrap();

        assert_eq!(
            decoder.finish().unwrap_err(),
            crate::ports::provider::ProviderError::UnexpectedEof
        );
    }
}
