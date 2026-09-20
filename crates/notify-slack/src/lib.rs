//! Optional `Notifier` provider: Slack (DESIGN.md §4.1, ADR-007).
//!
//! Gated behind the `slack` cargo feature (Cargo.toml, root). Queued with
//! bounded retry/backoff like every `Notifier` (via `RetryingNotifier`,
//! `monitra-provider`) — never blocks a probe.
//!
//! URL convention: `slack://hooks.example.com/services/x` posts a Slack
//! incoming-webhook payload (`{"text": …}`) to `https://hooks.example.com/services/x`
//! — the `slack://` scheme is a category marker (`registry::category_for_scheme`),
//! not a real transport, same convention `notify-webhook` uses.
//!
//! Unlike `notify-webhook`, there is no embedded default here — this
//! `Notifier` only exists once an operator attaches a `slack://` target
//! (§4.1: `notify-webhook` fills the always-present default role).

use async_trait::async_trait;
use monitra_provider::{Notifier, ProviderCategory, ProviderError};

pub struct SlackNotifier {
    endpoint: String,
    client: reqwest::Client,
}

impl SlackNotifier {
    /// `target` is the raw configured `slack://…` value.
    pub fn new(target: &str) -> Self {
        Self {
            endpoint: slack_url_to_endpoint(target),
            client: reqwest::Client::new(),
        }
    }
}

fn slack_url_to_endpoint(url: &str) -> String {
    match url.strip_prefix("slack://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

#[async_trait]
impl Notifier for SlackNotifier {
    fn name(&self) -> &'static str {
        "slack"
    }

    async fn notify(&self, message: &str) -> Result<(), ProviderError> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(&serde_json::json!({ "text": message }))
            .send()
            .await
            .map_err(|source| ProviderError::Unavailable {
                category: ProviderCategory::Notifier,
                detail: format!("slack: request to {} failed: {source}", self.endpoint),
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(ProviderError::Unavailable {
                category: ProviderCategory::Notifier,
                detail: format!("slack: {} returned {}", self.endpoint, response.status()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    fn spawn_mock_http_server(
        status_line: &'static str,
    ) -> (String, std::thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
        let addr = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            stream
                .write_all(format!("{status_line}\r\nContent-Length: 0\r\n\r\n").as_bytes())
                .expect("write response");
            buf[..n].to_vec()
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn slack_scheme_is_translated_to_https() {
        assert_eq!(
            slack_url_to_endpoint("slack://hooks.example.com/services/x"),
            "https://hooks.example.com/services/x"
        );
    }

    #[tokio::test]
    async fn posts_slack_payload_on_success() {
        let (base, handle) = spawn_mock_http_server("HTTP/1.1 200 OK");
        let notifier = SlackNotifier {
            endpoint: base,
            client: reqwest::Client::new(),
        };

        let result = notifier.notify("down: example").await;
        assert!(result.is_ok(), "expected success, got {result:?}");

        let request = handle.join().expect("mock server thread");
        let request = String::from_utf8_lossy(&request);
        assert!(request.contains("\"text\""));
        assert!(request.contains("down: example"));
    }

    #[tokio::test]
    async fn non_2xx_response_is_unavailable() {
        let (base, _handle) = spawn_mock_http_server("HTTP/1.1 500 Internal Server Error");
        let notifier = SlackNotifier {
            endpoint: base,
            client: reqwest::Client::new(),
        };

        let result = notifier.notify("down: example").await;
        assert!(matches!(result, Err(ProviderError::Unavailable { .. })));
    }
}
