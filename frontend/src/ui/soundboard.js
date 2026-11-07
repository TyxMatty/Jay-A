// Soundboard (stage 4, docs/ARQUITETURA.md section 7): the clips (import by button or by dropping
// files, play, volume, a global shortcut each, remove), where they play (Jay-C's call, the
// headphones), the soundboard's volume and "stop all". The engine keeps the library
// (src-tauri/src/soundboard.rs); every command answers with the whole state, and the page redraws.
import { h } from './dom.js';
import { t } from '../core/i18n.js';

const isTauri = '__TAURI_INTERNALS__' in window;
const MIN_DB = -30;
const MAX_DB = 6;

/** A keydown as a global shortcut ("Ctrl+Alt+KeyK"), or null while only modifiers are down. */
function shortcutOf(e) {
  if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return null;
  const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Super'].filter(Boolean);
  // Without a modifier only F1–F24: a plain letter would fire while typing anywhere.
  if (!mods.length && !/^F\d+$/.test(e.code)) return null;
  return [...mods, e.code].join('+');
}

/** "Ctrl+Alt+KeyK" → "Ctrl+Alt+K". */
const shortcutLabel = (shortcut) => shortcut.replace(/Key([A-Z])$/, '$1').replace(/Digit(\d)$/, '$1');

/** A button that records a shortcut: click, press the keys; Esc cancels, Backspace clears. */
function shortcutButton(current, save) {
  const button = h('button', { type: 'button', class: 'btn btn-secondary shortcut-btn' });
  const show = () => {
    button.textContent = current ? shortcutLabel(current) : t('Add shortcut');
    button.setAttribute('aria-label', current ? t('Shortcut: {keys}. Click to change.', { keys: shortcutLabel(current) }) : t('Add shortcut'));
  };
  const listen = (e) => {
    e.preventDefault();
    if (e.key === 'Escape') return stopListening();
    if (e.key === 'Backspace' || e.key === 'Delete') return stopListening(null);
    const shortcut = shortcutOf(e);
    if (shortcut) stopListening(shortcut);
  };
  const stopListening = (value) => {
    button.removeEventListener('keydown', listen);
    button.classList.remove('listening');
    if (value !== undefined) save(value);
    show();
  };
  button.addEventListener('click', () => {
    button.textContent = t('Press the keys…');
    button.classList.add('listening');
    button.addEventListener('keydown', listen);
  });
  button.addEventListener('blur', () => stopListening());
  show();
  return button;
}

function volumeSlider(label, db, save) {
  const value = h('span', { class: 'hint volume-value' });
  const show = (v) => (value.textContent = `${v > 0 ? '+' : ''}${Math.round(v)} dB`);
  const input = h('input', {
    type: 'range',
    min: MIN_DB,
    max: MAX_DB,
    step: 1,
    value: db,
    'aria-label': label,
    onInput: (e) => show(Number(e.target.value)),
    onChange: (e) => save(Number(e.target.value)),
  });
  show(db);
  return h('div', { class: 'volume-row' }, input, value);
}

export function createSoundboardPanel() {
  const panel = h('section', { class: 'app-panel' }, h('h1', {}, t('Soundboard')));
  if (!isTauri) {
    panel.append(h('p', { class: 'hint' }, t('The soundboard only works in the Jay-A app, not in the browser.')));
    return panel;
  }

  let invoke = null;
  let state = { clips: [], call: true, headphones: true, volume: 0, stopHotkey: null, refused: [] };
  const message = h('p', { class: 'hint', 'aria-live': 'polite' });
  const fail = (error) => (message.textContent = t('Something went wrong: {error}', { error: String(error) }));
  const apply = async (promise) => {
    try {
      state = await promise;
      render();
    } catch (error) {
      fail(error);
    }
  };

  const fileInput = h('input', { type: 'file', accept: 'audio/*,.mp3,.wav,.flac,.ogg,.m4a', multiple: true, hidden: true, onChange: (e) => importFiles(e.target.files) });
  async function importFiles(files) {
    for (const file of files) {
      message.textContent = t('Importing "{name}"…', { name: file.name });
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        state = await invoke('soundboard_import', bytes, { headers: { name: encodeURIComponent(file.name) } });
        message.textContent = t('Imported "{name}".', { name: file.name });
      } catch (error) {
        message.textContent = t("Couldn't import \"{name}\": {error}", { name: file.name, error });
      }
    }
    fileInput.value = '';
    render();
  }

  const grid = h('div', { class: 'clip-grid' });
  const drop = h(
    'div',
    {
      class: 'clip-drop',
      onDragover: (e) => (e.preventDefault(), drop.classList.add('over')),
      onDragleave: () => drop.classList.remove('over'),
      onDrop: (e) => (e.preventDefault(), drop.classList.remove('over'), importFiles(e.dataTransfer.files)),
    },
    h('p', { class: 'hint' }, t('Drop audio files here (up to 20 s each), or')),
    h('button', { type: 'button', class: 'btn', onClick: () => fileInput.click() }, t('Import…')),
    fileInput,
  );

  const call = h('input', { type: 'checkbox', role: 'switch', onChange: (e) => apply(invoke('soundboard_settings', { patch: { call: e.target.checked } })) });
  const headphones = h('input', { type: 'checkbox', role: 'switch', onChange: (e) => apply(invoke('soundboard_settings', { patch: { headphones: e.target.checked } })) });
  const toggleRow = (input, title, hint) => h('label', { class: 'toggle-row' }, input, h('span', { class: 'toggle-text' }, h('span', { class: 'toggle-title' }, title), h('span', { class: 'hint' }, hint)));
  const settingsBox = h('div');

  function clipCard(clip) {
    const refused = clip.hotkey && state.refused.includes(clip.hotkey);
    const name = h('input', {
      class: 'input clip-name',
      value: clip.name,
      maxlength: 64,
      'aria-label': t('Name'),
      onChange: (e) => apply(invoke('soundboard_update', { id: clip.id, patch: { name: e.target.value } })),
    });
    return h(
      'div',
      { class: 'clip-card' },
      h(
        'div',
        { class: 'clip-head' },
        h('button', { type: 'button', class: 'btn clip-play', 'aria-label': t('Play {name}', { name: clip.name }), onClick: () => invoke('soundboard_play', { id: clip.id }).catch(fail) }, '▶'),
        name,
        h('button', {
          type: 'button',
          class: 'btn btn-secondary',
          'aria-label': t('Remove {name}', { name: clip.name }),
          onClick: () => apply(invoke('soundboard_remove', { id: clip.id })),
        }, '✕'),
      ),
      volumeSlider(t('Volume of {name}', { name: clip.name }), clip.volume, (volume) => apply(invoke('soundboard_update', { id: clip.id, patch: { volume } }))),
      shortcutButton(clip.hotkey, (hotkey) => apply(invoke('soundboard_update', { id: clip.id, patch: { hotkey } }))),
      refused ? h('p', { class: 'hint warning' }, t('Another app already uses {keys}.', { keys: shortcutLabel(clip.hotkey) })) : '',
    );
  }

  function render() {
    call.checked = state.call;
    headphones.checked = state.headphones;
    grid.replaceChildren(...(state.clips.length ? state.clips.map(clipCard) : [h('p', { class: 'hint' }, t('No clips yet.'))]));
    const stopRefused = state.stopHotkey && state.refused.includes(state.stopHotkey);
    settingsBox.replaceChildren(
      h('div', { class: 'field' }, h('span', { class: 'field-label' }, t('Soundboard volume')), volumeSlider(t('Soundboard volume'), state.volume, (volume) => apply(invoke('soundboard_settings', { patch: { volume } })))),
      h(
        'div',
        { class: 'row-between' },
        h('button', { type: 'button', class: 'btn', onClick: () => invoke('soundboard_stop') }, t('Stop all')),
        shortcutButton(state.stopHotkey, (stopHotkey) => apply(invoke('soundboard_settings', { patch: { stopHotkey } }))),
      ),
      stopRefused ? h('p', { class: 'hint warning' }, t('Another app already uses {keys}.', { keys: shortcutLabel(state.stopHotkey) })) : '',
    );
  }

  panel.append(
    h('fieldset', {}, h('h3', {}, t('Clips')), drop, grid, message),
    h(
      'fieldset',
      {},
      h('h3', {}, t('Where it plays')),
      toggleRow(call, t('In the call'), t('While Jay-C uses this microphone. Muting in Jay-C mutes the clips too.')),
      toggleRow(headphones, t('In my headphones'), t('On speakers, echo cancellation keeps the clip from reaching the call twice.')),
      settingsBox,
    ),
  );

  import('@tauri-apps/api/core').then((core) => {
    invoke = core.invoke;
    apply(invoke('soundboard_state'));
  });
  render();
  return panel;
}
