//! Pure scheduling rules, independent of network and database state.
use std::collections::BTreeMap;

use crate::{
    model::{NotificationMode, PlannedNotification},
    sonarr::{Episode, FinaleType, Series, SeriesStatus},
};

pub fn plan(
    series: &Series,
    episodes: &[Episode],
    tracking_since: i64,
) -> Vec<PlannedNotification> {
    if !series.monitored {
        return Vec::new();
    }
    let mut plans = Vec::new();
    let mut seasons: BTreeMap<i64, Vec<&Episode>> = BTreeMap::new();
    for episode in episodes {
        if episode.series_id != series.id {
            continue;
        }
        // Specials have no reliable season completion boundary.
        if episode.season_number > 0 {
            seasons
                .entry(episode.season_number)
                .or_default()
                .push(episode);
        }
        if !episode.monitored || episode.has_file {
            continue;
        }
        let Some(date) = episode.air_date_utc else {
            continue;
        };
        if date.timestamp() < tracking_since {
            continue;
        }
        plans.push(PlannedNotification {
            key: format!("episode:{}", episode.id), series_id: series.id,
            mode: NotificationMode::Episode, season: episode.season_number,
            episode_id: Some(episode.id), due_at: date.timestamp(),
            content: truncate(format!("{} — S{:02}E{:02}: {} has reached its scheduled air time and is missing from your library. Check for a download.", series.title, episode.season_number, episode.episode_number, episode.title)),
            episode_ids: vec![episode.id],
        });
    }
    for (season, members) in &seasons {
        let missing: Vec<i64> = members
            .iter()
            .filter(|e| e.monitored && !e.has_file)
            .map(|e| e.id)
            .collect();
        if missing.is_empty() {
            continue;
        }
        // Require every known episode, including unmonitored episodes, to have a date.
        let Some(dates) = members
            .iter()
            .map(|e| e.air_date_utc.map(|d| d.timestamp()))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let Some(due_at) = dates.into_iter().max() else {
            continue;
        };
        if due_at < tracking_since {
            continue;
        }
        let last_number = members.iter().map(|e| e.episode_number).max();
        let has_finale = members.iter().any(|e| {
            Some(e.episode_number) == last_number
                && matches!(e.finale_type, Some(FinaleType::Season | FinaleType::Series))
        });
        // Never infer a completed season merely from the last currently listed episode.
        let has_later_season = seasons.keys().any(|number| number > season);
        if !has_finale && !has_later_season && series.status != SeriesStatus::Ended {
            continue;
        }
        plans.push(PlannedNotification {
            key: format!("season:{}:{season}", series.id), series_id: series.id,
            mode: NotificationMode::Season, season: *season, episode_id: None, due_at,
            content: truncate(format!("{} — Season {} has reached its final scheduled air time. {} monitored episode(s) are missing from your library. Check for downloads.", series.title, season, missing.len())),
            episode_ids: missing,
        });
    }
    plans
}

fn truncate(content: String) -> String {
    // Discord counts UTF-16 code units toward its 2000-character limit.
    let mut units = 0;
    content
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 1900
        })
        .collect()
}
