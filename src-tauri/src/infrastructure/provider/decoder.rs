use crate::ports::provider::{ProviderDialect, ProviderError, RunEvent};

pub(crate) const MAX_PROVIDER_STREAM_FRAME_BYTES: usize = 1024 * 1024;

pub(crate) fn checked_stream_frame_size(
    current: usize,
    additional: usize,
    frame_name: &'static str,
) -> Result<usize, ProviderError> {
    let size = current.checked_add(additional).ok_or_else(|| {
        ProviderError::InvalidResponse(format!(
            "{frame_name} exceeds the {MAX_PROVIDER_STREAM_FRAME_BYTES}-byte stream frame limit"
        ))
    })?;
    if size > MAX_PROVIDER_STREAM_FRAME_BYTES {
        return Err(ProviderError::InvalidResponse(format!(
            "{frame_name} exceeds the {MAX_PROVIDER_STREAM_FRAME_BYTES}-byte stream frame limit"
        )));
    }
    Ok(size)
}

pub(crate) fn append_stream_frame_fragment(
    buffer: &mut Vec<u8>,
    fragment: &[u8],
    frame_name: &'static str,
) -> Result<(), ProviderError> {
    checked_stream_frame_size(buffer.len(), fragment.len(), frame_name)?;
    buffer.extend_from_slice(fragment);
    Ok(())
}

pub trait ProviderStreamDecoder: Send {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<RunEvent>, ProviderError>;
    fn finish(&mut self) -> Result<Vec<RunEvent>, ProviderError>;
    fn is_terminal(&self) -> bool;
}

pub fn decode_http_error(
    dialect: ProviderDialect,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> ProviderError {
    decode_http_error_with_redaction(dialect, status, content_type, body, str::to_owned)
}

pub fn decode_http_error_with_redaction<F>(
    dialect: ProviderDialect,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
    redact_bounded: F,
) -> ProviderError
where
    F: Fn(&str) -> String,
{
    let retryable = matches!(status, 408 | 409 | 425 | 429 | 500..=599);
    let parsed = serde_json::from_slice::<serde_json::Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| extract_error_message(value, &redact_bounded))
        .unwrap_or_else(|| sanitize_plain_error(body, content_type, &redact_bounded));
    let provider_code = parsed
        .as_ref()
        .and_then(|value| extract_error_code(dialect, value, &redact_bounded));

    ProviderError::Http {
        status,
        provider_code,
        message,
        retryable,
    }
}

fn extract_error_message<F>(value: &serde_json::Value, redact_bounded: &F) -> Option<String>
where
    F: Fn(&str) -> String,
{
    value
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("error").and_then(serde_json::Value::as_str))
        .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
        // Provider credentials are opaque. Redact the raw decoded JSON field
        // before trim/truncation can change a whitespace-bearing secret.
        .map(redact_bounded)
        .map(|message| truncate(message.trim(), 512))
        .filter(|message| !message.is_empty())
}

fn extract_error_code<F>(
    dialect: ProviderDialect,
    value: &serde_json::Value,
    redact_bounded: &F,
) -> Option<String>
where
    F: Fn(&str) -> String,
{
    let candidates = match dialect {
        ProviderDialect::OpenAiChatCompletions => {
            [value.pointer("/error/code"), value.pointer("/error/type")]
        }
        ProviderDialect::AnthropicMessages => [value.pointer("/error/type"), None],
        ProviderDialect::GoogleGenerativeAi => [value.pointer("/error/status"), None],
        ProviderDialect::OllamaChat => [value.pointer("/error/code"), value.get("code")],
    };
    candidates.into_iter().flatten().find_map(|code| {
        let raw = match code {
            serde_json::Value::String(code) => code.clone(),
            serde_json::Value::Number(code) => code.to_string(),
            _ => return None,
        };
        // Provider credentials are opaque. Redact the exact decoded field
        // before trimming or truncating can transform a whitespace-bearing
        // secret.
        let redacted = redact_bounded(&raw);
        let normalized = truncate(redacted.trim(), 512);
        (!normalized.is_empty()).then_some(normalized)
    })
}

fn sanitize_plain_error<F>(body: &[u8], content_type: Option<&str>, redact_bounded: &F) -> String
where
    F: Fn(&str) -> String,
{
    let raw_text = String::from_utf8_lossy(body);
    let looks_like_markup = content_type.is_some_and(|kind| kind.contains("html"))
        || (raw_text.contains('<') && raw_text.contains('>'));
    // Redact before markup stripping, whitespace collapse, or truncation can
    // transform the exact credential into a value the final safety pass no
    // longer recognizes.
    let redacted = redact_bounded(&raw_text);
    let text = if looks_like_markup {
        strip_markup(&redacted)
    } else {
        redacted
    };
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let message = truncate(collapsed.trim(), 512);
    if message.is_empty() {
        "provider request failed without an error message".to_owned()
    } else {
        message
    }
}

fn strip_markup(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut inside_tag = false;
    for character in input.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => {
                inside_tag = false;
                output.push(' ');
            }
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    output
}

fn truncate(input: &str, limit: usize) -> String {
    let mut characters = input.chars();
    let prefix = characters.by_ref().take(limit).collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use crate::ports::provider::{ProviderDialect, ProviderError};

    use super::{decode_http_error, decode_http_error_with_redaction};

    #[test]
    fn provider_error_machine_codes_follow_the_authoritative_dialect() {
        let body =
            br#"{"error":{"code":"openai_code","type":"anthropic_type","status":"GOOGLE_STATUS"}}"#;
        let cases = [
            (ProviderDialect::OpenAiChatCompletions, "openai_code"),
            (ProviderDialect::AnthropicMessages, "anthropic_type"),
            (ProviderDialect::GoogleGenerativeAi, "GOOGLE_STATUS"),
            (ProviderDialect::OllamaChat, "openai_code"),
        ];

        for (dialect, expected) in cases {
            let ProviderError::Http { provider_code, .. } =
                decode_http_error(dialect, 429, Some("application/json"), body)
            else {
                panic!("expected an HTTP error");
            };
            assert_eq!(provider_code.as_deref(), Some(expected));
        }

        let ProviderError::Http { provider_code, .. } = decode_http_error(
            ProviderDialect::OpenAiChatCompletions,
            429,
            Some("application/json"),
            br#"{"error":{"type":"rate_limit_error"}}"#,
        ) else {
            panic!("expected an HTTP error");
        };
        assert_eq!(provider_code.as_deref(), Some("rate_limit_error"));

        let ProviderError::Http { provider_code, .. } = decode_http_error(
            ProviderDialect::OllamaChat,
            500,
            Some("application/json"),
            br#"{"code":"ollama_generic"}"#,
        ) else {
            panic!("expected an HTTP error");
        };
        assert_eq!(provider_code.as_deref(), Some("ollama_generic"));

        let ProviderError::Http { provider_code, .. } = decode_http_error(
            ProviderDialect::OpenAiChatCompletions,
            429,
            Some("application/json"),
            br#"{"error":{"code":null,"type":"rate_limit_error"}}"#,
        ) else {
            panic!("expected an HTTP error");
        };
        assert_eq!(provider_code.as_deref(), Some("rate_limit_error"));
    }

    #[test]
    fn provider_error_machine_codes_redact_before_normalization_and_degrade_safely() {
        let secret = " key  with spaces ";
        for (dialect, body) in [
            (
                ProviderDialect::AnthropicMessages,
                br#"{"error":{"type":" key  with spaces "}}"#.as_slice(),
            ),
            (
                ProviderDialect::GoogleGenerativeAi,
                br#"{"error":{"status":" key  with spaces "}}"#.as_slice(),
            ),
        ] {
            let ProviderError::Http { provider_code, .. } = decode_http_error_with_redaction(
                dialect,
                429,
                Some("application/json"),
                body,
                |value| value.replace(secret, "[SAFE]"),
            ) else {
                panic!("expected an HTTP error");
            };
            assert_eq!(provider_code.as_deref(), Some("[SAFE]"));
        }

        let long_code = "x".repeat(600);
        let body =
            serde_json::json!({ "error": { "status": format!("  {long_code}  ") } }).to_string();
        let ProviderError::Http { provider_code, .. } = decode_http_error(
            ProviderDialect::GoogleGenerativeAi,
            429,
            Some("application/json"),
            body.as_bytes(),
        ) else {
            panic!("expected an HTTP error");
        };
        let provider_code = provider_code.expect("a string status should be retained");
        assert_eq!(provider_code.chars().count(), 513);
        assert!(provider_code.ends_with('…'));

        for body in [
            br#"{"error":{"status":{"nested":true}}}"#.as_slice(),
            br#"{"error":{"message":"missing status"}}"#.as_slice(),
            b"{not-json".as_slice(),
        ] {
            let ProviderError::Http { provider_code, .. } = decode_http_error(
                ProviderDialect::GoogleGenerativeAi,
                429,
                Some("application/json"),
                body,
            ) else {
                panic!("expected an HTTP error");
            };
            assert_eq!(provider_code, None);
        }
    }

    #[test]
    fn provider_error_bodies_are_normalized_without_exposing_response_markup() {
        assert_eq!(
            decode_http_error(
                ProviderDialect::OpenAiChatCompletions,
                429,
                Some("application/json"),
                br#"{"error":{"message":"rate limit reached","type":"rate_limit_error","code":"rpm"}}"#,
            ),
            ProviderError::Http {
                status: 429,
                provider_code: Some("rpm".to_owned()),
                message: "rate limit reached".to_owned(),
                retryable: true,
            }
        );
        assert_eq!(
            decode_http_error(
                ProviderDialect::OpenAiChatCompletions,
                500,
                Some("application/json"),
                br#"{"error":"runner crashed"}"#,
            ),
            ProviderError::Http {
                status: 500,
                provider_code: None,
                message: "runner crashed".to_owned(),
                retryable: true,
            }
        );
        assert_eq!(
            decode_http_error(
                ProviderDialect::OpenAiChatCompletions,
                401,
                Some("text/html"),
                b"<h1>Unauthorized</h1>",
            ),
            ProviderError::Http {
                status: 401,
                provider_code: None,
                message: "Unauthorized".to_owned(),
                retryable: false,
            }
        );
        assert_eq!(
            decode_http_error(
                ProviderDialect::OpenAiChatCompletions,
                400,
                Some("application/json"),
                br#"{"error":{"message":"bad request","code":""}}"#,
            ),
            ProviderError::Http {
                status: 400,
                provider_code: None,
                message: "bad request".to_owned(),
                retryable: false,
            }
        );
    }

    #[test]
    fn raw_json_fields_are_redacted_before_trim_collapse_and_truncation() {
        let whitespace_secret = " key  with spaces ";
        let redact = |value: &str| value.replace(whitespace_secret, "[SAFE]");
        assert_eq!(
            decode_http_error_with_redaction(
                ProviderDialect::OpenAiChatCompletions,
                401,
                Some("application/json"),
                br#"{"error":{"message":"before  key  with spaces  after","code":" key  with spaces "}}"#,
                redact,
            ),
            ProviderError::Http {
                status: 401,
                provider_code: Some("[SAFE]".to_owned()),
                message: "before [SAFE] after".to_owned(),
                retryable: false,
            }
        );

        let boundary_secret = "secret-after-boundary";
        let boundary_message = format!("{}{}", "x".repeat(510), boundary_secret);
        let body = serde_json::json!({ "error": { "message": boundary_message } }).to_string();
        let error = decode_http_error_with_redaction(
            ProviderDialect::OpenAiChatCompletions,
            500,
            Some("application/json"),
            body.as_bytes(),
            |value| value.replace(boundary_secret, "[SAFE]"),
        );
        let ProviderError::Http { message, .. } = error else {
            panic!("expected an HTTP error");
        };
        assert!(!message.contains(boundary_secret));
        assert!(!message.ends_with("se…"));
    }

    #[test]
    fn raw_html_and_plain_text_are_redacted_before_lossy_normalization() {
        let repeated_whitespace_secret = "key  with  spaces";
        let html = format!("<p>before {repeated_whitespace_secret} after</p>");
        let html_error = decode_http_error_with_redaction(
            ProviderDialect::OpenAiChatCompletions,
            429,
            Some("text/html"),
            html.as_bytes(),
            |value| value.replace(repeated_whitespace_secret, "[SAFE]"),
        );
        assert_eq!(
            html_error,
            ProviderError::Http {
                status: 429,
                provider_code: None,
                message: "before [SAFE] after".to_owned(),
                retryable: true,
            }
        );

        let markup_secret = "<em>secret</em>";
        let plain = format!("provider reflected {markup_secret}");
        let plain_error = decode_http_error_with_redaction(
            ProviderDialect::OpenAiChatCompletions,
            401,
            Some("text/plain"),
            plain.as_bytes(),
            |value| value.replace(markup_secret, "[SAFE]"),
        );
        assert_eq!(
            plain_error,
            ProviderError::Http {
                status: 401,
                provider_code: None,
                message: "provider reflected [SAFE]".to_owned(),
                retryable: false,
            }
        );
    }
}
