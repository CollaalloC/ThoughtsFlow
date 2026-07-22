use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderName};
use reqwest::redirect::Policy;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    infrastructure::provider::{
        OllamaNdjsonDecoder, OpenAiSseDecoder, ProviderStreamDecoder, decode_http_error,
        provider_request_url,
    },
    ports::provider::{
        CanonicalMessage, CanonicalRequest, CredentialPlacement, MessageRole,
        ProviderConnectionFuture, ProviderConnectionStatus, ProviderConnectionTester,
        ProviderDialect, ProviderError, ProviderFuture, ProviderGateway, ProviderInvocation,
        ProviderTarget, RunEvent, SessionCredential,
    },
};

const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(20);
const CREDENTIAL_REDACTION_MARKER: &str = "[REDACTED]";

pub struct ReqwestProviderGateway {
    client: reqwest::Client,
}

impl ReqwestProviderGateway {
    fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub fn with_defaults() -> Result<Self, ProviderError> {
        reqwest::Client::builder()
            // A redirect can change the real Context destination after the
            // original endpoint has been validated and frozen in the Receipt.
            // Provider requests therefore fail closed on every 3xx response.
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent("ThoughsFlow/0.1")
            .build()
            .map(Self::new)
            .map_err(|error| ProviderError::Transport(error.to_string()))
    }

    async fn run_stream(
        &self,
        invocation: ProviderInvocation,
        cancellation: CancellationToken,
        events: mpsc::Sender<RunEvent>,
    ) -> Result<(), ProviderError> {
        let mut redactor = ProviderEventRedactor::new(invocation.credential.clone());
        if cancellation.is_cancelled() {
            return send_redacted_terminal(&events, &mut redactor, RunEvent::RunCancelled).await;
        }

        let url = provider_request_url(&invocation.target.base_url, invocation.target.dialect)?;
        let body = request_body(&invocation.request, invocation.target.dialect);
        let accept = match invocation.target.dialect {
            ProviderDialect::OpenAiChatCompletions => "text/event-stream",
            ProviderDialect::OllamaChat => "application/x-ndjson",
        };
        let mut request = self
            .client
            .post(url)
            .header(ACCEPT, accept)
            .header(CONTENT_TYPE, "application/json")
            .json(&body);
        request = apply_additional_headers(request, &invocation.target.additional_headers)?;
        request = apply_credential(
            request,
            &invocation.target.credential_placement,
            invocation.credential.as_ref(),
        )?;

        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return send_redacted_terminal(
                    &events,
                    &mut redactor,
                    RunEvent::RunCancelled,
                ).await;
            }
            response = request.send() => response,
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return send_failure(
                    &events,
                    &mut redactor,
                    ProviderError::Transport(error.to_string()),
                )
                .await;
            }
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = match collect_error_body(response, &cancellation).await {
                Ok(Some(body)) => body,
                Ok(None) => {
                    return send_redacted_terminal(&events, &mut redactor, RunEvent::RunCancelled)
                        .await;
                }
                Err(error) => return send_failure(&events, &mut redactor, error).await,
            };
            return send_failure(
                &events,
                &mut redactor,
                decode_http_error(status, content_type.as_deref(), &body),
            )
            .await;
        }

        if !send_redacted_event(
            &events,
            &mut redactor,
            RunEvent::RunStarted {
                run_id: invocation.request.run_id,
            },
            &cancellation,
        )
        .await?
        {
            return Ok(());
        }

        let mut decoder: Box<dyn ProviderStreamDecoder> = match invocation.target.dialect {
            ProviderDialect::OpenAiChatCompletions => Box::new(OpenAiSseDecoder::new()),
            ProviderDialect::OllamaChat => Box::new(OllamaNdjsonDecoder::new()),
        };
        let mut bytes = response.bytes_stream();
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    return send_redacted_terminal(
                        &events,
                        &mut redactor,
                        RunEvent::RunCancelled,
                    ).await;
                }
                next = bytes.next() => next,
            };
            match next {
                Some(Ok(chunk)) => {
                    let decoded = match decoder.push(&chunk) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            return send_failure(&events, &mut redactor, error).await;
                        }
                    };
                    for event in decoded {
                        if !send_redacted_event(&events, &mut redactor, event, &cancellation)
                            .await?
                        {
                            return Ok(());
                        }
                    }
                    if decoder.is_terminal() {
                        return Ok(());
                    }
                }
                Some(Err(error)) => {
                    return send_failure(
                        &events,
                        &mut redactor,
                        ProviderError::Transport(error.to_string()),
                    )
                    .await;
                }
                None => {
                    let decoded = match decoder.finish() {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            return send_failure(&events, &mut redactor, error).await;
                        }
                    };
                    for event in decoded {
                        if !send_redacted_event(&events, &mut redactor, event, &cancellation)
                            .await?
                        {
                            return Ok(());
                        }
                    }
                    return Ok(());
                }
            }
        }
    }
}

struct ProviderEventRedactor {
    credential: Option<SessionCredential>,
    pending: Option<PendingDelta>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeltaKind {
    Text,
    Reasoning,
}

struct PendingDelta {
    kind: DeltaKind,
    text: String,
}

impl ProviderEventRedactor {
    fn new(credential: Option<SessionCredential>) -> Self {
        Self {
            credential: credential.filter(|value| !value.is_empty()),
            pending: None,
        }
    }

    fn redact(&mut self, event: RunEvent) -> Vec<RunEvent> {
        match event {
            RunEvent::TextDelta { text } => self.redact_delta(DeltaKind::Text, text),
            RunEvent::ReasoningDelta { text } => self.redact_delta(DeltaKind::Reasoning, text),
            event => {
                // A Provider-controlled value can continue a credential prefix
                // in another event field. Discard that ambiguous suffix at the
                // boundary instead of emitting a marker that could itself form
                // part of the credential with the next field.
                self.pending = None;
                vec![self.redact_atomic_event(event)]
            }
        }
    }

    fn redact_delta(&mut self, kind: DeltaKind, value: String) -> Vec<RunEvent> {
        let credential = self
            .credential
            .as_ref()
            .map(SessionCredential::expose_secret);
        let mut events = Vec::with_capacity(2);
        let mut pending = match self.pending.take() {
            Some(pending) if pending.kind == kind => pending.text,
            Some(_) => String::new(),
            None => String::new(),
        };
        if let Some(text) = redact_streaming_value(&mut pending, &value, credential) {
            events.push(delta_event(kind, text));
        }
        if !pending.is_empty() {
            self.pending = Some(PendingDelta {
                kind,
                text: pending,
            });
        }
        events
    }

    fn redact_atomic_event(&self, event: RunEvent) -> RunEvent {
        let credential = self
            .credential
            .as_ref()
            .map(SessionCredential::expose_secret);
        match event {
            // The run id is generated locally and is not Provider-controlled.
            RunEvent::RunStarted { run_id } => RunEvent::RunStarted { run_id },
            RunEvent::ProviderMetadata {
                request_id,
                model,
                created_at,
            } => RunEvent::ProviderMetadata {
                request_id: redact_bounded_optional_value(request_id, credential),
                model: redact_bounded_optional_value(model, credential),
                created_at: redact_bounded_optional_value(created_at, credential),
            },
            RunEvent::RunCompleted { finish_reason } => RunEvent::RunCompleted {
                finish_reason: redact_bounded_optional_value(finish_reason, credential),
            },
            RunEvent::RunFailed {
                code,
                message,
                retryable,
                status,
            } => RunEvent::RunFailed {
                code: redact_bounded_value(&code, credential),
                message: redact_bounded_value(&message, credential),
                retryable,
                status,
            },
            event => event,
        }
    }
}

fn delta_event(kind: DeltaKind, text: String) -> RunEvent {
    match kind {
        DeltaKind::Text => RunEvent::TextDelta { text },
        DeltaKind::Reasoning => RunEvent::ReasoningDelta { text },
    }
}

fn redact_bounded_optional_value(
    value: Option<String>,
    credential: Option<&str>,
) -> Option<String> {
    value.map(|value| redact_bounded_value(&value, credential))
}

fn redact_bounded_value(value: &str, credential: Option<&str>) -> String {
    let mut pending = String::new();
    // A bounded Provider field is an explicit scanner boundary. Any trailing
    // credential prefix remains in `pending` and is intentionally discarded,
    // so adjacent fields cannot reconstruct the credential.
    redact_streaming_value(&mut pending, value, credential).unwrap_or_default()
}

fn marker_has_credential_boundary_overlap(marker: &str, credential: &str) -> bool {
    let marker_ends_with_prefix = credential
        .char_indices()
        .skip(1)
        .map(|(index, _)| index)
        .chain(std::iter::once(credential.len()))
        .any(|length| marker.ends_with(&credential[..length]));
    let marker_starts_with_suffix = credential
        .char_indices()
        .skip(1)
        .map(|(index, _)| index)
        .any(|index| marker.starts_with(&credential[index..]));
    marker_ends_with_prefix || marker_starts_with_suffix
}

fn redaction_marker(credential: &str) -> &'static str {
    if CREDENTIAL_REDACTION_MARKER.contains(credential)
        || marker_has_credential_boundary_overlap(CREDENTIAL_REDACTION_MARKER, credential)
    {
        return "";
    }
    CREDENTIAL_REDACTION_MARKER
}

fn redact_exact_value(value: &str, credential: Option<&str>) -> String {
    let Some(credential) = credential.filter(|value| !value.is_empty()) else {
        return value.to_owned();
    };
    let marker = redaction_marker(credential);
    let mut redacted = value.to_owned();
    while redacted.contains(credential) {
        redacted = redacted.replace(credential, marker);
    }
    redacted
}

fn redact_streaming_value(
    pending: &mut String,
    value: &str,
    credential: Option<&str>,
) -> Option<String> {
    let Some(credential) = credential.filter(|value| !value.is_empty()) else {
        return Some(value.to_owned());
    };

    let mut combined = std::mem::take(pending);
    combined.push_str(value);
    let redacted = redact_exact_value(&combined, Some(credential));
    let pending_bytes = credential
        .char_indices()
        .map(|(index, _)| index)
        .filter(|index| *index > 0)
        .rev()
        .find(|index| redacted.ends_with(&credential[..*index]))
        .unwrap_or(0);
    let emitted_bytes = redacted.len() - pending_bytes;
    *pending = redacted[emitted_bytes..].to_owned();
    let emitted = redacted[..emitted_bytes].to_owned();
    (!emitted.is_empty()).then_some(emitted)
}

impl ProviderGateway for ReqwestProviderGateway {
    fn stream<'a>(
        &'a self,
        invocation: ProviderInvocation,
        cancellation: CancellationToken,
        events: mpsc::Sender<RunEvent>,
    ) -> ProviderFuture<'a> {
        Box::pin(self.run_stream(invocation, cancellation, events))
    }
}

impl ProviderConnectionTester for ReqwestProviderGateway {
    fn test<'a>(
        &'a self,
        target: ProviderTarget,
        credential: Option<crate::ports::provider::SessionCredential>,
    ) -> ProviderConnectionFuture<'a> {
        Box::pin(async move {
            let mut endpoint =
                crate::infrastructure::provider::validate_base_url(&target.base_url)?;
            let base_path = endpoint.path().trim_end_matches('/');
            let path = match target.dialect {
                ProviderDialect::OllamaChat if base_path.ends_with("/api") => {
                    format!("{base_path}/tags")
                }
                ProviderDialect::OllamaChat => format!("{base_path}/api/tags"),
                ProviderDialect::OpenAiChatCompletions => format!("{base_path}/models"),
            };
            endpoint.set_path(&path);
            let mut request = self.client.get(endpoint).timeout(CONNECTION_TEST_TIMEOUT);
            request = apply_additional_headers(request, &target.additional_headers)?;
            request = apply_credential(request, &target.credential_placement, credential.as_ref())?;
            let response = request
                .send()
                .await
                .map_err(|error| ProviderError::Transport(error.to_string()))?;
            Ok(ProviderConnectionStatus {
                ok: response.status().is_success(),
                http_status: response.status().as_u16(),
            })
        })
    }
}

fn apply_additional_headers(
    mut request: reqwest::RequestBuilder,
    headers: &std::collections::BTreeMap<String, String>,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| ProviderError::InvalidResponse("invalid Provider header name".into()))?;
        request = request.header(name, value);
    }
    Ok(request)
}

fn apply_credential(
    request: reqwest::RequestBuilder,
    placement: &CredentialPlacement,
    credential: Option<&SessionCredential>,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    let Some(credential) = credential.filter(|value| !value.is_empty()) else {
        return Ok(request);
    };
    match placement {
        CredentialPlacement::None => Ok(request),
        CredentialPlacement::BearerHeader => Ok(request.bearer_auth(credential.expose_secret())),
        CredentialPlacement::Header(header_name) => {
            let header_name = HeaderName::from_bytes(header_name.as_bytes()).map_err(|_| {
                ProviderError::InvalidResponse("invalid credential header name".into())
            })?;
            Ok(request.header(header_name, credential.expose_secret()))
        }
    }
}

async fn send_redacted_event(
    events: &mpsc::Sender<RunEvent>,
    redactor: &mut ProviderEventRedactor,
    event: RunEvent,
    cancellation: &CancellationToken,
) -> Result<bool, ProviderError> {
    let terminal = event.is_terminal();
    if !terminal && cancellation.is_cancelled() {
        send_redacted_terminal(events, redactor, RunEvent::RunCancelled).await?;
        return Ok(false);
    }

    let redacted = redactor.redact(event);
    send_prepared_redacted_events(events, redactor, redacted, terminal, cancellation).await
}

async fn send_prepared_redacted_events(
    events: &mpsc::Sender<RunEvent>,
    redactor: &mut ProviderEventRedactor,
    redacted: Vec<RunEvent>,
    terminal: bool,
    cancellation: &CancellationToken,
) -> Result<bool, ProviderError> {
    let mut redacted = redacted.into_iter();
    while let Some(event) = redacted.next() {
        if terminal {
            events
                .send(event)
                .await
                .map_err(|_| ProviderError::EventChannelClosed)?;
            continue;
        }
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                // Redaction has already consumed this decoded event. Deliver
                // its safe representation before the cancellation terminal so
                // cancellation cannot silently discard accepted partial output.
                events
                    .send(event)
                    .await
                    .map_err(|_| ProviderError::EventChannelClosed)?;
                for remaining in redacted {
                    events
                        .send(remaining)
                        .await
                        .map_err(|_| ProviderError::EventChannelClosed)?;
                }
                send_redacted_terminal(events, redactor, RunEvent::RunCancelled).await?;
                return Ok(false);
            },
            result = events.send(event.clone()) => {
                result.map_err(|_| ProviderError::EventChannelClosed)?;
            },
        }
    }
    if !terminal && cancellation.is_cancelled() {
        send_redacted_terminal(events, redactor, RunEvent::RunCancelled).await?;
        return Ok(false);
    }
    Ok(true)
}

async fn send_failure(
    events: &mpsc::Sender<RunEvent>,
    redactor: &mut ProviderEventRedactor,
    error: ProviderError,
) -> Result<(), ProviderError> {
    send_redacted_terminal(events, redactor, error.to_run_failed()).await
}

async fn send_redacted_terminal(
    events: &mpsc::Sender<RunEvent>,
    redactor: &mut ProviderEventRedactor,
    event: RunEvent,
) -> Result<(), ProviderError> {
    for event in redactor.redact(event) {
        events
            .send(event)
            .await
            .map_err(|_| ProviderError::EventChannelClosed)?;
    }
    Ok(())
}

async fn collect_error_body(
    response: reqwest::Response,
    cancellation: &CancellationToken,
) -> Result<Option<Vec<u8>>, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while body.len() < MAX_ERROR_BODY_BYTES {
        let next = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(None),
            next = stream.next() => next,
        };
        match next {
            Some(Ok(chunk)) => {
                let remaining = MAX_ERROR_BODY_BYTES - body.len();
                body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            Some(Err(error)) => return Err(ProviderError::Transport(error.to_string())),
            None => break,
        }
    }
    Ok(Some(body))
}

fn request_body(request: &CanonicalRequest, dialect: ProviderDialect) -> Value {
    let messages = request
        .messages
        .iter()
        .map(canonical_message)
        .collect::<Vec<_>>();
    let mut body = Map::from_iter([
        ("model".to_owned(), json!(request.model)),
        ("messages".to_owned(), Value::Array(messages)),
        ("stream".to_owned(), Value::Bool(true)),
    ]);

    match dialect {
        ProviderDialect::OpenAiChatCompletions => {
            body.insert(
                "stream_options".to_owned(),
                json!({ "include_usage": true }),
            );
            insert_optional(&mut body, "temperature", request.temperature);
            insert_optional(&mut body, "top_p", request.top_p);
            insert_optional(&mut body, "max_tokens", request.max_output_tokens);
            if !request.stop.is_empty() {
                body.insert("stop".to_owned(), json!(request.stop));
            }
        }
        ProviderDialect::OllamaChat => {
            let mut options = Map::new();
            insert_optional(&mut options, "temperature", request.temperature);
            insert_optional(&mut options, "top_p", request.top_p);
            insert_optional(&mut options, "num_predict", request.max_output_tokens);
            if !request.stop.is_empty() {
                options.insert("stop".to_owned(), json!(request.stop));
            }
            if !options.is_empty() {
                body.insert("options".to_owned(), Value::Object(options));
            }
        }
    }
    Value::Object(body)
}

fn canonical_message(message: &CanonicalMessage) -> Value {
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    };
    json!({ "role": role, "content": message.content })
}

fn insert_optional<T: serde::Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), json!(value));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc as std_mpsc,
        thread,
        time::{Duration, Instant},
    };

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::ports::provider::{
        CanonicalRequest, CredentialPlacement, ProviderConnectionTester, ProviderDialect,
        ProviderGateway, ProviderInvocation, ProviderTarget, RunEvent, SessionCredential,
    };

    use super::{ProviderEventRedactor, ReqwestProviderGateway};

    const TEST_CREDENTIAL: &str = "sk-sensitive-token";

    fn openai_invocation(base_url: String, credential: Option<&str>) -> ProviderInvocation {
        ProviderInvocation {
            target: ProviderTarget {
                dialect: ProviderDialect::OpenAiChatCompletions,
                base_url,
                credential_placement: CredentialPlacement::Header("x-api-key".into()),
                additional_headers: std::collections::BTreeMap::new(),
            },
            credential: credential.map(SessionCredential::new),
            request: CanonicalRequest {
                run_id: "run-provider-contract".to_owned(),
                model: "fixture-model".to_owned(),
                messages: Vec::new(),
                temperature: None,
                top_p: None,
                max_output_tokens: None,
                stop: Vec::new(),
            },
        }
    }

    fn spawn_single_response(
        status: &str,
        headers: Vec<(String, String)>,
        body: String,
    ) -> (String, std_mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = std_mpsc::channel();
        let status = status.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            request_sender.send(request).unwrap();

            let mut response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                response.push_str(&name);
                response.push_str(": ");
                response.push_str(&value);
                response.push_str("\r\n");
            }
            response.push_str("\r\n");
            response.push_str(&body);
            stream.write_all(response.as_bytes()).unwrap();
        });
        (format!("http://{address}"), request_receiver, handle)
    }

    struct RedirectTarget {
        url: String,
        request_receiver: std_mpsc::Receiver<String>,
        stop_sender: std_mpsc::Sender<()>,
        handle: thread::JoinHandle<()>,
    }

    fn spawn_redirect_target() -> RedirectTarget {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = std_mpsc::channel();
        let (stop_sender, stop_receiver) = std_mpsc::channel();
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_http_request(&mut stream);
                        request_sender.send(request).unwrap();
                        let body = "data: [DONE]\n\n";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len(),
                        );
                        stream.write_all(response.as_bytes()).unwrap();
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("redirect target failed to accept: {error}"),
                }
                if stop_receiver.try_recv().is_ok() {
                    break;
                }
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        RedirectTarget {
            url: format!("http://{address}/stolen"),
            request_receiver,
            stop_sender,
            handle,
        }
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                continue;
            };
            let body_start = header_end + 4;
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= body_start + content_length {
                break;
            }
        }
        String::from_utf8_lossy(&request).into_owned()
    }

    #[test]
    fn credential_placement_controls_the_authoritative_request_header() {
        let client = reqwest::Client::new();
        let bearer = super::apply_credential(
            client.get("https://example.com/models"),
            &CredentialPlacement::BearerHeader,
            Some(&SessionCredential::new("secret-bearer")),
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            bearer.headers()[reqwest::header::AUTHORIZATION],
            "Bearer secret-bearer"
        );

        let api_key = super::apply_credential(
            client.get("https://example.com/models"),
            &CredentialPlacement::Header("x-api-key".into()),
            Some(&SessionCredential::new("secret-api-key")),
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(api_key.headers()["x-api-key"], "secret-api-key");
        assert!(
            !api_key
                .headers()
                .contains_key(reqwest::header::AUTHORIZATION),
            "custom API-key placement must not fall back to Authorization"
        );

        let local = super::apply_credential(
            client.get("https://example.com/models"),
            &CredentialPlacement::None,
            Some(&SessionCredential::new("must-not-be-sent")),
        )
        .unwrap()
        .build()
        .unwrap();
        assert!(!local.headers().contains_key(reqwest::header::AUTHORIZATION));

        let versioned = super::apply_additional_headers(
            client.get("https://example.com/models"),
            &std::collections::BTreeMap::from([("anthropic-version".into(), "2023-06-01".into())]),
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(versioned.headers()["anthropic-version"], "2023-06-01");
    }

    #[test]
    fn provider_events_redact_exact_credentials_across_stream_boundaries() {
        let mut redactor =
            ProviderEventRedactor::new(Some(SessionCredential::new(TEST_CREDENTIAL)));

        assert_eq!(
            redactor.redact(RunEvent::TextDelta {
                text: "before sk-sens".to_owned(),
            }),
            vec![RunEvent::TextDelta {
                text: "before ".to_owned(),
            }]
        );
        assert_eq!(
            redactor.redact(RunEvent::TextDelta {
                text: "itive-token after".to_owned(),
            }),
            vec![RunEvent::TextDelta {
                text: "[REDACTED] after".to_owned(),
            }]
        );
        assert_eq!(
            redactor.redact(RunEvent::ReasoningDelta {
                text: "reasoning sk-sens".to_owned(),
            }),
            vec![RunEvent::ReasoningDelta {
                text: "reasoning ".to_owned(),
            }]
        );
        assert_eq!(
            redactor.redact(RunEvent::ReasoningDelta {
                text: "itive-token".to_owned(),
            }),
            vec![RunEvent::ReasoningDelta {
                text: "[REDACTED]".to_owned(),
            }]
        );
    }

    #[test]
    fn provider_events_redact_metadata_errors_and_discard_ambiguous_partial_text() {
        let mut redactor =
            ProviderEventRedactor::new(Some(SessionCredential::new(TEST_CREDENTIAL)));
        assert!(
            redactor
                .redact(RunEvent::TextDelta {
                    text: "sk-sens".to_owned(),
                })
                .is_empty()
        );
        assert_eq!(
            redactor.redact(RunEvent::ProviderMetadata {
                request_id: Some(format!("id-{TEST_CREDENTIAL}")),
                model: Some(TEST_CREDENTIAL.to_owned()),
                created_at: None,
            }),
            vec![RunEvent::ProviderMetadata {
                request_id: Some("id-[REDACTED]".to_owned()),
                model: Some("[REDACTED]".to_owned()),
                created_at: None,
            }]
        );
        assert_eq!(
            redactor.redact(RunEvent::RunFailed {
                code: format!("bad-{TEST_CREDENTIAL}"),
                message: format!("provider reflected {TEST_CREDENTIAL}"),
                retryable: false,
                status: Some(401),
            }),
            vec![RunEvent::RunFailed {
                code: "bad-[REDACTED]".to_owned(),
                message: "provider reflected [REDACTED]".to_owned(),
                retryable: false,
                status: Some(401),
            }]
        );

        let mut completion_redactor =
            ProviderEventRedactor::new(Some(SessionCredential::new(TEST_CREDENTIAL)));
        assert_eq!(
            completion_redactor.redact(RunEvent::RunCompleted {
                finish_reason: Some(format!("stop-{TEST_CREDENTIAL}")),
            }),
            vec![RunEvent::RunCompleted {
                finish_reason: Some("stop-[REDACTED]".to_owned()),
            }]
        );
        assert!(
            !super::redact_exact_value("[REDACTED]", Some("RED")).contains("RED"),
            "the replacement marker must not itself re-emit a short credential"
        );
    }

    #[test]
    fn terminal_fields_cannot_complete_a_pending_credential_prefix() {
        let mut redactor =
            ProviderEventRedactor::new(Some(SessionCredential::new(TEST_CREDENTIAL)));
        assert!(
            redactor
                .redact(RunEvent::TextDelta {
                    text: "sk-sens".to_owned(),
                })
                .is_empty()
        );
        let events = redactor.redact(RunEvent::RunFailed {
            code: "provider_error".to_owned(),
            message: "itive-token".to_owned(),
            retryable: false,
            status: Some(401),
        });
        assert_eq!(
            events,
            vec![RunEvent::RunFailed {
                code: "provider_error".to_owned(),
                message: "itive-token".to_owned(),
                retryable: false,
                status: Some(401),
            }]
        );
        let persisted_strings = events
            .iter()
            .flat_map(|event| match event {
                RunEvent::TextDelta { text } => vec![text.as_str()],
                RunEvent::RunFailed { code, message, .. } => {
                    vec![code.as_str(), message.as_str()]
                }
                _ => Vec::new(),
            })
            .collect::<String>();
        assert!(!persisted_strings.contains(TEST_CREDENTIAL));
    }

    #[test]
    fn pending_text_and_reasoning_are_discarded_at_boundaries_without_reordering_safe_text() {
        let mut redactor =
            ProviderEventRedactor::new(Some(SessionCredential::new(TEST_CREDENTIAL)));
        assert_eq!(
            redactor.redact(RunEvent::ReasoningDelta {
                text: "thinking sk-sens".to_owned(),
            }),
            vec![RunEvent::ReasoningDelta {
                text: "thinking ".to_owned(),
            }]
        );
        assert_eq!(
            redactor.redact(RunEvent::TextDelta {
                text: "answer sk-sens".to_owned(),
            }),
            vec![RunEvent::TextDelta {
                text: "answer ".to_owned(),
            }]
        );
        assert_eq!(
            redactor.redact(RunEvent::RunCancelled),
            vec![RunEvent::RunCancelled]
        );
    }

    #[test]
    fn provider_controlled_atomic_fields_cannot_reconstruct_a_split_credential() {
        let mut redactor =
            ProviderEventRedactor::new(Some(SessionCredential::new(TEST_CREDENTIAL)));
        let failure = redactor.redact(RunEvent::RunFailed {
            code: "sk-sens".to_owned(),
            message: "itive-token".to_owned(),
            retryable: false,
            status: Some(401),
        });
        assert_eq!(
            failure,
            vec![RunEvent::RunFailed {
                code: String::new(),
                message: "itive-token".to_owned(),
                retryable: false,
                status: Some(401),
            }]
        );

        let metadata = redactor.redact(RunEvent::ProviderMetadata {
            request_id: Some("sk-sens".to_owned()),
            model: Some("itive-token".to_owned()),
            created_at: None,
        });
        let serialized = serde_json::to_string(&(failure, metadata)).unwrap();
        assert!(!serialized.contains(TEST_CREDENTIAL));
    }

    #[test]
    fn redaction_marker_cannot_supply_a_credential_prefix_at_a_field_boundary() {
        let credential = "]evil";
        let mut redactor = ProviderEventRedactor::new(Some(SessionCredential::new(credential)));
        assert!(
            redactor
                .redact(RunEvent::TextDelta {
                    text: credential.to_owned(),
                })
                .is_empty(),
            "an unsafe fixed marker must be omitted"
        );
        let terminal = redactor.redact(RunEvent::RunFailed {
            code: "provider_error".to_owned(),
            message: "evil".to_owned(),
            retryable: false,
            status: Some(401),
        });
        assert!(
            !serde_json::to_string(&terminal)
                .unwrap()
                .contains(credential)
        );
        assert!(
            !super::redact_exact_value("]]evilevil", Some(credential)).contains(credential),
            "removing one occurrence must not create another credential across the join"
        );
    }

    #[tokio::test]
    async fn cancellation_delivers_an_already_redacted_event_before_the_terminal() {
        let (sender, mut receiver) = mpsc::channel(3);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut redactor =
            ProviderEventRedactor::new(Some(SessionCredential::new(TEST_CREDENTIAL)));

        let delivered = super::send_prepared_redacted_events(
            &sender,
            &mut redactor,
            vec![RunEvent::TextDelta {
                text: "decoded output".to_owned(),
            }],
            false,
            &cancellation,
        )
        .await
        .unwrap();
        assert!(!delivered);
        assert_eq!(
            receiver.recv().await,
            Some(RunEvent::TextDelta {
                text: "decoded output".to_owned(),
            })
        );
        assert_eq!(receiver.recv().await, Some(RunEvent::RunCancelled));
    }

    #[tokio::test]
    async fn streaming_response_cannot_reflect_a_credential_into_run_events() {
        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"before sk-sens\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"itive-token after\"}}]}\n\n",
            "data: [DONE]\n\n",
        )
        .to_owned();
        let (base_url, request_receiver, server) = spawn_single_response(
            "200 OK",
            vec![("Content-Type".into(), "text/event-stream".into())],
            body,
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let (sender, mut receiver) = mpsc::channel(16);

        gateway
            .stream(
                openai_invocation(base_url, Some(TEST_CREDENTIAL)),
                CancellationToken::new(),
                sender,
            )
            .await
            .unwrap();
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            events.push(event);
        }
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(!serialized.contains(TEST_CREDENTIAL));
        let text = events
            .iter()
            .filter_map(|event| match event {
                RunEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text, "before [REDACTED] after");

        let request = request_receiver.recv().unwrap();
        assert!(request.to_ascii_lowercase().contains("x-api-key"));
        assert!(request.contains(TEST_CREDENTIAL));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn streaming_requests_do_not_follow_provider_redirects() {
        let redirect_target = spawn_redirect_target();
        let (base_url, request_receiver, server) = spawn_single_response(
            "307 Temporary Redirect",
            vec![("Location".into(), redirect_target.url.clone())],
            String::new(),
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let (sender, mut receiver) = mpsc::channel(4);

        gateway
            .stream(
                openai_invocation(base_url, Some(TEST_CREDENTIAL)),
                CancellationToken::new(),
                sender,
            )
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(RunEvent::RunFailed {
                status: Some(307),
                ..
            })
        ));
        assert!(receiver.recv().await.is_none());
        redirect_target.stop_sender.send(()).unwrap();
        redirect_target.handle.join().unwrap();
        assert!(matches!(
            redirect_target.request_receiver.try_recv(),
            Err(std_mpsc::TryRecvError::Empty | std_mpsc::TryRecvError::Disconnected)
        ));

        let request = request_receiver.recv().unwrap();
        assert!(request.contains(TEST_CREDENTIAL));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn connection_tests_do_not_follow_provider_redirects() {
        let redirect_target = spawn_redirect_target();
        let (base_url, request_receiver, server) = spawn_single_response(
            "308 Permanent Redirect",
            vec![("Location".into(), redirect_target.url.clone())],
            String::new(),
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();

        let status = gateway
            .test(
                openai_invocation(base_url, None).target,
                Some(SessionCredential::new(TEST_CREDENTIAL)),
            )
            .await
            .unwrap();
        assert!(!status.ok);
        assert_eq!(status.http_status, 308);
        redirect_target.stop_sender.send(()).unwrap();
        redirect_target.handle.join().unwrap();
        assert!(matches!(
            redirect_target.request_receiver.try_recv(),
            Err(std_mpsc::TryRecvError::Empty | std_mpsc::TryRecvError::Disconnected)
        ));

        let request = request_receiver.recv().unwrap();
        assert!(request.contains(TEST_CREDENTIAL));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn a_pre_cancelled_invocation_never_opens_a_connection() {
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let (sender, mut receiver) = mpsc::channel(2);
        let invocation = ProviderInvocation {
            target: ProviderTarget {
                dialect: ProviderDialect::OpenAiChatCompletions,
                base_url: "http://127.0.0.1:1/v1".to_owned(),
                credential_placement: CredentialPlacement::BearerHeader,
                additional_headers: std::collections::BTreeMap::new(),
            },
            credential: None,
            request: CanonicalRequest {
                run_id: "run-cancelled".to_owned(),
                model: "does-not-matter".to_owned(),
                messages: Vec::new(),
                temperature: None,
                top_p: None,
                max_output_tokens: None,
                stop: Vec::new(),
            },
        };

        gateway
            .stream(invocation, cancellation, sender)
            .await
            .unwrap();

        assert_eq!(receiver.recv().await, Some(RunEvent::RunCancelled));
        assert_eq!(receiver.recv().await, None);
    }
}
