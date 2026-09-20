//! Small, read-only interface to Sonarr's v3 API (also used by Sonarr v4).
use std::time::Duration;

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderValue},
};
use serde::{Deserialize, de::DeserializeOwned};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub id: i64,
    pub title: String,
    pub monitored: bool,
    pub status: SeriesStatus,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
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
    pub async fn poster(&self, id: i64) -> anyhow::Result<Option<Vec<u8>>> {
        ensure!(id > 0, "invalid series identifier");
        let response = self
            .client
            .get(self.base.join(&format!("mediacover/{id}/poster.jpg"))?)
            .send()
            .await
            .map_err(|e| e.without_url())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status().map_err(|e| e.without_url())?;
        let body = bounded_body(response, 5 * 1024 * 1024).await?;
        ensure!(body.starts_with(&[0xff, 0xd8, 0xff]), "invalid JPEG poster");
        Ok(Some(body))
    }

    pub async fn series(&self) -> anyhow::Result<Vec<Series>> {
        self.get("series", None).await
    }
    pub async fn series_by_id(&self, id: i64) -> anyhow::Result<Series> {
        self.get(&format!("series/{id}"), None).await
    }
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
        Ok(episodes)
    }

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
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.without_url())?
            .error_for_status()
            .map_err(|e| e.without_url())?;
        let body = bounded_body(response, 32 * 1024 * 1024).await?;
        serde_json::from_slice(&body).context("invalid Sonarr API response")
    }
}

pub(crate) async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
        ensure!(
            body.len().saturating_add(chunk.len()) <= limit,
            "HTTP response exceeds size limit"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
