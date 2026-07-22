use std::net::IpAddr;

use url::{Host, Url};

use crate::ports::provider::{ProviderDialect, ProviderError, ProviderModelCatalogKind};

pub fn validate_base_url(base_url: &str) -> Result<Url, ProviderError> {
    let url =
        Url::parse(base_url).map_err(|error| ProviderError::InvalidEndpoint(error.to_string()))?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProviderError::CredentialsInUrl);
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProviderError::InvalidEndpoint(
            "Base URL must not include a query string or fragment".into(),
        ));
    }

    match url.scheme() {
        "https" => {}
        "http" => {
            let host = url
                .host()
                .ok_or_else(|| ProviderError::InvalidEndpoint("URL has no host".to_owned()))?;
            if !is_loopback_host(host) {
                return Err(ProviderError::InsecureRemoteEndpoint(
                    url.host_str().unwrap_or("unknown").to_owned(),
                ));
            }
        }
        scheme => return Err(ProviderError::UnsupportedScheme(scheme.to_owned())),
    }

    if url.host_str().is_none() {
        return Err(ProviderError::InvalidEndpoint("URL has no host".to_owned()));
    }

    Ok(url)
}

pub fn provider_request_url(
    base_url: &str,
    dialect: ProviderDialect,
    model: &str,
) -> Result<Url, ProviderError> {
    let mut url = validate_base_url(base_url)?;
    let base_path = url.path().trim_end_matches('/');
    let path = match dialect {
        ProviderDialect::OpenAiChatCompletions if base_path.ends_with("/chat/completions") => {
            base_path.to_owned()
        }
        ProviderDialect::OpenAiChatCompletions => format!("{base_path}/chat/completions"),
        ProviderDialect::OllamaChat if base_path.ends_with("/api/chat") => base_path.to_owned(),
        ProviderDialect::OllamaChat if base_path.ends_with("/api") => {
            format!("{base_path}/chat")
        }
        ProviderDialect::OllamaChat => format!("{base_path}/api/chat"),
        ProviderDialect::AnthropicMessages if base_path.ends_with("/v1/messages") => {
            base_path.to_owned()
        }
        ProviderDialect::AnthropicMessages if base_path.ends_with("/v1") => {
            format!("{base_path}/messages")
        }
        ProviderDialect::AnthropicMessages => format!("{base_path}/v1/messages"),
        ProviderDialect::GoogleGenerativeAi => {
            let model = normalized_google_model(model)?;
            let prefix =
                if base_path.ends_with("/v1beta/models") || base_path.ends_with("/v1/models") {
                    base_path.trim_end_matches("/models").to_owned()
                } else if base_path.ends_with("/v1beta") || base_path.ends_with("/v1") {
                    base_path.to_owned()
                } else {
                    format!("{base_path}/v1beta")
                };
            url.set_path(&prefix);
            url.path_segments_mut()
                .map_err(|_| {
                    ProviderError::InvalidEndpoint(
                        "Google Provider URL cannot contain path segments".into(),
                    )
                })?
                .push("models")
                .push(&format!("{model}:streamGenerateContent"));
            url.set_query(Some("alt=sse"));
            return Ok(url);
        }
    };
    url.set_path(&path);
    Ok(url)
}

fn normalized_google_model(model: &str) -> Result<&str, ProviderError> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    let invalid = model.is_empty()
        || model == "."
        || model == ".."
        || model.starts_with("models/")
        || model.trim() != model
        || model.chars().any(|character| {
            character.is_control()
                || matches!(character, '/' | '\\' | '?' | '#' | '%' | ':')
                || is_bidi_control(character)
        });
    if invalid {
        return Err(ProviderError::InvalidEndpoint(
            "Google model must be one unambiguous model identifier, with at most one `models/` prefix"
                .into(),
        ));
    }
    Ok(model)
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// Builds the metadata-only model catalog endpoint without discarding a
/// self-hosted API prefix. Known chat endpoints are replaced rather than
/// blindly appended, so both an API prefix and a frozen chat endpoint are safe
/// inputs.
pub fn provider_models_url(
    base_url: &str,
    catalog: ProviderModelCatalogKind,
) -> Result<Url, ProviderError> {
    let mut url = validate_base_url(base_url)?;
    let base_path = url.path().trim_end_matches('/');
    let path = match catalog {
        ProviderModelCatalogKind::OpenAi if base_path.ends_with("/models") => base_path.to_owned(),
        ProviderModelCatalogKind::OpenAi if base_path.ends_with("/chat/completions") => {
            let prefix = base_path.trim_end_matches("/chat/completions");
            format!("{prefix}/models")
        }
        ProviderModelCatalogKind::OpenAi => format!("{base_path}/models"),
        ProviderModelCatalogKind::Ollama if base_path.ends_with("/api/tags") => {
            base_path.to_owned()
        }
        ProviderModelCatalogKind::Ollama if base_path.ends_with("/api/chat") => {
            let prefix = base_path.trim_end_matches("/api/chat");
            format!("{prefix}/api/tags")
        }
        ProviderModelCatalogKind::Ollama if base_path.ends_with("/api") => {
            format!("{base_path}/tags")
        }
        ProviderModelCatalogKind::Ollama => format!("{base_path}/api/tags"),
        ProviderModelCatalogKind::Anthropic if base_path.ends_with("/v1/models") => {
            base_path.to_owned()
        }
        ProviderModelCatalogKind::Anthropic if base_path.ends_with("/v1/messages") => {
            let prefix = base_path.trim_end_matches("/messages");
            format!("{prefix}/models")
        }
        ProviderModelCatalogKind::Anthropic if base_path.ends_with("/v1") => {
            format!("{base_path}/models")
        }
        ProviderModelCatalogKind::Anthropic => format!("{base_path}/v1/models"),
        ProviderModelCatalogKind::Google if base_path.ends_with("/models") => base_path.to_owned(),
        ProviderModelCatalogKind::Google
            if base_path.ends_with("/v1beta") || base_path.ends_with("/v1") =>
        {
            format!("{base_path}/models")
        }
        ProviderModelCatalogKind::Google => format!("{base_path}/v1beta/models"),
    };
    url.set_path(&path);
    Ok(url)
}

fn is_loopback_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(host) => host.trim_end_matches('.').eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => IpAddr::V4(address).is_loopback(),
        Host::Ipv6(address) => IpAddr::V6(address).is_loopback(),
    }
}

#[cfg(test)]
mod tests {
    use crate::ports::provider::{ProviderDialect, ProviderError, ProviderModelCatalogKind};

    use super::{provider_models_url, provider_request_url, validate_base_url};

    #[test]
    fn endpoint_policy_accepts_https_and_exact_loopback_http_hosts() {
        assert_eq!(
            validate_base_url("https://models.example.com/v1")
                .unwrap()
                .as_str(),
            "https://models.example.com/v1"
        );
        assert!(validate_base_url("http://localhost:11434").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080/v1").is_ok());
        assert!(validate_base_url("http://[::1]:11434").is_ok());
    }

    #[test]
    fn endpoint_policy_rejects_remote_http_embedded_credentials_and_non_http_schemes() {
        assert!(matches!(
            validate_base_url("http://models.example.com/v1"),
            Err(ProviderError::InsecureRemoteEndpoint(_))
        ));
        assert_eq!(
            validate_base_url("https://user:secret@models.example.com/v1"),
            Err(ProviderError::CredentialsInUrl)
        );
        assert_eq!(
            validate_base_url("file:///tmp/provider"),
            Err(ProviderError::UnsupportedScheme("file".to_owned()))
        );
        assert!(matches!(
            validate_base_url("not a URL"),
            Err(ProviderError::InvalidEndpoint(_))
        ));
        assert!(matches!(
            validate_base_url("https://models.example.com/v1?key=secret"),
            Err(ProviderError::InvalidEndpoint(_))
        ));
        assert!(matches!(
            validate_base_url("https://models.example.com/v1#fragment"),
            Err(ProviderError::InvalidEndpoint(_))
        ));
    }

    #[test]
    fn provider_paths_are_appended_without_discarding_an_openai_v1_prefix() {
        assert_eq!(
            provider_request_url(
                "https://models.example.com/v1",
                ProviderDialect::OpenAiChatCompletions,
                "ignored",
            )
            .unwrap()
            .as_str(),
            "https://models.example.com/v1/chat/completions"
        );
        assert_eq!(
            provider_request_url(
                "http://localhost:11434",
                ProviderDialect::OllamaChat,
                "ignored",
            )
            .unwrap()
            .as_str(),
            "http://localhost:11434/api/chat"
        );
    }

    #[test]
    fn anthropic_request_paths_preserve_prefixes_and_accept_a_frozen_endpoint() {
        let cases = [
            (
                "https://api.anthropic.com",
                "https://api.anthropic.com/v1/messages",
            ),
            (
                "https://proxy.example.com/tenant/v1",
                "https://proxy.example.com/tenant/v1/messages",
            ),
            (
                "https://proxy.example.com/tenant/v1/messages/",
                "https://proxy.example.com/tenant/v1/messages",
            ),
        ];

        for (base_url, expected) in cases {
            assert_eq!(
                provider_request_url(base_url, ProviderDialect::AnthropicMessages, "ignored")
                    .unwrap()
                    .as_str(),
                expected
            );
        }
    }

    #[test]
    fn google_request_paths_normalize_one_models_prefix_and_encode_the_model_segment() {
        let cases = [
            (
                "https://generativelanguage.googleapis.com",
                "gemini 2.5-pro",
                "https://generativelanguage.googleapis.com/v1beta/models/gemini%202.5-pro:streamGenerateContent?alt=sse",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta",
                "models/gemini-2.5-pro",
                "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse",
            ),
            (
                "https://generativelanguage.googleapis.com/v1",
                "gemini-2.5-pro",
                "https://generativelanguage.googleapis.com/v1/models/gemini-2.5-pro:streamGenerateContent?alt=sse",
            ),
            (
                "https://proxy.example.com/tenant/v1beta",
                "gemini-2.5-flash",
                "https://proxy.example.com/tenant/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
            ),
            (
                "https://proxy.example.com/tenant/v1/models/",
                "models/gemini-2.5-flash",
                "https://proxy.example.com/tenant/v1/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
            ),
            (
                "https://proxy.example.com/tenant/v1beta/models/",
                "gemini-2.5-flash",
                "https://proxy.example.com/tenant/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
            ),
        ];

        for (base_url, model, expected) in cases {
            assert_eq!(
                provider_request_url(base_url, ProviderDialect::GoogleGenerativeAi, model)
                    .unwrap()
                    .as_str(),
                expected
            );
        }
    }

    #[test]
    fn google_request_path_rejects_ambiguous_or_injectable_model_ids() {
        for model in [
            "",
            "models/",
            "models/models/gemini-pro",
            "../gemini-pro",
            "gemini/../pro",
            "gemini\\pro",
            "gemini?key=secret",
            "gemini#fragment",
            "gemini%2Fpro",
            "gemini%252Fpro",
            "gemini:streamGenerateContent",
            "gemini\u{202e}pro",
            "gemini\npro",
        ] {
            assert!(matches!(
                provider_request_url(
                    "https://generativelanguage.googleapis.com/v1beta",
                    ProviderDialect::GoogleGenerativeAi,
                    model,
                ),
                Err(ProviderError::InvalidEndpoint(_))
            ));
        }
    }

    #[test]
    fn model_catalog_paths_preserve_prefixes_and_replace_chat_endpoints() {
        let cases = [
            (
                "https://models.example.com/gateway/v1",
                ProviderModelCatalogKind::OpenAi,
                "https://models.example.com/gateway/v1/models",
            ),
            (
                "https://models.example.com/gateway/v1/chat/completions",
                ProviderModelCatalogKind::OpenAi,
                "https://models.example.com/gateway/v1/models",
            ),
            (
                "https://models.example.com/gateway/v1/models/",
                ProviderModelCatalogKind::OpenAi,
                "https://models.example.com/gateway/v1/models",
            ),
            (
                "http://localhost:11434",
                ProviderModelCatalogKind::Ollama,
                "http://localhost:11434/api/tags",
            ),
            (
                "http://localhost:11434/prefix/api/chat",
                ProviderModelCatalogKind::Ollama,
                "http://localhost:11434/prefix/api/tags",
            ),
            (
                "http://localhost:11434/prefix/api",
                ProviderModelCatalogKind::Ollama,
                "http://localhost:11434/prefix/api/tags",
            ),
            (
                "https://api.anthropic.com",
                ProviderModelCatalogKind::Anthropic,
                "https://api.anthropic.com/v1/models",
            ),
            (
                "https://proxy.example.com/tenant/v1/messages",
                ProviderModelCatalogKind::Anthropic,
                "https://proxy.example.com/tenant/v1/models",
            ),
            (
                "https://generativelanguage.googleapis.com",
                ProviderModelCatalogKind::Google,
                "https://generativelanguage.googleapis.com/v1beta/models",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta",
                ProviderModelCatalogKind::Google,
                "https://generativelanguage.googleapis.com/v1beta/models",
            ),
            (
                "https://generativelanguage.googleapis.com/v1",
                ProviderModelCatalogKind::Google,
                "https://generativelanguage.googleapis.com/v1/models",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta/models/",
                ProviderModelCatalogKind::Google,
                "https://generativelanguage.googleapis.com/v1beta/models",
            ),
            (
                "https://proxy.example.com/tenant/v1/models/",
                ProviderModelCatalogKind::Google,
                "https://proxy.example.com/tenant/v1/models",
            ),
        ];

        for (base_url, catalog, expected) in cases {
            assert_eq!(
                provider_models_url(base_url, catalog).unwrap().as_str(),
                expected
            );
        }
    }
}
