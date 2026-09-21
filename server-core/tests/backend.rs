use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{FromRequest, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use server_core::{
    api,
    database::{Database, DeliveryState},
    discord::{Delivery, Discord},
    model::NotificationMode,
    planner::plan,
    service::Service,
    sonarr::{Episode, FinaleType, Series, SeriesStatus, Sonarr},
};

static NEXT_DB: AtomicU64 = AtomicU64::new(0);

struct TestDb {
    db: Database,
    path: std::path::PathBuf,
}
impl TestDb {
    async fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "jelly-alert-test-{}-{}.db",
            std::process::id(),
            NEXT_DB.fetch_add(1, Ordering::Relaxed)
        ));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        sqlx::query("UPDATE settings SET tracking_since = 0")
            .execute(db.pool())
            .await
            .unwrap();
        Self { db, path }
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

fn series() -> Series {
    Series {
        id: 1,
        title: "Test Show".into(),
        monitored: true,
        status: SeriesStatus::Continuing,
    }
}
fn episode(id: i64, number: i64, timestamp: i64) -> Episode {
    Episode {
        id,
        series_id: 1,
        season_number: 1,
        episode_number: number,
        title: format!("Episode {number}"),
        air_date_utc: Utc.timestamp_opt(timestamp, 0).single(),
        has_file: false,
        monitored: true,
        finale_type: None,
    }
}
fn season_plan(episodes: &[Episode]) -> Vec<server_core::model::PlannedNotification> {
    plan(&series(), episodes, 0)
        .into_iter()
        .filter(|p| p.mode == NotificationMode::Season && !p.awaiting_confirmation)
        .collect()
}

#[test]
fn episode_rules_and_initial_backlog() {
    let mut episodes = vec![
        episode(1, 1, 100),
        episode(2, 2, 200),
        episode(3, 3, 300),
        episode(4, 4, 400),
        episode(5, 5, 500),
    ];
    episodes[1].has_file = true;
    episodes[2].monitored = false;
    episodes[3].air_date_utc = None;
    let plans = plan(&series(), &episodes, 101);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].episode_id, Some(5));
    let mut unmonitored = series();
    unmonitored.monitored = false;
    assert!(plan(&unmonitored, &episodes, 0).is_empty());
}

#[test]
fn season_waits_for_finale_and_all_dates() {
    let mut episodes = vec![episode(1, 1, 100), episode(2, 2, 200)];
    assert!(season_plan(&episodes).is_empty());
    episodes[1].finale_type = Some(FinaleType::Midseason);
    assert!(season_plan(&episodes).is_empty());
    episodes[1].finale_type = Some(FinaleType::Season);
    let plans = season_plan(&episodes);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].due_at, 200);
    episodes[0].monitored = false;
    episodes[0].air_date_utc = None;
    assert!(season_plan(&episodes).is_empty());
}

#[test]
fn ended_and_previous_seasons_have_completion_evidence() {
    let episodes = vec![episode(1, 1, 100)];
    let mut show = series();
    show.status = SeriesStatus::Ended;
    assert!(
        plan(&show, &episodes, 0)
            .iter()
            .any(|p| p.mode == NotificationMode::Season)
    );
    let mut next = episode(2, 1, 200);
    next.season_number = 2;
    assert_eq!(season_plan(&[episodes[0].clone(), next]).len(), 1);
}

#[test]
fn season_with_files_and_specials_are_not_notified() {
    let mut item = episode(1, 1, 100);
    item.finale_type = Some(FinaleType::Season);
    item.has_file = true;
    assert!(season_plan(&[item.clone()]).is_empty());
    item.has_file = false;
    item.season_number = 0;
    assert!(season_plan(&[item]).is_empty());
}

#[test]
fn utc_offsets_and_unknown_sonarr_values_are_safe() {
    let item: Episode = serde_json::from_value(json!({"id":1,"seriesId":1,"seasonNumber":1,"episodeNumber":1,"title":"Pilot","airDateUtc":"2026-09-20T20:00:00+10:00","hasFile":false,"monitored":true,"finaleType":"new-value"})).unwrap();
    assert_eq!(
        item.air_date_utc.unwrap().to_rfc3339(),
        "2026-09-20T10:00:00+00:00"
    );
    assert_eq!(item.finale_type, Some(FinaleType::Unknown));
    assert!(season_plan(&[item]).is_empty());
}

#[tokio::test]
async fn migrations_restart_and_deduplication() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Test Show".into())]).await.unwrap();
    let plans = plan(&series(), &[episode(1, 1, 100)], 0);
    db.replace_plans(1, &plans).await.unwrap();
    assert!(!db.claim(&plans[0], 99).await.unwrap());
    assert!(db.claim(&plans[0], 100).await.unwrap());
    db.finish(&plans[0].key, DeliveryState::Sent, 101)
        .await
        .unwrap();
    let reopened = Database::connect(fixture.path.to_str().unwrap())
        .await
        .unwrap();
    reopened.replace_plans(1, &plans).await.unwrap();
    assert!(reopened.next_due().await.unwrap().is_none());
    assert!(!reopened.claim(&plans[0], 200).await.unwrap());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 5);
    reopened.pool().close().await;
}

#[tokio::test]
async fn crash_after_claim_never_retries() {
    let fixture = TestDb::new().await;
    fixture.db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let plans = plan(&series(), &[episode(1, 1, 100)], 0);
    fixture.db.replace_plans(1, &plans).await.unwrap();
    assert!(fixture.db.claim(&plans[0], 100).await.unwrap());
    let reopened = Database::connect(fixture.path.to_str().unwrap())
        .await
        .unwrap();
    reopened.replace_plans(1, &plans).await.unwrap();
    assert!(reopened.next_due().await.unwrap().is_none());
    reopened.pool().close().await;
}

#[tokio::test]
async fn exclusions_modes_and_removed_shows_block_claims() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let plans = plan(&series(), &[episode(1, 1, 100)], 0);
    db.replace_plans(1, &plans).await.unwrap();
    db.set_excluded(1, true).await.unwrap();
    assert!(db.next_due().await.unwrap().is_none());
    assert!(!db.claim(&plans[0], 100).await.unwrap());
    db.sync_shows(&[(1, "Renamed".into())]).await.unwrap();
    assert!(db.shows().await.unwrap()[0].excluded);
    db.set_excluded(1, false).await.unwrap();
    db.set_show_mode(1, NotificationMode::Season).await.unwrap();
    assert!(!db.claim(&plans[0], 100).await.unwrap());
    db.set_show_mode(1, NotificationMode::Episode)
        .await
        .unwrap();
    db.sync_shows(&[]).await.unwrap();
    assert!(!db.claim(&plans[0], 100).await.unwrap());
}

#[tokio::test]
async fn season_delivery_covers_episodes_across_mode_changes() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let mut last = episode(2, 2, 200);
    last.finale_type = Some(FinaleType::Season);
    let plans = plan(&series(), &[episode(1, 1, 100), last], 0);
    db.replace_plans(1, &plans).await.unwrap();
    db.set_show_mode(1, NotificationMode::Season).await.unwrap();
    let season = plans
        .iter()
        .find(|p| p.mode == NotificationMode::Season)
        .unwrap();
    assert!(db.claim(season, 200).await.unwrap());
    db.finish(&season.key, DeliveryState::Sent, 200)
        .await
        .unwrap();
    db.set_show_mode(1, NotificationMode::Episode)
        .await
        .unwrap();
    assert!(db.next_due().await.unwrap().is_none());
    assert!(!db.claim(&plans[0], 200).await.unwrap());
}

#[tokio::test]
async fn already_notified_episodes_do_not_create_a_season_loop() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let mut last = episode(1, 1, 100);
    last.finale_type = Some(FinaleType::Season);
    let plans = plan(&series(), &[last], 0);
    db.replace_plans(1, &plans).await.unwrap();
    assert!(db.claim(&plans[0], 100).await.unwrap());
    db.finish(&plans[0].key, DeliveryState::Sent, 100)
        .await
        .unwrap();
    db.set_show_mode(1, NotificationMode::Season).await.unwrap();
    let season = plans
        .iter()
        .find(|p| p.mode == NotificationMode::Season)
        .unwrap();
    assert!(!db.claim(season, 100).await.unwrap());
    assert!(db.next_due().await.unwrap().is_none());
}

#[tokio::test]
async fn concurrent_claims_have_one_winner() {
    let fixture = TestDb::new().await;
    fixture.db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let plans = plan(&series(), &[episode(1, 1, 100)], 0);
    fixture.db.replace_plans(1, &plans).await.unwrap();
    let (left, right) = tokio::join!(
        fixture.db.claim(&plans[0], 100),
        fixture.db.claim(&plans[0], 100)
    );
    assert_ne!(left.unwrap(), right.unwrap());
}

#[tokio::test]
async fn rate_limits_release_reservation_and_survive_rescans() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let plans = plan(&series(), &[episode(1, 1, 100)], 0);
    db.replace_plans(1, &plans).await.unwrap();
    assert!(db.claim(&plans[0], 100).await.unwrap());
    db.rate_limited(&plans[0].key, 200).await.unwrap();
    db.replace_plans(1, &plans).await.unwrap();
    assert_eq!(db.next_due().await.unwrap().unwrap().2, 200);
    assert!(!db.claim(&plans[0], 199).await.unwrap());
    assert!(db.claim(&plans[0], 200).await.unwrap());
}

#[tokio::test]
async fn verification_backoff_survives_rescans() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let plans = plan(&series(), &[episode(1, 1, 100)], 0);
    db.replace_plans(1, &plans).await.unwrap();
    db.defer(&plans[0].key, 200).await.unwrap();
    db.replace_plans(1, &plans).await.unwrap();
    assert_eq!(db.next_due().await.unwrap().unwrap().2, 200);
    assert!(!db.claim(&plans[0], 199).await.unwrap());
    assert!(db.claim(&plans[0], 200).await.unwrap());
}

#[tokio::test]
async fn rescheduling_and_downloads_replace_pending_plans() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    db.replace_plans(1, &plan(&series(), &[episode(1, 1, 100)], 0))
        .await
        .unwrap();
    db.replace_plans(1, &plan(&series(), &[episode(1, 1, 300)], 0))
        .await
        .unwrap();
    assert_eq!(db.next_due().await.unwrap().unwrap().2, 300);
    let mut downloaded = episode(1, 1, 300);
    downloaded.has_file = true;
    db.replace_plans(1, &plan(&series(), &[downloaded], 0))
        .await
        .unwrap();
    assert!(db.next_due().await.unwrap().is_none());
}

#[derive(Clone)]
struct MockState {
    episodes: Arc<Mutex<Value>>,
    status: Arc<Mutex<StatusCode>>,
    messages: Arc<Mutex<Vec<Value>>>,
    requests: Arc<AtomicUsize>,
    fail_sonarr: Arc<Mutex<bool>>,
}
struct MockServer {
    base: String,
    state: MockState,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn mock_server() -> MockServer {
    let state = MockState {
        episodes: Arc::new(Mutex::new(json!([]))),
        status: Arc::new(Mutex::new(StatusCode::OK)),
        messages: Arc::new(Mutex::new(vec![])),
        requests: Arc::new(AtomicUsize::new(0)),
        fail_sonarr: Arc::new(Mutex::new(false)),
    };
    let router = Router::new()
        .route(
            "/sonarr/api/v3/series",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers.get("X-Api-Key").unwrap(), "sonarr-secret");
                Json(json!([{"id":1,"title":"Test Show","monitored":true,"status":"continuing"}]))
            }),
        )
        .route(
            "/sonarr/api/v3/series/1",
            get(|| async {
                Json(json!({"id":1,"title":"Test Show","monitored":true,"status":"continuing"}))
            }),
        )
        .route(
            "/sonarr/api/v3/episode",
            get(
                |State(state): State<MockState>,
                 Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    assert_eq!(query.get("seriesId").unwrap(), "1");
                    if *state.fail_sonarr.lock().unwrap() {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    Json(state.episodes.lock().unwrap().clone()).into_response()
                },
            ),
        )
        .route(
            "/hook",
            post(
                |State(state): State<MockState>,
                 Query(query): Query<std::collections::HashMap<String, String>>,
                 request: axum::extract::Request| async move {
                    let multipart = request
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.starts_with("multipart/form-data"));
                    let message: Value = if multipart {
                        let mut form = axum::extract::Multipart::from_request(request, &())
                            .await
                            .unwrap();
                        let mut payload = None;
                        let mut image = None;
                        while let Some(field) = form.next_field().await.unwrap() {
                            match field.name().unwrap() {
                                "payload_json" => {
                                    payload = Some(
                                        serde_json::from_str::<Value>(&field.text().await.unwrap())
                                            .unwrap(),
                                    )
                                }
                                "files[0]" => {
                                    assert_eq!(field.file_name(), Some("series.jpg"));
                                    assert_eq!(field.content_type(), Some("image/jpeg"));
                                    image = Some(field.bytes().await.unwrap());
                                }
                                other => panic!("Unexpected multipart field: {other}"),
                            }
                        }
                        assert_eq!(image.unwrap().as_ref(), &[0xff, 0xd8, 0xff, 0xd9]);
                        let payload = payload.unwrap();
                        assert_eq!(
                            payload["embeds"][0]["image"]["url"],
                            "attachment://series.jpg"
                        );
                        payload
                    } else {
                        Json::<Value>::from_request(request, &()).await.unwrap().0
                    };
                    assert!(message.get("content").is_none());
                    assert_eq!(message["embeds"][0]["author"]["name"], "Jelly Name");
                    assert!(
                        message["embeds"][0]["description"]
                            .as_str()
                            .is_some_and(|text| !text.is_empty())
                    );
                    if !multipart {
                        assert!(message["embeds"][0].get("image").is_none());
                    }
                    assert_eq!(query.get("wait").unwrap(), "true");
                    state.requests.fetch_add(1, Ordering::Relaxed);
                    state.messages.lock().unwrap().push(message);
                    let status = *state.status.lock().unwrap();
                    (
                        status,
                        Json(if status == StatusCode::TOO_MANY_REQUESTS {
                            json!({"retry_after": 1.2})
                        } else {
                            json!({"id":"123"})
                        }),
                    )
                },
            ),
        )
        .route(
            "/sonarr/api/v3/mediacover/1/poster.jpg",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers.get("X-Api-Key").unwrap(), "sonarr-secret");
                (
                    [("content-type", "image/jpeg")],
                    vec![0xff_u8, 0xd8, 0xff, 0xd9],
                )
            }),
        )
        .route(
            "/sonarr/api/v3/mediacover/2/poster.jpg",
            get(|| async { "not an image" }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    MockServer { base, state, task }
}
fn episode_json(timestamp: i64, has_file: bool) -> Value {
    json!([{"id":1,"seriesId":1,"seasonNumber":1,"episodeNumber":1,"title":"Pilot","airDateUtc":Utc.timestamp_opt(timestamp,0).single().unwrap().to_rfc3339(),"hasFile":has_file,"monitored":true}])
}
fn service(db: Database, mock: &MockServer) -> Service {
    Service::new(
        db,
        Sonarr::new(&format!("{}/sonarr", mock.base), "sonarr-secret").unwrap(),
        Discord::new(&format!("{}/hook", mock.base)).unwrap(),
    )
}

#[tokio::test]
async fn worker_end_to_end_rechecks_library_and_sends_once() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    let now = Utc::now().timestamp();
    *mock.state.episodes.lock().unwrap() = episode_json(now - 1, false);
    let service = service(fixture.db.clone(), &mock);
    service.scan().await.unwrap();
    service.delivery_step().await.unwrap();
    service.scan().await.unwrap();
    service.delivery_step().await.unwrap();
    assert_eq!(
        mock.state.messages.lock().unwrap()[0]["embeds"][0]["image"]["url"],
        "attachment://series.jpg"
    );
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 1);
    assert_eq!(
        mock.state.messages.lock().unwrap()[0]["allowed_mentions"]["parse"],
        json!([])
    );
}

#[tokio::test]
async fn worker_does_not_send_if_file_arrives_between_scan_and_delivery() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    let now = Utc::now().timestamp();
    *mock.state.episodes.lock().unwrap() = episode_json(now - 1, false);
    let service = service(fixture.db.clone(), &mock);
    service.scan().await.unwrap();
    *mock.state.episodes.lock().unwrap() = episode_json(now - 1, true);
    service.delivery_step().await.unwrap();
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn worker_postpones_on_sonarr_failure_and_air_date_changes() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    let now = Utc::now().timestamp();
    *mock.state.episodes.lock().unwrap() = episode_json(now - 1, false);
    let service = service(fixture.db.clone(), &mock);
    service.scan().await.unwrap();
    *mock.state.fail_sonarr.lock().unwrap() = true;
    service.delivery_step().await.unwrap();
    assert!(fixture.db.next_due().await.unwrap().unwrap().2 > now);
    *mock.state.fail_sonarr.lock().unwrap() = false;
    *mock.state.episodes.lock().unwrap() = episode_json(now + 3600, false);
    service.scan().await.unwrap();
    assert_eq!(
        service.delivery_step().await.unwrap(),
        Duration::from_secs(30)
    );
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn discord_classifies_errors_without_automatic_post_retries() {
    let mock = mock_server().await;
    let discord = Discord::new(&format!("{}/hook", mock.base)).unwrap();
    for (status, expected) in [
        (StatusCode::OK, Delivery::Sent),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Delivery::RateLimited { retry_seconds: 2 },
        ),
        (StatusCode::NOT_FOUND, Delivery::Disabled),
        (StatusCode::BAD_REQUEST, Delivery::Rejected),
        (StatusCode::INTERNAL_SERVER_ERROR, Delivery::Uncertain),
    ] {
        *mock.state.status.lock().unwrap() = status;
        assert_eq!(discord.send("Test").await.unwrap(), expected);
    }
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 5);
}

#[tokio::test]
async fn disabled_webhook_stops_further_delivery() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    *mock.state.episodes.lock().unwrap() = episode_json(Utc::now().timestamp() - 1, false);
    *mock.state.status.lock().unwrap() = StatusCode::NOT_FOUND;
    let service = service(fixture.db.clone(), &mock);
    service.scan().await.unwrap();
    service.delivery_step().await.unwrap();
    let disabled: bool = sqlx::query_scalar("SELECT webhook_disabled FROM settings")
        .fetch_one(fixture.db.pool())
        .await
        .unwrap();
    assert!(disabled);
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn api_trusted_lan_validation_and_persistent_policy() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    fixture.db.sync_shows(&[(1, "Show".into())]).await.unwrap();
    let router = api::router(service(fixture.db.clone(), &mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/api/shows"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let response = client
        .put(format!("{base}/api/shows/1/mode"))
        .header("Content-Type", "application/json")
        .body(r#"{"mode":"season"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        fixture.db.shows().await.unwrap()[0].mode,
        NotificationMode::Season
    );
    let invalid = client
        .put(format!("{base}/api/shows/1/mode"))
        .header("Content-Type", "application/json")
        .body(r#"{"mode":"nonsense"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = client
        .put(format!("{base}/api/shows/1/exclusion"))
        .header("Content-Type", "application/json")
        .body(r#"{"excluded":true}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(fixture.db.shows().await.unwrap()[0].excluded);
    for path in ["/api/health", "/api/shows", "/api/notifications?limit=1000"] {
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    for body in [r#"{"mode":"season"}"#, r#"{"mode":"episode"}"#] {
        assert_eq!(
            client
                .put(format!("{base}/api/settings"))
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        fixture.db.shows().await.unwrap()[0].mode,
        NotificationMode::Season
    );
    assert_eq!(
        client
            .put(format!("{base}/api/shows/1/mode"))
            .header("Content-Type", "application/json")
            .body(r#"{"mode":"default"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(!fixture.db.shows().await.unwrap()[0].mode_overridden);
    assert_eq!(
        fixture.db.shows().await.unwrap()[0].mode,
        NotificationMode::Episode
    );
    for body in [
        r#"{"mode":"default"}"#,
        r#"{}"#,
        r#"{"mode":"season","extra":true}"#,
    ] {
        assert_eq!(
            client
                .put(format!("{base}/api/settings"))
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let settings: serde_json::Value = serde_json::from_str(
        &client
            .get(format!("{base}/api/settings"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(settings["mode"], "episode");
    fixture
        .db
        .set_show_mode(1, NotificationMode::Episode)
        .await
        .unwrap();
    assert_eq!(
        client
            .put(format!("{base}/api/settings/reset-all"))
            .header("Content-Type", "application/json")
            .body(r#"{"mode":"invalid"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(fixture.db.shows().await.unwrap()[0].mode_overridden);
    assert_eq!(
        client
            .put(format!("{base}/api/settings/reset-all"))
            .header("Content-Type", "application/json")
            .body(r#"{"mode":"season"}"#)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let show = fixture.db.shows().await.unwrap().remove(0);
    assert_eq!(show.mode, NotificationMode::Season);
    assert!(!show.mode_overridden);
    assert!(show.excluded);
    assert_eq!(
        fixture.db.default_mode().await.unwrap(),
        NotificationMode::Season
    );

    task.abort();
}

#[test]
fn season_with_missing_episode_numbers_is_not_complete() {
    let mut finale = episode(3, 3, 300);
    finale.finale_type = Some(FinaleType::Season);
    assert!(season_plan(&[episode(1, 1, 100), finale]).is_empty());
}

#[tokio::test]
async fn connection_failure_is_safe_to_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let discord = Discord::new(&format!("http://{address}/hook")).unwrap();
    assert_eq!(
        discord.send("Test").await.unwrap(),
        Delivery::Retryable { retry_seconds: 30 }
    );
}

#[tokio::test]
async fn uncertain_delivery_is_persisted_and_never_repeated() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    *mock.state.episodes.lock().unwrap() = episode_json(Utc::now().timestamp() - 1, false);
    *mock.state.status.lock().unwrap() = StatusCode::INTERNAL_SERVER_ERROR;
    let service = service(fixture.db.clone(), &mock);
    service.scan().await.unwrap();
    service.delivery_step().await.unwrap();
    service.scan().await.unwrap();
    service.delivery_step().await.unwrap();
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 1);
    let state: String =
        sqlx::query_scalar("SELECT state FROM notifications WHERE key = 'episode:1'")
            .fetch_one(fixture.db.pool())
            .await
            .unwrap();
    assert_eq!(state, "uncertain");
}

#[tokio::test]
async fn workers_stop_promptly_when_idle() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    let service = service(fixture.db.clone(), &mock);
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let scanner = service.clone();
    let scan_receiver = receiver.clone();
    let scan_task = tokio::spawn(async move {
        scanner
            .scan_loop(Duration::from_secs(300), scan_receiver)
            .await
    });
    let delivery_task = tokio::spawn(async move { service.delivery_loop(receiver).await });
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        scan_task.await.unwrap();
        delivery_task.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn activity_views_filter_policy_paginate_and_include_message_details() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Test Show".into())]).await.unwrap();
    let mut finale = episode(2, 2, 200);
    finale.finale_type = Some(FinaleType::Season);
    let plans = plan(&series(), &[episode(1, 1, 100), finale], 0);
    db.replace_plans(1, &plans).await.unwrap();
    let router = api::router(service(db.clone(), &mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let get_page = |query: &'static str| {
        let client = client.clone();
        let url = format!("{base}/api/notifications?{query}");
        async move {
            let response = client.get(url).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            serde_json::from_slice::<Vec<Value>>(&response.bytes().await.unwrap()).unwrap()
        }
    };
    let upcoming = get_page("view=upcoming").await;
    assert_eq!(upcoming.len(), 2);
    assert_eq!(upcoming[0]["due_at"], 100);
    assert_eq!(upcoming[1]["due_at"], 200);
    assert_eq!(upcoming[0]["season"], 1);
    assert!(
        upcoming[0]["content"]
            .as_str()
            .unwrap()
            .contains("Test Show")
    );
    assert_eq!(
        get_page("view=upcoming&limit=1&offset=1").await[0],
        upcoming[1]
    );
    assert!(get_page("view=history").await.is_empty());
    db.set_show_mode(1, NotificationMode::Season).await.unwrap();
    let seasons = get_page("view=upcoming").await;
    assert_eq!(seasons.len(), 1);
    assert_eq!(seasons[0]["mode"], "season");
    db.set_excluded(1, true).await.unwrap();
    assert!(get_page("view=upcoming").await.is_empty());
    db.set_excluded(1, false).await.unwrap();
    db.sync_shows(&[]).await.unwrap();
    assert!(get_page("view=upcoming").await.is_empty());
    db.sync_shows(&[(1, "Test Show".into())]).await.unwrap();
    db.set_show_mode(1, NotificationMode::Episode)
        .await
        .unwrap();
    let first = plans
        .iter()
        .find(|item| item.episode_id == Some(1))
        .unwrap();
    assert!(db.claim(first, 100).await.unwrap());
    db.finish(&first.key, DeliveryState::Sent, 101)
        .await
        .unwrap();
    // The sent episode is covered, while the second episode is still eligible.
    let remaining = get_page("view=upcoming").await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["key"], "episode:2");
    let history = get_page("view=history").await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["state"], "sent");
    assert_eq!(history[0]["sent_at"], 101);
    assert_eq!(
        client
            .get(format!("{base}/api/notifications?view=invalid"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    task.abort();
}

#[tokio::test]
async fn individual_modes_control_deadlines_claims_and_survive_rescans() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Episodes".into()), (2, "Seasons".into())])
        .await
        .unwrap();
    let first = plan(&series(), &[episode(1, 1, 100)], 0);
    let mut second_series = series();
    second_series.id = 2;
    let mut second_episode = episode(2, 1, 200);
    second_episode.series_id = 2;
    second_episode.finale_type = Some(FinaleType::Season);
    let second = plan(&second_series, &[second_episode], 0);
    db.replace_plans(1, &first).await.unwrap();
    db.replace_plans(2, &second).await.unwrap();
    assert!(db.set_show_mode(2, NotificationMode::Season).await.unwrap());
    assert!(
        !db.set_show_mode(999, NotificationMode::Season)
            .await
            .unwrap()
    );
    assert_eq!(db.next_due().await.unwrap().unwrap().0, first[0].key);
    assert!(db.claim(&first[0], 100).await.unwrap());
    let episode_plan = second
        .iter()
        .find(|p| p.mode == NotificationMode::Episode)
        .unwrap();
    let season_plan = second
        .iter()
        .find(|p| p.mode == NotificationMode::Season)
        .unwrap();
    assert_eq!(db.next_due().await.unwrap().unwrap().0, season_plan.key);
    assert!(!db.claim(episode_plan, 200).await.unwrap());
    // A mode change after selecting a deadline must invalidate that old claim.
    db.set_show_mode(2, NotificationMode::Episode)
        .await
        .unwrap();
    assert!(!db.claim(season_plan, 200).await.unwrap());
    db.set_show_mode(2, NotificationMode::Season).await.unwrap();
    assert!(db.claim(season_plan, 200).await.unwrap());
    db.sync_shows(&[]).await.unwrap();
    db.sync_shows(&[
        (1, "Renamed episodes".into()),
        (2, "Renamed seasons".into()),
        (3, "New show".into()),
    ])
    .await
    .unwrap();
    let reopened = Database::connect(fixture.path.to_str().unwrap())
        .await
        .unwrap();
    let shows = reopened.shows().await.unwrap();
    assert_eq!(
        shows.iter().find(|s| s.id == 1).unwrap().mode,
        NotificationMode::Episode
    );
    assert_eq!(
        shows.iter().find(|s| s.id == 2).unwrap().mode,
        NotificationMode::Season
    );
    assert_eq!(
        shows.iter().find(|s| s.id == 3).unwrap().mode,
        NotificationMode::Episode
    );
    reopened.pool().close().await;
}

#[tokio::test]
async fn per_show_migration_preserves_legacy_policy_and_history() {
    use sqlx::{
        SqlSafeStr,
        migrate::{Migration, MigrationType, Migrator},
        sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    };
    let path = std::env::temp_dir().join(format!(
        "jelly-alert-legacy-{}-{}.db",
        std::process::id(),
        NEXT_DB.fetch_add(1, Ordering::Relaxed)
    ));
    let pool = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    Migrator::with_migrations(vec![Migration::new(
        202609200001,
        "initial".into(),
        MigrationType::Simple,
        include_str!("../migrations/202609200001_initial.sql").into_sql_str(),
        false,
    )])
    .run(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO settings (id, mode, tracking_since) VALUES (1, 'season', 123)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO shows (id, title, excluded) VALUES (1, 'Existing show', 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO notifications (key, series_id, mode, season, due_at, content, state) VALUES ('old', 1, 'season', 1, 100, 'Already sent', 'sent')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO covered_episodes VALUES (1, 'old')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let db = Database::connect(path.to_str().unwrap()).await.unwrap();
    assert_eq!(db.shows().await.unwrap()[0].mode, NotificationMode::Season);
    assert!(db.shows().await.unwrap()[0].excluded);
    assert!(db.shows().await.unwrap()[0].mode_overridden);
    db.set_default_mode(NotificationMode::Episode)
        .await
        .unwrap();
    assert_eq!(db.shows().await.unwrap()[0].mode, NotificationMode::Season);
    assert_eq!(db.tracking_since().await.unwrap(), 123);
    let covered: i64 = sqlx::query_scalar("SELECT count(*) FROM covered_episodes")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(covered, 1);
    let state: String = sqlx::query_scalar("SELECT state FROM notifications WHERE key = 'old'")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(state, "sent");
    db.pool().close().await;
    drop(TestDb { db, path });
}

#[tokio::test]
async fn poster_proxy_bounds_sources_and_rejects_invalid_images() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    fixture
        .db
        .sync_shows(&[
            (1, "Artwork".into()),
            (2, "Invalid".into()),
            (3, "Missing".into()),
        ])
        .await
        .unwrap();
    let router = api::router(service(fixture.db.clone(), &mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/shows/1/poster"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    assert_eq!(
        response.headers()["cache-control"],
        "private, max-age=86400"
    );
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        &[0xff, 0xd8, 0xff, 0xd9]
    );
    for (id, expected) in [
        (2, StatusCode::BAD_GATEWAY),
        (3, StatusCode::NOT_FOUND),
        (99, StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            client
                .get(format!("{base}/api/shows/{id}/poster"))
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    task.abort();
}

#[tokio::test]
async fn test_sends_leave_normal_delivery_untouched() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    *mock.state.episodes.lock().unwrap() = episode_json(Utc::now().timestamp() - 1, false);
    let service = service(fixture.db.clone(), &mock);
    service.scan().await.unwrap();
    let before: String = sqlx::query_scalar("SELECT json_object('state', state, 'attempted_at', attempted_at, 'sent_at', sent_at) FROM notifications WHERE key = 'episode:1'")
        .fetch_one(fixture.db.pool()).await.unwrap();
    let router = api::router(service.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    for (upstream, expected) in [
        (StatusCode::OK, StatusCode::NO_CONTENT),
        (StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS),
        (StatusCode::NOT_FOUND, StatusCode::BAD_GATEWAY),
        (StatusCode::INTERNAL_SERVER_ERROR, StatusCode::BAD_GATEWAY),
    ] {
        *mock.state.status.lock().unwrap() = upstream;
        assert_eq!(
            client
                .post(format!("{base}/api/notifications/episode:1/test"))
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
        let after: String = sqlx::query_scalar("SELECT json_object('state', state, 'attempted_at', attempted_at, 'sent_at', sent_at) FROM notifications WHERE key = 'episode:1'")
            .fetch_one(fixture.db.pool()).await.unwrap();
        assert_eq!(before, after);
        let covered: i64 = sqlx::query_scalar("SELECT count(*) FROM covered_episodes")
            .fetch_one(fixture.db.pool())
            .await
            .unwrap();
        assert_eq!(covered, 0);
        assert!(service.health.read().await.last_delivery_at.is_none());
    }
    assert_eq!(
        client
            .post(format!("{base}/api/notifications/episode:999/test"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 4);
    *mock.state.status.lock().unwrap() = StatusCode::OK;
    service.delivery_step().await.unwrap();
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 5);
    let state: String =
        sqlx::query_scalar("SELECT state FROM notifications WHERE key = 'episode:1'")
            .fetch_one(fixture.db.pool())
            .await
            .unwrap();
    assert_eq!(state, "sent");
    let messages = mock.state.messages.lock().unwrap();
    assert_eq!(messages[0]["embeds"], messages[4]["embeds"]);
    task.abort();
}

#[tokio::test]
async fn defaults_preserve_explicit_choices_and_apply_to_new_and_reset_shows() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Inherited".into()), (2, "Explicit".into())])
        .await
        .unwrap();
    // Selecting the current default still expresses an explicit preference.
    db.set_show_mode(2, NotificationMode::Episode)
        .await
        .unwrap();
    db.set_excluded(1, true).await.unwrap();
    db.set_default_mode(NotificationMode::Season).await.unwrap();
    db.sync_shows(&[
        (1, "Inherited".into()),
        (2, "Explicit".into()),
        (3, "New".into()),
    ])
    .await
    .unwrap();
    let shows = db.shows().await.unwrap();
    for id in [1, 3] {
        let show = shows.iter().find(|s| s.id == id).unwrap();
        assert_eq!(show.mode, NotificationMode::Season);
        assert!(!show.mode_overridden);
    }
    let explicit = shows.iter().find(|s| s.id == 2).unwrap();
    assert_eq!(explicit.mode, NotificationMode::Episode);
    assert!(explicit.mode_overridden);
    assert!(shows.iter().find(|s| s.id == 1).unwrap().excluded);
    assert!(db.reset_show_mode(2).await.unwrap());
    assert!(!db.reset_show_mode(999).await.unwrap());
    assert_eq!(
        db.shows()
            .await
            .unwrap()
            .iter()
            .find(|s| s.id == 2)
            .unwrap()
            .mode,
        NotificationMode::Season
    );
    db.set_default_mode(NotificationMode::Episode)
        .await
        .unwrap();
    assert!(
        db.shows()
            .await
            .unwrap()
            .iter()
            .all(|s| s.mode == NotificationMode::Episode && !s.mode_overridden)
    );
    let reopened = Database::connect(fixture.path.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        reopened.default_mode().await.unwrap(),
        NotificationMode::Episode
    );
}

#[tokio::test]
async fn reset_all_modes_includes_inactive_shows_and_restores_inheritance() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    db.sync_shows(&[(1, "Active".into()), (2, "Removed".into())])
        .await
        .unwrap();
    db.set_show_mode(1, NotificationMode::Episode)
        .await
        .unwrap();
    db.set_show_mode(2, NotificationMode::Episode)
        .await
        .unwrap();
    db.set_excluded(2, true).await.unwrap();
    db.sync_shows(&[(1, "Active".into())]).await.unwrap();
    db.reset_all_show_modes(NotificationMode::Season)
        .await
        .unwrap();
    let reopened = Database::connect(fixture.path.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        reopened.default_mode().await.unwrap(),
        NotificationMode::Season
    );
    let shows = reopened.shows().await.unwrap();
    assert!(
        shows
            .iter()
            .all(|s| s.mode == NotificationMode::Season && !s.mode_overridden)
    );
    let removed = shows.iter().find(|s| s.id == 2).unwrap();
    assert!(!removed.active);
    assert!(removed.excluded);
    reopened
        .set_default_mode(NotificationMode::Episode)
        .await
        .unwrap();
    assert!(
        reopened
            .shows()
            .await
            .unwrap()
            .iter()
            .all(|s| s.mode == NotificationMode::Episode && !s.mode_overridden)
    );
}

#[tokio::test]
async fn game_changer_unmarked_finale_is_visible_but_never_delivered_early() {
    let fixture = TestDb::new().await;
    let db = &fixture.db;
    let mut show = series();
    show.title = "Game Changer".into();
    db.sync_shows(&[(1, show.title.clone())]).await.unwrap();
    db.set_show_mode(1, NotificationMode::Season).await.unwrap();
    let mut episodes: Vec<_> = (1..=10)
        .map(|n| {
            let mut e = episode(n, n, 100 * n);
            e.season_number = 8;
            e
        })
        .collect();
    episodes[9].title = "TBA".into();
    let plans = plan(&show, &episodes, 950);
    let pending = plans
        .iter()
        .find(|p| p.mode == NotificationMode::Season)
        .unwrap();
    assert!(pending.awaiting_confirmation);
    assert_eq!(pending.due_at, 1000);
    assert!(pending.content.contains("awaiting finale confirmation"));
    db.replace_plans(1, &plans).await.unwrap();
    let mock = mock_server().await;
    let router = api::router(service(db.clone(), &mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let body = reqwest::get(format!("{base}/api/notifications?view=upcoming"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let upcoming: Vec<Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(upcoming.len(), 1);
    assert_eq!(upcoming[0]["key"], "season:1:8");
    assert_eq!(upcoming[0]["awaiting_confirmation"], true);
    task.abort();
    assert!(db.next_due().await.unwrap().is_none());
    assert!(!db.claim(pending, 2000).await.unwrap());
    episodes[9].finale_type = Some(FinaleType::Season);
    let confirmed = plan(&show, &episodes, 950);
    let ready = confirmed
        .iter()
        .find(|p| p.mode == NotificationMode::Season)
        .unwrap();
    assert!(!ready.awaiting_confirmation);
    db.replace_plans(1, &confirmed).await.unwrap();
    assert_eq!(db.next_due().await.unwrap().unwrap().0, "season:1:8");
    // A later metadata refresh can revoke confirmation before delivery.
    episodes[9].finale_type = None;
    db.replace_plans(1, &plan(&show, &episodes, 950))
        .await
        .unwrap();
    assert!(!db.claim(ready, 2000).await.unwrap());
    db.replace_plans(1, &confirmed).await.unwrap();
    assert!(db.claim(ready, 2000).await.unwrap());
}

#[tokio::test]
async fn test_notifications_send_embeds_when_series_artwork_is_unavailable() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    *mock.state.episodes.lock().unwrap() = episode_json(Utc::now().timestamp() - 1, false);
    let service = service(fixture.db.clone(), &mock);
    service.scan().await.unwrap();
    fixture
        .db
        .sync_shows(&[
            (1, "Original".into()),
            (2, "Invalid artwork".into()),
            (3, "Missing artwork".into()),
        ])
        .await
        .unwrap();
    for series_id in [2_i64, 3] {
        sqlx::query("UPDATE notifications SET series_id = ? WHERE key = 'episode:1'")
            .bind(series_id)
            .execute(fixture.db.pool())
            .await
            .unwrap();
        assert_eq!(
            service.test_notification("episode:1").await.unwrap(),
            Some(Delivery::Sent)
        );
    }
    let messages = mock.state.messages.lock().unwrap();
    assert_eq!(messages.len(), 2);
    for message in messages.iter() {
        assert_eq!(message["embeds"][0]["author"]["name"], "Jelly Name");
        assert!(message["embeds"][0].get("image").is_none());
    }
}

#[test]
fn series_progress_distinguishes_airing_library_and_unconfirmed_seasons() {
    use std::collections::{HashMap, HashSet};
    let make_show = || server_core::model::Show {
        id: 1,
        title: "Game Changer".into(),
        active: true,
        excluded: false,
        mode: NotificationMode::Season,
        mode_overridden: false,
    };
    let mut episodes = vec![episode(1, 1, 100), episode(2, 2, 200), episode(3, 3, 300)];
    episodes[0].has_file = true;
    let covered = HashSet::new();
    let deliveries = HashMap::new();
    let result = server_core::progress::build(
        make_show(),
        &series(),
        &episodes,
        0,
        150,
        &covered,
        &deliveries,
        false,
        0,
    );
    assert_eq!(result.counts.aired, 1);
    assert_eq!(result.counts.in_library, 1);
    assert_eq!(result.next_release.unwrap().episode, 2);
    let notification = result.next_notification.unwrap();
    assert_eq!(notification.episodes_remaining, 2);
    assert!(notification.awaiting_confirmation);
    assert!(!result.seasons[0].completion_confirmed);
    assert!(result.seasons[0].final_air_at.is_none());
    episodes[2].finale_type = Some(FinaleType::Season);
    let result = server_core::progress::build(
        make_show(),
        &series(),
        &episodes,
        0,
        250,
        &covered,
        &deliveries,
        false,
        0,
    );
    assert_eq!(result.next_notification.unwrap().episodes_remaining, 1);
    assert!(result.seasons[0].completion_confirmed);
    assert_eq!(result.seasons[0].final_air_at, Some(300));
    // Unknown dates must not turn into an invented completion time.
    episodes[1].air_date_utc = None;
    let result = server_core::progress::build(
        make_show(),
        &series(),
        &episodes,
        0,
        250,
        &covered,
        &deliveries,
        false,
        0,
    );
    assert_eq!(result.counts.undated, 1);
    assert!(result.next_notification.is_none());
    assert!(!result.seasons[0].completion_confirmed);
}

#[test]
fn series_progress_respects_notification_history_policy_and_specials() {
    use std::collections::{HashMap, HashSet};
    let mut show = server_core::model::Show {
        id: 1,
        title: "Show".into(),
        active: true,
        excluded: false,
        mode: NotificationMode::Episode,
        mode_overridden: false,
    };
    let mut episodes = vec![episode(1, 1, 100), episode(2, 2, 200), episode(3, 1, 300)];
    episodes[2].season_number = 0;
    let mut covered = HashSet::from([1]);
    let deliveries = HashMap::from([("episode:2".into(), "uncertain".into())]);
    let result = server_core::progress::build(
        show,
        &series(),
        &episodes,
        0,
        50,
        &covered,
        &deliveries,
        false,
        0,
    );
    assert_eq!(result.counts.total, 2); // Specials are separate.
    assert_eq!(result.next_notification.unwrap().season, 0);
    covered.insert(3);
    show = server_core::model::Show {
        id: 1,
        title: "Show".into(),
        active: true,
        excluded: true,
        mode: NotificationMode::Episode,
        mode_overridden: false,
    };
    let result = server_core::progress::build(
        show,
        &series(),
        &episodes,
        0,
        50,
        &covered,
        &deliveries,
        false,
        0,
    );
    assert!(result.next_notification.is_none());
    assert!(result.notification_block.unwrap().contains("excluded"));
}

#[tokio::test]
async fn series_progress_api_is_read_only_and_reports_upstream_failure() {
    let fixture = TestDb::new().await;
    let mock = mock_server().await;
    fixture
        .db
        .sync_shows(&[(1, "Test Show".into())])
        .await
        .unwrap();
    *mock.state.episodes.lock().unwrap() = episode_json(Utc::now().timestamp() + 3600, false);
    let router = api::router(service(fixture.db.clone(), &mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let body = reqwest::get(format!("{base}/api/shows/1"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result["counts"]["total"], 1);
    assert_eq!(result["next_notification"]["episodes_remaining"], 1);
    assert_eq!(result["next_release"]["episode"], 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications")
        .fetch_one(fixture.db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(mock.state.requests.load(Ordering::Relaxed), 0);
    assert_eq!(
        reqwest::get(format!("{base}/api/shows/999"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    *mock.state.fail_sonarr.lock().unwrap() = true;
    assert_eq!(
        reqwest::get(format!("{base}/api/shows/1"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_GATEWAY
    );
    task.abort();
}

#[tokio::test]
async fn embed_colors_persist_validate_and_apply_to_both_delivery_modes() {
    for (mode, key, expected) in [
        (NotificationMode::Episode, "episode:1", 0),
        (NotificationMode::Season, "season:1:1", 0xffffff),
    ] {
        let fixture = TestDb::new().await;
        let mock = mock_server().await;
        let mut episodes = episode_json(Utc::now().timestamp() - 1, false);
        episodes[0]["finaleType"] = json!("season");
        *mock.state.episodes.lock().unwrap() = episodes;
        let service = service(fixture.db.clone(), &mock);
        service.scan().await.unwrap();
        fixture.db.set_show_mode(1, mode).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/settings/colors",
            listener.local_addr().unwrap()
        );
        let router = api::router(service.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::new();
        let defaults: Value = serde_json::from_slice(
            &client
                .get(&url)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            defaults,
            json!({"episode_color": 0x5865f2, "season_color": 0xf1c40f})
        );
        let colors = json!({"episode_color": 0, "season_color": 0xffffff});
        assert_eq!(
            client
                .put(&url)
                .header("content-type", "application/json")
                .body(colors.to_string())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        for invalid in [
            json!({"episode_color": -1, "season_color": 5}),
            json!({"episode_color": 5, "season_color": 0x1000000}),
            json!({"episode_color": "#ffffff", "season_color": 5}),
            json!({"episode_color": 1.5, "season_color": 5}),
            json!({"episode_color": 5}),
        ] {
            assert_eq!(
                client
                    .put(&url)
                    .header("content-type", "application/json")
                    .body(invalid.to_string())
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        assert_eq!(
            serde_json::from_slice::<Value>(
                &client
                    .get(&url)
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap()
            )
            .unwrap(),
            colors
        );
        fixture
            .db
            .set_default_mode(NotificationMode::Season)
            .await
            .unwrap();
        fixture.db.reset_all_show_modes(mode).await.unwrap();
        let reopened = Database::connect(fixture.path.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(
            reopened.embed_colors().await.unwrap().for_mode(mode),
            expected
        );
        assert_eq!(
            service.test_notification(key).await.unwrap(),
            Some(Delivery::Sent)
        );
        service.delivery_step().await.unwrap();
        let messages = mock.state.messages.lock().unwrap();
        assert_eq!(messages.len(), 2);
        for message in messages.iter() {
            assert_eq!(message["embeds"][0]["color"], expected);
        }
        task.abort();
    }
}
