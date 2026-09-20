import { env } from '$env/dynamic/private';
import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';

// One origin for browsers in development and production. No credentials reach clients.
const proxy: RequestHandler = async ({ params, request, url, fetch }) => {
	const path = params.path;
	const allowed = request.method === 'GET'
		? ['health', 'shows', 'notifications'].includes(path)
		: request.method === 'PUT'
			? /^shows\/[1-9]\d*\/(exclusion|mode)$/.test(path)
			: request.method === 'POST' && path === 'webhook/resume';
	if (!allowed) return json({ error: 'Unknown API endpoint.' }, { status: 404 });
	if (request.method !== 'GET' && request.headers.get('origin') !== url.origin) {
		return json({ error: 'Use Jelly Alert to make this change.' }, { status: 403 });
	}
	try {
		const upstream = new URL(`/api/${path}`, env.SERVER_CORE_URL || 'http://127.0.0.1:8090');
		upstream.search = url.search;
		const response = await fetch(upstream, {
			method: request.method,
			headers: { 'content-type': 'application/json' },
			body: request.method === 'PUT' ? await request.text() : undefined,
			signal: AbortSignal.timeout(30_000),
			redirect: 'error'
		});
		return new Response(response.body, {
			status: response.status,
			headers: { 'content-type': response.headers.get('content-type') || 'application/json', 'cache-control': 'no-store' }
		});
	} catch {
		return json({ error: 'Cannot reach Server Core. Check that it is running, then retry.' }, { status: 502 });
	}
};

export const GET = proxy;
export const PUT = proxy;
export const POST = proxy;
