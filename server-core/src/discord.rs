use std::time::Duration;

use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Discord {
    client: Client,
    webhook: Url,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    RateLimited { retry_seconds: u64 },
    Disabled,
    Rejected,
    Uncertain,
}

#[derive(Serialize)]
struct Message<'a> {
    content: &'a str,
    allowed_mentions: AllowedMentions,
}
#[derive(Serialize)]
struct AllowedMentions {
    parse: [String; 0],
}
#[derive(Deserialize)]
struct RateLimit {
    retry_after: f64,
}

impl Discord {
    pub fn new(webhook: &str) -> anyhow::Result<Self> {
        let mut webhook =
            Url::parse(webhook).map_err(|_| anyhow::anyhow!("invalid Discord webhook URL"))?;
        webhook.query_pairs_mut().append_pair("wait", "true");
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            // Webhook POSTs must never be retried by the HTTP client.
            .retry(reqwest::retry::never())
            .build()?;
        Ok(Self { client, webhook })
    }

    pub async fn send(&self, content: &str) -> anyhow::Result<Delivery> {
        let body = serde_json::to_vec(&Message {
            content,
            allowed_mentions: AllowedMentions { parse: [] },
        })?;
        let response = match self
            .client
            .post(self.webhook.clone())
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => return Ok(Delivery::Uncertain),
        };
        let status = response.status();
        if status.is_success() {
            return Ok(Delivery::Sent);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let header_delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<f64>().ok());
            let body_delay = crate::sonarr::bounded_body(response, 65536)
                .await
                .ok()
                .and_then(|body| serde_json::from_slice::<RateLimit>(&body).ok())
                .map(|rate| rate.retry_after);
            let delay = body_delay
                .or(header_delay)
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(60.0);
            // A malicious or corrupt duration cannot overflow timestamps.
            return Ok(Delivery::RateLimited {
                retry_seconds: delay.ceil().clamp(1.0, 604800.0) as u64,
            });
        }
        if matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ) {
            return Ok(Delivery::Disabled);
        }
        if status.is_client_error() {
            return Ok(Delivery::Rejected);
        }
        // A proxy/server error may happen after Discord accepted the message.
        Ok(Delivery::Uncertain)
    }
}
