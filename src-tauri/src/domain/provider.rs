#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamProtocol {
    OpenAiSse,
    OllamaNdjson,
    AnthropicSse,
    GoogleSse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthPlacement {
    None,
    BearerHeader,
    ApiKeyHeader,
    QueryParam,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticHeader {
    pub name: &'static str,
    pub value: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolProfile {
    pub stream_protocol: StreamProtocol,
    pub auth_placement: AuthPlacement,
    pub auth_header_name: Option<&'static str>,
    pub models_endpoint: Option<&'static str>,
    pub requires_additional_headers: bool,
    pub additional_headers: &'static [StaticHeader],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderTemplate {
    pub provider_id: &'static str,
    pub revision: u16,
    pub display_name: &'static str,
    pub default_base_url: &'static str,
    pub protocol: ProtocolProfile,
    pub runtime_available: bool,
}

const NO_ADDITIONAL_HEADERS: &[StaticHeader] = &[];
const ANTHROPIC_HEADERS: &[StaticHeader] = &[StaticHeader {
    name: "anthropic-version",
    value: "2023-06-01",
}];

const PROVIDER_TEMPLATES: [ProviderTemplate; 7] = [
    ProviderTemplate {
        provider_id: "openai",
        revision: 1,
        display_name: "OpenAI",
        default_base_url: "https://api.openai.com/v1",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::OpenAiSse,
            auth_placement: AuthPlacement::BearerHeader,
            auth_header_name: Some("Authorization"),
            models_endpoint: Some("/models"),
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        runtime_available: true,
    },
    ProviderTemplate {
        provider_id: "openai-compatible",
        revision: 1,
        display_name: "Generic OpenAI-compatible",
        default_base_url: "http://127.0.0.1:8000/v1",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::OpenAiSse,
            auth_placement: AuthPlacement::BearerHeader,
            auth_header_name: Some("Authorization"),
            models_endpoint: Some("/models"),
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        runtime_available: true,
    },
    ProviderTemplate {
        provider_id: "ollama",
        revision: 2,
        display_name: "Ollama",
        default_base_url: "http://127.0.0.1:11434",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::OllamaNdjson,
            // Ollama itself does not require authentication, but remote or
            // proxied Ollama endpoints may use an optional Bearer token. The
            // provider client emits this header only when a session credential
            // exists, preserving both local no-auth and legacy authenticated
            // profiles.
            auth_placement: AuthPlacement::BearerHeader,
            auth_header_name: Some("Authorization"),
            models_endpoint: Some("/api/tags"),
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        runtime_available: true,
    },
    ProviderTemplate {
        provider_id: "anthropic",
        revision: 1,
        display_name: "Anthropic",
        default_base_url: "https://api.anthropic.com",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::AnthropicSse,
            auth_placement: AuthPlacement::ApiKeyHeader,
            auth_header_name: Some("x-api-key"),
            models_endpoint: Some("/v1/models"),
            requires_additional_headers: true,
            additional_headers: ANTHROPIC_HEADERS,
        },
        runtime_available: false,
    },
    ProviderTemplate {
        provider_id: "google",
        revision: 1,
        display_name: "Google Gemini",
        default_base_url: "https://generativelanguage.googleapis.com",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::GoogleSse,
            auth_placement: AuthPlacement::ApiKeyHeader,
            auth_header_name: Some("x-goog-api-key"),
            models_endpoint: Some("/v1beta/models"),
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        runtime_available: false,
    },
    ProviderTemplate {
        provider_id: "azure-openai",
        revision: 1,
        display_name: "Azure OpenAI",
        default_base_url: "https://RESOURCE.openai.azure.com/openai/v1",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::OpenAiSse,
            auth_placement: AuthPlacement::ApiKeyHeader,
            auth_header_name: Some("api-key"),
            models_endpoint: None,
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        runtime_available: false,
    },
    ProviderTemplate {
        provider_id: "openrouter",
        revision: 1,
        display_name: "OpenRouter",
        default_base_url: "https://openrouter.ai/api/v1",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::OpenAiSse,
            auth_placement: AuthPlacement::BearerHeader,
            auth_header_name: Some("Authorization"),
            models_endpoint: Some("/models"),
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        runtime_available: true,
    },
];

pub fn provider_templates() -> &'static [ProviderTemplate] {
    &PROVIDER_TEMPLATES
}

pub fn provider_template(provider_id: &str) -> Option<&'static ProviderTemplate> {
    PROVIDER_TEMPLATES
        .iter()
        .find(|template| template.provider_id == provider_id)
}
