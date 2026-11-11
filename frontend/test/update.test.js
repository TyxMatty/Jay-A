import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { HIDDEN, next, percentOf } from '../src/core/update.js';

const run = (state, ...events) => events.reduce(next, state);
const downloading = () => run(HIDDEN, { type: 'available', version: '0.3.5' }, { type: 'open' }, { type: 'start' });

test('the pill shows only for a version that is a non-empty string', () => {
  for (const bad of [undefined, null, '', '  ', 2, {}]) assert.equal(next(HIDDEN, { type: 'available', version: bad }), HIDDEN);
  assert.deepEqual(next(HIDDEN, { type: 'available', version: '0.3.5' }), { name: 'available', version: '0.3.5' });
});

test('start while downloading or installing changes nothing', () => {
  const d = downloading();
  assert.equal(d.name, 'downloading');
  assert.equal(next(d, { type: 'start' }), d);
  const i = next(d, { type: 'progress', done: 5, total: 5 });
  assert.equal(i.name, 'installing');
  assert.equal(next(i, { type: 'start' }), i);
});

test('percent is floored and clamped, and a zero total is 0', () => {
  assert.equal(percentOf(1, 3), 33);
  assert.equal(percentOf(7, 5), 100);
  assert.equal(percentOf(-1, 5), 0);
  assert.equal(percentOf(5, 0), 0);
  assert.equal(next(downloading(), { type: 'progress', done: 50, total: 200 }).percent, 25);
});

test('an error shows its code and a retry downloads again', () => {
  const e = next(downloading(), { type: 'error', code: 'call' });
  assert.deepEqual(e, { name: 'error', version: '0.3.5', code: 'call' });
  assert.equal(next(e, { type: 'start' }).name, 'downloading');
  assert.equal(next(e, { type: 'close' }).name, 'available');
  assert.equal(next(HIDDEN, { type: 'error', code: 'x' }), HIDDEN);
});

test('the commands take nothing from the page', async () => {
  const rust = await readFile(new URL('../src-tauri/src/update.rs', import.meta.url), 'utf8');
  const commands = [...rust.matchAll(/#\[tauri::command\]\s*pub (?:async )?fn (\w+)\(([^)]*)\)/g)];
  assert.deepEqual(commands.map((c) => c[1]), ['update_check', 'update_install']);
  for (const [, name, params] of commands) assert.equal(params.trim(), 'app: AppHandle', name);
  const page = await readFile(new URL('../src/ui/update.js', import.meta.url), 'utf8');
  assert.match(page, /invoke\('update_install'\)/);
  assert.match(page, /invoke\('update_check'\)/);
});
