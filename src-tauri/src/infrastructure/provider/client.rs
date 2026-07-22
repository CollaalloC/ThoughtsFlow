use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    infrastructure::provider::{
        OllamaNdjsonDecoder, OpenAiSseDecoder, ProviderStreamDecoder, decode_http_error,
        provider_request_url,
    },
    ports::provider::{
        CanonicalMessage, CanonicalRequest, MessageRole, ProviderDialect, ProviderError,
        ProviderFuture, ProviderGateway, ProviderInvocation, RunEvent,
    },
};

const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

pub struct ReqwestProviderGateway {
    client: reqwest::Client,
}

impl ReqwestProviderGateway {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub fn with_defaults() -> Result<Self, ProviderError> {
        reqwest::Client::builder()
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
        if cancellation.is_cancelled() {
            return send_terminal(&events, RunEvent::RunCancelled).await;
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
        if let Some(credential) = invocation
            .credential
            .as_ref()
            .filter(|credential| !credential.is_empty())
        {
            request = request.bearer_auth(credential.expose_secret());
        }

        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return send_terminal(&events, RunEvent::RunCancelled).await;
            }
            response = request.send() => response,
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return send_failure(&events, ProviderError::Transport(error.to_string())).await;
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
                Ok(None) => return send_terminal(&events, RunEvent::RunCancelled).await,
                Err(error) => return send_failure(&events, error).await,
            };
            return send_failure(
                &events,
                decode_http_error(status, content_type.as_deref(), &body),
            )
            .await;
        }

        if !send_event(
            &events,
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
                    return send_terminal(&events, RunEvent::RunCancelled).await;
                }
                next = bytes.next() => next,
            };
            match next {
                Some(Ok(chunk)) => {
                    let decoded = match decoder.push(&chunk) {
                        Ok(decoded) => decoded,
                        Err(error) => return send_failure(&events, error).await,
                    };
                    for event in decoded {
                        if !send_event(&events, event, &cancellation).await? {
                            return Ok(());
                        }
                    }
                    if decoder.is_terminal() {
                        return Ok(());
                    }
                }
                Some(Err(error)) => {
                    return send_failure(&events, ProviderError::Transport(error.to_string()))
                        .await;
                }
                None => {
                    let decoded = match decoder.finish() {
                        Ok(decoded) => decoded,
                        Err(error) => return send_failure(&events, error).await,
                    };
                    for event in decoded {
                        if !send_event(&events, event, &cancellation).await? {
                            return Ok(());
                        }
                    }
                    return Ok(());
                }
            }
        }
    }
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

async fn send_event(
    events: &mpsc::Sender<RunEvent>,
    event: RunEvent,
    cancellation: &CancellationToken,
) -> Result<bool, ProviderError> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            send_terminal(events, RunEvent::RunCancelled).await?;
            Ok(false)
        },
        result = events.send(event) => {
            result.map_err(|_| ProviderError::EventChannelClosed)?;
            Ok(true)
        },
    }
}

async fn send_failure(
    events: &mpsc::Sender<RunEvent>,
    error: ProviderError,
) -> Result<(), ProviderError> {
    send_terminal(events, error.to_run_failed()).await
}

async fn send_terminal(
    events: &mpsc::Sender<RunEvent>,
    event: RunEvent,
) -> Result<(), ProviderError> {
    events
        .send(event)
        .await
        .map_err(|_| ProviderError::EventChannelClosed)
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
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::ports::provider::{
        CanonicalRequest, ProviderDialect, ProviderGateway, ProviderInvocation, ProviderTarget,
        RunEvent,
    };

    use super::ReqwestProviderGateway;

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
