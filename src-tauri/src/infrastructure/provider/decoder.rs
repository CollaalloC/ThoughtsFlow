use crate::ports::provider::{ProviderError, RunEvent};

pub trait ProviderStreamDecoder: Send {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<RunEvent>, ProviderError>;
    fn finish(&mut self) -> Result<Vec<RunEvent>, ProviderError>;
    fn is_terminal(&self) -> bool;
}

pub fn decode_http_error(status: u16, content_type: Option<&str>, body: &[u8]) -> ProviderError {
    let retryable = matches!(status, 408 | 409 | 425 | 429 | 500..=599);
    let parsed = serde_json::from_slice::<serde_json::Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(extract_error_message)
        .unwrap_or_else(|| sanitize_plain_error(body, content_type));
    let provider_code = parsed.as_ref().and_then(extract_error_code);

    ProviderError::Http {
        status,
        provider_code,
        message,
        retryable,
    }
}

fn extract_error_message(value: &serde_json::Value) -> Option<String> {
    value
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("error").and_then(serde_json::Value::as_str))
        .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
        .map(|message| truncate(message.trim(), 512))
        .filter(|message| !message.is_empty())
}

fn extract_error_code(value: &serde_json::Value) -> Option<String> {
    value
        .pointer("/error/code")
        .or_else(|| value.get("code"))
        .and_then(|code| match code {
            serde_json::Value::String(code) => Some(code.clone()),
            serde_json::Value::Number(code) => Some(code.to_string()),
            _ => None,
        })
}

fn sanitize_plain_error(body: &[u8], content_type: Option<&str>) -> String {
    let text = String::from_utf8_lossy(body);
    let looks_like_markup = content_type.is_some_and(|kind| kind.contains("html"))
        || (text.contains('<') && text.contains('>'));
    let text = if looks_like_markup {
        strip_markup(&text)
    } else {
        text.into_owned()
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
    use crate::ports::provider::ProviderError;

    use super::decode_http_error;

    #[test]
    fn provider_error_bodies_are_normalized_without_exposing_response_markup() {
        assert_eq!(
            decode_http_error(
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
            decode_http_error(401, Some("text/html"), b"<h1>Unauthorized</h1>"),
            ProviderError::Http {
                status: 401,
                provider_code: None,
                message: "Unauthorized".to_owned(),
                retryable: false,
            }
        );
    }
}
