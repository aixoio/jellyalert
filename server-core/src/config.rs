use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ServerConfig {
    pub discord_webhook_url: String,
    pub sqlite_database_path: String,
}

impl ServerConfig {
    pub fn read_from_path(path: &PathBuf) -> anyhow::Result<ServerConfig> {
        let file_contents = fs::read_to_string(path)?;
        let server_config = toml::from_str(&file_contents)?;

        Ok(server_config)
    }
}
