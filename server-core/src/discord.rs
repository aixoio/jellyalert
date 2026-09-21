use std::time::Duration;

use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, instrument, warn};

#[derive(Clone)]
pub struct Discord {
    client: Client,
    webhook: Url,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    RateLimited { retry_seconds: u64 },
    Retryable { retry_seconds: u64 },
    Disabled,
    Rejected,
    Uncertain,
}

#[derive(Serialize)]
struct Message<'a> {
    content: &'static str,
    embeds: [Embed<'a>; 1],
    allowed_mentions: AllowedMentions,
}
#[derive(Serialize)]
struct Embed<'a> {
    author: Author,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    description: &'a str,
    color: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<EmbedImage>,
}
#[derive(Serialize)]
struct Author {
    name: &'static str,
}
#[derive(Serialize)]
struct EmbedImage {
    url: &'static str,
}
#[derive(Serialize)]
struct AllowedMentions {
    parse: [&'static str; 1],
}
#[derive(Deserialize)]
struct RateLimit {
    retry_after: f64,
}

#[derive(Clone, Copy)]
pub struct SeriesMetadata<'a> {
    pub title: &'a str,
    pub year: Option<i32>,
    pub imdb_id: Option<&'a str>,
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
        self.send_with_poster(content, None, 0x5865f2, None).await
    }

    #[instrument(
        skip(self, content, poster, series),
        fields(message.length = content.len(), message.color = color, message.has_poster = poster.is_some())
    )]
    pub async fn send_with_poster(
        &self,
        content: &str,
        poster: Option<Vec<u8>>,
        color: u32,
        series: Option<SeriesMetadata<'_>>,
    ) -> anyhow::Result<Delivery> {
        info!(message = %content, "preparing Discord message");
        let title = series.map(series_title);
        let url = series.and_then(|series| series.imdb_id.and_then(imdb_series_url));
        let body = serde_json::to_string(&Message {
            content: "@everyone",
            embeds: [Embed {
                author: Author { name: "Jelly Name" },
                title,
                url,
                description: content,
                color,
                image: poster.as_ref().map(|_| EmbedImage {
                    url: "attachment://series.jpg",
                }),
            }],
            allowed_mentions: AllowedMentions {
                parse: ["everyone"],
            },
        })?;
        let request = self.client.post(self.webhook.clone());
        let request = if let Some(poster) = poster {
            request.multipart(
                reqwest::multipart::Form::new()
                    .text("payload_json", body)
                    .part(
                        "files[0]",
                        reqwest::multipart::Part::bytes(poster)
                            .file_name("series.jpg")
                            .mime_str("image/jpeg")?,
                    ),
            )
        } else {
            request
                .header("Content-Type", "application/json")
                .body(body)
        };
        debug!("posting Discord webhook message");
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_connect() => {
                warn!(error = %error.without_url(), retry_seconds = 30, "Discord connection failed before delivery");
                return Ok(Delivery::Retryable { retry_seconds: 30 });
            }
            Err(error) => {
                error!(error = %error.without_url(), "Discord request failed with an uncertain delivery outcome");
                return Ok(Delivery::Uncertain);
            }
        };
        let status = response.status();
        debug!(%status, "Discord responded to webhook request");
        if status.is_success() {
            info!(%status, message = %content, "Discord message sent");
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
            let retry_seconds = delay.ceil().clamp(1.0, 604800.0) as u64;
            warn!(%status, retry_seconds, "Discord rate limited the webhook request");
            return Ok(Delivery::RateLimited { retry_seconds });
        }
        if matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ) {
            error!(%status, "Discord rejected or could not find the webhook");
            return Ok(Delivery::Disabled);
        }
        if status.is_client_error() {
            error!(%status, "Discord rejected the webhook message");
            return Ok(Delivery::Rejected);
        }
        // A proxy/server error may happen after Discord accepted the message.
        error!(%status, "Discord returned a server error with an uncertain delivery outcome");
        Ok(Delivery::Uncertain)
    }
}

fn series_title(series: SeriesMetadata<'_>) -> String {
    let title = match series.year.filter(|year| *year > 0) {
        Some(year) => format!("{} ({year})", series.title),
        None => series.title.to_owned(),
    };
    // Discord embed titles are limited to 256 UTF-16 code units.
    let mut units = 0;
    title
        .chars()
        .take_while(|character| {
            units += character.len_utf16();
            units <= 256
        })
        .collect()
}

fn imdb_series_url(id: &str) -> Option<String> {
    let digits = id.strip_prefix("tt")?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("https://www.imdb.com/title/{id}/"))
}
