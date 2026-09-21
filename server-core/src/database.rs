use std::time::Duration;

use sqlx::{
    Row, SqlSafeStr, SqlitePool,
    migrate::{Migration, MigrationType, Migrator},
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use tracing::{debug, info, instrument, trace};

use crate::model::{EmbedColors, NotificationMode, PlannedNotification, Show};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    #[instrument(skip(database_path))]
    pub async fn connect(database_path: &str) -> anyhow::Result<Self> {
        info!("connecting to SQLite database");
        let options = SqliteConnectOptions::new()
            .filename(database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await?;
        let db = Self { pool };
        db.migrate().await?;
        info!("SQLite database connected and migrated");
        Ok(db)
    }

    #[instrument(skip(self))]
    async fn migrate(&self) -> anyhow::Result<()> {
        debug!("checking database migrations");
        // Embed SQL, not generated Rust: the SQLx migrator still validates checksums.
        Migrator::with_migrations(vec![
            Migration::new(
                202609200001,
                "initial".into(),
                MigrationType::Simple,
                include_str!("../migrations/202609200001_initial.sql").into_sql_str(),
                false,
            ),
            Migration::new(
                202609200002,
                "per show mode".into(),
                MigrationType::Simple,
                include_str!("../migrations/202609200002_per_show_mode.sql").into_sql_str(),
                false,
            ),
            Migration::new(
                202609210001,
                "default mode".into(),
                MigrationType::Simple,
                include_str!("../migrations/202609210001_default_mode.sql").into_sql_str(),
                false,
            ),
            Migration::new(
                202609210002,
                "season confirmation".into(),
                MigrationType::Simple,
                include_str!("../migrations/202609210002_season_confirmation.sql").into_sql_str(),
                false,
            ),
            Migration::new(
                202609210003,
                "embed colors".into(),
                MigrationType::Simple,
                include_str!("../migrations/202609210003_embed_colors.sql").into_sql_str(),
                false,
            ),
        ])
        .run(&self.pool)
        .await?;
        sqlx::query("INSERT OR IGNORE INTO settings (id, tracking_since) VALUES (1, unixepoch())")
            .execute(&self.pool)
            .await?;
        debug!("database migrations and settings row verified");
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn set_show_mode(&self, id: i64, mode: NotificationMode) -> anyhow::Result<bool> {
        let found = sqlx::query("UPDATE shows SET mode = ?, mode_overridden = 1 WHERE id = ?")
            .bind(mode.as_str())
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1;
        info!(
            series_id = id,
            mode = mode.as_str(),
            found,
            "database show mode updated"
        );
        Ok(found)
    }

    pub async fn embed_colors(&self) -> anyhow::Result<EmbedColors> {
        let (episode_color, season_color): (u32, u32) =
            sqlx::query_as("SELECT episode_color, season_color FROM settings WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        trace!(episode_color, season_color, "database embed colors loaded");
        Ok(EmbedColors {
            episode_color,
            season_color,
        })
    }

    pub async fn set_embed_colors(&self, colors: EmbedColors) -> anyhow::Result<()> {
        anyhow::ensure!(
            colors.episode_color <= 0xffffff && colors.season_color <= 0xffffff,
            "invalid embed color"
        );
        sqlx::query("UPDATE settings SET episode_color = ?, season_color = ? WHERE id = 1")
            .bind(colors.episode_color)
            .bind(colors.season_color)
            .execute(&self.pool)
            .await?;
        info!(
            episode_color = colors.episode_color,
            season_color = colors.season_color,
            "database embed colors updated"
        );
        Ok(())
    }

    pub async fn default_mode(&self) -> anyhow::Result<NotificationMode> {
        let mode: String = sqlx::query_scalar("SELECT default_mode FROM settings WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        trace!(%mode, "database default notification mode loaded");
        NotificationMode::try_from(mode.as_str())
    }

    pub async fn set_default_mode(&self, mode: NotificationMode) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE settings SET default_mode = ? WHERE id = 1")
            .bind(mode.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE shows SET mode = ? WHERE mode_overridden = 0")
            .bind(mode.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        info!(
            mode = mode.as_str(),
            "database default notification mode updated"
        );
        Ok(())
    }

    pub async fn reset_all_show_modes(&self, mode: NotificationMode) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE settings SET default_mode = ? WHERE id = 1")
            .bind(mode.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE shows SET mode = ?, mode_overridden = 0")
            .bind(mode.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        info!(
            mode = mode.as_str(),
            "database default and all show modes reset"
        );
        Ok(())
    }

    pub async fn reset_show_mode(&self, id: i64) -> anyhow::Result<bool> {
        let found = sqlx::query("UPDATE shows SET mode = (SELECT default_mode FROM settings WHERE id = 1), mode_overridden = 0 WHERE id = ?")
            .bind(id).execute(&self.pool).await?.rows_affected() == 1;
        info!(series_id = id, found, "database show mode reset to default");
        Ok(found)
    }

    pub async fn tracking_since(&self) -> anyhow::Result<i64> {
        let tracking_since = sqlx::query_scalar("SELECT tracking_since FROM settings WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        trace!(tracking_since, "database tracking cutoff loaded");
        Ok(tracking_since)
    }

    pub async fn shows(&self) -> anyhow::Result<Vec<Show>> {
        let shows = sqlx::query("SELECT id, title, excluded, active, mode, mode_overridden FROM shows ORDER BY title, id")
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                Ok(Show {
                    id: row.try_get("id")?,
                    title: row.try_get("title")?,
                    excluded: row.try_get("excluded")?,
                    active: row.try_get("active")?,
                    mode_overridden: row.try_get("mode_overridden")?,
                    mode: NotificationMode::try_from(row.try_get::<&str, _>("mode")?)?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        trace!(count = shows.len(), "database shows loaded");
        Ok(shows)
    }

    pub async fn set_excluded(&self, id: i64, excluded: bool) -> anyhow::Result<bool> {
        let found = sqlx::query("UPDATE shows SET excluded = ? WHERE id = ?")
            .bind(excluded)
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1;
        info!(
            series_id = id,
            excluded, found, "database show exclusion updated"
        );
        Ok(found)
    }

    pub async fn sync_shows(&self, shows: &[(i64, String)]) -> anyhow::Result<()> {
        debug!(
            count = shows.len(),
            "synchronizing Sonarr shows to database"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE shows SET active = 0")
            .execute(&mut *tx)
            .await?;
        for (id, title) in shows {
            sqlx::query("INSERT INTO shows (id, title, mode) VALUES (?, ?, (SELECT default_mode FROM settings WHERE id = 1)) ON CONFLICT(id) DO UPDATE SET title = excluded.title, active = 1")
                .bind(id).bind(title).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        info!(count = shows.len(), "database shows synchronized");
        Ok(())
    }

    pub async fn replace_plans(
        &self,
        series_id: i64,
        plans: &[PlannedNotification],
    ) -> anyhow::Result<()> {
        debug!(
            series_id,
            plan_count = plans.len(),
            "replacing pending notification plans"
        );
        let mut tx = self.pool.begin().await?;
        // A refresh must not bypass a backoff after a failed Sonarr verification.
        let retries: std::collections::HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
            "SELECT key, retry_at FROM notifications WHERE series_id = ? AND state = 'pending'",
        )
        .bind(series_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        sqlx::query("DELETE FROM notifications WHERE series_id = ? AND state = 'pending'")
            .bind(series_id)
            .execute(&mut *tx)
            .await?;
        for plan in plans {
            sqlx::query("INSERT OR IGNORE INTO notifications (key, series_id, mode, season, episode_id, due_at, content, retry_at, awaiting_confirmation) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&plan.key).bind(series_id).bind(plan.mode.as_str()).bind(plan.season)
                .bind(plan.episode_id).bind(plan.due_at).bind(&plan.content)
                .bind(retries.get(&plan.key).copied().unwrap_or(0)).bind(plan.awaiting_confirmation).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        info!(
            series_id,
            plan_count = plans.len(),
            "database notification plans replaced"
        );
        Ok(())
    }

    pub async fn next_due(&self) -> anyhow::Result<Option<(String, i64, i64)>> {
        let next = sqlx::query_as("SELECT n.key, n.series_id, max(n.due_at, n.retry_at, s.webhook_retry_at) FROM notifications n JOIN shows sh ON sh.id = n.series_id JOIN settings s ON s.id = 1 WHERE n.state = 'pending' AND n.awaiting_confirmation = 0 AND n.mode = sh.mode AND sh.excluded = 0 AND sh.active = 1 AND s.webhook_disabled = 0 AND (n.episode_id IS NULL OR NOT EXISTS (SELECT 1 FROM covered_episodes c WHERE c.episode_id = n.episode_id)) ORDER BY max(n.due_at, n.retry_at, s.webhook_retry_at), n.key LIMIT 1")
            .fetch_optional(&self.pool).await?;
        trace!(next = ?next, "database next due notification checked");
        Ok(next)
    }

    /// Reserve before sending: never automatically resend an ambiguous delivery.
    pub async fn claim(&self, plan: &PlannedNotification, now: i64) -> anyhow::Result<bool> {
        debug!(notification_key = %plan.key, series_id = plan.series_id, now, "attempting database notification claim");
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("UPDATE notifications SET state = 'sending', attempted_at = ? WHERE key = ? AND state = 'pending' AND awaiting_confirmation = 0 AND due_at <= ? AND retry_at <= ? AND EXISTS (SELECT 1 FROM settings WHERE id = 1 AND webhook_disabled = 0 AND webhook_retry_at <= ?) AND EXISTS (SELECT 1 FROM shows WHERE id = notifications.series_id AND mode = notifications.mode AND excluded = 0 AND active = 1)")
            .bind(now).bind(&plan.key).bind(now).bind(now).bind(now).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            debug!(notification_key = %plan.key, "database notification claim rejected");
            return Ok(false);
        }
        let mut uncovered = false;
        for id in &plan.episode_ids {
            let changed = sqlx::query("INSERT OR IGNORE INTO covered_episodes (episode_id, notification_key) VALUES (?, ?)")
                .bind(id).bind(&plan.key).execute(&mut *tx).await?.rows_affected();
            uncovered |= changed > 0;
            trace!(notification_key = %plan.key, episode_id = id, newly_covered = changed > 0, "checked episode notification coverage");
        }
        if !uncovered {
            sqlx::query("UPDATE notifications SET state = 'covered' WHERE key = ?")
                .bind(&plan.key)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            info!(notification_key = %plan.key, "notification marked covered without delivery");
            return Ok(false);
        }
        tx.commit().await?;
        info!(notification_key = %plan.key, "database notification claimed for delivery");
        Ok(true)
    }

    pub async fn finish(&self, key: &str, state: DeliveryState, now: i64) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE notifications SET state = ?, sent_at = ? WHERE key = ? AND state = 'sending'",
        )
        .bind(state.as_str())
        .bind(if state == DeliveryState::Sent {
            Some(now)
        } else {
            None
        })
        .bind(key)
        .execute(&self.pool)
        .await?;
        info!(
            notification_key = key,
            state = state.as_str(),
            now,
            "database notification attempt finalized"
        );
        Ok(())
    }

    pub async fn rate_limited(&self, key: &str, retry_at: i64) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM covered_episodes WHERE notification_key = ?")
            .bind(key)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE notifications SET state = 'pending', attempted_at = NULL WHERE key = ?",
        )
        .bind(key)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE settings SET webhook_retry_at = ? WHERE id = 1")
            .bind(retry_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        info!(
            notification_key = key,
            retry_at, "database notification released for retry"
        );
        Ok(())
    }

    pub async fn defer(&self, key: &str, retry_at: i64) -> anyhow::Result<()> {
        sqlx::query("UPDATE notifications SET retry_at = ? WHERE key = ? AND state = 'pending'")
            .bind(retry_at)
            .bind(key)
            .execute(&self.pool)
            .await?;
        info!(
            notification_key = key,
            retry_at, "database notification deferred"
        );
        Ok(())
    }

    pub async fn disable_webhook(&self) -> anyhow::Result<()> {
        sqlx::query("UPDATE settings SET webhook_disabled = 1 WHERE id = 1")
            .execute(&self.pool)
            .await?;
        info!("database Discord webhook disabled");
        Ok(())
    }

    pub async fn resume_webhook(&self) -> anyhow::Result<()> {
        sqlx::query("UPDATE settings SET webhook_disabled = 0, webhook_retry_at = 0 WHERE id = 1")
            .execute(&self.pool)
            .await?;
        info!("database Discord webhook resumed");
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryState {
    Sent,
    Uncertain,
    Failed,
}
impl DeliveryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Uncertain => "uncertain",
            Self::Failed => "failed",
        }
    }
}
