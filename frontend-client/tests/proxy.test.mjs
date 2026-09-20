import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { mockCore } from './mock-core.mjs';

const listen = (server) => new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));

test('same-origin client bridge forwards API controls and handles failures', { timeout: 40_000 }, async (t) => {
	const mock = mockCore();
	const backendPort = await listen(mock.server);
	const reserve = createServer();
	const frontendPort = await listen(reserve);
	await new Promise((resolve) => reserve.close(resolve));
	const base = `http://127.0.0.1:${frontendPort}`;
	const vite = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(frontendPort), '--strictPort'], {
		cwd: new URL('..', import.meta.url),
		env: { ...process.env, SERVER_CORE_URL: `http://127.0.0.1:${backendPort}` },
		stdio: 'pipe'
	});
	let log = '';
	vite.stdout.on('data', (chunk) => log += chunk);
	vite.stderr.on('data', (chunk) => log += chunk);
	t.after(async () => {
		const stopped = once(vite, 'exit');
		if (vite.exitCode === null) { vite.kill('SIGTERM'); await stopped; }
		mock.server.closeAllConnections();
		await new Promise((resolve) => mock.server.close(resolve));
	});
	let ready = false;
	for (let attempt = 0; attempt < 100; attempt++) {
		try { ready = (await fetch(`${base}/api/shows`)).ok; } catch { /* Vite is starting. */ }
		if (ready) break;
		if (vite.exitCode !== null) throw new Error(log);
		await new Promise((resolve) => setTimeout(resolve, 100));
	}
	assert.ok(ready, log);
	const put = (path, body, origin = base) => fetch(`${base}/api/${path}`, {
		method: 'PUT', headers: { origin, 'content-type': 'application/json' }, body: JSON.stringify(body)
	});
	assert.equal((await (await fetch(`${base}/api/shows`)).json())[0].mode, 'episode');
	assert.equal((await put('shows/1/mode', { mode: 'season' })).status, 204);
	assert.equal(mock.state.shows[0].mode, 'season');
	assert.equal(mock.state.shows[1].mode, 'episode');
	assert.equal((await put('shows/1/exclusion', { excluded: true })).status, 204);
	assert.equal(mock.state.shows[0].excluded, true);
	assert.equal((await put('shows/999/exclusion', { excluded: true })).status, 404);
	assert.equal((await put('shows/1/mode', { mode: 'invalid' })).status, 422);
	assert.equal((await put('shows/1/mode', { mode: 'episode' }, 'https://other.example')).status, 403);
	assert.equal(mock.state.shows[0].mode, 'season');
	assert.equal((await fetch(`${base}/api/arbitrary-endpoint`)).status, 404);
	assert.equal((await put('settings', { mode: 'season' })).status, 404);
	assert.equal((await put('shows/999/mode', { mode: 'season' })).status, 404);
	const poster = await fetch(`${base}/api/shows/1/poster`);
	assert.equal(poster.status, 200);
	assert.equal(poster.headers.get('content-type'), 'image/jpeg');
	assert.equal(poster.headers.get('cache-control'), 'private, max-age=86400');
	assert.deepEqual(new Uint8Array(await poster.arrayBuffer()), new Uint8Array([255, 216, 255, 217]));
	assert.equal((await fetch(`${base}/api/shows/2/poster`)).headers.get('cache-control'), 'no-store');
	const page = await fetch(`${base}/api/notifications?view=upcoming&limit=3&offset=2`);
	assert.equal(page.headers.get('cache-control'), 'no-store');
	assert.equal((await page.json()).length, 3);
	assert.equal(mock.state.requests.at(-1).query, '?view=upcoming&limit=3&offset=2');
	assert.ok(mock.state.requests.every((request) => request.authorization === undefined));
	assert.equal((await fetch(`${base}/api/webhook/resume`, { method: 'POST', headers: { origin: base } })).status, 204);
	assert.equal(mock.state.paused, false);
	mock.server.closeAllConnections();
	await new Promise((resolve) => mock.server.close(resolve));
	const offline = await fetch(`${base}/api/health`);
	assert.equal(offline.status, 502);
	assert.match((await offline.json()).error, /Cannot reach Server Core/);
});
