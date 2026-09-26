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
pub enum ProviderModelCatalogStrategy {
    RemoteOpenAi,
    RemoteOllama,
    RemoteAnthropic,
    RemoteGoogle,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderTemplate {
    pub provider_id: &'static str,
    pub revision: u16,
    pub display_name: &'static str,
    pub default_base_url: &'static str,
    pub protocol: ProtocolProfile,
    pub model_catalog: ProviderModelCatalogStrategy,
    pub runtime_available: bool,
}

const NO_ADDITIONAL_HEADERS: &[StaticHeader] = &[];
const ANTHROPIC_HEADERS: &[StaticHeader] = &[StaticHeader {
    name: "anthropic-version",
    value: "2023-06-01",
}];
// Endpoint/protocol sources and account prerequisites: docs/MODEL_CONNECTIONS.md.
const fn openai_template(
    provider_id: &'static str,
    display_name: &'static str,
    default_base_url: &'static str,
) -> ProviderTemplate {
    ProviderTemplate {
        provider_id,
        revision: 1,
        display_name,
        default_base_url,
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::OpenAiSse,
            auth_placement: AuthPlacement::BearerHeader,
            auth_header_name: Some("Authorization"),
            models_endpoint: Some("/models"),
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        model_catalog: ProviderModelCatalogStrategy::RemoteOpenAi,
        runtime_available: true,
    }
}

// Some compatible chat APIs use a separate, vendor-specific catalog. Keep
// manual model selection available without guessing a /models endpoint.
const fn manual_openai_template(
    provider_id: &'static str,
    display_name: &'static str,
    default_base_url: &'static str,
) -> ProviderTemplate {
    let mut template = openai_template(provider_id, display_name, default_base_url);
    template.protocol.models_endpoint = None;
    template.model_catalog = ProviderModelCatalogStrategy::Unsupported;
    template
}

const PROVIDER_TEMPLATES: &[ProviderTemplate] = &[
    openai_template("openai", "OpenAI", "https://api.openai.com/v1"),
    openai_template(
        "openai-compatible",
        "Generic OpenAI-compatible",
        "http://127.0.0.1:8000/v1",
    ),
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
        model_catalog: ProviderModelCatalogStrategy::RemoteOllama,
        runtime_available: true,
    },
    ProviderTemplate {
        provider_id: "anthropic",
        revision: 3,
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
        model_catalog: ProviderModelCatalogStrategy::RemoteAnthropic,
        runtime_available: true,
    },
    ProviderTemplate {
        provider_id: "google",
        revision: 2,
        display_name: "Google Gemini",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta",
        protocol: ProtocolProfile {
            stream_protocol: StreamProtocol::GoogleSse,
            auth_placement: AuthPlacement::ApiKeyHeader,
            auth_header_name: Some("x-goog-api-key"),
            models_endpoint: Some("/v1beta/models"),
            requires_additional_headers: false,
            additional_headers: NO_ADDITIONAL_HEADERS,
        },
        model_catalog: ProviderModelCatalogStrategy::RemoteGoogle,
        runtime_available: true,
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
        model_catalog: ProviderModelCatalogStrategy::Unsupported,
        runtime_available: false,
    },
    openai_template("openrouter", "OpenRouter", "https://openrouter.ai/api/v1"),
    openai_template(
        "omp-gateway",
        "OMP Gateway (Auth Broker)",
        "http://127.0.0.1:4000/v1",
    ),
    openai_template("deepseek", "DeepSeek", "https://api.deepseek.com/v1"),
    openai_template("xai", "xAI", "https://api.x.ai/v1"),
    openai_template("mistral", "Mistral", "https://api.mistral.ai/v1"),
    openai_template("groq", "Groq", "https://api.groq.com/openai/v1"),
    openai_template("together", "Together AI", "https://api.together.ai/v1"),
    openai_template(
        "moonshot",
        "Moonshot / Kimi (China)",
        "https://api.moonshot.cn/v1",
    ),
    manual_openai_template(
        "qwen-beijing",
        "Qwen / DashScope (Beijing)",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
    ),
    manual_openai_template(
        "qwen-singapore",
        "Qwen / DashScope (Singapore)",
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    ),
    manual_openai_template("zai", "Z.AI (API)", "https://api.z.ai/api/paas/v4"),
    openai_template(
        "siliconflow",
        "SiliconFlow (China)",
        "https://api.siliconflow.cn/v1",
    ),
];

pub fn provider_templates() -> &'static [ProviderTemplate] {
    PROVIDER_TEMPLATES
}

pub fn provider_template(provider_id: &str) -> Option<&'static ProviderTemplate> {
    PROVIDER_TEMPLATES
        .iter()
        .find(|template| template.provider_id == provider_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_catalog_uses_authenticated_discovery_instead_of_stale_model_ids() {
        let template = provider_template("anthropic").unwrap();
        assert_eq!(
            template.model_catalog,
            ProviderModelCatalogStrategy::RemoteAnthropic
        );
        assert_eq!(template.revision, 3);
        assert_eq!(template.protocol.auth_header_name, Some("x-api-key"));
        assert_eq!(template.protocol.models_endpoint, Some("/v1/models"));
    }

    #[test]
    fn every_template_has_an_explicit_model_catalog_strategy() {
        assert!(matches!(
            provider_template("openai").unwrap().model_catalog,
            ProviderModelCatalogStrategy::RemoteOpenAi
        ));
        assert!(matches!(
            provider_template("ollama").unwrap().model_catalog,
            ProviderModelCatalogStrategy::RemoteOllama
        ));
        assert!(matches!(
            provider_template("google").unwrap().model_catalog,
            ProviderModelCatalogStrategy::RemoteGoogle
        ));
        assert_eq!(
            provider_template("google").unwrap().default_base_url,
            "https://generativelanguage.googleapis.com/v1beta"
        );
        assert!(matches!(
            provider_template("azure-openai").unwrap().model_catalog,
            ProviderModelCatalogStrategy::Unsupported
        ));
    }
}
