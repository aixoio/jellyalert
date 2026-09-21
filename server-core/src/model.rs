use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationMode {
    #[default]
    Episode,
    Season,
}

impl NotificationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Episode => "episode",
            Self::Season => "season",
        }
    }
}

impl TryFrom<&str> for NotificationMode {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "episode" => Ok(Self::Episode),
            "season" => Ok(Self::Season),
            _ => anyhow::bail!("invalid notification mode in database"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Show {
    pub mode_overridden: bool,
    pub id: i64,
    pub title: String,
    pub excluded: bool,
    pub active: bool,
    pub mode: NotificationMode,
}

#[derive(Debug, Clone)]
pub struct PlannedNotification {
    pub awaiting_confirmation: bool,
    pub key: String,
    pub series_id: i64,
    pub mode: NotificationMode,
    pub season: i64,
    pub episode_id: Option<i64>,
    pub due_at: i64,
    pub content: String,
    pub episode_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedColors {
    pub episode_color: u32,
    pub season_color: u32,
}

impl EmbedColors {
    pub fn for_mode(self, mode: NotificationMode) -> u32 {
        match mode {
            NotificationMode::Episode => self.episode_color,
            NotificationMode::Season => self.season_color,
        }
    }
}
