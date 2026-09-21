# Jelly Alert

Jelly Alert watches a Sonarr library and sends Discord notifications when a monitored episode or completed season reaches its scheduled air time but is still missing from the library. A SvelteKit dashboard provides service health, per-show notification settings, upcoming alerts, delivery history, show progress, and Discord embed previews.

This repository contains both parts of the application:

| Directory | Purpose | Stack |
| --- | --- | --- |
| [`server-core`](./server-core) | Reads Sonarr, plans and delivers notifications, stores state, and exposes the JSON API | Rust, Axum, Tokio, SQLx, SQLite |
| [`frontend-client`](./frontend-client) | Trusted-LAN dashboard and same-origin API proxy | Svelte 5, SvelteKit 2, TypeScript, DaisyUI |

The browser talks only to SvelteKit. SvelteKit proxies `/api` requests to Server Core, which communicates with Sonarr, SQLite, and Discord. Jelly Alert does not contact Jellyfin or a download client, and an air time does not guarantee that a downloadable release exists.

## Features

- Every-episode or full-season notifications, configurable globally or per show
- Persistent exclusions, notification preferences, delivery history, and duplicate prevention
- Sonarr-backed show, season, episode, airing, and library progress
- Upcoming alerts and delivery history with Discord-style embed previews
- Configurable episode and season embed colors and poster artwork
- Health reporting for Sonarr scans, Discord delivery, and unresolved attempts
- SQLite migrations embedded into the backend binary
- Light and dark dashboard themes with responsive layouts

## Docker setup (recommended)

Requires Docker with Compose **2.23.1 or newer**, Sonarr, and a Discord webhook. No local Node.js or Rust installation is needed.

1. Clone this repository and open `compose.yaml`.
2. Set `sonarr_url`, `sonarr_api_key`, and `discord_webhook_url` near the bottom. Set `ORIGIN` to the exact dashboard URL you will open, such as `http://192.168.1.50:3000` for LAN access.
3. From the repository root, run:

```sh
docker compose up -d
```

Open [http://localhost:3000](http://localhost:3000), or your configured LAN URL. The first run compiles the SvelteKit frontend and Rust backend; subsequent starts reuse the images. The frontend runs as a production Node.js server. Docker restarts both services after crashes and host reboots while Docker is running.

`docker-compose up -d` also works if your installation provides that command for modern Compose; legacy Compose v1 is unsupported. Omit `-d` to watch logs in the foreground (Ctrl+C stops the services).

For Sonarr on the Docker host, use `http://host.docker.internal:8989`; Sonarr must listen on an address reachable from containers. For another machine, use its LAN address. `localhost` inside Docker refers to the container itself. Include any Sonarr URL subpath, but omit `/api/v3`.

```sh
docker compose logs -f             # View logs
docker compose up -d               # Apply compose.yaml configuration changes
docker compose up -d --build       # Rebuild after pulling source updates
docker compose down               # Stop; keep database and settings
```

SQLite, settings, and notification history persist in the `jellyalert-data` volume. **`docker compose down -v` deletes this data.** Keep credentials in `compose.yaml` private and do not commit your edited values. Escape literal `$` characters as `$$`. To change the dashboard port, update both `ports` and `ORIGIN`. Only the frontend is published; the backend is reached internally. The dashboard has no login and is for a trusted LAN.

## Local development requirements

- A running [Sonarr](https://sonarr.tv/) instance and API key
- A [Discord webhook](https://support.discord.com/hc/en-us/articles/228383668-Intro-to-Webhooks)
- A Rust toolchain that supports Rust 2024 edition
- Node.js and [pnpm](https://pnpm.io/) for the frontend

The default development ports are `8090` for Server Core and `5173` for the frontend.

## Local development quick start

Clone the repository, then configure and start the backend:

```sh
cd server-core
cp server-config.example.toml server-config.toml
```

Edit `server-config.toml` with your Sonarr URL, Sonarr API key, and Discord webhook. The Sonarr URL should be its base URL, including any reverse-proxy subpath, but without `/api/v3`.

```sh
cargo run --release -- server-config.toml
```

In a second terminal, install and start the frontend:

```sh
cd frontend-client
pnpm install
pnpm dev --host 0.0.0.0
```

Open [http://localhost:5173](http://localhost:5173). SvelteKit forwards API requests to `http://127.0.0.1:8090` by default.

To use a backend at another address:

```sh
cd frontend-client
cp .env.example .env
```

Set `SERVER_CORE_URL` in `.env`, then restart the frontend. This value is server-only and is never sent to the browser.

## Backend configuration

`server-core/server-config.toml` accepts the following settings:

| Setting | Description | Default |
| --- | --- | --- |
| `sonarr_url` | Sonarr base URL without `/api/v3` | Required |
| `sonarr_api_key` | Sonarr API key | Required |
| `discord_webhook_url` | HTTPS Discord webhook URL | Required |
| `sqlite_database_path` | SQLite database file | Required |
| `listen_address` | Backend API bind address | `127.0.0.1:8090` |
| `scan_interval_seconds` | Sonarr scan interval, from 10 to 86400 seconds | `300` |

Keep this file private because it contains credentials:

```sh
chmod 600 server-core/server-config.toml
```

Relative database paths resolve from the backend process's working directory. The database and schema are created automatically. Run only one Server Core instance against a database.

## Development and verification

Run backend checks from `server-core`:

```sh
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```

Run frontend checks from `frontend-client`:

```sh
pnpm check
pnpm test
pnpm build
```

Backend integration tests use local mock HTTP servers and temporary SQLite databases. Frontend tests use a local mock backend and isolated Vite process. Neither suite contacts a real Sonarr instance or sends Discord messages.

## How notifications work

On its first run, Jelly Alert records a tracking start time so it does not notify for an existing historical backlog. It then scans monitored Sonarr shows and plans alerts according to the default and per-show settings.

- **Episode mode** plans an alert for a missing monitored episode at its `airDateUtc`.
- **Season mode** plans one alert after the latest known episode air time, but only when Sonarr provides enough evidence that the season is complete. Specials remain episode-based.
- Immediately before delivery, Server Core rechecks Sonarr so downloaded, rescheduled, unmonitored, or excluded items are not sent incorrectly.
- Notification state and covered episodes are reserved before the Discord request to prioritize avoiding duplicate messages across failures and restarts.

The Discord webhook API cannot make its network delivery atomic with the SQLite commit. An ambiguous network failure or crash can therefore leave an alert marked `sending` or `uncertain`; Jelly Alert will not automatically resend it because that could create a duplicate. These cases appear in health and activity views.

See the [Server Core documentation](./server-core/README.md) for the complete scheduling, season-completion, retry, delivery, logging, and API behavior.

## Deployment notes

- The backend is intended to run under a process supervisor. An example systemd unit is provided at [`server-core/deploy/jelly-alert.service`](./server-core/deploy/jelly-alert.service).
- The backend binary contains its database migrations; deployment does not require the source tree or migration directory.
- `pnpm preview` is for previewing a production frontend build, not for supervising a production service.
- The frontend uses `@sveltejs/adapter-node`: build with `pnpm build`, then run `node build`. Docker Compose supervises this process.
- The API has no authentication and is designed for a trusted LAN. Do not expose the backend or dashboard directly to the public internet without adding an authenticated reverse proxy or equivalent access control.
- Backend configuration is read at startup. Restart Server Core after changing Sonarr or Discord credentials.

For component-specific details, see:

- [Server Core README](./server-core/README.md) — scheduling rules, operations, logging, API endpoints, and failure semantics
- [Frontend Client README](./frontend-client/README.md) — dashboard behavior, development, testing, and build notes

## License

Jelly Alert is available under the [MIT License](./LICENSE).
