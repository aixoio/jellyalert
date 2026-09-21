use std::{sync::Arc, time::Duration};

use chrono::Utc;
use tokio::sync::{Mutex, Notify, RwLock, watch};

use crate::{
    database::{Database, DeliveryState},
    discord::{Delivery, Discord},
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
    pub async fn test_notification(&self, key: &str) -> anyhow::Result<Option<Delivery>> {
        let _guard = self.delivery_gate.lock().await;
        let content: Option<(String, i64)> =
            sqlx::query_as("SELECT content, series_id FROM notifications WHERE key = ?")
                .bind(key)
                .fetch_optional(self.db.pool())
                .await?;
        match content {
            Some((content, series_id)) => {
                let poster = self.sonarr.poster(series_id).await.ok().flatten();
                Ok(Some(self.discord.send_with_poster(&content, poster).await?))
            }
            None => Ok(None),
        }
    }

    pub async fn scan(&self) -> anyhow::Result<()> {
        let series = self.sonarr.series().await?;
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
        let mut failed = 0;
        for show in series {
            if excluded.contains(&show.id) {
                continue;
            }
            if let Err(error) = self.refresh(&show).await {
                eprintln!("Sonarr refresh failed for series {}: {error}", show.id);
                failed += 1;
            }
            self.wake.notify_one();
        }
        self.wake.notify_one();
        anyhow::ensure!(failed == 0, "{failed} series could not be refreshed");
        Ok(())
    }

    async fn refresh(
        &self,
        series: &Series,
    ) -> anyhow::Result<Vec<crate::model::PlannedNotification>> {
        let episodes = self.sonarr.episodes(series.id).await?;
        let plans = planner::plan(series, &episodes, self.db.tracking_since().await?);
        self.db.replace_plans(series.id, &plans).await?;
        Ok(plans)
    }

    pub async fn scan_loop(&self, interval: Duration, mut shutdown: watch::Receiver<bool>) {
        let mut failures: u32 = 0;
        loop {
            if *shutdown.borrow() {
                return;
            }
            let result = tokio::select! {
                result = self.scan() => result,
                _ = shutdown.changed() => return,
            };
            let succeeded = result.is_ok();
            if let Err(error) = result {
                eprintln!("Scan failed; will retry: {error}");
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
            tokio::select! { _ = tokio::time::sleep(delay) => {}, _ = shutdown.changed() => return }
        }
    }

    pub async fn delivery_loop(&self, mut shutdown: watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                return;
            }
            let wait = match self.delivery_step().await {
                Ok(wait) => wait,
                Err(error) => {
                    eprintln!("Delivery worker failed; will retry: {error}");
                    Duration::from_secs(30)
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {},
                _ = self.wake.notified() => {},
                _ = shutdown.changed() => return,
            }
        }
    }

    pub async fn delivery_step(&self) -> anyhow::Result<Duration> {
        let Some((key, series_id, due_at)) = self.db.next_due().await? else {
            return Ok(Duration::from_secs(30));
        };
        let now = Utc::now();
        // Recheck wall time at least every 30 seconds to tolerate system clock corrections.
        let remaining_ms = due_at
            .saturating_mul(1000)
            .saturating_sub(now.timestamp_millis());
        if remaining_ms > 0 {
            return Ok(Duration::from_millis(remaining_ms.min(30000) as u64));
        }
        // Refresh immediately before notification: files and air dates may have changed.
        let refreshed = async {
            let series = self.sonarr.series_by_id(series_id).await?;
            anyhow::ensure!(series.id == series_id, "Sonarr returned the wrong series");
            self.refresh(&series).await
        }
        .await;
        let plans = match refreshed {
            Ok(plans) => plans,
            Err(error) => {
                self.db.defer(&key, now.timestamp() + 60).await?;
                eprintln!("Notification postponed: cannot verify series {series_id}: {error}");
                return Ok(Duration::from_secs(1));
            }
        };
        let Some(plan) = plans.iter().find(|plan| plan.key == key) else {
            return Ok(Duration::ZERO);
        };
        let poster = self.sonarr.poster(series_id).await.ok().flatten();
        let _guard = self.delivery_gate.lock().await;
        if !self.db.claim(plan, Utc::now().timestamp()).await? {
            return Ok(Duration::ZERO);
        }
        let delivery = self.discord.send_with_poster(&plan.content, poster).await?;
        let finished = Utc::now().timestamp();
        match delivery {
            Delivery::Sent => {
                self.db.finish(&key, DeliveryState::Sent, finished).await?;
                self.health.write().await.last_delivery_at = Some(finished);
            }
            Delivery::RateLimited { retry_seconds } | Delivery::Retryable { retry_seconds } => {
                self.db
                    .rate_limited(&key, finished.saturating_add(retry_seconds as i64))
                    .await?
            }
            Delivery::Disabled => {
                self.db.disable_webhook().await?;
                // Discord explicitly rejected this request, so it is safe to
                // retain it for delivery after the webhook has been repaired.
                self.db.rate_limited(&key, finished).await?;
                eprintln!(
                    "Discord webhook disabled after rejection; update configuration and resume through the API"
                );
            }
            Delivery::Rejected => {
                self.db
                    .finish(&key, DeliveryState::Failed, finished)
                    .await?;
                eprintln!("Discord rejected notification {key}");
            }
            Delivery::Uncertain => {
                self.db
                    .finish(&key, DeliveryState::Uncertain, finished)
                    .await?;
                eprintln!(
                    "Delivery outcome uncertain for {key}; it will not be resent automatically"
                );
            }
        }
        Ok(Duration::from_millis(500))
    }
}
