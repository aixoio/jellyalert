# Jelly Alert client

Svelte 5 + SvelteKit 2, TypeScript, and DaisyUI 5. A trusted-LAN dashboard for the Rust service in `../server-core`. No login, new runtime dependencies, custom theme, fonts, or icon packages.

## Development

```sh
cd frontend-client
pnpm install
pnpm dev --host 0.0.0.0
```

Open port 5173 on this machine or its LAN address. Start Server Core on `127.0.0.1:8090` first. SvelteKit forwards `/api` requests to the backend, so remote browsers do not need access to port 8090. To use a different backend address, copy `.env.example` to `.env`, update `SERVER_CORE_URL`, and restart Vite. The address is a server-only environment variable.

The backend no longer requires `api_token`; older config files containing that field still work. Sonarr credentials, Discord webhook, scan interval, and listen/database paths remain in Server Core's TOML configuration and require a backend restart when changed. They are not sent to browsers.

## Controls

- **Settings:** choose episode or full-season alerts, then save. Unsubmitted edits survive automatic refreshes.
- **Shows:** search, filter, paginate, and toggle tracking. Changes save immediately. Excluded shows never create new Discord delivery attempts; prior history remains.
- **Activity:** upcoming alerts show only the current mode and active, non-excluded shows, ordered by the next air time. Delivery history includes sent, failed, unconfirmed, recorded, and already-covered attempts. Expand a message to read its plain text.
- **Status:** Sonarr scan health, Discord readiness/backoff, last attempt, tracking start, and unresolved attempts. When a webhook is disabled, repair the backend configuration and restart it, then use **Resume notifications**.

Times use the browser's local time zone. Data refreshes every 30 seconds while the page is visible, with no overlapping refreshes. Mutations disable competing controls until they finish. Failed refreshes retain previous data and display a stale-data notice. The client does not poll Sonarr directly or schedule notifications.

DaisyUI supplies all component styles and its default light/dark themes (following system preference). Tailwind is present only as DaisyUI's standard build integration; components are not restyled with Tailwind utilities. Additional CSS only arranges responsive layouts.

## Checks and production build

```sh
pnpm check
pnpm test
pnpm build
pnpm preview --host 0.0.0.0
```

`pnpm test` uses Node's built-in test runner, a local mock backend, and an isolated Vite process. No real Discord messages are sent. Browser checks can use `node tests/mock-core.mjs`, then `SERVER_CORE_URL=http://127.0.0.1:8091 pnpm dev --port 5174`; these fixtures are never used by the normal application.

The existing `adapter-auto` is retained. `pnpm preview` previews the production build; it is not a production process supervisor. For a persistent standalone Node deployment, select `@sveltejs/adapter-node` and a supervisor (that additional package has not been installed). A server runtime is required for the `/api` proxy; this client is not a static export.

References: [SvelteKit routing](https://svelte.dev/docs/kit/routing), [Svelte runes](https://svelte.dev/docs/svelte/what-are-runes), [DaisyUI SvelteKit setup](https://daisyui.com/docs/install/sveltekit/).
