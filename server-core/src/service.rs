use std::{sync::Arc, time::Duration};

use chrono::Utc;
use tokio::sync::{Mutex, Notify, RwLock, watch};
use tracing::{debug, error, info, instrument, trace, warn};

use crate::{
    database::{Database, DeliveryState},
    discord::{Delivery, Discord, SeriesMetadata},
    planner,
    sonarr::{Series, Sonarr},
};

#[derive(Clone)]
pub struct Service {
    pub db: Database,
    pub sonarr: Sonarr,
    pub discord: Discord,
    pub wake: Arc<Notify>,
    /// Serialize policy changes with a delivery already in flight.
    pub delivery_gate: Arc<Mutex<()>>,
    pub health: Arc<RwLock<Health>>,
}

#[derive(Default, Clone, serde::Serialize)]
pub struct Health {
    pub last_scan_at: Option<i64>,
    pub last_scan_succeeded: bool,
    pub last_delivery_at: Option<i64>,
}

impl Service {
    pub fn new(db: Database, sonarr: Sonarr, discord: Discord) -> Self {
        Self {
            db,
            sonarr,
            discord,
            wake: Arc::new(Notify::new()),
            delivery_gate: Arc::new(Mutex::new(())),
            health: Arc::new(RwLock::new(Health::default())),
        }
    }

    /// Send the stored preview once without claiming, covering, or completing it.
    /// Test outcomes never change normal delivery state or worker health.
    #[instrument(skip(self), fields(notification.key = key))]
    pub async fn test_notification(&self, key: &str) -> anyhow::Result<Option<Delivery>> {
        debug!("looking up test notification");
        let _guard = self.delivery_gate.lock().await;
        let content: Option<(String, i64, String)> =
            sqlx::query_as("SELECT content, series_id, mode FROM notifications WHERE key = ?")
                .bind(key)
                .fetch_optional(self.db.pool())
                .await?;
        match content {
            Some((content, series_id, mode)) => {
                let series = match self.sonarr.series_by_id(series_id).await {
                    Ok(series) if series.id == series_id => Some(series),
                    Ok(_) => {
                        warn!(
                            series_id,
                            "test notification will be sent without series metadata because Sonarr returned the wrong series"
                        );
                        None
                    }
                    Err(error) => {
                        warn!(series_id, error = %error, "test notification will be sent without series metadata");
                        None
                    }
                };
                let poster = match self.sonarr.poster(series_id).await {
                    Ok(poster) => poster,
                    Err(error) => {
                        warn!(series_id, error = %error, "test notification will be sent without a poster");
                        None
                    }
                };
                let mode = crate::model::NotificationMode::try_from(mode.as_str())?;
                let color = self.db.embed_colors().await?.for_mode(mode);
                let delivery = self
                    .discord
                    .send_with_poster(
                        &content,
                        poster,
                        color,
                        series.as_ref().map(|series| SeriesMetadata {
                            title: &series.title,
                            year: series.year,
                            imdb_id: series.imdb_id.as_deref(),
                        }),
                    )
                    .await?;
                info!(series_id, mode = mode.as_str(), outcome = ?delivery, "test notification attempt completed");
                Ok(Some(delivery))
            }
            None => {
                debug!("test notification was not found");
                Ok(None)
            }
        }
    }

    #[instrument(skip(self))]
    pub async fn scan(&self) -> anyhow::Result<()> {
        info!("starting Sonarr scan");
        let series = self.sonarr.series().await?;
        debug!(series_count = series.len(), "loaded series from Sonarr");
        anyhow::ensure!(
            series.iter().all(|s| s.id > 0),
            "Sonarr returned invalid series identifiers"
        );
        self.db
            .sync_shows(
                &series
                    .iter()
                    .map(|s| (s.id, s.title.clone()))
                    .collect::<Vec<_>>(),
            )
            .await?;
        let excluded: std::collections::HashSet<i64> = self
            .db
            .shows()
            .await?
            .into_iter()
            .filter(|s| s.excluded)
            .map(|s| s.id)
            .collect();
        debug!(excluded_count = excluded.len(), "loaded exclusion policy");
        let mut failed = 0;
        for show in series {
            if excluded.contains(&show.id) {
                trace!(series_id = show.id, title = %show.title, "skipping excluded series");
                continue;
            }
            debug!(series_id = show.id, title = %show.title, "refreshing series");
            if let Err(error) = self.refresh(&show).await {
                error!(series_id = show.id, title = %show.title, error = %error, "Sonarr series refresh failed");
                failed += 1;
            }
            self.wake.notify_one();
        }
        self.wake.notify_one();
        anyhow::ensure!(failed == 0, "{failed} series could not be refreshed");
        info!("Sonarr scan completed successfully");
        Ok(())
    }

    #[instrument(skip(self, series), fields(series_id = series.id, series.title = %series.title))]
    async fn refresh(
        &self,
        series: &Series,
    ) -> anyhow::Result<Vec<crate::model::PlannedNotification>> {
        let episodes = self.sonarr.episodes(series.id).await?;
        debug!(episode_count = episodes.len(), "loaded episodes for series");
        let tracking_since = self.db.tracking_since().await?;
        let plans = planner::plan(series, &episodes, tracking_since);
        debug!(
            plan_count = plans.len(),
            tracking_since, "planned series notifications"
        );
        self.db.replace_plans(series.id, &plans).await?;
        debug!("persisted series notification plans");
        Ok(plans)
    }

    pub async fn scan_loop(&self, interval: Duration, mut shutdown: watch::Receiver<bool>) {
        info!(interval_seconds = interval.as_secs(), "scan worker started");
        let mut failures: u32 = 0;
        loop {
            if *shutdown.borrow() {
                info!("scan worker stopping");
                return;
            }
            let result = tokio::select! {
                result = self.scan() => result,
                _ = shutdown.changed() => return,
            };
            let succeeded = result.is_ok();
            if let Err(error) = result {
                error!(error = %error, "scan failed; will retry");
            }
            {
                let mut health = self.health.write().await;
                health.last_scan_at = Some(Utc::now().timestamp());
                health.last_scan_succeeded = succeeded;
            }
            failures = if succeeded {
                0
            } else {
                failures.saturating_add(1)
            };
            let delay = if succeeded {
                interval
            } else {
                Duration::from_secs((5_u64 << failures.min(8)).min(interval.as_secs()))
            };
            debug!(
                succeeded,
                failures,
                delay_seconds = delay.as_secs_f64(),
                "scan worker waiting"
            );
            tokio::select! {
                _ = tokio::time::sleep(delay) => trace!("scan timer elapsed"),
                _ = shutdown.changed() => {
                    info!("scan worker stopping");
                    return
                }
            }
        }
    }

    pub async fn delivery_loop(&self, mut shutdown: watch::Receiver<bool>) {
        info!("delivery worker started");
        loop {
            if *shutdown.borrow() {
                info!("delivery worker stopping");
                return;
            }
            let wait = match self.delivery_step().await {
                Ok(wait) => wait,
                Err(error) => {
                    error!(error = %error, "delivery worker step failed; will retry");
                    Duration::from_secs(30)
                }
            };
            trace!(wait_seconds = wait.as_secs_f64(), "delivery worker waiting");
            tokio::select! {
                _ = tokio::time::sleep(wait) => trace!("delivery timer elapsed"),
                _ = self.wake.notified() => trace!("delivery worker notified"),
                _ = shutdown.changed() => {
                    info!("delivery worker stopping");
                    return
                },
            }
        }
    }

    #[instrument(skip(self))]
    pub async fn delivery_step(&self) -> anyhow::Result<Duration> {
        trace!("checking for next due notification");
        let Some((key, series_id, due_at)) = self.db.next_due().await? else {
            trace!("no notification is ready or scheduled");
            return Ok(Duration::from_secs(30));
        };
        debug!(notification_key = %key, series_id, due_at, "found next notification candidate");
        let now = Utc::now();
        // Recheck wall time at least every 30 seconds to tolerate system clock corrections.
        let remaining_ms = due_at
            .saturating_mul(1000)
            .saturating_sub(now.timestamp_millis());
        if remaining_ms > 0 {
            trace!(notification_key = %key, remaining_ms, "notification is not due yet");
            return Ok(Duration::from_millis(remaining_ms.min(30000) as u64));
        }
        info!(notification_key = %key, series_id, "notification is due; verifying current Sonarr state");
        // Refresh immediately before notification: files and air dates may have changed.
        let refreshed = async {
            let series = self.sonarr.series_by_id(series_id).await?;
            anyhow::ensure!(series.id == series_id, "Sonarr returned the wrong series");
            let plans = self.refresh(&series).await?;
            Ok::<_, anyhow::Error>((series, plans))
        }
        .await;
        let (series, plans) = match refreshed {
            Ok(refreshed) => refreshed,
            Err(error) => {
                self.db.defer(&key, now.timestamp() + 60).await?;
                warn!(notification_key = %key, series_id, error = %error, "notification postponed because current series state could not be verified");
                return Ok(Duration::from_secs(1));
            }
        };
        let Some(plan) = plans.iter().find(|plan| plan.key == key) else {
            info!(notification_key = %key, series_id, "notification is no longer applicable after refresh");
            return Ok(Duration::ZERO);
        };
        let poster = match self.sonarr.poster(series_id).await {
            Ok(poster) => poster,
            Err(error) => {
                warn!(notification_key = %key, series_id, error = %error, "notification will be sent without a poster");
                None
            }
        };
        let _guard = self.delivery_gate.lock().await;
        let color = self.db.embed_colors().await?.for_mode(plan.mode);
        if !self.db.claim(plan, Utc::now().timestamp()).await? {
            info!(notification_key = %key, series_id, "notification claim was rejected because policy or coverage changed");
            return Ok(Duration::ZERO);
        }
        info!(notification_key = %key, series_id, mode = plan.mode.as_str(), has_poster = poster.is_some(), "sending Discord notification");
        let delivery = self
            .discord
            .send_with_poster(
                &plan.content,
                poster,
                color,
                Some(SeriesMetadata {
                    title: &series.title,
                    year: series.year,
                    imdb_id: series.imdb_id.as_deref(),
                }),
            )
            .await?;
        let finished = Utc::now().timestamp();
        match delivery {
            Delivery::Sent => {
                self.db.finish(&key, DeliveryState::Sent, finished).await?;
                self.health.write().await.last_delivery_at = Some(finished);
                info!(notification_key = %key, series_id, "Discord notification sent and recorded");
            }
            Delivery::RateLimited { retry_seconds } | Delivery::Retryable { retry_seconds } => {
                self.db
                    .rate_limited(&key, finished.saturating_add(retry_seconds as i64))
                    .await?;
                warn!(notification_key = %key, series_id, retry_seconds, outcome = ?delivery, "Discord notification will be retried");
            }
            Delivery::Disabled => {
                self.db.disable_webhook().await?;
                // Discord explicitly rejected this request, so it is safe to
                // retain it for delivery after the webhook has been repaired.
                self.db.rate_limited(&key, finished).await?;
                error!(notification_key = %key, series_id, "Discord webhook disabled after rejection; update configuration and resume through the API");
            }
            Delivery::Rejected => {
                self.db
                    .finish(&key, DeliveryState::Failed, finished)
                    .await?;
                error!(notification_key = %key, series_id, "Discord rejected notification");
            }
            Delivery::Uncertain => {
                self.db
                    .finish(&key, DeliveryState::Uncertain, finished)
                    .await?;
                error!(notification_key = %key, series_id, "Discord delivery outcome is uncertain; notification will not be resent automatically");
            }
        }
        Ok(Duration::from_millis(500))
    }
}
