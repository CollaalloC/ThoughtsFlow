use std::{collections::BTreeMap, fmt, future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDialect {
    OpenAiChatCompletions,
    OllamaChat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "header_name", rename_all = "snake_case")]
pub enum CredentialPlacement {
    None,
    BearerHeader,
    Header(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTarget {
    pub dialect: ProviderDialect,
    pub base_url: String,
    pub credential_placement: CredentialPlacement,
    pub additional_headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalRequest {
    pub run_id: String,
    pub model: String,
    pub messages: Vec<CanonicalMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
}

/// A process-memory-only credential. It deliberately does not implement
/// `Serialize`, `Deserialize`, or a revealing `Debug` implementation.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionCredential(String);

impl SessionCredential {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SessionCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionCredential(<redacted>)")
    }
}

impl Drop for SessionCredential {
    fn drop(&mut self) {
        // Best-effort overwrite of the owned allocation. This is not a promise
        // of secure erasure from process memory or intermediate caller copies.
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
    }
}

pub struct ProviderInvocation {
    pub target: ProviderTarget,
    pub credential: Option<SessionCredential>,
    pub request: CanonicalRequest,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    RunStarted {
        run_id: String,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    UsageUpdated {
        usage: Usage,
    },
    ProviderMetadata {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
    },
    RunCompleted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_reason: Option<String>,
    },
    RunFailed {
        code: String,
        message: String,
        retryable: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
    },
    RunCancelled,
}

impl RunEvent {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::RunCompleted { .. } | Self::RunFailed { .. } | Self::RunCancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    InvalidEndpoint(String),
    UnsupportedScheme(String),
    CredentialsInUrl,
    InsecureRemoteEndpoint(String),
    InvalidResponse(String),
    UnexpectedEof,
    Http {
        status: u16,
        provider_code: Option<String>,
        message: String,
        retryable: bool,
    },
    Transport(String),
    EventChannelClosed,
}

impl ProviderError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidEndpoint(_) => "invalid_endpoint",
            Self::UnsupportedScheme(_) => "unsupported_scheme",
            Self::CredentialsInUrl => "credentials_in_url",
            Self::InsecureRemoteEndpoint(_) => "insecure_remote_endpoint",
            Self::InvalidResponse(_) => "invalid_response",
            Self::UnexpectedEof => "unexpected_eof",
            Self::Http { .. } => "provider_http_error",
            Self::Transport(_) => "transport_error",
            Self::EventChannelClosed => "event_channel_closed",
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::Http { retryable, .. } => *retryable,
            Self::Transport(_) | Self::UnexpectedEof => true,
            _ => false,
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn to_run_failed(&self) -> RunEvent {
        RunEvent::RunFailed {
            code: match self {
                Self::Http {
                    provider_code: Some(code),
                    ..
                } => code.clone(),
                _ => self.code().to_owned(),
            },
            message: self.to_string(),
            retryable: self.retryable(),
            status: self.status(),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEndpoint(message) => {
                write!(formatter, "invalid provider endpoint: {message}")
            }
            Self::UnsupportedScheme(scheme) => {
                write!(formatter, "unsupported provider URL scheme: {scheme}")
            }
            Self::CredentialsInUrl => {
                formatter.write_str("provider URL must not contain credentials")
            }
            Self::InsecureRemoteEndpoint(host) => write!(
                formatter,
                "plain HTTP is only allowed for loopback endpoints, not {host}"
            ),
            Self::InvalidResponse(message) => {
                write!(formatter, "invalid provider response: {message}")
            }
            Self::UnexpectedEof => {
                formatter.write_str("provider stream ended before a terminal event")
            }
            Self::Http {
                status, message, ..
            } => write!(formatter, "provider returned HTTP {status}: {message}"),
            Self::Transport(message) => write!(formatter, "provider transport failed: {message}"),
            Self::EventChannelClosed => formatter.write_str("provider event receiver was closed"),
        }
    }
}

impl std::error::Error for ProviderError {}

pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ProviderError>> + Send + 'a>>;

pub trait ProviderGateway: Send + Sync {
    /// Streams normalized events into a caller-owned bounded channel.
    ///
    /// The caller owns backpressure policy. Dropping or cancelling the token
    /// stops response processing without waiting for the remote stream to end.
    fn stream<'a>(
        &'a self,
        invocation: ProviderInvocation,
        cancellation: CancellationToken,
        events: mpsc::Sender<RunEvent>,
    ) -> ProviderFuture<'a>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderConnectionStatus {
    pub ok: bool,
    pub http_status: u16,
}

pub type ProviderConnectionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProviderConnectionStatus, ProviderError>> + Send + 'a>>;

pub trait ProviderConnectionTester: Send + Sync {
    fn test<'a>(
        &'a self,
        target: ProviderTarget,
        credential: Option<SessionCredential>,
    ) -> ProviderConnectionFuture<'a>;
}

/// The response shape exposed by a Provider's model-list endpoint.
///
/// This is deliberately separate from the streaming dialect: a future
/// Provider may use one request protocol for runs and another response shape
/// for model discovery. Application code resolves this value from the Rust
/// template catalog; it is never accepted from the WebView.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderModelCatalogKind {
    OpenAi,
    Ollama,
    Google,
}

/// An authority-resolved model discovery request.
///
/// `ProviderTarget` freezes the validated endpoint, credential placement, and
/// static headers selected by the Rust template catalog. Credentials remain a
/// separate process-memory-only argument to `ProviderModelCatalog::list_models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModelQuery {
    pub target: ProviderTarget,
    pub catalog: ProviderModelCatalogKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredModel {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
}

pub type ProviderModelsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<DiscoveredModel>, ProviderError>> + Send + 'a>>;

/// Narrow read port for Provider metadata. Discovery never sends workspace
/// Context and never persists the raw Provider response.
pub trait ProviderModelCatalog: Send + Sync {
    fn list_models<'a>(
        &'a self,
        query: ProviderModelQuery,
        credential: Option<SessionCredential>,
    ) -> ProviderModelsFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::SessionCredential;

    #[test]
    fn session_credential_debug_output_is_redacted() {
        let credential = SessionCredential::new("sk-should-never-leak");

        assert_eq!(format!("{credential:?}"), "SessionCredential(<redacted>)");
    }
}
