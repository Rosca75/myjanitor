// Contract tests for the bridge: run with `node --test ui/`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import api, { COMMANDS, EVENTS, call, on, backendKind, useMock } from './api.js';

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, '..', '..');

test('api.js lists exactly the commands and events of docs/IPC.md, in order', () => {
  const doc = readFileSync(join(repo, 'docs', 'IPC.md'), 'utf8');
  const headings = doc.split('\n').filter((l) => l.startsWith('### `')).map((l) => l.slice(5).split('`')[0]);
  assert.deepEqual(headings, [...COMMANDS, ...EVENTS]);
});

test('every command has a camelCase function', () => {
  for (const cmd of COMMANDS) {
    const name = cmd.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
    assert.equal(typeof api[name], 'function', name);
  }
});

test('the mock backend serves start-up commands and rejects the rest like the engine', async () => {
  useMock(true);
  assert.equal(await backendKind(), 'mock');
  const info = await api.appInfo();
  assert.equal(info.bundle_id, 'com.myjanitor.app');
  const scope = await api.scopeAddRoot({ path: '/home/me' });
  assert.deepEqual(scope.roots, ['/home/me']);
  await assert.rejects(call('plan_apply', { confirmations: [] }), (e) => e.code === 'not_implemented');
  await assert.rejects(call('no_such_command'), (e) => e.code === 'invalid_input');
  const off = await on('scan://progress', () => {});
  assert.equal(typeof off, 'function');
});

test('no colour literal outside ui/styles/tokens.css', () => {
  const literal = /#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/;
  const offenders = [];
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const p = join(dir, name);
      if (statSync(p).isDirectory()) walk(p);
      else if (/\.(css|js|html)$/.test(name) && !name.endsWith('.test.js') && p !== join(here, '..', 'styles', 'tokens.css')) {
        readFileSync(p, 'utf8').split('\n').forEach((line, i) => {
          if (literal.test(line)) offenders.push(`${p}:${i + 1}`);
        });
      }
    }
  };
  walk(join(here, '..'));
  assert.deepEqual(offenders, []);
});
