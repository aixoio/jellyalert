import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

const source = await readFile(new URL('../src/lib/api.ts', import.meta.url), 'utf8');
const javascript = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 }
}).outputText;
const { api } = await import(`data:text/javascript;base64,${Buffer.from(javascript).toString('base64')}`);

test('test-send network failures explain ambiguous delivery instead of inviting a retry', async (t) => {
    t.mock.method(globalThis, 'fetch', async () => { throw new TypeError('Failed to fetch'); });
    await assert.rejects(api('notifications/episode%3A1/test', { method: 'POST' }), /check Discord before sending another test/);
    await assert.rejects(api('health'), /Failed to fetch/);
});

test('test-send empty errors remain cautious and definite server errors are preserved', async (t) => {
    t.mock.method(globalThis, 'fetch', async () => new Response('', { status: 502 }));
    await assert.rejects(api('notifications/episode%3A1/test', { method: 'POST' }), /may have arrived/);
    globalThis.fetch.mock.mockImplementation(async () => Response.json({ error: 'Discord rejected the webhook.' }, { status: 502 }));
    await assert.rejects(api('notifications/episode%3A1/test', { method: 'POST' }), /Discord rejected the webhook/);
});
