import { createServer } from 'node:http';
import { pathToFileURL } from 'node:url';

export function mockCore() {
	const now = Math.floor(Date.now() / 1000);
	const state = {
		paused: true,
        colors: { episode_color: 0x5865f2, season_color: 0xf1c40f },
		mode: 'episode',
		requests: [],
		shows: Array.from({ length: 26 }, (_, index) => ({
			id: index + 1,
			mode: 'episode',
			mode_overridden: false,
			title: index === 0 ? 'The Last Lighthouse' : `Test show ${String(index + 1).padStart(2, '0')}`,
			excluded: index === 1,
			active: index !== 25
		}))
	};
	const server = createServer(async (request, response) => {
		const url = new URL(request.url, 'http://localhost');
		state.requests.push({ method: request.method, path: url.pathname, query: url.search, authorization: request.headers.authorization });
		const send = (status, data) => {
			response.writeHead(status, { 'content-type': 'application/json' });
			response.end(data === undefined ? undefined : JSON.stringify(data));
		};
		let body = '';
		for await (const chunk of request) body += chunk;
        if (url.pathname === '/api/settings/colors') {
            if (request.method === 'GET') return send(200, state.colors);
            const colors = JSON.parse(body);
            if (![colors.episode_color, colors.season_color].every((color) => Number.isInteger(color) && color >= 0 && color <= 0xffffff)) return send(422);
            state.colors = colors;
            return send(204);
        }
        if (['/api/settings', '/api/settings/reset-all'].includes(url.pathname)) {
            if (request.method === 'GET') return send(200, { mode: state.mode });
            const input = JSON.parse(body);
            if (!['episode', 'season'].includes(input.mode)) return send(422);
            state.mode = input.mode;
            for (const show of state.shows) {
                if (url.pathname.endsWith('/reset-all')) show.mode_overridden = false;
                if (!show.mode_overridden) show.mode = state.mode;
            }
            return send(204);
        }
        const modeRoute = url.pathname.match(/^\/api\/shows\/(\d+)\/mode$/);
        if (modeRoute && request.method === 'PUT') {
            const show = state.shows.find((show) => show.id === Number(modeRoute[1]));
            if (!show) return send(404, { error: 'Show not found.' });
            const input = JSON.parse(body);
            if (!['episode', 'season', 'default'].includes(input.mode)) return send(422, { error: 'Invalid mode.' });
            show.mode_overridden = input.mode !== 'default';
            show.mode = show.mode_overridden ? input.mode : state.mode;
            return send(204);
        }
		if (url.pathname === '/api/shows/1/poster') {
			response.writeHead(200, { 'content-type': 'image/jpeg' });
			return response.end(Buffer.from([255, 216, 255, 217]));
		}
        const detailRoute = url.pathname.match(/^\/api\/shows\/(\d+)$/);
        if (detailRoute) {
            const show = state.shows.find((s) => s.id === Number(detailRoute[1]));
            if (!show) return send(404, { error: 'Show not found.' });
            const counts = { total: 2, aired: 1, in_library: 1, undated: 0 };
            const notification = { season: 1, episode: null, due_at: now + 3600, awaiting_confirmation: true, episodes_remaining: 1 };
            return send(200, { show, status: 'continuing', monitored: true, as_of: now, tracking_since: now - 86400,
                counts, notification_block: null, next_notification: notification,
                next_release: { season: 1, episode: 2, title: 'The next round', air_at: now + 3600 },
                seasons: [{ number: 1, counts, completion_confirmed: false, final_air_at: null, notification,
                    episodes: [{ id: 1, number: 1, title: 'Pilot', air_at: now - 86400, has_file: true, monitored: true, notified: false },
                        { id: 2, number: 2, title: 'The next round', air_at: now + 3600, has_file: false, monitored: true, notified: false }] }]
            });
        }
		if (url.pathname === '/api/shows') return send(200, state.shows);
		const exclusion = url.pathname.match(/^\/api\/shows\/(\d+)\/exclusion$/);
		if (exclusion) {
			const show = state.shows.find((show) => show.id === Number(exclusion[1]));
			if (!show) return send(404, { error: 'Show not found.' });
			show.excluded = JSON.parse(body).excluded;
			return send(204);
		}
		if (url.pathname === '/api/health') return send(200, {
			worker: { last_scan_at: now, last_scan_succeeded: true, last_delivery_at: now - 60 },
			webhook_disabled: state.paused, webhook_retry_at: 0, tracking_since: now - 86400, unresolved_deliveries: 1
		});
		if (request.method === 'POST' && url.pathname === '/api/notifications/episode:1/test') return send(204);
		if (url.pathname === '/api/webhook/resume') { state.paused = false; return send(204); }
		if (url.pathname === '/api/notifications') {
			const history = url.searchParams.get('view') === 'history';
			const items = Array.from({ length: history ? 3 : 24 }, (_, index) => ({
				key: `episode:${index + 1}`, series_id: 1, mode: state.shows[0].mode, season: 2,
				content: `The Last Lighthouse — Season 2, episode ${index + 1} has reached its air time. Check for downloads.`,
				due_at: now + (history ? -1 : 1) * (index + 1) * 3600,
				state: history ? ['sent', 'uncertain', 'failed'][index] : 'pending',
				attempted_at: history ? now - 60 : null, sent_at: history && index === 0 ? now - 60 : null
			}));
			const offset = Number(url.searchParams.get('offset') || 0);
			return send(200, items.slice(offset, offset + Number(url.searchParams.get('limit') || 50)));
		}
		send(404, { error: 'Not found.' });
	});
	return { server, state };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
	const { server } = mockCore();
	server.listen(8091, '127.0.0.1', () => console.log('Mock Server Core on http://127.0.0.1:8091 (no external requests)'));
}
