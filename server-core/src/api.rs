//! JSON API for the future web frontend. Every endpoint requires bearer authentication.
use std::sync::Arc;

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

use crate::{
    model::{NotificationMode, Show},
    service::Service,
};

#[derive(Clone)]
pub struct ApiState {
    pub service: Service,
    token: Arc<str>,
}

pub fn router(service: Service, token: String) -> Router {
    let state = ApiState {
        service,
        token: token.into(),
    };
    Router::new()
        .route("/api/health", get(health))
        .route("/api/settings", get(settings).put(update_settings))
        .route("/api/shows", get(shows))
        .route("/api/shows/{id}/exclusion", put(exclude))
        .route("/api/notifications", get(notifications))
        .route("/api/webhook/resume", post(resume))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
}

async fn authenticate(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    if !token.is_some_and(|value| tokens_equal(value.as_bytes(), state.token.as_bytes())) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "valid bearer token required",
            }),
        )
            .into_response();
    }
    next.run(request).await
}

fn tokens_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
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
        eprintln!("API database operation failed: {}", self.0);
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
        mode: state.service.db.mode().await?,
    }))
}

async fn update_settings(
    State(state): State<ApiState>,
    Json(settings): Json<Settings>,
) -> ApiResult<Settings> {
    let _guard = state.service.delivery_gate.lock().await;
    state.service.db.set_mode(settings.mode).await?;
    state.service.wake.notify_one();
    Ok(Json(settings))
}

async fn shows(State(state): State<ApiState>) -> ApiResult<Vec<Show>> {
    Ok(Json(state.service.db.shows().await?))
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
    let _guard = state.service.delivery_gate.lock().await;
    let found = state
        .service
        .db
        .set_excluded(id, exclusion.excluded)
        .await?;
    state.service.wake.notify_one();
    Ok(if found {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn resume(State(state): State<ApiState>) -> Result<StatusCode, ApiError> {
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
}

#[derive(Serialize)]
struct Notification {
    key: String,
    series_id: i64,
    mode: NotificationMode,
    due_at: i64,
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
    let rows = sqlx::query("SELECT key, series_id, mode, due_at, state, attempted_at, sent_at FROM notifications ORDER BY due_at DESC, key LIMIT ? OFFSET ?")
        .bind(page.limit.unwrap_or(50).clamp(1, 100)).bind(page.offset.unwrap_or(0)).fetch_all(state.service.db.pool()).await?;
    let notifications = rows
        .iter()
        .map(|row| {
            Ok(Notification {
                key: row.try_get("key")?,
                series_id: row.try_get("series_id")?,
                mode: NotificationMode::try_from(row.try_get::<&str, _>("mode")?)?,
                due_at: row.try_get("due_at")?,
                state: NotificationState::try_from(row.try_get::<&str, _>("state")?)?,
                attempted_at: row.try_get("attempted_at")?,
                sent_at: row.try_get("sent_at")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Json(notifications))
}
