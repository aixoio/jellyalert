//! JSON API for trusted LAN clients. No authentication is required.

use std::time::Instant;

use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tracing::{Instrument, debug, error, info, info_span, warn};

use crate::{
    model::{EmbedColors, NotificationMode, Show},
    service::Service,
};

#[derive(Clone)]
pub struct ApiState {
    pub service: Service,
}

pub fn router(service: Service) -> Router {
    let state = ApiState { service };
    Router::new()
        .route("/api/health", get(health))
        .route("/api/settings", get(settings).put(update_settings))
        .route(
            "/api/settings/colors",
            get(embed_colors).put(update_embed_colors),
        )
        .route("/api/settings/reset-all", put(reset_all_modes))
        .route("/api/shows/{id}/mode", put(update_show_mode))
        .route("/api/shows", get(shows))
        .route("/api/shows/{id}", get(show_progress))
        .route("/api/shows/{id}/poster", get(poster))
        .route("/api/shows/{id}/exclusion", put(exclude))
        .route("/api/notifications", get(notifications))
        .route("/api/notifications/{key}/test", post(test_notification))
        .route("/api/webhook/resume", post(resume))
        .with_state(state)
        .layer(middleware::from_fn(log_request))
}

async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let version = request.version();
    let span = info_span!(
        "api_request",
        http.method = %method,
        http.uri = %uri,
        http.version = ?version,
        http.status_code = tracing::field::Empty,
        elapsed_ms = tracing::field::Empty,
    );
    async move {
        info!("API request received");
        let started = Instant::now();
        let response = next.run(request).await;
        let status = response.status();
        let elapsed_ms = started.elapsed().as_millis();
        tracing::Span::current().record("http.status_code", status.as_u16());
        tracing::Span::current().record("elapsed_ms", elapsed_ms);
        if status.is_server_error() {
            error!(%status, elapsed_ms, "API request completed");
        } else if status.is_client_error() {
            warn!(%status, elapsed_ms, "API request completed");
        } else {
            info!(%status, elapsed_ms, "API request completed");
        }
        response
    }
    .instrument(span)
    .await
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}
struct ApiError(anyhow::Error);
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self(error.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        error!(error = %self.0, "API operation failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "internal server error",
            }),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    mode: NotificationMode,
}

async fn settings(State(state): State<ApiState>) -> ApiResult<Settings> {
    Ok(Json(Settings {
        mode: state.service.db.default_mode().await?,
    }))
}

async fn update_settings(
    State(state): State<ApiState>,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    info!(
        mode = settings.mode.as_str(),
        "updating default notification mode"
    );
    let _guard = state.service.delivery_gate.lock().await;
    state.service.db.set_default_mode(settings.mode).await?;
    state.service.wake.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

async fn embed_colors(State(state): State<ApiState>) -> ApiResult<EmbedColors> {
    Ok(Json(state.service.db.embed_colors().await?))
}

async fn update_embed_colors(
    State(state): State<ApiState>,
    Json(colors): Json<EmbedColors>,
) -> Result<Response, ApiError> {
    if colors.episode_color > 0xffffff || colors.season_color > 0xffffff {
        warn!(
            episode_color = colors.episode_color,
            season_color = colors.season_color,
            "rejected invalid embed colors"
        );
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                error: "Colors must be RGB integers between 0 and 16777215.",
            }),
        )
            .into_response());
    }
    let _guard = state.service.delivery_gate.lock().await;
    state.service.db.set_embed_colors(colors).await?;
    info!(
        episode_color = colors.episode_color,
        season_color = colors.season_color,
        "updated embed colors"
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn reset_all_modes(
    State(state): State<ApiState>,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    info!(mode = settings.mode.as_str(), "resetting all show modes");
    let _guard = state.service.delivery_gate.lock().await;
    state.service.db.reset_all_show_modes(settings.mode).await?;
    state.service.wake.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShowMode {
    Default,
    Episode,
    Season,
}

impl ShowMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Episode => "episode",
            Self::Season => "season",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShowSettings {
    mode: ShowMode,
}

async fn update_show_mode(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(settings): Json<ShowSettings>,
) -> Result<StatusCode, ApiError> {
    info!(
        series_id = id,
        mode = settings.mode.as_str(),
        "updating show notification mode"
    );
    let _guard = state.service.delivery_gate.lock().await;
    let found = match settings.mode {
        ShowMode::Default => state.service.db.reset_show_mode(id).await?,
        ShowMode::Episode => {
            state
                .service
                .db
                .set_show_mode(id, NotificationMode::Episode)
                .await?
        }
        ShowMode::Season => {
            state
                .service
                .db
                .set_show_mode(id, NotificationMode::Season)
                .await?
        }
    };
    state.service.wake.notify_one();
    debug!(
        series_id = id,
        found, "show notification mode update completed"
    );
    Ok(if found {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn shows(State(state): State<ApiState>) -> ApiResult<Vec<Show>> {
    Ok(Json(state.service.db.shows().await?))
}

async fn show_progress(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    debug!(series_id = id, "building live show progress");
    let Some(show) = state
        .service
        .db
        .shows()
        .await?
        .into_iter()
        .find(|s| s.id == id)
    else {
        debug!(series_id = id, "show progress requested for unknown show");
        return Ok((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Show not found.",
            }),
        )
            .into_response());
    };
    if !show.active {
        debug!(series_id = id, "show progress requested for inactive show");
        return Ok((StatusCode::NOT_FOUND, Json(ErrorBody { error: "This show has been removed from Sonarr. Restore it there to view current progress." })).into_response());
    }
    let result = tokio::try_join!(
        state.service.sonarr.series_by_id(id),
        state.service.sonarr.episodes(id)
    );
    let (series, episodes) = match result {
        Ok(data) if data.0.id == id => data,
        _ => {
            warn!(
                series_id = id,
                "could not load current progress from Sonarr"
            );
            return Ok((
                StatusCode::BAD_GATEWAY,
                Json(ErrorBody {
                    error: "Cannot load current series progress from Sonarr. Please retry.",
                }),
            )
                .into_response());
        }
    };
    let covered: Vec<i64> = sqlx::query_scalar("SELECT c.episode_id FROM covered_episodes c JOIN notifications n ON n.key = c.notification_key WHERE n.series_id = ?")
        .bind(id).fetch_all(state.service.db.pool()).await?;
    let deliveries: Vec<(String, String)> =
        sqlx::query_as("SELECT key, state FROM notifications WHERE series_id = ?")
            .bind(id)
            .fetch_all(state.service.db.pool())
            .await?;
    let (disabled, retry_at): (bool, i64) =
        sqlx::query_as("SELECT webhook_disabled, webhook_retry_at FROM settings WHERE id = 1")
            .fetch_one(state.service.db.pool())
            .await?;
    let progress = crate::progress::build(
        show,
        &series,
        &episodes,
        state.service.db.tracking_since().await?,
        chrono::Utc::now().timestamp(),
        &covered.into_iter().collect(),
        &deliveries.into_iter().collect(),
        disabled,
        retry_at,
    );
    Ok(Json(progress).into_response())
}

async fn poster(State(state): State<ApiState>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    debug!(series_id = id, "loading show poster");
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM shows WHERE id = ?)")
        .bind(id)
        .fetch_one(state.service.db.pool())
        .await?;
    if !exists {
        debug!(series_id = id, "poster requested for unknown show");
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    Ok(match state.service.sonarr.poster(id).await {
        Ok(Some(bytes)) => (
            [
                ("content-type", "image/jpeg"),
                ("cache-control", "private, max-age=86400"),
                ("x-content-type-options", "nosniff"),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            warn!(series_id = id, error = %error, "could not load poster from Sonarr");
            StatusCode::BAD_GATEWAY.into_response()
        }
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exclusion {
    excluded: bool,
}

async fn exclude(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(exclusion): Json<Exclusion>,
) -> Result<StatusCode, ApiError> {
    info!(
        series_id = id,
        excluded = exclusion.excluded,
        "updating show exclusion"
    );
    let _guard = state.service.delivery_gate.lock().await;
    let found = state
        .service
        .db
        .set_excluded(id, exclusion.excluded)
        .await?;
    state.service.wake.notify_one();
    debug!(series_id = id, found, "show exclusion update completed");
    Ok(if found {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn resume(State(state): State<ApiState>) -> Result<StatusCode, ApiError> {
    info!("resuming Discord webhook delivery");
    let _guard = state.service.delivery_gate.lock().await;
    state.service.db.resume_webhook().await?;
    state.service.wake.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct HealthResponse {
    worker: crate::service::Health,
    webhook_disabled: bool,
    webhook_retry_at: i64,
    tracking_since: i64,
    unresolved_deliveries: i64,
}
async fn health(State(state): State<ApiState>) -> ApiResult<HealthResponse> {
    debug!("checking service health");
    let row = sqlx::query(
        "SELECT webhook_disabled, webhook_retry_at, tracking_since FROM settings WHERE id = 1",
    )
    .fetch_one(state.service.db.pool())
    .await?;
    let unresolved_deliveries = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE state IN ('sending', 'uncertain', 'failed')",
    )
    .fetch_one(state.service.db.pool())
    .await?;
    Ok(Json(HealthResponse {
        worker: state.service.health.read().await.clone(),
        webhook_disabled: row.try_get("webhook_disabled")?,
        webhook_retry_at: row.try_get("webhook_retry_at")?,
        tracking_since: row.try_get("tracking_since")?,
        unresolved_deliveries,
    }))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
    view: Option<NotificationView>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum NotificationView {
    Upcoming,
    History,
}

#[derive(Serialize)]
struct Notification {
    awaiting_confirmation: bool,
    key: String,
    series_id: i64,
    mode: NotificationMode,
    due_at: i64,
    season: i64,
    content: String,
    state: NotificationState,
    attempted_at: Option<i64>,
    sent_at: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum NotificationState {
    Pending,
    Sending,
    Sent,
    Uncertain,
    Failed,
    Covered,
}
impl TryFrom<&str> for NotificationState {
    type Error = anyhow::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "sending" => Ok(Self::Sending),
            "sent" => Ok(Self::Sent),
            "uncertain" => Ok(Self::Uncertain),
            "failed" => Ok(Self::Failed),
            "covered" => Ok(Self::Covered),
            _ => anyhow::bail!("invalid delivery state in database"),
        }
    }
}

async fn notifications(
    State(state): State<ApiState>,
    Query(page): Query<Page>,
) -> ApiResult<Vec<Notification>> {
    debug!(limit = ?page.limit, offset = ?page.offset, "listing notifications");
    let query = match page.view {
        Some(NotificationView::Upcoming) => {
            "SELECT n.key, n.series_id, n.mode, n.due_at, n.season, n.content, n.awaiting_confirmation, n.state, n.attempted_at, n.sent_at FROM notifications n JOIN shows s ON s.id = n.series_id WHERE n.state = 'pending' AND n.mode = s.mode AND s.active = 1 AND s.excluded = 0 AND (n.episode_id IS NULL OR NOT EXISTS (SELECT 1 FROM covered_episodes c WHERE c.episode_id = n.episode_id)) ORDER BY n.due_at ASC, n.key LIMIT ? OFFSET ?"
        }
        Some(NotificationView::History) => {
            "SELECT key, series_id, mode, due_at, season, content, awaiting_confirmation, state, attempted_at, sent_at FROM notifications WHERE state <> 'pending' ORDER BY due_at DESC, key LIMIT ? OFFSET ?"
        }
        None => {
            "SELECT key, series_id, mode, due_at, season, content, awaiting_confirmation, state, attempted_at, sent_at FROM notifications ORDER BY due_at DESC, key LIMIT ? OFFSET ?"
        }
    };
    let rows = sqlx::query(query)
        .bind(page.limit.unwrap_or(50).clamp(1, 100))
        .bind(page.offset.unwrap_or(0))
        .fetch_all(state.service.db.pool())
        .await?;
    let notifications = rows
        .iter()
        .map(|row| {
            Ok(Notification {
                awaiting_confirmation: row.try_get("awaiting_confirmation")?,
                key: row.try_get("key")?,
                series_id: row.try_get("series_id")?,
                mode: NotificationMode::try_from(row.try_get::<&str, _>("mode")?)?,
                due_at: row.try_get("due_at")?,
                season: row.try_get("season")?,
                content: row.try_get("content")?,
                state: NotificationState::try_from(row.try_get::<&str, _>("state")?)?,
                attempted_at: row.try_get("attempted_at")?,
                sent_at: row.try_get("sent_at")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    debug!(count = notifications.len(), "notifications listed");
    Ok(Json(notifications))
}

async fn test_notification(
    State(state): State<ApiState>,
    Path(key): Path<String>,
) -> Result<Response, ApiError> {
    use crate::discord::Delivery;
    info!(notification_key = %key, "sending test notification");
    let result = state.service.test_notification(&key).await?;
    let (status, error) = match result {
        Some(Delivery::Sent) => return Ok(StatusCode::NO_CONTENT.into_response()),
        None => (
            StatusCode::NOT_FOUND,
            "Notification no longer exists. Refresh activity.",
        ),
        Some(Delivery::RateLimited { retry_seconds }) => {
            return Ok((StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", retry_seconds.to_string())],
                Json(serde_json::json!({"error": format!("Discord rate limited the test. Try again in {retry_seconds} seconds.")}))).into_response());
        }
        Some(Delivery::Retryable { .. }) => (
            StatusCode::BAD_GATEWAY,
            "Could not connect to Discord. The test was not sent.",
        ),
        Some(Delivery::Disabled) => (
            StatusCode::BAD_GATEWAY,
            "Discord rejected the webhook. Check Server Core configuration.",
        ),
        Some(Delivery::Rejected) => (
            StatusCode::BAD_GATEWAY,
            "Discord rejected the test message.",
        ),
        Some(Delivery::Uncertain) => (
            StatusCode::BAD_GATEWAY,
            "Test delivery could not be confirmed. It may have arrived; check Discord before sending another test.",
        ),
    };
    Ok((status, Json(ErrorBody { error })).into_response())
}
