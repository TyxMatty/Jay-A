// Takes the README's screenshots from the running app (scripts/dev-debug.ps1 first): the Filters
// tab with the effect chain, and its top in each style. Saves PNGs to docs/images/.
//
//   node scripts/screenshots.mjs [port]
import { writeFileSync, mkdirSync } from 'node:fs';

const port = process.argv[2] ?? '9224';
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === 'page' && /^https?:\/\/([^/]+:5175|tauri\.localhost)\//.test(t.url));
if (!page) throw new Error('no Jay-A page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => ((ws.onopen = resolve), (ws.onerror = reject)));
let id = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  pending.get(msg.id)?.(msg);
};
const send = (method, params = {}) =>
  new Promise((resolve) => {
    pending.set(++id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });
const run = async (code) => {
  const reply = await send('Runtime.evaluate', { expression: `(async () => {${code}})()`, awaitPromise: true, returnByValue: true, userGesture: true });
  if (reply.result?.exceptionDetails) throw new Error(reply.result.exceptionDetails.exception?.description);
  return reply.result?.result?.value;
};
const shot = async (file, height) => {
  await send('Emulation.setDeviceMetricsOverride', { width: 1100, height, deviceScaleFactor: 1, mobile: false });
  await new Promise((r) => setTimeout(r, 600));
  const { result } = await send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(`docs/images/${file}`, Buffer.from(result.data, 'base64'));
  console.log('saved', file);
};
const tab = (name) => `[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent.trim() === ${JSON.stringify(name)})?.click();`;
const style = (value) => `${tab('Settings')} await new Promise((r) => setTimeout(r, 300)); document.querySelector('input[name=style][value=${value}], input[value=${value}]')?.click(); await new Promise((r) => setTimeout(r, 300)); ${tab('Filters')}`;

mkdirSync('docs/images', { recursive: true });
for (const s of ['calm', 'neon', 'soft']) {
  await run(style(s));
  await run(`window.scrollTo(0, 0); document.querySelectorAll('.app-panel').forEach((p) => (p.scrollTop = 0));`);
  await shot(`filters-${s}.png`, 640);
}
// The chain, with the first effect open, in Calm.
await run(style('calm'));
await run(`const card = document.querySelectorAll('.effect-card')[1]; card?.querySelector('.effect-title')?.click(); await new Promise((r) => setTimeout(r, 300)); document.querySelector('.effect-card')?.scrollIntoView({ block: 'start' });`);
await shot('chain.png', 760);
await send('Emulation.clearDeviceMetricsOverride');
ws.close();
