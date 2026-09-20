import { createServer } from 'node:http';
import { pathToFileURL } from 'node:url';

export function mockCore() {
	const now = Math.floor(Date.now() / 1000);
	const state = {
		mode: 'episode',
		paused: true,
		requests: [],
		shows: Array.from({ length: 26 }, (_, index) => ({
			id: index + 1,
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
		if (url.pathname === '/api/settings') {
			if (request.method === 'PUT') {
				const input = JSON.parse(body);
				if (!['episode', 'season'].includes(input.mode)) return send(422, { error: 'Invalid mode.' });
				state.mode = input.mode;
			}
			return send(200, { mode: state.mode });
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
		if (url.pathname === '/api/webhook/resume') { state.paused = false; return send(204); }
		if (url.pathname === '/api/notifications') {
			const history = url.searchParams.get('view') === 'history';
			const items = Array.from({ length: history ? 3 : 24 }, (_, index) => ({
				key: `episode:${index + 1}`, series_id: 1, mode: state.mode, season: 2,
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
