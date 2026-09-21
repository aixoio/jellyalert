use std::{net::SocketAddr, path::Path};

use anyhow::{Context, ensure};
use serde::Deserialize;
use tracing::{debug, instrument};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

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
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8090))
}
fn default_poll() -> u64 {
    300
}
fn default_log_level() -> LogLevel {
    LogLevel::Debug
}

impl ServerConfig {
    #[instrument(skip(path), fields(config_path = %path.display()))]
    pub fn read_from_path(path: &Path) -> anyhow::Result<Self> {
        debug!("reading server configuration");
        let file_contents = std::fs::read_to_string(path).context("cannot read configuration")?;
        let config: Self = toml::from_str(&file_contents).map_err(|_| {
            anyhow::anyhow!("invalid TOML configuration; check required fields and types")
        })?;
        config.validate()?;
        debug!("server configuration parsed and validated");
        Ok(config)
    }

    #[instrument(skip(self))]
    pub fn validate(&self) -> anyhow::Result<()> {
        debug!("validating server configuration");
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
        debug!("server configuration checks passed");
        Ok(())
    }
}

#[instrument(skip(value), fields(service = name))]
pub fn validate_url(value: &str, name: &str) -> anyhow::Result<reqwest::Url> {
    debug!("validating service URL");
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
    debug!(scheme = url.scheme(), host = ?url.host_str(), "service URL checks passed");
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, ServerConfig};

    const REQUIRED_CONFIG: &str = r#"
discord_webhook_url = "https://discord.com/api/webhooks/id/token"
sqlite_database_path = "jelly-alert.db"
sonarr_url = "http://127.0.0.1:8989"
sonarr_api_key = "key"
"#;

    #[test]
    fn log_level_defaults_to_debug() {
        let config: ServerConfig = toml::from_str(REQUIRED_CONFIG).unwrap();
        assert_eq!(config.log_level.as_str(), LogLevel::Debug.as_str());
    }

    #[test]
    fn log_level_accepts_every_documented_value() {
        for level in ["off", "error", "warn", "info", "debug", "trace"] {
            let config: ServerConfig =
                toml::from_str(&format!("{REQUIRED_CONFIG}\nlog_level = \"{level}\"\n")).unwrap();
            assert_eq!(config.log_level.as_str(), level);
        }
    }

    #[test]
    fn log_level_rejects_unknown_values() {
        let result = toml::from_str::<ServerConfig>(&format!(
            "{REQUIRED_CONFIG}\nlog_level = \"verbose\"\n"
        ));
        assert!(result.is_err());
    }
}
