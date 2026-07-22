use std::{collections::BTreeSet, time::Duration};

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderName};
use reqwest::redirect::Policy;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    infrastructure::provider::{
        AnthropicSseDecoder, GoogleSseDecoder, OllamaNdjsonDecoder, OpenAiSseDecoder,
        ProviderStreamDecoder, decode_http_error_with_redaction, provider_models_url,
        provider_request_url,
    },
    ports::provider::{
        CanonicalMessage, CanonicalRequest, CredentialPlacement, DiscoveredModel, MessageRole,
        ProviderConnectionFuture, ProviderConnectionStatus, ProviderConnectionTester,
        ProviderDialect, ProviderError, ProviderFuture, ProviderGateway, ProviderInvocation,
        ProviderModelCatalog, ProviderModelCatalogKind, ProviderModelQuery, ProviderModelsFuture,
        ProviderTarget, RunEvent, SessionCredential,
    },
};

const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(20);
const MODEL_CATALOG_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_MODEL_CATALOG_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISCOVERED_MODELS: usize = 5_000;
const MAX_MODEL_ID_CHARS: usize = 512;
const MAX_MODEL_DISPLAY_NAME_CHARS: usize = 1_024;
const CREDENTIAL_REDACTION_MARKER: &str = "[REDACTED]";

pub struct ReqwestProviderGateway {
    client: reqwest::Client,
    model_catalog_timeout: Duration,
}

impl ReqwestProviderGateway {
    fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            model_catalog_timeout: MODEL_CATALOG_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_model_catalog_timeout(mut self, timeout: Duration) -> Self {
        self.model_catalog_timeout = timeout;
        self
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

        let url = provider_request_url(
            &invocation.target.base_url,
            invocation.target.dialect,
            &invocation.request.model,
        )?;
        let body = request_body(&invocation.request, invocation.target.dialect)?;
        let accept = match invocation.target.dialect {
            ProviderDialect::OpenAiChatCompletions
            | ProviderDialect::AnthropicMessages
            | ProviderDialect::GoogleGenerativeAi => "text/event-stream",
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
                decode_redacted_http_error(
                    invocation.target.dialect,
                    status,
                    content_type.as_deref(),
                    &body,
                    invocation.credential.as_ref(),
                ),
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
            ProviderDialect::AnthropicMessages => Box::new(AnthropicSseDecoder::new()),
            ProviderDialect::GoogleGenerativeAi => Box::new(GoogleSseDecoder::new()),
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
            let catalog = match target.dialect {
                ProviderDialect::OpenAiChatCompletions => ProviderModelCatalogKind::OpenAi,
                ProviderDialect::OllamaChat => ProviderModelCatalogKind::Ollama,
                ProviderDialect::AnthropicMessages => ProviderModelCatalogKind::Anthropic,
                ProviderDialect::GoogleGenerativeAi => ProviderModelCatalogKind::Google,
            };
            let endpoint = provider_models_url(&target.base_url, catalog)?;
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

impl ProviderModelCatalog for ReqwestProviderGateway {
    fn list_models<'a>(
        &'a self,
        query: ProviderModelQuery,
        credential: Option<SessionCredential>,
    ) -> ProviderModelsFuture<'a> {
        Box::pin(async move {
            let endpoint = provider_models_url(&query.target.base_url, query.catalog)
                .map_err(|error| redact_provider_error(error, credential.as_ref()))?;
            let mut request = self
                .client
                .get(endpoint)
                .header(ACCEPT, "application/json")
                .timeout(self.model_catalog_timeout);
            request = apply_additional_headers(request, &query.target.additional_headers)?;
            request = apply_credential(
                request,
                &query.target.credential_placement,
                credential.as_ref(),
            )?;

            let response = request.send().await.map_err(|error| {
                redact_provider_error(
                    ProviderError::Transport(error.to_string()),
                    credential.as_ref(),
                )
            })?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let content_type = response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let no_cancellation = CancellationToken::new();
                let body = collect_error_body(response, &no_cancellation)
                    .await
                    .map_err(|error| redact_provider_error(error, credential.as_ref()))?
                    .unwrap_or_default();
                return Err(redact_provider_error(
                    decode_redacted_http_error(
                        query.target.dialect,
                        status,
                        content_type.as_deref(),
                        &body,
                        credential.as_ref(),
                    ),
                    credential.as_ref(),
                ));
            }

            let body = collect_bounded_response_body(response, MAX_MODEL_CATALOG_BODY_BYTES)
                .await
                .map_err(|error| redact_provider_error(error, credential.as_ref()))?;
            parse_model_catalog(query.catalog, &body, credential.as_ref())
                .map_err(|error| redact_provider_error(error, credential.as_ref()))
        })
    }
}

async fn collect_bounded_response_body(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, ProviderError> {
    if response
        .content_length()
        .is_some_and(|content_length| content_length > limit as u64)
    {
        return Err(ProviderError::InvalidResponse(format!(
            "Provider response exceeds the {limit}-byte limit"
        )));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ProviderError::Transport(error.to_string()))?;
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(ProviderError::InvalidResponse(format!(
                "Provider response exceeds the {limit}-byte limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_model_catalog(
    catalog: ProviderModelCatalogKind,
    body: &[u8],
    credential: Option<&SessionCredential>,
) -> Result<Vec<DiscoveredModel>, ProviderError> {
    let document: Value = serde_json::from_slice(body).map_err(|error| {
        ProviderError::InvalidResponse(format!("model catalog is not valid JSON: {error}"))
    })?;
    let root = document.as_object().ok_or_else(|| {
        ProviderError::InvalidResponse("model catalog root must be an object".to_owned())
    })?;
    let collection_name = match catalog {
        ProviderModelCatalogKind::OpenAi | ProviderModelCatalogKind::Anthropic => "data",
        ProviderModelCatalogKind::Ollama | ProviderModelCatalogKind::Google => "models",
    };
    let entries = root
        .get(collection_name)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProviderError::InvalidResponse(format!(
                "model catalog must contain a `{collection_name}` array"
            ))
        })?;
    if entries.len() > MAX_DISCOVERED_MODELS {
        return Err(ProviderError::InvalidResponse(format!(
            "model catalog contains more than {MAX_DISCOVERED_MODELS} entries"
        )));
    }

    let secret = credential
        .filter(|credential| !credential.is_empty())
        .map(SessionCredential::expose_secret);
    let mut models = entries
        .iter()
        .filter_map(|entry| parse_discovered_model(catalog, entry, secret))
        .collect::<Vec<_>>();
    models.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut seen = BTreeSet::new();
    models.retain(|model| seen.insert(model.id.clone()));
    Ok(models)
}

fn parse_discovered_model(
    catalog: ProviderModelCatalogKind,
    entry: &Value,
    credential: Option<&str>,
) -> Option<DiscoveredModel> {
    let entry = entry.as_object()?;
    let id = match catalog {
        ProviderModelCatalogKind::OpenAi | ProviderModelCatalogKind::Anthropic => {
            string_field(entry, &["id"])
        }
        ProviderModelCatalogKind::Ollama => string_field(entry, &["model", "name"]),
        ProviderModelCatalogKind::Google => string_field(entry, &["name"]),
    }?;
    // Redact before whitespace normalization. Session credentials are opaque
    // strings and may themselves contain leading or trailing whitespace.
    let id = redact_bounded_value(id, credential).trim().to_owned();
    if id.is_empty() || id.chars().count() > MAX_MODEL_ID_CHARS || contains_unsafe_text_control(&id)
    {
        return None;
    }

    let display_name = match catalog {
        ProviderModelCatalogKind::OpenAi
        | ProviderModelCatalogKind::Ollama
        | ProviderModelCatalogKind::Anthropic => {
            string_field(entry, &["display_name", "displayName"]).unwrap_or(&id)
        }
        ProviderModelCatalogKind::Google => string_field(entry, &["displayName"]).unwrap_or(&id),
    };
    let display_name = redact_bounded_value(display_name, credential)
        .trim()
        .to_owned();
    let display_name = if display_name.is_empty()
        || display_name.chars().count() > MAX_MODEL_DISPLAY_NAME_CHARS
        || contains_unsafe_text_control(&display_name)
    {
        id.clone()
    } else {
        display_name
    };

    let context_window = match catalog {
        ProviderModelCatalogKind::OpenAi
        | ProviderModelCatalogKind::Ollama
        | ProviderModelCatalogKind::Anthropic => u64_field(
            entry,
            &["context_window", "contextWindow", "context_length"],
        ),
        ProviderModelCatalogKind::Google => u64_field(entry, &["contextWindow", "inputTokenLimit"]),
    };
    let supports_tools = bool_field(entry, &["supports_tools", "supportsTools"])
        .or_else(|| nested_capability_bool(entry, &["tools", "tool_calling", "toolCalling"]))
        .or_else(|| tools_capability_list(entry, "capabilities"))
        .or_else(|| tools_capability_list(entry, "supported_parameters"));

    Some(DiscoveredModel {
        id,
        display_name,
        context_window,
        supports_tools,
    })
}

fn string_field<'a>(entry: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| entry.get(*name).and_then(Value::as_str))
}

fn u64_field(entry: &Map<String, Value>, names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|name| entry.get(*name).and_then(Value::as_u64))
}

fn bool_field(entry: &Map<String, Value>, names: &[&str]) -> Option<bool> {
    names
        .iter()
        .find_map(|name| entry.get(*name).and_then(Value::as_bool))
}

fn nested_capability_bool(entry: &Map<String, Value>, names: &[&str]) -> Option<bool> {
    let capabilities = entry.get("capabilities")?.as_object()?;
    bool_field(capabilities, names)
}

fn tools_capability_list(entry: &Map<String, Value>, field: &str) -> Option<bool> {
    let capabilities = entry.get(field)?.as_array()?;
    Some(capabilities.iter().any(|capability| {
        capability.as_str().is_some_and(|capability| {
            matches!(capability, "tools" | "tool_calling" | "toolCalling")
        })
    }))
}

fn contains_unsafe_text_control(value: &str) -> bool {
    value.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
    })
}

fn decode_redacted_http_error(
    dialect: ProviderDialect,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
    credential: Option<&SessionCredential>,
) -> ProviderError {
    let secret = credential
        .filter(|credential| !credential.is_empty())
        .map(SessionCredential::expose_secret);
    decode_http_error_with_redaction(dialect, status, content_type, body, |value| {
        redact_bounded_value(value, secret)
    })
}

fn redact_provider_error(
    error: ProviderError,
    credential: Option<&SessionCredential>,
) -> ProviderError {
    let secret = credential
        .filter(|credential| !credential.is_empty())
        .map(SessionCredential::expose_secret);
    let redact = |value: String| redact_bounded_value(&value, secret);
    match error {
        ProviderError::InvalidEndpoint(message) => ProviderError::InvalidEndpoint(redact(message)),
        ProviderError::UnsupportedScheme(scheme) => {
            ProviderError::UnsupportedScheme(redact(scheme))
        }
        ProviderError::CredentialsInUrl => ProviderError::CredentialsInUrl,
        ProviderError::InsecureRemoteEndpoint(host) => {
            ProviderError::InsecureRemoteEndpoint(redact(host))
        }
        ProviderError::InvalidResponse(message) => ProviderError::InvalidResponse(redact(message)),
        ProviderError::UnexpectedEof => ProviderError::UnexpectedEof,
        ProviderError::Http {
            status,
            provider_code,
            message,
            retryable,
        } => ProviderError::Http {
            status,
            provider_code: provider_code.map(&redact),
            message: redact(message),
            retryable,
        },
        ProviderError::Transport(message) => ProviderError::Transport(redact(message)),
        ProviderError::EventChannelClosed => ProviderError::EventChannelClosed,
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

fn request_body(
    request: &CanonicalRequest,
    dialect: ProviderDialect,
) -> Result<Value, ProviderError> {
    match dialect {
        ProviderDialect::OpenAiChatCompletions => {
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
            Ok(Value::Object(body))
        }
        ProviderDialect::OllamaChat => {
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
            Ok(Value::Object(body))
        }
        ProviderDialect::AnthropicMessages => anthropic_request_body(request),
        ProviderDialect::GoogleGenerativeAi => Ok(google_request_body(request)),
    }
}

fn anthropic_request_body(request: &CanonicalRequest) -> Result<Value, ProviderError> {
    let max_tokens = request.max_output_tokens.ok_or_else(|| {
        ProviderError::InvalidResponse(
            "Anthropic requires max_output_tokens to be resolved before the request is sent".into(),
        )
    })?;
    let system = request
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::System)
        .map(|message| json!({ "type": "text", "text": message.content }))
        .collect::<Vec<_>>();
    let messages = request
        .messages
        .iter()
        .filter_map(|message| match message.role {
            MessageRole::System => None,
            MessageRole::User => Some(json!({ "role": "user", "content": message.content })),
            MessageRole::Assistant => {
                Some(json!({ "role": "assistant", "content": message.content }))
            }
        })
        .collect::<Vec<_>>();
    let mut body = Map::from_iter([
        ("model".to_owned(), json!(request.model)),
        ("messages".to_owned(), Value::Array(messages)),
        ("stream".to_owned(), Value::Bool(true)),
        ("max_tokens".to_owned(), json!(max_tokens)),
    ]);
    if !system.is_empty() {
        body.insert("system".to_owned(), Value::Array(system));
    }
    insert_optional(&mut body, "temperature", request.temperature);
    insert_optional(&mut body, "top_p", request.top_p);
    if !request.stop.is_empty() {
        body.insert("stop_sequences".to_owned(), json!(request.stop));
    }
    Ok(Value::Object(body))
}

fn google_request_body(request: &CanonicalRequest) -> Value {
    let system_parts = request
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::System)
        .map(|message| json!({ "text": message.content }))
        .collect::<Vec<_>>();

    let mut grouped_contents: Vec<(&str, Vec<Value>)> = Vec::new();
    for message in request
        .messages
        .iter()
        .filter(|message| message.role != MessageRole::System)
    {
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "model",
            MessageRole::System => unreachable!("system messages were filtered above"),
        };
        let part = json!({ "text": message.content });
        match grouped_contents.last_mut() {
            Some((previous_role, parts)) if *previous_role == role => parts.push(part),
            _ => grouped_contents.push((role, vec![part])),
        }
    }
    let contents = grouped_contents
        .into_iter()
        .map(|(role, parts)| json!({ "role": role, "parts": parts }))
        .collect::<Vec<_>>();
    let mut body = Map::from_iter([("contents".to_owned(), Value::Array(contents))]);
    if !system_parts.is_empty() {
        body.insert(
            "systemInstruction".to_owned(),
            json!({ "parts": system_parts }),
        );
    }
    let mut generation_config = Map::new();
    insert_optional(&mut generation_config, "temperature", request.temperature);
    insert_optional(&mut generation_config, "topP", request.top_p);
    insert_optional(
        &mut generation_config,
        "maxOutputTokens",
        request.max_output_tokens,
    );
    if !request.stop.is_empty() {
        generation_config.insert("stopSequences".to_owned(), json!(request.stop));
    }
    if !generation_config.is_empty() {
        body.insert(
            "generationConfig".to_owned(),
            Value::Object(generation_config),
        );
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
        CanonicalMessage, CanonicalRequest, CredentialPlacement, MessageRole,
        ProviderConnectionTester, ProviderDialect, ProviderError, ProviderGateway,
        ProviderInvocation, ProviderModelCatalog, ProviderModelCatalogKind, ProviderModelQuery,
        ProviderTarget, RunEvent, SessionCredential,
    };

    use super::{ProviderEventRedactor, ReqwestProviderGateway};

    const TEST_CREDENTIAL: &str = "sk-sensitive-token";

    fn canonical_request(max_output_tokens: Option<u32>) -> CanonicalRequest {
        CanonicalRequest {
            run_id: "run-provider-contract".to_owned(),
            model: "provider-model".to_owned(),
            messages: vec![
                CanonicalMessage {
                    role: MessageRole::System,
                    content: "Primary policy".to_owned(),
                },
                CanonicalMessage {
                    role: MessageRole::User,
                    content: "Question".to_owned(),
                },
                CanonicalMessage {
                    role: MessageRole::System,
                    content: "Pinned policy".to_owned(),
                },
                CanonicalMessage {
                    role: MessageRole::Assistant,
                    content: "Earlier answer".to_owned(),
                },
            ],
            temperature: Some(0.25),
            top_p: Some(0.75),
            max_output_tokens,
            stop: vec!["END".to_owned()],
        }
    }

    #[test]
    fn anthropic_request_uses_top_level_system_and_requires_explicit_effective_max_tokens() {
        let body = super::request_body(
            &canonical_request(Some(4096)),
            ProviderDialect::AnthropicMessages,
        )
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "model": "provider-model",
                "stream": true,
                "system": [
                    { "type": "text", "text": "Primary policy" },
                    { "type": "text", "text": "Pinned policy" }
                ],
                "messages": [
                    { "role": "user", "content": "Question" },
                    { "role": "assistant", "content": "Earlier answer" }
                ],
                "temperature": 0.25,
                "top_p": 0.75,
                "max_tokens": 4096,
                "stop_sequences": ["END"]
            })
        );

        assert!(matches!(
            super::request_body(
                &canonical_request(None),
                ProviderDialect::AnthropicMessages,
            ),
            Err(ProviderError::InvalidResponse(message))
                if message.contains("max_output_tokens")
        ));
    }

    #[test]
    fn google_request_uses_camel_case_system_instruction_contents_and_generation_config() {
        let mut request = canonical_request(Some(8192));
        request.messages.insert(
            2,
            CanonicalMessage {
                role: MessageRole::User,
                content: "Pinned question".to_owned(),
            },
        );
        let body = super::request_body(&request, ProviderDialect::GoogleGenerativeAi).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "systemInstruction": {
                    "parts": [
                        { "text": "Primary policy" },
                        { "text": "Pinned policy" }
                    ]
                },
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            { "text": "Question" },
                            { "text": "Pinned question" }
                        ]
                    },
                    { "role": "model", "parts": [{ "text": "Earlier answer" }] }
                ],
                "generationConfig": {
                    "temperature": 0.25,
                    "topP": 0.75,
                    "maxOutputTokens": 8192,
                    "stopSequences": ["END"]
                }
            })
        );
        assert!(body.get("system_instruction").is_none());
        assert!(body.get("model").is_none());
        assert!(body.get("stream").is_none());
    }

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

    fn native_invocation(
        base_url: String,
        dialect: ProviderDialect,
        model: &str,
        credential_header: &str,
        additional_headers: std::collections::BTreeMap<String, String>,
        max_output_tokens: u32,
    ) -> ProviderInvocation {
        let mut request = canonical_request(Some(max_output_tokens));
        request.model = model.into();
        ProviderInvocation {
            target: ProviderTarget {
                dialect,
                base_url,
                credential_placement: CredentialPlacement::Header(credential_header.into()),
                additional_headers,
            },
            credential: Some(SessionCredential::new(TEST_CREDENTIAL)),
            request,
        }
    }

    #[tokio::test]
    async fn native_provider_requests_send_exact_paths_headers_and_bodies() {
        let error_body = r#"{"error":{"message":"contract captured"}}"#.to_owned();
        let (anthropic_base, anthropic_request, anthropic_server) = spawn_single_response(
            "400 Bad Request",
            vec![("Content-Type".into(), "application/json".into())],
            error_body.clone(),
        );
        let anthropic = native_invocation(
            format!("{anthropic_base}/tenant"),
            ProviderDialect::AnthropicMessages,
            "claude-sonnet-5",
            "x-api-key",
            std::collections::BTreeMap::from([("anthropic-version".into(), "2023-06-01".into())]),
            4_096,
        );
        let expected_anthropic_body =
            super::request_body(&anthropic.request, ProviderDialect::AnthropicMessages).unwrap();
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        gateway
            .stream(anthropic, CancellationToken::new(), sender)
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(RunEvent::RunFailed {
                status: Some(400),
                ..
            })
        ));
        let raw = anthropic_request.recv().unwrap();
        let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /tenant/v1/messages HTTP/1.1"));
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("accept: text/event-stream"));
        assert!(headers.contains("x-api-key: sk-sensitive-token"));
        assert!(headers.contains("anthropic-version: 2023-06-01"));
        assert!(!headers.contains("authorization:"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            expected_anthropic_body
        );
        anthropic_server.join().unwrap();

        let (google_base, google_request, google_server) = spawn_single_response(
            "400 Bad Request",
            vec![("Content-Type".into(), "application/json".into())],
            error_body,
        );
        let google = native_invocation(
            format!("{google_base}/tenant/v1beta"),
            ProviderDialect::GoogleGenerativeAi,
            "models/gemini 2.5-pro",
            "x-goog-api-key",
            std::collections::BTreeMap::new(),
            8_192,
        );
        let expected_google_body =
            super::request_body(&google.request, ProviderDialect::GoogleGenerativeAi).unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        gateway
            .stream(google, CancellationToken::new(), sender)
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(RunEvent::RunFailed {
                status: Some(400),
                ..
            })
        ));
        let raw = google_request.recv().unwrap();
        let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with(
            "POST /tenant/v1beta/models/gemini%202.5-pro:streamGenerateContent?alt=sse HTTP/1.1"
        ));
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("accept: text/event-stream"));
        assert!(headers.contains("x-goog-api-key: sk-sensitive-token"));
        assert!(!headers.contains("authorization:"));
        assert!(!headers.contains("anthropic-version:"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            expected_google_body
        );
        google_server.join().unwrap();
    }

    #[tokio::test]
    async fn native_http_429_machine_codes_reach_run_failed_events() {
        let cases = [
            (
                ProviderDialect::OpenAiChatCompletions,
                "fixture-model",
                "x-api-key",
                r#"{"error":{"code":"insufficient_quota","message":"quota exhausted"}}"#,
                "quota_exhausted",
            ),
            (
                ProviderDialect::OpenAiChatCompletions,
                "fixture-model",
                "x-api-key",
                r#"{"error":{"code":"requests","message":"slow down"}}"#,
                "rate_limited",
            ),
            (
                ProviderDialect::AnthropicMessages,
                "claude-fixture",
                "x-api-key",
                r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#,
                "rate_limited",
            ),
            (
                ProviderDialect::GoogleGenerativeAi,
                "gemini-fixture",
                "x-goog-api-key",
                r#"{"error":{"code":429,"message":"quota exhausted","status":"RESOURCE_EXHAUSTED"}}"#,
                "rate_limited",
            ),
            (
                ProviderDialect::OllamaChat,
                "ollama-fixture",
                "x-api-key",
                r#"{"error":"busy","code":"queue_full"}"#,
                "queue_full",
            ),
        ];

        for (dialect, model, credential_header, body, expected_code) in cases {
            let (base_url, _, server) = spawn_single_response(
                "429 Too Many Requests",
                vec![("Content-Type".into(), "application/json".into())],
                body.to_owned(),
            );
            let invocation = native_invocation(
                base_url,
                dialect,
                model,
                credential_header,
                std::collections::BTreeMap::new(),
                1_024,
            );
            let gateway = ReqwestProviderGateway::with_defaults().unwrap();
            let (sender, mut receiver) = mpsc::channel(2);

            gateway
                .stream(invocation, CancellationToken::new(), sender)
                .await
                .unwrap();

            assert!(matches!(
                receiver.recv().await,
                Some(RunEvent::RunFailed {
                    code,
                    retryable: true,
                    status: Some(429),
                    ..
                }) if code == expected_code
            ));
            assert!(receiver.recv().await.is_none());
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn retryable_gateway_errors_do_not_fabricate_quota_or_rate_codes() {
        let cases = [
            (
                "500 Internal Server Error",
                ProviderDialect::OpenAiChatCompletions,
                r#"{"error":{"code":"insufficient_quota","message":"failed"}}"#,
                "insufficient_quota",
                Some(500),
            ),
            (
                "529 Site Overloaded",
                ProviderDialect::AnthropicMessages,
                r#"{"error":{"type":"rate_limit_error","message":"overloaded"}}"#,
                "rate_limit_error",
                Some(529),
            ),
            (
                "429 Too Many Requests",
                ProviderDialect::AnthropicMessages,
                r#"{"error":{"type":"other_limit","message":"slow down"}}"#,
                "other_limit",
                Some(429),
            ),
            (
                "429 Too Many Requests",
                ProviderDialect::GoogleGenerativeAi,
                r#"{"error":{"status":"resource_exhausted","message":"slow down"}}"#,
                "resource_exhausted",
                Some(429),
            ),
            (
                "429 Too Many Requests",
                ProviderDialect::AnthropicMessages,
                r#"{"error":{"message":"rate_limit_error"}}"#,
                "provider_http_error",
                Some(429),
            ),
        ];

        for (http_status, dialect, body, expected_code, expected_status) in cases {
            let (base_url, _, server) = spawn_single_response(
                http_status,
                vec![("Content-Type".into(), "application/json".into())],
                body.to_owned(),
            );
            let invocation = native_invocation(
                base_url,
                dialect,
                "fixture-model",
                "x-api-key",
                std::collections::BTreeMap::new(),
                1_024,
            );
            let gateway = ReqwestProviderGateway::with_defaults().unwrap();
            let (sender, mut receiver) = mpsc::channel(2);

            gateway
                .stream(invocation, CancellationToken::new(), sender)
                .await
                .unwrap();

            assert!(matches!(
                receiver.recv().await,
                Some(RunEvent::RunFailed {
                    code,
                    retryable: true,
                    status,
                    ..
                }) if code == expected_code && status == expected_status
            ));
            assert!(receiver.recv().await.is_none());
            server.join().unwrap();
        }
    }

    fn model_query(
        base_url: String,
        catalog: ProviderModelCatalogKind,
        credential_placement: CredentialPlacement,
    ) -> ProviderModelQuery {
        ProviderModelQuery {
            target: ProviderTarget {
                dialect: match catalog {
                    ProviderModelCatalogKind::Ollama => ProviderDialect::OllamaChat,
                    ProviderModelCatalogKind::Anthropic => ProviderDialect::AnthropicMessages,
                    ProviderModelCatalogKind::Google => ProviderDialect::GoogleGenerativeAi,
                    ProviderModelCatalogKind::OpenAi => ProviderDialect::OpenAiChatCompletions,
                },
                base_url,
                credential_placement,
                additional_headers: std::collections::BTreeMap::new(),
            },
            catalog,
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
            let _ = request_sender.send(request);

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

    fn spawn_raw_response(
        chunks: Vec<(Duration, Vec<u8>)>,
    ) -> (String, std_mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = std_mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let _ = request_sender.send(request);
            for (delay, chunk) in chunks {
                thread::sleep(delay);
                if stream.write_all(&chunk).is_err() {
                    break;
                }
            }
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
    async fn streaming_http_errors_redact_before_html_and_whitespace_normalization() {
        let credential = "key  with  spaces";
        let collapsed_credential = "key with spaces";
        let body = format!("<p>provider reflected {credential}</p>");
        let (base_url, _, server) = spawn_single_response(
            "401 Unauthorized",
            vec![("Content-Type".into(), "text/html".into())],
            body,
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let (sender, mut receiver) = mpsc::channel(4);

        gateway
            .stream(
                openai_invocation(base_url, Some(credential)),
                CancellationToken::new(),
                sender,
            )
            .await
            .unwrap();

        let failure = receiver.recv().await.unwrap();
        let rendered = serde_json::to_string(&failure).unwrap();
        assert!(matches!(
            failure,
            RunEvent::RunFailed {
                status: Some(401),
                ..
            }
        ));
        assert!(!rendered.contains(credential));
        assert!(!rendered.contains(collapsed_credential));
        assert!(rendered.contains("[REDACTED]"));
        assert!(receiver.recv().await.is_none());
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

    #[tokio::test]
    async fn openai_model_catalog_uses_authoritative_headers_and_normalizes_models() {
        let reflected_id = format!("reflected-{TEST_CREDENTIAL}-model");
        let overlong_id = "x".repeat(super::MAX_MODEL_ID_CHARS + 1);
        let overlong_display_name = "y".repeat(super::MAX_MODEL_DISPLAY_NAME_CHARS + 1);
        let body = serde_json::json!({
            "data": [
                {
                    "id": "zeta-model",
                    "display_name": "Zeta",
                    "context_window": 8192,
                    "supports_tools": true
                },
                {
                    "id": "alpha-model",
                    "displayName": "Alpha",
                    "capabilities": { "tools": false }
                },
                { "id": "alpha-model", "display_name": "Later Alpha" },
                {
                    "id": "beta-model",
                    "display_name": "Beta",
                    "supported_parameters": ["temperature", "tools"]
                },
                { "id": "gamma-model", "display_name": overlong_display_name },
                { "id": "" },
                { "id": overlong_id },
                { "id": reflected_id }
            ]
        })
        .to_string();
        let (base_url, request_receiver, server) = spawn_single_response(
            "200 OK",
            vec![("Content-Type".into(), "application/json".into())],
            body,
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let mut query = model_query(
            format!("{base_url}/gateway/v1/chat/completions"),
            ProviderModelCatalogKind::OpenAi,
            CredentialPlacement::Header("x-api-key".into()),
        );
        query
            .target
            .additional_headers
            .insert("x-static-revision".into(), "catalog-v1".into());

        let models = gateway
            .list_models(query, Some(SessionCredential::new(TEST_CREDENTIAL)))
            .await
            .unwrap();

        assert_eq!(models[0].id, "alpha-model");
        assert_eq!(models[0].display_name, "Alpha");
        assert_eq!(models[0].supports_tools, Some(false));
        assert_eq!(models[1].id, "beta-model");
        assert_eq!(models[1].supports_tools, Some(true));
        assert_eq!(models[2].id, "gamma-model");
        assert_eq!(models[2].display_name, "gamma-model");
        assert_eq!(
            models[3].id,
            format!("reflected-{}-model", super::CREDENTIAL_REDACTION_MARKER)
        );
        assert_eq!(models[4].id, "zeta-model");
        assert_eq!(models[4].context_window, Some(8192));
        assert_eq!(models[4].supports_tools, Some(true));
        assert_eq!(models.len(), 5, "duplicate and empty IDs must be removed");
        assert!(!format!("{models:?}").contains(TEST_CREDENTIAL));

        let request = request_receiver.recv().unwrap();
        let request_lower = request.to_ascii_lowercase();
        assert!(request.starts_with("GET /gateway/v1/models HTTP/1.1"));
        assert!(request_lower.contains("accept: application/json"));
        assert!(request_lower.contains("x-api-key: sk-sensitive-token"));
        assert!(request_lower.contains("x-static-revision: catalog-v1"));
        server.join().unwrap();
    }

    #[test]
    fn model_fields_redact_whitespace_credentials_before_normalizing_text() {
        let credential = SessionCredential::new(" secret ");
        let body = serde_json::json!({
            "data": [
                { "id": " secret ", "display_name": "reflected  secret " },
                { "id": "safe-model", "display_name": "Safe \u{202e}spoof" },
                { "id": "bad\nid", "display_name": "ignored" }
            ]
        })
        .to_string();

        let models = super::parse_model_catalog(
            ProviderModelCatalogKind::OpenAi,
            body.as_bytes(),
            Some(&credential),
        )
        .unwrap();

        assert_eq!(models.len(), 2);
        assert!(models.iter().any(|model| {
            model.id == super::CREDENTIAL_REDACTION_MARKER
                && model.display_name == format!("reflected {}", super::CREDENTIAL_REDACTION_MARKER)
        }));
        assert!(
            models
                .iter()
                .any(|model| { model.id == "safe-model" && model.display_name == "safe-model" })
        );
        let rendered = format!("{models:?}");
        assert!(!rendered.contains(credential.expose_secret()));
        assert!(!rendered.contains('\u{202e}'));
        assert!(!rendered.contains("bad\nid"));
    }

    #[tokio::test]
    async fn ollama_and_google_model_catalogs_use_their_native_paths_and_shapes() {
        let ollama_body = serde_json::json!({
            "models": [
                { "model": "qwen3:8b" },
                { "name": "gemma3:4b", "context_window": 32768 }
            ]
        })
        .to_string();
        let (ollama_base, ollama_request, ollama_server) = spawn_single_response(
            "200 OK",
            vec![("Content-Type".into(), "application/json".into())],
            ollama_body,
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let ollama_models = gateway
            .list_models(
                model_query(
                    format!("{ollama_base}/tenant/api/chat"),
                    ProviderModelCatalogKind::Ollama,
                    CredentialPlacement::None,
                ),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            ollama_models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gemma3:4b", "qwen3:8b"]
        );
        assert_eq!(ollama_models[0].context_window, Some(32768));
        assert!(
            ollama_request
                .recv()
                .unwrap()
                .starts_with("GET /tenant/api/tags HTTP/1.1")
        );
        ollama_server.join().unwrap();

        let google_body = serde_json::json!({
            "models": [{
                "name": "models/gemini-2.5-pro",
                "displayName": "Gemini 2.5 Pro",
                "inputTokenLimit": 1_048_576,
                "supportsTools": true
            }]
        })
        .to_string();
        let (google_base, google_request, google_server) = spawn_single_response(
            "200 OK",
            vec![("Content-Type".into(), "application/json".into())],
            google_body,
        );
        let google_models = gateway
            .list_models(
                model_query(
                    google_base,
                    ProviderModelCatalogKind::Google,
                    CredentialPlacement::Header("x-goog-api-key".into()),
                ),
                Some(SessionCredential::new(TEST_CREDENTIAL)),
            )
            .await
            .unwrap();
        assert_eq!(google_models[0].id, "models/gemini-2.5-pro");
        assert_eq!(google_models[0].display_name, "Gemini 2.5 Pro");
        assert_eq!(google_models[0].context_window, Some(1_048_576));
        assert_eq!(google_models[0].supports_tools, Some(true));
        let request = google_request.recv().unwrap();
        assert!(request.starts_with("GET /v1beta/models HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-goog-api-key: sk-sensitive-token")
        );
        google_server.join().unwrap();
    }

    #[tokio::test]
    async fn model_catalog_accepts_empty_lists_and_rejects_malformed_documents() {
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let (empty_base, _, empty_server) = spawn_single_response(
            "200 OK",
            vec![("Content-Type".into(), "application/json".into())],
            r#"{"data":[]}"#.to_owned(),
        );
        assert!(
            gateway
                .list_models(
                    model_query(
                        empty_base,
                        ProviderModelCatalogKind::OpenAi,
                        CredentialPlacement::None,
                    ),
                    None,
                )
                .await
                .unwrap()
                .is_empty()
        );
        empty_server.join().unwrap();

        for malformed in [r#"{"models":[]}"#, r#"{"data":{}}"#, "{not-json"] {
            let (base_url, _, server) = spawn_single_response(
                "200 OK",
                vec![("Content-Type".into(), "application/json".into())],
                malformed.to_owned(),
            );
            let error = gateway
                .list_models(
                    model_query(
                        base_url,
                        ProviderModelCatalogKind::OpenAi,
                        CredentialPlacement::None,
                    ),
                    None,
                )
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                crate::ports::provider::ProviderError::InvalidResponse(_)
            ));
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn model_catalog_preserves_http_retryability_without_echoing_credentials() {
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let (unauthorized_base, _, unauthorized_server) = spawn_single_response(
            "401 Unauthorized",
            vec![("Content-Type".into(), "application/json".into())],
            format!(
                "{{\"error\":{{\"code\":\"bad-{TEST_CREDENTIAL}\",\"message\":\"reflected {TEST_CREDENTIAL}\"}}}}"
            ),
        );
        let unauthorized = gateway
            .list_models(
                model_query(
                    unauthorized_base,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::BearerHeader,
                ),
                Some(SessionCredential::new(TEST_CREDENTIAL)),
            )
            .await
            .unwrap_err();
        assert_eq!(unauthorized.status(), Some(401));
        assert!(!unauthorized.retryable());
        assert!(!format!("{unauthorized:?}").contains(TEST_CREDENTIAL));
        unauthorized_server.join().unwrap();

        let (rate_limit_base, _, rate_limit_server) = spawn_single_response(
            "429 Too Many Requests",
            vec![("Content-Type".into(), "text/html".into())],
            format!("<p>slow down {TEST_CREDENTIAL}</p>"),
        );
        let rate_limited = gateway
            .list_models(
                model_query(
                    rate_limit_base,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::BearerHeader,
                ),
                Some(SessionCredential::new(TEST_CREDENTIAL)),
            )
            .await
            .unwrap_err();
        assert_eq!(rate_limited.status(), Some(429));
        assert!(rate_limited.retryable());
        assert!(!format!("{rate_limited:?}").contains(TEST_CREDENTIAL));
        rate_limit_server.join().unwrap();
    }

    #[tokio::test]
    async fn model_catalog_http_errors_redact_before_json_field_trimming() {
        let credential = " key ";
        let body = format!(
            "{{\"error\":{{\"code\":\"{credential}\",\"message\":\"before{credential}after\"}}}}"
        );
        let (base_url, _, server) = spawn_single_response(
            "401 Unauthorized",
            vec![("Content-Type".into(), "application/json".into())],
            body,
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();

        let error = gateway
            .list_models(
                model_query(
                    base_url,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::None,
                ),
                Some(SessionCredential::new(credential)),
            )
            .await
            .unwrap_err();

        let rendered = format!("{error:?}");
        assert_eq!(error.status(), Some(401));
        assert!(!rendered.contains(credential));
        assert!(!rendered.contains(credential.trim()));
        assert!(rendered.contains("[REDACTED]"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn model_catalog_handles_fragmented_json_and_rejects_oversized_or_excessive_lists() {
        let body = br#"{"data":[{"id":"fragmented-model","display_name":"Fragmented"}]}"#;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let chunks = vec![
            (Duration::ZERO, header.into_bytes()),
            (Duration::from_millis(2), body[..9].to_vec()),
            (Duration::from_millis(2), body[9..31].to_vec()),
            (Duration::from_millis(2), body[31..].to_vec()),
        ];
        let (fragmented_base, _, fragmented_server) = spawn_raw_response(chunks);
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let fragmented = gateway
            .list_models(
                model_query(
                    fragmented_base,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::None,
                ),
                None,
            )
            .await
            .unwrap();
        assert_eq!(fragmented[0].id, "fragmented-model");
        fragmented_server.join().unwrap();

        let oversized_header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            super::MAX_MODEL_CATALOG_BODY_BYTES + 1
        );
        let (oversized_base, _, oversized_server) =
            spawn_raw_response(vec![(Duration::ZERO, oversized_header.into_bytes())]);
        let oversized = gateway
            .list_models(
                model_query(
                    oversized_base,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::None,
                ),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            oversized,
            crate::ports::provider::ProviderError::InvalidResponse(_)
        ));
        oversized_server.join().unwrap();

        let entries = (0..=super::MAX_DISCOVERED_MODELS)
            .map(|index| serde_json::json!({ "id": format!("model-{index}") }))
            .collect::<Vec<_>>();
        let (excessive_base, _, excessive_server) = spawn_single_response(
            "200 OK",
            vec![("Content-Type".into(), "application/json".into())],
            serde_json::json!({ "data": entries }).to_string(),
        );
        let excessive = gateway
            .list_models(
                model_query(
                    excessive_base,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::None,
                ),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            excessive,
            crate::ports::provider::ProviderError::InvalidResponse(_)
        ));
        excessive_server.join().unwrap();
    }

    #[tokio::test]
    async fn model_catalog_reports_disconnects_and_total_timeouts() {
        let partial = b"{\"data\":[";
        let partial_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}",
            String::from_utf8_lossy(partial)
        );
        let (disconnect_base, _, disconnect_server) =
            spawn_raw_response(vec![(Duration::ZERO, partial_response.into_bytes())]);
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let disconnected = gateway
            .list_models(
                model_query(
                    disconnect_base,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::None,
                ),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            disconnected,
            crate::ports::provider::ProviderError::Transport(_)
        ));
        assert!(disconnected.retryable());
        disconnect_server.join().unwrap();

        let delayed_response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"data\":[]}".to_vec();
        let (timeout_base, timeout_request, timeout_server) =
            spawn_raw_response(vec![(Duration::from_millis(150), delayed_response)]);
        let timeout_gateway = ReqwestProviderGateway::with_defaults()
            .unwrap()
            .with_model_catalog_timeout(Duration::from_millis(25));
        let timeout_error = timeout_gateway
            .list_models(
                model_query(
                    timeout_base,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::None,
                ),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            timeout_error,
            crate::ports::provider::ProviderError::Transport(_)
        ));
        assert!(timeout_error.retryable());
        timeout_request.recv().unwrap();
        timeout_server.join().unwrap();
    }

    #[tokio::test]
    async fn model_catalog_does_not_follow_redirects_or_forward_credentials() {
        let redirect_target = spawn_redirect_target();
        let (base_url, request_receiver, server) = spawn_single_response(
            "307 Temporary Redirect",
            vec![("Location".into(), redirect_target.url.clone())],
            String::new(),
        );
        let gateway = ReqwestProviderGateway::with_defaults().unwrap();
        let error = gateway
            .list_models(
                model_query(
                    base_url,
                    ProviderModelCatalogKind::OpenAi,
                    CredentialPlacement::BearerHeader,
                ),
                Some(SessionCredential::new(TEST_CREDENTIAL)),
            )
            .await
            .unwrap_err();
        assert_eq!(error.status(), Some(307));
        redirect_target.stop_sender.send(()).unwrap();
        redirect_target.handle.join().unwrap();
        assert!(matches!(
            redirect_target.request_receiver.try_recv(),
            Err(std_mpsc::TryRecvError::Empty | std_mpsc::TryRecvError::Disconnected)
        ));
        assert!(request_receiver.recv().unwrap().contains(TEST_CREDENTIAL));
        server.join().unwrap();
    }
}
