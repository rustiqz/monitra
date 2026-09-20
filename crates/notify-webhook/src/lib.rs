//! `Notifier` provider: webhook (DESIGN.md §4.1, Appendix — "default build").
//!
//! Compiled into the default build (not feature-gated). Fills both roles
//! §4.1 names for the embedded `Notifier` default: with no target attached,
//! `notify` logs instead of sending; once a `webhook://` URL is attached
//! (`monitra service attach webhook://…`), it POSTs a JSON body to it.
//!
//! URL convention: the `webhook://` scheme is only a category marker
//! (`registry::category_for_scheme`) — the actual destination is whatever
//! follows it, with `https://` assumed. `webhook://hooks.example.com/x`
//! posts to `https://hooks.example.com/x`.

use async_trait::async_trait;
use monitra_provider::{Notifier, ProviderCategory, ProviderError};

pub struct WebhookNotifier {
    endpoint: Option<String>,
    client: reqwest::Client,
}

impl WebhookNotifier {
    /// `target` is the raw configured value (e.g. `webhook://hooks.example.com/x`),
    /// or `None` when nothing is attached (log-only, §4.1's "logs instead of
    /// POSTing" default).
    pub fn new(target: Option<String>) -> Self {
        Self {
            endpoint: target.as_deref().map(webhook_url_to_endpoint),
            client: reqwest::Client::new(),
        }
    }
}

/// `webhook://host/path` -> `https://host/path`. The `webhook://` scheme is
/// only ever used as a category marker (`registry::category_for_scheme`);
/// this is where that marker gets turned into a real destination.
fn webhook_url_to_endpoint(url: &str) -> String {
    match url.strip_prefix("webhook://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

#[async_trait]
impl Notifier for WebhookNotifier {
    fn name(&self) -> &'static str {
        "webhook"
    }

    async fn notify(&self, message: &str) -> Result<(), ProviderError> {
        let Some(endpoint) = &self.endpoint else {
            tracing::info!(
                message,
                "notify (webhook): no target attached, logging only"
            );
            return Ok(());
        };

        let response = self
            .client
            .post(endpoint)
            .json(&serde_json::json!({ "message": message }))
            .send()
            .await
            .map_err(|source| ProviderError::Unavailable {
                category: ProviderCategory::Notifier,
                detail: format!("webhook: request to {endpoint} failed: {source}"),
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(ProviderError::Unavailable {
                category: ProviderCategory::Notifier,
                detail: format!("webhook: {endpoint} returned {}", response.status()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// A minimal one-shot HTTP mock: bare TCP, not axum (matching this
    /// project's convention for test-only HTTP peers, e.g. `tests/scale.rs`)
    /// — a real `reqwest::Client` request against it exercises the actual
    /// HTTP path, not a trait-level fake.
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
    fn webhook_scheme_is_translated_to_https() {
        assert_eq!(
            webhook_url_to_endpoint("webhook://hooks.example.com/x"),
            "https://hooks.example.com/x"
        );
    }

    #[tokio::test]
    async fn no_target_logs_instead_of_sending() {
        let notifier = WebhookNotifier::new(None);
        assert!(notifier.notify("test message").await.is_ok());
    }

    #[tokio::test]
    async fn posts_to_the_attached_target_on_success() {
        let (base, handle) = spawn_mock_http_server("HTTP/1.1 200 OK");
        let notifier = WebhookNotifier {
            endpoint: Some(base),
            client: reqwest::Client::new(),
        };

        let result = notifier.notify("down: example").await;
        assert!(result.is_ok(), "expected success, got {result:?}");

        let request = handle.join().expect("mock server thread");
        let request = String::from_utf8_lossy(&request);
        assert!(request.contains("down: example"));
    }

    #[tokio::test]
    async fn non_2xx_response_is_unavailable() {
        let (base, _handle) = spawn_mock_http_server("HTTP/1.1 500 Internal Server Error");
        let notifier = WebhookNotifier {
            endpoint: Some(base),
            client: reqwest::Client::new(),
        };

        let result = notifier.notify("down: example").await;
        assert!(matches!(result, Err(ProviderError::Unavailable { .. })));
    }
}
