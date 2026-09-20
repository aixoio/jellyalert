use std::{net::SocketAddr, path::Path};

use anyhow::{Context, ensure};
use serde::Deserialize;

// Deliberately no Debug/Serialize: configuration contains credentials.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub discord_webhook_url: String,
    pub sqlite_database_path: String,
    pub sonarr_url: String,
    pub sonarr_api_key: String,
    // Accept old configuration files without requiring or using their token.
    #[serde(default, rename = "api_token")]
    pub _legacy_api_token: Option<String>,
    #[serde(default = "default_listen")]
    pub listen_address: SocketAddr,
    #[serde(default = "default_poll")]
    pub scan_interval_seconds: u64,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8090))
}
fn default_poll() -> u64 {
    300
}

impl ServerConfig {
    pub fn read_from_path(path: &Path) -> anyhow::Result<Self> {
        let file_contents = std::fs::read_to_string(path).context("cannot read configuration")?;
        let config: Self = toml::from_str(&file_contents).map_err(|_| {
            anyhow::anyhow!("invalid TOML configuration; check required fields and types")
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.sqlite_database_path.trim().is_empty(),
            "database path is required"
        );
        ensure!(
            !self.sonarr_api_key.trim().is_empty(),
            "Sonarr API key is required"
        );
        ensure!(
            (10..=86400).contains(&self.scan_interval_seconds),
            "scan interval must be between 10 and 86400 seconds"
        );
        validate_url(&self.sonarr_url, "Sonarr")?;
        let webhook = validate_url(&self.discord_webhook_url, "Discord webhook")?;
        ensure!(
            webhook.scheme() == "https",
            "Discord webhook must use HTTPS"
        );
        ensure!(
            matches!(
                webhook.host_str(),
                Some("discord.com" | "canary.discord.com" | "ptb.discord.com")
            ) && webhook.path().starts_with("/api/webhooks/"),
            "invalid Discord webhook host or path"
        );
        Ok(())
    }
}

pub fn validate_url(value: &str, name: &str) -> anyhow::Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).map_err(|_| anyhow::anyhow!("invalid {name} URL"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "{name} requires an HTTP(S) URL"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.query().is_none(),
        "{name} URL must not contain credentials, query, or fragment"
    );
    Ok(url)
}
