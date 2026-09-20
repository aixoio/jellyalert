# Jelly Alert backend

Rust service that reads Sonarr's v3 API, schedules Discord notifications, and exposes a JSON API for a future frontend. No frontend or download client is included.

## Run

```sh
cd server-core
cp server-config.example.toml server-config.toml
# Edit server-config.toml with your Sonarr URL/key, Discord webhook, and API token.
cargo run --release -- server-config.toml
```

Generate an API token with `openssl rand -hex 32`. Keep the config private (`chmod 600 server-config.toml`). The Sonarr URL is the instance's base URL, including a reverse-proxy subpath if needed, without `/api/v3`. The default API address is `127.0.0.1:8090`.

SQLite is created automatically at `sqlite_database_path`. Relative paths resolve from the process's working directory. SQL migrations live in `migrations/`, are embedded as SQL using `include_str!`, and run through SQLx's `Migrator` with checksum verification. Deployment needs the binary and configuration, not the source tree or a migration directory. No SQLx code-generation macros are used; the SQLx macros feature is disabled. Both Rust crate roots forbid unsafe code.

## Notification behavior

- By default, monitor every monitored Sonarr series and episode. Sonarr's `hasFile` is the library availability check, as requested; Jellyfin is not contacted.
- Episode mode schedules a missing episode at its `airDateUtc`, interpreted in UTC. Unknown dates wait for a later scan. Sonarr rescheduling, file imports, and monitoring changes are rechecked before sending.
- Season mode schedules one notification at the latest air time of all episodes in a season, when at least one monitored episode is missing. Every known episode must have an air date, and numbering must be consecutive from episode 1. Completion requires a season/series finale on the final numbered episode, a later season in Sonarr, or an ended series. A midseason finale is insufficient. Specials (season 0) only get individual episode notifications.
- Sonarr metadata is the authority for season boundaries. Incomplete or incorrect upstream metadata can delay notifications or produce an incorrect boundary. An ongoing season with no finale or later season is deliberately held until Sonarr supplies completion evidence.
- First startup records a persistent tracking start time. Episodes, and seasons whose final air time, predate that time are not notified. This prevents importing a historical backlog. Subsequent restarts retain that timestamp and catch up on eligible missed air times.
- Excluded shows never get new delivery attempts. Their exclusion survives restarts, rescans, renames, and removal/readdition with the same Sonarr ID. Unexcluding resumes tracking and may catch up on episodes that aired after the tracking start time.
- Modes and exclusions are persistent. Episode delivery history is shared with season mode: an episode covered by a season notification will not later receive an individual alert. A season whose missing episodes have all already been notified is marked `covered` instead of sent again.

An air time is **not proof that a downloadable release exists**. Jelly Alert does not query indexers. Discord messages say that the scheduled air time has been reached and prompt you to check for downloads.

## Delivery and long-running operation

The scanner and delivery worker run independently. The default scan interval is five minutes; known scheduled notifications use deadline sleeps rather than waiting for a scan tick. Wall-clock time is rechecked at least every 30 seconds so clock adjustments do not leave an old monotonic deadline in place. A newly announced episode can only be discovered on a scan. Actual delivery also includes final Sonarr verification, network latency, Discord rate limits, and any preceding due deliveries; it is never intentionally sent before the recorded air time. Keep the host clock synchronized.

The process reuses HTTP clients and a bounded SQLite pool, uses SQLite WAL with full synchronous commits, limits HTTP response sizes, and applies connection/request/database timeouts. Failed scans back off; a failed pre-send Sonarr check defers that notification so other due notifications can proceed. Workers are supervised; unexpected worker exit causes the process to exit. SIGINT and SIGTERM request graceful shutdown, allowing an in-flight webhook to finish and persist its result, with a bounded shutdown deadline. Use a process supervisor to restart after a crash or host reboot; `deploy/jelly-alert.service` is an example, not an installed service. Run one backend instance per database.

### Duplicate prevention tradeoff

Discord webhook execution does not provide an application idempotency key that can make a SQLite commit and an HTTP delivery atomic. Jelly Alert prioritizes the requirement to avoid duplicate notifications:

1. Atomically record a durable `sending` reservation and its covered episodes **before** posting.
2. Send once with `wait=true`, disabled mentions, and automatic HTTP retries disabled.
3. Record `sent` on success. A timeout after connecting, a server/proxy error, or an interrupted/crashed delivery remains `uncertain` or `sending` and is **never automatically repeated**.

Consequently, a crash between reserving and posting, or an ambiguous network failure, can lose a notification. Guaranteed delivery and guaranteed no duplicates cannot both be promised with this webhook interface. Unresolved deliveries are visible in the health and history APIs. The durable history is retained to prevent duplicates across restarts; do not delete it to reclaim space.

A definite connection failure before transmission is retried after 30 seconds. Discord 429 responses release the reservation and persist their retry delay. A 401/403/404 disables the webhook globally to avoid hammering a revoked endpoint; its definitely rejected notification stays pending. After repairing the config and restarting, call `/api/webhook/resume`. Other 4xx responses are recorded as `failed`. Configuration is read at startup, so changes to credentials require a restart.

No real Sonarr or Discord credentials were used during development. Tests use local mock HTTP servers; production integration and months of uptime have not been empirically verified.

## API

All endpoints require `Authorization: Bearer <api_token>`. JSON mutation bodies require `Content-Type: application/json`. Secret configuration is never returned. There is no cross-origin browser access by default; a future frontend can share the API's origin. For access beyond localhost, put it behind an HTTPS reverse proxy and keep the API token private.

| Method | Path | Body / result |
| --- | --- | --- |
| GET | `/api/settings` | `{"mode":"episode"}` or `{"mode":"season"}` |
| PUT | `/api/settings` | Body: `{"mode":"episode"}` or `{"mode":"season"}`; returns saved settings |
| GET | `/api/shows` | Array of `{id,title,excluded,active}`, sorted by title |
| PUT | `/api/shows/{id}/exclusion` | Body: `{"excluded":true}` or `false`; 204, or 404 for unknown show |
| GET | `/api/notifications?limit=50&offset=0` | History/plans, newest due time first; limit clamped to 1–100 |
| GET | `/api/health` | Worker scan/delivery times, webhook state, tracking start, unresolved delivery count |
| POST | `/api/webhook/resume` | Reenable a repaired webhook; 204 |

Timestamps are Unix seconds in UTC. Notification states are `pending`, `sending`, `sent`, `uncertain`, `failed`, and `covered`. `pending` may be held by the selected mode, exclusions, or webhook backoff; it is not a promise of immediate delivery. Health returns 200 if the API/database work; inspect `last_scan_succeeded` and `last_scan_at` to assess Sonarr health. Invalid tokens return 401, malformed/unknown fields or modes are rejected, and internal error details stay in stderr.

Settings/exclusion mutations are serialized with an in-flight Discord request. Once a successful mutation response is returned, the new policy applies to subsequent sends; a message already in flight cannot be recalled. The route may wait for that bounded request to finish.

```sh
export JELLY_ALERT_TOKEN='your-private-api-token'
curl -H "Authorization: Bearer $JELLY_ALERT_TOKEN" \
  http://127.0.0.1:8090/api/shows
curl -X PUT -H "Authorization: Bearer $JELLY_ALERT_TOKEN" \
  -H 'Content-Type: application/json' -d '{"mode":"season"}' \
  http://127.0.0.1:8090/api/settings
curl -X PUT -H "Authorization: Bearer $JELLY_ALERT_TOKEN" \
  -H 'Content-Type: application/json' -d '{"excluded":true}' \
  http://127.0.0.1:8090/api/shows/123/exclusion
```

## Verification

```sh
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```

Integration tests bind local loopback sockets, use temporary SQLite files, and do not contact real Sonarr/Discord services. They cover scheduling rules, UTC conversion, migration/restart persistence, concurrent reservations, mode switching, exclusions, rate limiting, downloaded/rescheduled episodes, Sonarr outages, ambiguous deliveries, API authentication, and shutdown.

## API references

- [Sonarr v3 API specification](https://github.com/Sonarr/Sonarr/blob/develop/src/Sonarr.Api.V3/openapi.json)
- [Sonarr episode resource, including finale metadata](https://github.com/Sonarr/Sonarr/blob/develop/src/Sonarr.Api.V3/Episodes/EpisodeResource.cs)
- [Discord webhook execution](https://docs.discord.com/developers/resources/webhook#execute-webhook)
- [Discord rate limits](https://docs.discord.com/developers/topics/rate-limits)
- [SQLx Migrator](https://docs.rs/sqlx/latest/sqlx/migrate/struct.Migrator.html)
- [Axum server lifecycle](https://docs.rs/axum/latest/axum/serve/fn.serve.html)
