//! Pure scheduling rules, independent of network and database state.
use std::collections::BTreeMap;
use tracing::{debug, instrument, trace};

use crate::{
    model::{NotificationMode, PlannedNotification},
    sonarr::{Episode, FinaleType, Series, SeriesStatus},
};

#[instrument(
    skip(series, episodes),
    fields(series_id = series.id, series.title = %series.title, episode_count = episodes.len(), tracking_since)
)]
pub fn plan(
    series: &Series,
    episodes: &[Episode],
    tracking_since: i64,
) -> Vec<PlannedNotification> {
    if !series.monitored {
        debug!("no notifications planned because the series is unmonitored");
        return Vec::new();
    }
    debug!("evaluating series notification plans");
    let mut plans = Vec::new();
    let mut seasons: BTreeMap<i64, Vec<&Episode>> = BTreeMap::new();
    for episode in episodes {
        if episode.series_id != series.id {
            trace!(
                episode_id = episode.id,
                episode_series_id = episode.series_id,
                "ignoring episode belonging to another series"
            );
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
            trace!(
                episode_id = episode.id,
                monitored = episode.monitored,
                has_file = episode.has_file,
                "episode does not need a missing-library notification"
            );
            continue;
        }
        let Some(date) = episode.air_date_utc else {
            trace!(episode_id = episode.id, "episode has no air date");
            continue;
        };
        if date.timestamp() < tracking_since {
            trace!(
                episode_id = episode.id,
                due_at = date.timestamp(),
                "episode predates the tracking cutoff"
            );
            continue;
        }
        trace!(
            episode_id = episode.id,
            due_at = date.timestamp(),
            "episode notification planned"
        );
        plans.push(PlannedNotification {
            awaiting_confirmation: false,
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
            trace!(season, "season has no missing monitored episodes");
            continue;
        }
        // Require every known episode, including unmonitored episodes, to have a date.
        let Some(dates) = members
            .iter()
            .map(|e| e.air_date_utc.map(|d| d.timestamp()))
            .collect::<Option<Vec<_>>>()
        else {
            trace!(season, "season has episodes with unknown air dates");
            continue;
        };
        let Some(due_at) = dates.into_iter().max() else {
            trace!(season, "season has no dated episodes");
            continue;
        };
        if due_at < tracking_since {
            trace!(season, due_at, "season predates the tracking cutoff");
            continue;
        }
        let mut numbers: Vec<_> = members.iter().map(|e| e.episode_number).collect();
        numbers.sort_unstable();
        if numbers
            .iter()
            .copied()
            .zip(1_i64..)
            .any(|(actual, expected)| actual != expected)
        {
            trace!(season, "season episode numbering is incomplete");
            continue;
        }
        let last_number = members.iter().map(|e| e.episode_number).max();
        let has_finale = members.iter().any(|e| {
            Some(e.episode_number) == last_number
                && matches!(e.finale_type, Some(FinaleType::Season | FinaleType::Series))
        });
        // Never infer a completed season merely from the last currently listed episode.
        let has_later_season = seasons.keys().any(|number| number > season);
        let awaiting_confirmation =
            !has_finale && !has_later_season && series.status != SeriesStatus::Ended;
        trace!(
            season,
            due_at,
            missing_count = missing.len(),
            has_finale,
            has_later_season,
            awaiting_confirmation,
            "season notification planned"
        );
        plans.push(PlannedNotification {
            awaiting_confirmation,
            key: format!("season:{}:{season}", series.id), series_id: series.id,
            mode: NotificationMode::Season, season: *season, episode_id: None, due_at,
            content: if awaiting_confirmation {
                truncate(format!("{} — Season {} is awaiting finale confirmation in Sonarr. The latest listed episode is not a confirmed season finale. No full-season alert will be sent until completion is confirmed.", series.title, season))
            } else { truncate(format!("{} — Season {} has reached its final scheduled air time. {} monitored episode(s) are missing from your library. Check for downloads.", series.title, season, missing.len())) },
            episode_ids: missing,
        });
    }
    debug!(
        plan_count = plans.len(),
        "series notification planning complete"
    );
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
