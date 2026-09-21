# Jelly Alert client

Svelte 5 + SvelteKit 2, TypeScript, and DaisyUI 5. A trusted-LAN dashboard for the Rust service in `../server-core`. No login, custom theme, fonts, or icon packages.

## Development

```sh
cd frontend-client
pnpm install
pnpm dev --host 0.0.0.0
```

Open port 5173 on this machine or its LAN address. Start Server Core on `127.0.0.1:8090` first. SvelteKit forwards `/api` requests to the backend, so remote browsers do not need access to port 8090. To use a different backend address, copy `.env.example` to `.env`, update `SERVER_CORE_URL`, and restart Vite. The address is a server-only environment variable.

The backend no longer requires `api_token`; older config files containing that field still work. Sonarr credentials, Discord webhook, scan interval, and listen/database paths remain in Server Core's TOML configuration and require a backend restart when changed. They are not sent to browsers.

## Pages and controls

The shared navigation opens three separate SvelteKit routes: **Overview** (`/`), **Shows** (`/shows`), and **Activity** (`/activity`). Each route can be bookmarked or refreshed directly and only requests its own data. Overview contains service status and webhook recovery; Shows contains per-series preferences; Activity contains upcoming alerts and delivery history.

- **Shows:** search, filter, paginate, toggle tracking, and choose **Every episode** or **Full seasons** independently for each show. Choices save immediately and do not change other shows. New shows default to episode alerts; existing shows retain their previous global preference after migration. Excluded shows never create new Discord delivery attempts; prior history remains.
- **Activity:** upcoming alerts show only each show’s mode and active, non-excluded shows, ordered by the next air time. Delivery history includes sent, failed, unconfirmed, recorded, and already-covered attempts. Expand a message to read its plain text.
- **Status:** Sonarr scan health, Discord readiness/backoff, last attempt, tracking start, and unresolved attempts. When a webhook is disabled, repair the backend configuration and restart it, then use **Resume notifications**.

Times use the browser's local time zone. Data refreshes every 30 seconds while the page is visible, with no overlapping refreshes. Mutations disable competing controls until they finish. Failed refreshes retain previous data and display a stale-data notice. The client does not poll Sonarr directly or schedule notifications.

DaisyUI supplies all component styles and its default light/dark themes (following system preference). Tailwind is present only as DaisyUI's standard build integration; components are not restyled with Tailwind utilities. Additional CSS only arranges responsive layouts.

## Checks and production build

```sh
pnpm check
pnpm test
pnpm test:production
pnpm build
pnpm preview --host 0.0.0.0
```

`pnpm test:production` builds and checks the Node server with LAN hosts, published ports, and an explicit HTTPS origin, including rejected cross-origin mutations.

`pnpm test` uses Node's built-in test runner, a local mock backend, and an isolated Vite process. No real Discord messages are sent. Browser checks can use `node tests/mock-core.mjs`, then `SERVER_CORE_URL=http://127.0.0.1:8091 pnpm dev --port 5174`; these fixtures are never used by the normal application.

Production builds use [`@sveltejs/adapter-node`](https://svelte.dev/docs/kit/adapter-node) and run with `node build` (or `pnpm start`). Direct HTTP access accepts same-origin requests using the incoming Host header, including its port. For HTTPS reverse proxies, set `ORIGIN` to the exact browser-facing URL. Set `SERVER_CORE_URL` to the backend URL. For a complete build with automatic restarts and persistent backend data, follow the [Docker setup](../README.md#docker-setup-recommended). `pnpm preview` is only for local previews.

References: [SvelteKit routing](https://svelte.dev/docs/kit/routing), [Svelte runes](https://svelte.dev/docs/svelte/what-are-runes), [DaisyUI SvelteKit setup](https://daisyui.com/docs/install/sveltekit/).

- **Settings:** choose episode or full-season notifications for all shows using the default, including new shows. Individual choices are preserved. Use “Use default” on a show to restore inheritance. Existing shows are preserved as individual choices during upgrade.

“Reset all to default” on Settings asks for confirmation, then saves the selected default and removes all individual notification overrides, including inactive and excluded shows. Exclusions remain unchanged.

Upcoming activity includes seasons awaiting finale confirmation. These show their latest listed air time and explain why automatic delivery is blocked, rather than presenting an unconfirmed episode as the season finale.

Activity message previews start collapsed. Expand **Preview Discord embed** on an individual entry to see the Jelly Name author, notification text, and matching series poster; select it again to hide the preview. Waiting seasons use **Preview waiting notice**. Expanded previews remain open during automatic refreshes while their entry remains on the page.

Click a show’s title or poster to open its series page. It shows airing and library progress for the series and each season, expandable episode details, release countdowns, remaining episodes before a notification, and explicit warnings for unconfirmed season endings. Details refresh every 30 seconds while visible.

Settings includes separate episode and full-season color pickers with live embed samples. **Save colors** persists both colors in the server database. Activity previews use the saved colors; existing Discord messages retain their original appearance.
