import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

test('the release asset is the Jay-I setup, built from a pinned Jay-I', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/installer.yml', import.meta.url), 'utf8');
  for (const [, ref] of workflow.matchAll(/^\s*-?\s*uses:\s*\S+@(\S+)/gm)) {
    assert.match(ref, /^[0-9a-f]{40}$/, `every action is pinned by full SHA, got ${ref}`);
  }
  const pin = workflow.match(/repository: TyxMatty\/Jay-I\s+ref: (\S+)/);
  assert.ok(pin, 'the workflow checks out TyxMatty/Jay-I');
  assert.match(pin[1], /^[0-9a-f]{40}$/);
  assert.match(workflow, /jay-i\/scripts\/build-setup\.ps1/);
  // RNF-12: the installer is up to 20 MB (the NSIS alone is 13.5 MB, so the setup can't fit 15).
  assert.match(workflow, /20 \* 1024 \* 1024/);
  // Only the setup is uploaded: the hub takes any asset ending in _x64-setup.exe.
  const upload = workflow.match(/upload-artifact@[\s\S]*?\n\s+path: (\S+)/);
  assert.ok(upload, 'the workflow uploads an artifact');
  assert.doesNotMatch(upload[1], /bundle\/nsis/);
  assert.match(upload[1], /^setup-out\//);
  // The updater (the jay-i crate) and the setup come from the same Jay-I commit.
  const cargo = readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
  const dep = cargo.match(/^jay-i\s*=.*$/m);
  assert.ok(dep, 'Cargo.toml depends on jay-i');
  assert.ok(dep[0].includes(`rev = "${pin[1]}"`), `Cargo.toml jay-i rev must be ${pin[1]}`);
});

test('the installed page talks to no web address but the IPC bridge', () => {
  const conf = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  const connect = String(conf.app.security.csp['connect-src']);
  const addresses = connect.match(/https?:\/\/\S+/g) ?? [];
  assert.deepEqual(addresses.filter((a) => a !== 'http://ipc.localhost'), []);
});
