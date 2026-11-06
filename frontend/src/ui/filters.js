// Filters: the microphone (pick it, set WebRTC's cleanup, test it with a live meter and hear it),
// Jay-C's use of it (stage 3, the Jay Link) and the effect chain (stage 2c, ui/chain-editor.js).
// The engine (crates/jay-audio) runs the audio in Rust and keeps the mic's settings, since Jay-C
// uses them with the window closed too; the page only gets the levels, about 20 times a second.
// Changing a setting reopens a running mic with it; chain changes apply live.
import { h } from './dom.js';
import { settings } from '../store/settings.js';
import { t } from '../core/i18n.js';
import { createChainEditor } from './chain-editor.js';

const isTauri = '__TAURI_INTERNALS__' in window;
const METER_FLOOR = -70;
const meterPercent = (db) => `${Math.max(0, Math.min(100, ((db - METER_FLOOR) / -METER_FLOOR) * 100))}%`;

function toggle(title, hint, onChange) {
  const input = h('input', { type: 'checkbox', role: 'switch', onChange: (e) => onChange(e.target.checked) });
  const row = h('label', { class: 'toggle-row' }, input, h('span', { class: 'toggle-text' }, h('span', { class: 'toggle-title' }, title), h('span', { class: 'hint' }, hint)));
  return { row, input };
}

/** What the page says about Jay-C, from the engine's link_state. */
function linkText(link) {
  if (!link.connected) return t("Jay-C isn't connected. In a call with Jay-A's filter, Jay-C gets this microphone with this chain.");
  if (!link.usingMic) return t('Jay-C {version} is connected.', { version: link.version });
  return [
    link.inCall ? t('Jay-C {version} is using this microphone in a call.', { version: link.version }) : t('Jay-C {version} is using this microphone.', { version: link.version }),
    link.echoCancellation ? t('Echo cancellation is on.') : t("Echo cancellation is off: Jay-C's audio wasn't found."),
    link.inCall && link.muted ? t("You're muted in Jay-C.") : '',
  ]
    .filter(Boolean)
    .join(' ');
}

export function createFiltersPanel() {
  const panel = h('section', { class: 'app-panel' }, h('h1', {}, t('Filters')));
  if (!isTauri) {
    panel.append(h('p', { class: 'hint' }, t('The microphone only works in the Jay-A app, not in the browser.')));
    return panel;
  }

  /** The engine's mic settings (config.json): { device, noiseSuppression, autoGainControl }. */
  let options = { device: null, noiseSuppression: true, autoGainControl: true };
  const micSelect = h('select', { class: 'input', id: 'filters-mic', onChange: (e) => setOptions({ device: e.target.value || null }) });
  const noise = toggle(
    t('Noise suppression'),
    t('Removes steady noise like fans and hiss. For keyboards, clicks and voices in the background, add "Noise removal (AI)" to the effect chain.'),
    (on) => setOptions({ noiseSuppression: on }),
  );
  const gain = toggle(t('Automatic gain'), t('Keeps your voice at a similar volume, near or far from the microphone.'), (on) => setOptions({ autoGainControl: on }));
  const monitor = toggle(t('Hear my microphone'), t('Use headphones: on speakers, the microphone picks itself up again and howls.'), (on) => settings.update({ monitor: on }));
  monitor.input.checked = settings.get().monitor;
  const meterFill = h('div', { class: 'mic-meter-fill' });
  const meter = h('div', { class: 'mic-meter', 'aria-hidden': 'true' }, meterFill);
  const status = h('span', { class: 'hint mic-meter-status' });
  const testButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => (testing ? stop() : start()) });
  const message = h('p', { class: 'hint', 'aria-live': 'polite' });
  const linkStatus = h('p', { class: 'hint', 'aria-live': 'polite' }, linkText({ connected: false }));

  panel.append(
    h(
      'fieldset',
      {},
      h('h3', {}, t('Microphone')),
      h('div', { class: 'field' }, h('label', { for: 'filters-mic' }, t('Microphone')), micSelect),
      noise.row,
      gain.row,
      h('div', { class: 'field' }, h('span', { class: 'field-label' }, t('Level')), meter, h('div', { class: 'row-between' }, status, testButton), message),
      monitor.row,
      linkStatus,
    ),
  );

  let invoke = null;
  let Channel = null;
  let testing = false;
  let channel = null;
  let chainEditor = null;
  let mics = [];

  const render = () => {
    testButton.textContent = testing ? t('Stop test') : t('Test microphone');
    monitor.input.disabled = !testing;
    if (!testing) {
      meterFill.style.width = '0';
      status.textContent = '';
      chainEditor?.setMeters([]);
    }
  };

  const renderOptions = () => {
    noise.input.checked = options.noiseSuppression;
    gain.input.checked = options.autoGainControl;
    micSelect.replaceChildren(
      h('option', { value: '' }, t('Windows default')),
      ...mics.map((m) => h('option', { value: m.id, selected: m.id === options.device }, m.name)),
    );
  };

  async function setOptions(patch) {
    options = { ...options, ...patch };
    renderOptions();
    try {
      await invoke('set_mic_options', { options });
      if (testing) await start(); // the reopened mic's name
    } catch (error) {
      message.textContent = t("Couldn't open the microphone: {error}", { error });
    }
  }

  async function start() {
    const mine = new Channel();
    channel = mine;
    mine.onmessage = (event) => {
      if (channel !== mine) return; // an earlier test's channel
      if (event.error) {
        message.textContent = t('The microphone stopped: {error}', { error: event.error });
        testing = false;
        render();
        return;
      }
      meterFill.style.width = meterPercent(event.level);
      status.textContent = event.level <= -90 ? t('Silence') : `${Math.round(event.level)} dB`;
      chainEditor?.setMeters(event.effects);
    };
    try {
      const device = await invoke('start_mic', { monitor: settings.get().monitor, onEvent: mine });
      message.textContent = t('Testing: {device}', { device });
      testing = true;
    } catch (error) {
      message.textContent = t("Couldn't open the microphone: {error}", { error });
      testing = false;
    }
    render();
  }

  async function stop() {
    channel = null;
    testing = false;
    message.textContent = '';
    render();
    await invoke('stop_mic').catch(() => {});
  }

  settings.subscribe((next, prev) => {
    monitor.input.checked = next.monitor;
    if (testing && next.monitor !== prev.monitor) invoke('set_monitor', { on: next.monitor }).catch((error) => (message.textContent = String(error)));
  });

  Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]).then(async ([core, event]) => {
    ({ invoke, Channel } = core);
    chainEditor = createChainEditor({
      invoke,
      webrtcFlags: () => ({ noiseSuppression: options.noiseSuppression, autoGainControl: options.autoGainControl }),
      // A preset brings its WebRTC settings: they go to the mic's (and reopen a running one).
      applyWebrtc: (flags) => {
        const patch = Object.fromEntries(Object.entries(flags).filter(([, v]) => typeof v === 'boolean'));
        if (Object.keys(patch).length) setOptions(patch);
      },
    });
    panel.append(chainEditor.element);
    // The window went to the tray: the test ended.
    event.listen('mic-stopped', () => {
      channel = null;
      testing = false;
      message.textContent = '';
      render();
    });
    // The first Jay-C to connect brought the user's settings from it.
    event.listen('mic-options', (e) => {
      options = e.payload;
      renderOptions();
    });
    event.listen('link-state', (e) => (linkStatus.textContent = linkText(e.payload)));
    invoke('link_state').then((link) => (linkStatus.textContent = linkText(link)));
    [options, mics] = await Promise.all([invoke('mic_options'), invoke('list_mics').catch(() => [])]);
    renderOptions();
  });
  render();
  return panel;
}
