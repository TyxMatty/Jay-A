// Filters › Effect chain (stage 2c, docs/ARQUITETURA.md section 5): the user's effects in order,
// each with on/off, its controls (built from the engine's ParamSpecs), meters, and presets.
// The engine keeps and saves the chain (src-tauri/src/audio.rs); this page draws it and sends
// the changes. Reordering works by dragging the grip or with the arrow buttons (keyboard).
import { cardTitle } from './card-title.js';
import { h } from './dom.js';
import { icons } from './icons.js';
import { t } from '../core/i18n.js';

const SAMPLE_RATE = 48000;
/** The meters' left end, dBFS. */
const METER_FLOOR = -60;
const MAX_PRESET_BYTES = 256 * 1024;

const effectName = (kind) =>
  ({
    denoise: t('Noise removal (AI)'),
    filter: t('Filter'),
    gate: t('Gate'),
    eq: t('Equalizer'),
    compressor: t('Compressor'),
    deesser: t('De-esser'),
    limiter: t('Limiter'),
  })[kind] ?? kind;

const paramLabel = (key) =>
  ({
    inputGain: t('Input gain'),
    outputGain: t('Output gain'),
    mix: t('Mix'),
    type: t('Type'),
    frequency: t('Frequency'),
    q: t('Q'),
    gain: t('Gain'),
    slope: t('Slope'),
    open: t('Opens at'),
    hysteresis: t('Hysteresis'),
    attack: t('Attack'),
    hold: t('Hold'),
    release: t('Release'),
    reduction: t('Reduction'),
    threshold: t('Threshold'),
    ratio: t('Ratio'),
    knee: t('Knee'),
    makeup: t('Makeup gain'),
    detection: t('Detection'),
    ceiling: t('Ceiling'),
    lookahead: t('Lookahead'),
    model: t('Model'),
    limit: t('Max reduction'),
  })[key] ?? key;

const choiceLabel = (label) =>
  ({
    highpass: t('High-pass'),
    lowpass: t('Low-pass'),
    lowshelf: t('Low shelf'),
    highshelf: t('High shelf'),
    bell: t('Bell'),
    notch: t('Notch'),
    off: t('Off'),
    rms: 'RMS',
    peak: t('Peak'),
    deepfilternet: 'DeepFilterNet',
    rnnoise: t('RNNoise (lighter)'),
    12: '12 dB/oct',
    24: '24 dB/oct',
  })[label] ?? label;

const presetName = (preset) =>
  (preset.builtIn && { 'clean-voice': t('Clean voice'), podcast: t('Podcast'), 'noise-only': t('Noise only'), radio: t('Radio') }[preset.id]) || preset.name;

// ---------- one parameter ----------

const decimals = (spec) => (spec.step >= 1 ? 0 : spec.step >= 0.1 ? 1 : 2);
const format = (spec, value) => Number(value).toFixed(decimals(spec));
const SLIDER = 1000;
const toSlider = (spec, v) =>
  Math.round(spec.scale === 'log' ? (Math.log(v / spec.min) / Math.log(spec.max / spec.min)) * SLIDER : ((v - spec.min) / (spec.max - spec.min)) * SLIDER);
const fromSlider = (spec, p) => {
  const v = spec.scale === 'log' ? spec.min * (spec.max / spec.min) ** (p / SLIDER) : spec.min + ((spec.max - spec.min) * p) / SLIDER;
  return Math.round(v / spec.step) * spec.step;
};

let uid = 0;

/**
 * A control for one parameter: a select for choices, else a slider (optional) and a number field,
 * plus "back to default". `send(value, persist)` resolves to the value the engine set (clamped).
 */
function paramControl(spec, value, send, { label = paramLabel(spec.key), slider = true, reset = true } = {}) {
  const id = `param-${++uid}`;
  if (spec.scale.choice) {
    const select = h(
      'select',
      { class: 'input', id, onChange: (e) => send(Number(e.target.value), true) },
      spec.scale.choice.map((c, i) => h('option', { value: i, selected: i === value }, choiceLabel(c))),
    );
    return h('div', { class: 'param' }, h('label', { for: id }, label), select);
  }
  let current = value;
  const number = h('input', { type: 'number', class: 'input param-number', id, min: spec.min, max: spec.max, step: spec.step });
  const range = slider ? h('input', { type: 'range', min: 0, max: SLIDER, step: 1, 'aria-label': label }) : null;
  const show = (v) => {
    current = v;
    number.value = format(spec, v);
    if (range) {
      range.value = String(toSlider(spec, v));
      range.setAttribute('aria-valuetext', `${format(spec, v)} ${spec.unit}`.trim());
    }
  };
  const commit = (v) => send(v, true).then(show, () => show(current));
  if (range) {
    range.addEventListener('input', () => {
      const v = fromSlider(spec, Number(range.value));
      number.value = format(spec, v);
      send(v, false).catch(() => {});
    });
    range.addEventListener('change', () => commit(fromSlider(spec, Number(range.value))));
  }
  number.addEventListener('change', () => {
    const v = Number(number.value);
    if (number.value !== '' && Number.isFinite(v)) commit(v);
    else show(current);
  });
  show(value);
  return h(
    'div',
    { class: 'param' },
    h('label', { for: id }, label),
    range,
    h('span', { class: 'param-value' }, number, h('span', { class: 'hint param-unit' }, spec.unit)),
    reset
      ? h('button', {
          type: 'button',
          class: 'icon-btn icon-btn-small',
          html: icons.reset,
          'aria-label': t('{name}: back to default', { name: label }),
          title: t('Back to default'),
          onClick: () => commit(spec.default),
        })
      : null,
  );
}

// ---------- the editor ----------

/**
 * @param {{ invoke: Function,
 *   webrtcFlags: () => { noiseSuppression: boolean, autoGainControl: boolean },
 *   applyWebrtc: (flags: { noiseSuppression?: boolean, autoGainControl?: boolean }) => void }} options
 * @returns {{ element: HTMLElement, setMeters: (effects: Array<[number, number]>) => void }}
 */
export function createChainEditor({ invoke, webrtcFlags, applyWebrtc }) {
  let catalog = null; // { common: ParamSpec[], kinds: string[], byKind: { [kind]: ParamSpec[] } }
  let chain = { slots: [], latency: 0 };
  let presets = [];
  const expanded = new Set();
  let meterBars = [];
  let dragId = null;

  const message = h('div', { class: 'hint chain-message', 'aria-live': 'polite' });
  const say = (text, details = []) => {
    message.replaceChildren(text, details.length ? h('ul', {}, details.map((d) => h('li', {}, d))) : '');
  };
  const fail = (error) => say(t('Something went wrong: {error}', { error: String(error) }));

  // ----- presets -----
  const presetSelect = h('select', { class: 'input', id: 'chain-preset', onChange: () => renderPresetButtons() });
  const loadButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => loadPreset() }, t('Load'));
  const saveButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => openSave() }, t('Save as…'));
  const importButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => fileInput.click() }, t('Import…'));
  const exportButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => exportPreset() }, t('Export'));
  const deleteButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => deletePreset() }, t('Delete'));
  const fileInput = h('input', { type: 'file', accept: '.json,application/json', hidden: true, onChange: () => importPreset() });
  const nameInput = h('input', { class: 'input', id: 'chain-preset-name', maxlength: 64, placeholder: t('Preset name') });
  const saveForm = h(
    'form',
    { class: 'preset-save', hidden: true, onSubmit: (e) => (e.preventDefault(), savePreset()) },
    h('label', { for: 'chain-preset-name', class: 'field-label' }, t('Save the chain as')),
    h('div', { class: 'input-row' }, nameInput, h('button', { type: 'submit', class: 'btn' }, t('Save')), h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => closeSave() }, t('Cancel'))),
  );

  const selected = () => presets.find((p) => p.id === presetSelect.value);
  const renderPresetButtons = () => {
    loadButton.disabled = !selected();
    deleteButton.disabled = !selected() || selected().builtIn;
  };
  const renderPresets = (selectId = presetSelect.value) => {
    const group = (label, list) => h('optgroup', { label }, list.map((p) => h('option', { value: p.id, selected: p.id === selectId }, presetName(p))));
    const mine = presets.filter((p) => !p.builtIn);
    presetSelect.replaceChildren(group(t('From Jay-A'), presets.filter((p) => p.builtIn)), mine.length ? group(t('My presets'), mine) : '');
    renderPresetButtons();
  };
  const refreshPresets = async (selectId) => {
    presets = await invoke('presets_list');
    renderPresets(selectId);
  };

  async function applyLoaded(result, doneText) {
    chain = result.state;
    applyWebrtc({ noiseSuppression: result.noiseSuppression ?? undefined, autoGainControl: result.autoGainControl ?? undefined });
    renderChain();
    say(result.warnings.length ? t('{done} Some things were skipped or fixed:', { done: doneText }) : doneText, result.warnings);
  }

  async function loadPreset() {
    const preset = selected();
    if (!preset) return;
    try {
      await applyLoaded(await invoke('chain_load', { json: preset.json }), t('Loaded "{name}".', { name: presetName(preset) }));
    } catch (error) {
      fail(error);
    }
  }

  function openSave() {
    saveForm.hidden = false;
    nameInput.value = selected() && !selected().builtIn ? selected().name : '';
    nameInput.focus();
  }
  function closeSave() {
    saveForm.hidden = true;
    saveButton.focus();
  }
  async function savePreset() {
    const name = nameInput.value.trim();
    if (!name) return nameInput.focus();
    try {
      const json = await invoke('chain_export', { name, ...webrtcFlags() });
      const id = await invoke('preset_save', { name, json });
      await refreshPresets(id);
      saveForm.hidden = true;
      presetSelect.focus();
      say(t('Saved "{name}".', { name }));
    } catch (error) {
      fail(error);
    }
  }

  async function importPreset() {
    const file = fileInput.files[0];
    fileInput.value = '';
    if (!file) return;
    if (file.size > MAX_PRESET_BYTES) return say(t('This file is too big to be a preset.'));
    try {
      const json = await file.text();
      const result = await invoke('chain_load', { json });
      let name = file.name.replace(/\.json$/i, '');
      try {
        name = JSON.parse(json).name || name;
      } catch {
        // chain_load already checked it; keep the file name
      }
      const id = await invoke('preset_save', { name, json });
      await refreshPresets(id);
      await applyLoaded(result, t('Imported "{name}".', { name }));
    } catch (error) {
      fail(error);
    }
  }

  async function exportPreset() {
    const preset = selected();
    const name = preset && !preset.builtIn ? preset.name : 'Jay-A';
    try {
      const json = await invoke('chain_export', { name, ...webrtcFlags() });
      const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
      const link = h('a', { href: url, download: `${name.replace(/[^\w-]+/g, '-') || 'preset'}.json`, hidden: true });
      document.body.append(link);
      link.click();
      link.remove();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      say(t('Exported to your Downloads folder.'));
    } catch (error) {
      fail(error);
    }
  }

  async function deletePreset() {
    const preset = selected();
    if (!preset || preset.builtIn || !confirm(t('Delete the preset "{name}"?', { name: preset.name }))) return;
    try {
      await invoke('preset_delete', { id: preset.id });
      await refreshPresets('');
      say(t('Deleted "{name}".', { name: preset.name }));
    } catch (error) {
      fail(error);
    }
  }

  // ----- the chain -----
  const list = h('ol', { class: 'chain' });
  const empty = h('p', { class: 'hint' }, t('The chain is empty: only WebRTC cleans the microphone. Add an effect or load a preset.'));
  const latency = h('p', { class: 'hint chain-latency' });
  const addSelect = h('select', { class: 'input', id: 'chain-add', 'aria-label': t('Effect to add') });
  const addButton = h('button', { type: 'button', class: 'btn btn-secondary', html: icons.plus, onClick: () => add() });
  addButton.append(t('Add'));

  async function edit(op, args, focus) {
    try {
      chain = await invoke('chain_edit', { op, ...args });
      renderChain(focus);
    } catch (error) {
      fail(error);
    }
  }

  function add() {
    const kind = addSelect.value;
    // The new card opens (its title's click), and the focus goes to it.
    edit('add', { kind }, () => {
      const title = list.lastElementChild?.querySelector('.effect-title');
      title?.click();
      title?.focus();
    });
  }

  /** The cards' titles: the effect's name, numbered when there's more than one of it. */
  function titles() {
    const seen = {};
    const totals = {};
    for (const s of chain.slots) totals[s.kind] = (totals[s.kind] ?? 0) + 1;
    return chain.slots.map((s) => {
      seen[s.kind] = (seen[s.kind] ?? 0) + 1;
      return totals[s.kind] > 1 ? `${effectName(s.kind)} ${seen[s.kind]}` : effectName(s.kind);
    });
  }

  function card(slot, index, title) {
    const specs = [...catalog.common, ...catalog.byKind[slot.kind]];
    const valueOf = (key) => slot.values[specs.findIndex((s) => s.key === key)];
    const send = (key) => async (value, persist) => {
      const set = await invoke('chain_set_param', { id: slot.id, key, value, persist }).catch((error) => {
        fail(error);
        throw error;
      });
      slot.values[specs.findIndex((s) => s.key === key)] = set;
      return set;
    };
    const control = (spec, options) => paramControl(spec, valueOf(spec.key), send(spec.key), options);

    const bodyId = `effect-body-${++uid}`;
    const body = h('div', { class: 'effect-body', id: bodyId, hidden: !expanded.has(slot.id) });
    const own = catalog.byKind[slot.kind];
    if (slot.kind === 'eq') {
      for (let band = 0; band < own.length / 4; band++) {
        const [kind, freq, gain, q] = own.slice(band * 4, band * 4 + 4);
        body.append(
          h(
            'fieldset',
            { class: 'eq-band' },
            h('h4', {}, t('Band {n}', { n: band + 1 })),
            control(kind, { label: t('Type') }),
            control(freq, { label: t('Frequency'), slider: false, reset: false }),
            control(gain, { label: t('Gain'), reset: false }),
            control(q, { label: t('Q'), slider: false, reset: false }),
          ),
        );
      }
    } else {
      body.append(...own.map((spec) => control(spec)));
    }
    body.append(
      h('div', { class: 'effect-common' }, ...catalog.common.filter((s) => s.key !== 'enabled').map((spec) => control(spec))),
    );

    const enabled = valueOf('enabled') === 1;
    const li = h('li', { class: `effect-card${enabled ? '' : ' is-off'}`, dataset: { id: slot.id } });
    const titleButton = h(
      'button',
      {
        type: 'button',
        class: 'effect-title',
        'aria-expanded': String(!body.hidden),
        'aria-controls': bodyId,
        onClick: () => {
          body.hidden = !body.hidden;
          titleButton.setAttribute('aria-expanded', String(!body.hidden));
          if (body.hidden) expanded.delete(slot.id);
          else expanded.add(slot.id);
        },
      },
      title,
    );
    const toggle = h('input', {
      type: 'checkbox',
      role: 'switch',
      checked: enabled,
      'aria-label': t('{name} on', { name: title }),
      onChange: (e) => {
        li.classList.toggle('is-off', !e.target.checked);
        send('enabled')(e.target.checked ? 1 : 0, true).catch(() => {});
      },
    });
    const meterIn = h('span', { class: 'meter-fill' });
    const meterOut = h('span', { class: 'meter-fill' });
    meterBars[index] = [meterIn, meterOut];
    const action = (name, icon, label, onClick, disabled = false) =>
      h('button', { type: 'button', class: 'icon-btn icon-btn-small', dataset: { action: name }, html: icon, 'aria-label': label, title: label, disabled, onClick });
    const last = chain.slots.length - 1;
    const grip = h('span', { class: 'effect-grip', html: icons.grip, draggable: 'true', title: t('Drag to reorder'), 'aria-hidden': 'true' });
    grip.addEventListener('dragstart', (e) => {
      dragId = slot.id;
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', slot.id);
      e.dataTransfer.setDragImage(li, 16, 16);
    });
    grip.addEventListener('dragend', () => {
      dragId = null;
      clearDropMarks();
    });
    li.append(
      h(
        'div',
        { class: 'effect-head' },
        grip,
        h('label', { class: 'toggle-row effect-switch' }, toggle),
        titleButton,
        h('span', { class: 'effect-meters', 'aria-hidden': 'true', title: t('In / out') }, h('span', { class: 'meter' }, meterIn), h('span', { class: 'meter' }, meterOut)),
        action('up', icons.arrowUp, t('Move {name} up', { name: title }), () => move(slot.id, index - 1, 'up'), index === 0),
        action('down', icons.arrowDown, t('Move {name} down', { name: title }), () => move(slot.id, index + 1, 'down'), index === last),
        action('duplicate', icons.copy, t('Duplicate {name}', { name: title }), () => edit('duplicate', { id: slot.id })),
        action('remove', icons.trash, t('Remove {name}', { name: title }), () => {
          const next = chain.slots[index + 1] ?? chain.slots[index - 1];
          edit('remove', { id: slot.id }, () => (next ? focusAction(next.id, 'remove') : addSelect.focus()));
        }),
      ),
      body,
    );
    return li;
  }

  function focusAction(id, name) {
    const card = [...list.children].find((c) => c.dataset.id === id);
    const button = card?.querySelector(`[data-action="${name}"]`);
    (button && !button.disabled ? button : card?.querySelector('[data-action]:not(:disabled)'))?.focus();
  }

  function move(id, to, button) {
    edit('move', { id, to }, () => focusAction(id, button));
  }

  // Dragging: drop above or below the card under the pointer.
  const clearDropMarks = () => list.querySelectorAll('.drop-before, .drop-after').forEach((c) => c.classList.remove('drop-before', 'drop-after'));
  const dropTarget = (e) => {
    const li = e.target.closest?.('.effect-card');
    if (!li) return null;
    const box = li.getBoundingClientRect();
    return { li, after: e.clientY > box.top + box.height / 2 };
  };
  list.addEventListener('dragover', (e) => {
    const target = dragId && dropTarget(e);
    if (!target) return;
    e.preventDefault();
    clearDropMarks();
    target.li.classList.add(target.after ? 'drop-after' : 'drop-before');
  });
  list.addEventListener('drop', (e) => {
    const target = dragId && dropTarget(e);
    clearDropMarks();
    if (!target) return;
    e.preventDefault();
    const ids = chain.slots.map((s) => s.id);
    const from = ids.indexOf(dragId);
    let to = ids.indexOf(target.li.dataset.id) + (target.after ? 1 : 0);
    if (from < to) to -= 1;
    if (to !== from) move(dragId, to, 'up');
  });

  function renderChain(afterRender) {
    meterBars = [];
    const names = titles();
    list.replaceChildren(...chain.slots.map((slot, i) => card(slot, i, names[i])));
    empty.hidden = chain.slots.length > 0;
    const ms = ((chain.latency / SAMPLE_RATE) * 1000).toLocaleString(document.documentElement.lang, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
    latency.textContent = t('Latency: {ms} ms', { ms });
    afterRender?.();
  }

  const percent = (peak) => {
    const db = peak > 0 ? 20 * Math.log10(peak) : -Infinity;
    return `${Math.max(0, Math.min(100, ((db - METER_FLOOR) / -METER_FLOOR) * 100))}%`;
  };

  const element = h(
    'fieldset',
    { class: 'chain-editor' },
    cardTitle(icons.sliders, t('Effect chain')),
    h(
      'div',
      { class: 'field' },
      h('label', { for: 'chain-preset' }, t('Preset')),
      h('div', { class: 'input-row' }, presetSelect, loadButton),
      h('div', { class: 'button-row' }, saveButton, importButton, exportButton, deleteButton, fileInput),
      saveForm,
    ),
    message,
    empty,
    list,
    h('div', { class: 'input-row chain-add' }, addSelect, addButton),
    latency,
  );

  Promise.all([invoke('effects_catalog'), invoke('chain_state'), invoke('presets_list')])
    .then(([cat, state, list]) => {
      catalog = {
        common: cat.common,
        kinds: cat.effects.map((e) => e.kind),
        byKind: Object.fromEntries(cat.effects.map((e) => [e.kind, e.params])),
      };
      addSelect.replaceChildren(...catalog.kinds.map((k) => h('option', { value: k }, effectName(k))));
      chain = state;
      presets = list;
      renderPresets();
      renderChain();
    })
    .catch(fail);

  return {
    element,
    setMeters(effects) {
      meterBars.forEach(([inBar, outBar], i) => {
        const [input, output] = effects[i] ?? [0, 0];
        inBar.style.width = percent(input);
        outBar.style.width = percent(output);
      });
    },
  };
}
