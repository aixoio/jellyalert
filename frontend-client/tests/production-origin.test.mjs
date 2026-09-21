import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { request } from 'node:http';
import { mockCore } from './mock-core.mjs';

const listen = (server) => new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));

// Run explicitly after building; exercise adapter-node rather than Vite.
test('production proxy validates LAN and configured reverse-proxy origins', {
	skip: process.env.TEST_PRODUCTION !== '1', timeout: 30_000
}, async (t) => {
	for (const origin of ['', 'https://jellyalert.example.com']) {
		await t.test(origin || 'direct HTTP with a published port', async (t) => {
			const mock = mockCore();
			const backendPort = await listen(mock.server);
			const reserve = createServer();
			const port = await listen(reserve);
			await new Promise((resolve) => reserve.close(resolve));
			const server = spawn(process.execPath, ['build/index.js'], {
				cwd: new URL('..', import.meta.url),
				env: { ...process.env, ORIGIN: origin || undefined, HOST: '127.0.0.1', PORT: String(port), SERVER_CORE_URL: `http://127.0.0.1:${backendPort}` },
				stdio: 'pipe'
			});
			let log = '';
			server.stdout.on('data', (chunk) => log += chunk);
			server.stderr.on('data', (chunk) => log += chunk);
			t.after(async () => {
				if (server.exitCode === null) {
					const stopped = once(server, 'exit');
					server.kill('SIGTERM');
					await stopped;
				}
				mock.server.closeAllConnections();
				await new Promise((resolve) => mock.server.close(resolve));
			});
			const base = `http://127.0.0.1:${port}`;
			let ready = false;
			for (let i = 0; i < 100; i++) {
				try { ready = (await fetch(`${base}/api/health`)).ok; } catch { /* Starting. */ }
				if (ready || server.exitCode !== null) break;
				await new Promise((resolve) => setTimeout(resolve, 50));
			}
			assert.ok(ready, log);
			// Model Docker's externally published host/port, distinct from Node's port.
			for (const host of ['192.168.1.50:1589', 'jellyalert.lan:8080']) {
				const expected = origin || `http://${host}`;
				const send = (requestOrigin, path = 'notifications/episode%3A1/test', method = 'POST') => new Promise((resolve, reject) => {
					const req = request(`${base}/api/${path}`, {
						method,
						headers: { host, ...(requestOrigin === undefined ? {} : { origin: requestOrigin }), 'content-type': 'application/json' }
					}, (res) => {
						res.resume();
						res.on('end', () => resolve({ status: res.statusCode }));
					});
					req.on('error', reject);
					req.end(method === 'PUT' ? JSON.stringify({ mode: 'season' }) : undefined);
				});
				assert.equal((await send(expected)).status, 204);
				assert.equal((await send(expected, 'settings', 'PUT')).status, 204);
				for (const rejected of [undefined, 'null', 'https://other.example', 'http://localhost:1589', `${expected}:9999`]) {
					const before = mock.state.requests.length;
					assert.equal((await send(rejected)).status, 403);
					assert.equal((await send(rejected, 'settings', 'PUT')).status, 403);
					assert.equal(mock.state.requests.length, before, 'rejected requests must not reach the backend');
				}
			}
		});
	}
});
