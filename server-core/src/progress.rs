//! Read-only progress projections. Notification candidates use the delivery planner.
use crate::{
    model::{NotificationMode, Show},
    planner,
    sonarr::{Episode, FinaleType, Series, SeriesStatus},
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default, Serialize)]
pub struct Counts {
    pub total: usize,
    pub aired: usize,
    pub in_library: usize,
    pub undated: usize,
}
#[derive(Serialize)]
pub struct EpisodeProgress {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub air_at: Option<i64>,
    pub has_file: bool,
    pub monitored: bool,
    pub notified: bool,
}
#[derive(Serialize)]
pub struct SeasonProgress {
    pub number: i64,
    pub counts: Counts,
    pub completion_confirmed: bool,
    pub final_air_at: Option<i64>,
    pub episodes: Vec<EpisodeProgress>,
    pub notification: Option<NextNotification>,
}
#[derive(Clone, Serialize)]
pub struct NextNotification {
    pub season: i64,
    pub episode: Option<i64>,
    pub due_at: i64,
    pub awaiting_confirmation: bool,
    pub episodes_remaining: usize,
}
#[derive(Serialize)]
pub struct NextRelease {
    pub season: i64,
    pub episode: i64,
    pub title: String,
    pub air_at: i64,
}
#[derive(Serialize)]
pub struct Progress {
    pub show: Show,
    pub status: SeriesStatus,
    pub monitored: bool,
    pub as_of: i64,
    pub counts: Counts,
    pub seasons: Vec<SeasonProgress>,
    pub next_release: Option<NextRelease>,
    pub next_notification: Option<NextNotification>,
    pub notification_block: Option<String>,
    pub tracking_since: i64,
}

pub fn build(
    show: Show,
    series: &Series,
    episodes: &[Episode],
    tracking_since: i64,
    now: i64,
    covered: &HashSet<i64>,
    deliveries: &HashMap<String, String>,
    webhook_disabled: bool,
    webhook_retry_at: i64,
) -> Progress {
    let mut grouped: BTreeMap<i64, Vec<&Episode>> = BTreeMap::new();
    for e in episodes.iter().filter(|e| e.series_id == show.id) {
        grouped.entry(e.season_number).or_default().push(e);
    }
    let notification_block = if !show.active {
        Some("This show has been removed from Sonarr.")
    } else if show.excluded {
        Some("Notifications are excluded for this show.")
    } else if !series.monitored {
        Some("This series is not monitored in Sonarr.")
    } else if webhook_disabled {
        Some("Discord delivery is paused. Resume it from Overview.")
    } else {
        None
    }
    .map(str::to_owned);
    let candidates: Vec<_> = planner::plan(series, episodes, tracking_since)
        .into_iter()
        .filter(|p| {
            p.mode == show.mode
                && !deliveries
                    .get(&p.key)
                    .is_some_and(|state| state != "pending")
                && p.episode_ids.iter().any(|id| !covered.contains(id))
        })
        .map(|p| {
            let episode = p
                .episode_id
                .and_then(|id| episodes.iter().find(|e| e.id == id))
                .map(|e| e.episode_number);
            let episodes_remaining = episodes
                .iter()
                .filter(|e| {
                    e.series_id == show.id
                        && e.season_number == p.season
                        && (p.mode == NotificationMode::Season || Some(e.id) == p.episode_id)
                        && e.air_date_utc.is_none_or(|d| d.timestamp() > now)
                })
                .count();
            NextNotification {
                season: p.season,
                episode,
                due_at: p.due_at.max(webhook_retry_at),
                awaiting_confirmation: p.awaiting_confirmation,
                episodes_remaining,
            }
        })
        .collect();
    let mut counts = Counts::default();
    let seasons = grouped
        .iter()
        .map(|(&number, members)| {
            let mut ordered = members.clone();
            ordered.sort_by_key(|e| (e.episode_number, e.id));
            let consecutive = ordered
                .iter()
                .zip(1_i64..)
                .all(|(e, n)| e.episode_number == n);
            let evidence = ordered.last().is_some_and(|e| {
                matches!(e.finale_type, Some(FinaleType::Season | FinaleType::Series))
            }) || grouped.keys().any(|n| *n > number)
                || series.status == SeriesStatus::Ended;
            let completion_confirmed = number > 0
                && consecutive
                && evidence
                && ordered.iter().all(|e| e.air_date_utc.is_some());
            let season_counts = Counts {
                total: ordered.len(),
                aired: ordered
                    .iter()
                    .filter(|e| e.air_date_utc.is_some_and(|d| d.timestamp() <= now))
                    .count(),
                in_library: ordered.iter().filter(|e| e.has_file).count(),
                undated: ordered.iter().filter(|e| e.air_date_utc.is_none()).count(),
            };
            if number > 0 {
                counts.total += season_counts.total;
                counts.aired += season_counts.aired;
                counts.in_library += season_counts.in_library;
                counts.undated += season_counts.undated;
            }
            SeasonProgress {
                number,
                counts: season_counts,
                completion_confirmed,
                final_air_at: if completion_confirmed {
                    ordered
                        .iter()
                        .filter_map(|e| e.air_date_utc.map(|d| d.timestamp()))
                        .max()
                } else {
                    None
                },
                episodes: ordered
                    .iter()
                    .map(|e| EpisodeProgress {
                        id: e.id,
                        number: e.episode_number,
                        title: e.title.clone(),
                        air_at: e.air_date_utc.map(|d| d.timestamp()),
                        has_file: e.has_file,
                        monitored: e.monitored,
                        notified: covered.contains(&e.id),
                    })
                    .collect(),
                notification: candidates
                    .iter()
                    .filter(|p| p.season == number)
                    .min_by_key(|p| p.due_at)
                    .cloned(),
            }
        })
        .collect();
    let next_release = episodes
        .iter()
        .filter(|e| e.series_id == show.id)
        .filter_map(|e| e.air_date_utc.map(|d| (e, d.timestamp())))
        .filter(|(_, at)| *at > now)
        .min_by_key(|(e, at)| (*at, e.id))
        .map(|(e, air_at)| NextRelease {
            season: e.season_number,
            episode: e.episode_number,
            title: e.title.clone(),
            air_at,
        });
    let next_notification = candidates
        .into_iter()
        .min_by_key(|p| (p.awaiting_confirmation, p.due_at));
    Progress {
        show,
        status: series.status,
        monitored: series.monitored,
        as_of: now,
        counts,
        seasons,
        next_release,
        next_notification,
        notification_block,
        tracking_since,
    }
}
