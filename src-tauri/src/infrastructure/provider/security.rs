use std::net::IpAddr;

use url::{Host, Url};

use crate::ports::provider::{ProviderDialect, ProviderError};

pub fn validate_base_url(base_url: &str) -> Result<Url, ProviderError> {
    let url =
        Url::parse(base_url).map_err(|error| ProviderError::InvalidEndpoint(error.to_string()))?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProviderError::CredentialsInUrl);
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
    use crate::ports::provider::{ProviderDialect, ProviderError};

    use super::{provider_request_url, validate_base_url};

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
    }

    #[test]
    fn provider_paths_are_appended_without_discarding_an_openai_v1_prefix() {
        assert_eq!(
            provider_request_url(
                "https://models.example.com/v1",
                ProviderDialect::OpenAiChatCompletions,
            )
            .unwrap()
            .as_str(),
            "https://models.example.com/v1/chat/completions"
        );
        assert_eq!(
            provider_request_url("http://localhost:11434", ProviderDialect::OllamaChat)
                .unwrap()
                .as_str(),
            "http://localhost:11434/api/chat"
        );
    }
}
