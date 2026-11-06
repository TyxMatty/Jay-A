// Filters › Microphone (stage 1): pick the mic, set its cleanup, test it with a live meter and hear
// it. The engine (crates/jay-audio) captures and cleans the audio in Rust; the page only gets its
// level, 50 times a second. Changing an option during the test reopens the mic with it.
import { h } from './dom.js';
import { settings } from '../store/settings.js';
import { t } from '../core/i18n.js';

const isTauri = '__TAURI_INTERNALS__' in window;
const METER_FLOOR = -70;
const meterPercent = (db) => `${Math.max(0, Math.min(100, ((db - METER_FLOOR) / -METER_FLOOR) * 100))}%`;

function toggle(key, title, hint) {
  const input = h('input', { type: 'checkbox', role: 'switch', checked: settings.get()[key], onChange: (e) => settings.update({ [key]: e.target.checked }) });
  const row = h('label', { class: 'toggle-row' }, input, h('span', { class: 'toggle-text' }, h('span', { class: 'toggle-title' }, title), h('span', { class: 'hint' }, hint)));
  return { row, input };
}

export function createFiltersPanel() {
  const panel = h('section', { class: 'app-panel' }, h('h1', {}, t('Filters')));
  if (!isTauri) {
    panel.append(h('p', { class: 'hint' }, t('The microphone only works in the Jay-A app, not in the browser.')));
    return panel;
  }

  const micSelect = h('select', { class: 'input', id: 'filters-mic', onChange: (e) => settings.update({ micDevice: e.target.value || null }) });
  const noise = toggle('noiseSuppression', t('Noise suppression'), t('Removes steady noise like fans and hiss. Turn it off to play an instrument.'));
  const gain = toggle('autoGainControl', t('Automatic gain'), t('Keeps your voice at a similar volume, near or far from the microphone.'));
  const monitor = toggle('monitor', t('Hear my microphone'), t('Use headphones: on speakers, the microphone picks itself up again and howls.'));
  const meterFill = h('div', { class: 'mic-meter-fill' });
  const meter = h('div', { class: 'mic-meter', 'aria-hidden': 'true' }, meterFill);
  const status = h('span', { class: 'hint mic-meter-status' });
  const testButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => (testing ? stop() : start()) });
  const message = h('p', { class: 'hint', 'aria-live': 'polite' });

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
    ),
    h('div', { class: 'placeholder-card' }, h('p', {}, t('Echo cancellation arrives with the Jay-C link (stage 3); volume, sensitivity, EQ and presets in stage 2.'))),
  );

  let invoke = null;
  let Channel = null;
  let testing = false;
  let channel = null;

  const render = () => {
    testButton.textContent = testing ? t('Stop test') : t('Test microphone');
    monitor.input.disabled = !testing;
    if (!testing) {
      meterFill.style.width = '0';
      status.textContent = '';
    }
  };

  const listMics = async () => {
    const mics = await invoke('list_mics').catch(() => []);
    const wanted = settings.get().micDevice;
    micSelect.replaceChildren(
      h('option', { value: '' }, t('Windows default')),
      ...mics.map((m) => h('option', { value: m.id, selected: m.id === wanted }, m.name)),
    );
  };

  async function start() {
    const { micDevice, noiseSuppression, autoGainControl, monitor: hear } = settings.get();
    const mine = new Channel();
    channel = mine;
    mine.onmessage = (event) => {
      if (channel !== mine) return; // a reopened mic's old channel
      if (event.error) {
        message.textContent = t('The microphone stopped: {error}', { error: event.error });
        testing = false;
        render();
        return;
      }
      meterFill.style.width = meterPercent(event.level);
      status.textContent = event.level <= -90 ? t('Silence') : `${Math.round(event.level)} dB`;
    };
    try {
      const device = await invoke('start_mic', { options: { device: micDevice, noiseSuppression, autoGainControl }, monitor: hear, onEvent: mine });
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
    if (!testing) return;
    if (next.micDevice !== prev.micDevice || next.noiseSuppression !== prev.noiseSuppression || next.autoGainControl !== prev.autoGainControl) start();
    else if (next.monitor !== prev.monitor) invoke('set_monitor', { on: next.monitor }).catch((error) => (message.textContent = String(error)));
  });

  Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]).then(([core, event]) => {
    ({ invoke, Channel } = core);
    // The window went to the tray: the engine closed the mic.
    event.listen('mic-stopped', () => {
      channel = null;
      testing = false;
      message.textContent = '';
      render();
    });
    listMics();
  });
  render();
  return panel;
}
