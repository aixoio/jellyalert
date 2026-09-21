//! Small, read-only interface to Sonarr's v3 API (also used by Sonarr v4).
use std::time::Duration;

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::{debug, instrument, trace, warn};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub id: i64,
    pub title: String,
    pub monitored: bool,
    pub status: SeriesStatus,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SeriesStatus {
    Continuing,
    Ended,
    Upcoming,
    Deleted,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub id: i64,
    pub series_id: i64,
    pub season_number: i64,
    pub episode_number: i64,
    pub title: String,
    pub air_date_utc: Option<DateTime<Utc>>,
    pub has_file: bool,
    pub monitored: bool,
    #[serde(default)]
    pub finale_type: Option<FinaleType>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FinaleType {
    Season,
    Series,
    Midseason,
    #[serde(other)]
    Unknown,
}

#[derive(Clone)]
pub struct Sonarr {
    client: Client,
    base: Url,
}

impl Sonarr {
    pub fn new(base: &str, api_key: &str) -> anyhow::Result<Self> {
        let mut base = crate::config::validate_url(base, "Sonarr")?;
        let path = format!("{}/api/v3/", base.path().trim_end_matches('/'));
        base.set_path(&path);
        let mut headers = HeaderMap::new();
        let mut key = HeaderValue::from_str(api_key).context("invalid Sonarr API key header")?;
        key.set_sensitive(true);
        headers.insert("X-Api-Key", key);
        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { client, base })
    }

    /// Fetch only Sonarr's fixed JPEG poster path; never follow external artwork URLs.
    #[instrument(skip(self), fields(series_id = id))]
    pub async fn poster(&self, id: i64) -> anyhow::Result<Option<Vec<u8>>> {
        ensure!(id > 0, "invalid series identifier");
        debug!("requesting Sonarr poster");
        let response = self
            .client
            .get(self.base.join(&format!("mediacover/{id}/poster.jpg"))?)
            // Artwork is optional; leave time for Discord delivery within the UI timeout.
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| e.without_url())?;
        debug!(status = %response.status(), "Sonarr poster response received");
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            debug!("Sonarr poster is not available");
            return Ok(None);
        }
        let response = response.error_for_status().map_err(|e| e.without_url())?;
        let body = bounded_body(response, 5 * 1024 * 1024).await?;
        ensure!(body.starts_with(&[0xff, 0xd8, 0xff]), "invalid JPEG poster");
        debug!(bytes = body.len(), "Sonarr poster loaded");
        Ok(Some(body))
    }

    pub async fn series(&self) -> anyhow::Result<Vec<Series>> {
        let series: Vec<Series> = self.get("series", None).await?;
        debug!(count = series.len(), "Sonarr series loaded");
        Ok(series)
    }
    #[instrument(skip(self), fields(series_id = id))]
    pub async fn series_by_id(&self, id: i64) -> anyhow::Result<Series> {
        self.get(&format!("series/{id}"), None).await
    }
    #[instrument(skip(self), fields(series_id))]
    pub async fn episodes(&self, series_id: i64) -> anyhow::Result<Vec<Episode>> {
        let episodes: Vec<Episode> = self.get("episode", Some(series_id)).await?;
        ensure!(
            episodes.iter().all(|e| e.series_id == series_id
                && e.id > 0
                && e.season_number >= 0
                && e.episode_number > 0),
            "Sonarr returned invalid episode identifiers"
        );
        let mut ids = std::collections::HashSet::new();
        ensure!(
            episodes.iter().all(|e| ids.insert(e.id)),
            "Sonarr returned duplicate episode identifiers"
        );
        debug!(
            count = episodes.len(),
            "Sonarr episodes loaded and validated"
        );
        Ok(episodes)
    }

    #[instrument(skip(self), fields(sonarr.path = path, series_id = ?series_id))]
    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        series_id: Option<i64>,
    ) -> anyhow::Result<T> {
        let mut url = self.base.join(path)?;
        if let Some(id) = series_id {
            url.query_pairs_mut()
                .append_pair("seriesId", &id.to_string());
        }
        debug!("sending Sonarr API request");
        let response = match self.client.get(url).send().await {
            Ok(response) => response,
            Err(error) => {
                let error = error.without_url();
                warn!(error = %error, "Sonarr API request failed");
                return Err(error.into());
            }
        };
        debug!(status = %response.status(), "Sonarr API response received");
        let response = response.error_for_status().map_err(|e| e.without_url())?;
        let body = bounded_body(response, 32 * 1024 * 1024).await?;
        debug!(bytes = body.len(), "decoding Sonarr API response");
        serde_json::from_slice(&body).context("invalid Sonarr API response")
    }
}

pub(crate) async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
        trace!(
            chunk_bytes = chunk.len(),
            accumulated_bytes = body.len(),
            limit,
            "reading HTTP response body chunk"
        );
        ensure!(
            body.len().saturating_add(chunk.len()) <= limit,
            "HTTP response exceeds size limit"
        );
        body.extend_from_slice(&chunk);
    }
    trace!(
        bytes = body.len(),
        limit, "HTTP response body read complete"
    );
    Ok(body)
}
