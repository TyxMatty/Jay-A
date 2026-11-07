// Convert (stage 5, docs/ARQUITETURA.md section 9): files in (dropped or chosen in Windows'
// dialog, as paths), the output's format and options, then the batch with each file's progress
// and result. The engine converts (src-tauri/src/convert.rs); a bad file fails alone.
import { h } from './dom.js';
import { t } from '../core/i18n.js';
import { onFileDrop } from './file-drop.js';

const isTauri = '__TAURI_INTERNALS__' in window;

const FORMATS = [
  ['mp3', 'MP3'],
  ['ogg', 'OGG Vorbis'],
  ['opus', 'Opus'],
  ['flac', 'FLAC'],
  ['wav16', 'WAV 16-bit'],
  ['wav24', 'WAV 24-bit'],
  ['wav32', 'WAV 32-bit float'],
];
/** Quality choices per format (kbps; for OGG, Vorbis quality × 10), the default first. */
const QUALITY = {
  mp3: [[192, '192 kbps'], [128, '128 kbps'], [256, '256 kbps'], [320, '320 kbps']],
  opus: [[96, '96 kbps'], [64, '64 kbps'], [128, '128 kbps'], [192, '192 kbps']],
  ogg: [[5, 'q5 (~160 kbps)'], [3, 'q3 (~112 kbps)'], [7, 'q7 (~224 kbps)'], [9, 'q9 (~320 kbps)']],
};

const select = (id, options, onChange) => h('select', { class: 'input', id, onChange }, options.map(([value, label]) => h('option', { value }, label)));
const field = (id, label, control) => h('div', { class: 'field' }, h('label', { for: id }, label), control);
const baseName = (path) => path.split(/[\\/]/).pop();

export function createConvertPanel() {
  const panel = h('section', { class: 'app-panel' }, h('h1', {}, t('Convert')));
  if (!isTauri) {
    panel.append(h('p', { class: 'hint' }, t('Converting only works in the Jay-A app, not in the browser.')));
    return panel;
  }

  let invoke = null;
  let Channel = null;
  /** [{ path, state, progress, output, error }] */
  let files = [];
  let running = false;
  let folder = null;

  const list = h('ul', { class: 'convert-list' });
  const message = h('p', { class: 'hint', 'aria-live': 'polite' });
  const format = select('convert-format', FORMATS, () => renderQuality());
  const quality = h('select', { class: 'input', id: 'convert-quality' });
  const qualityField = field('convert-quality', t('Quality'), quality);
  const rate = select('convert-rate', [['', t('Same as the file')], ['44100', '44.1 kHz'], ['48000', '48 kHz']]);
  const channels = select('convert-channels', [['', t('Same as the file')], ['1', t('Mono')], ['2', t('Stereo')]]);
  const onExists = select('convert-exists', [['keep', t('Keep both')], ['replace', t('Replace it')], ['skip', t('Skip the file')]]);
  const normalize = h('input', { type: 'checkbox', role: 'switch' });
  const folderLabel = h('span', { class: 'hint' });
  const startButton = h('button', { type: 'button', class: 'btn', onClick: () => (running ? cancel() : start()) });
  const clearButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => ((files = []), render()) }, t('Clear list'));

  const drop = h(
    'div',
    { class: 'clip-drop' },
    h('p', { class: 'hint' }, t('Drop audio files here, or')),
    h('button', { type: 'button', class: 'btn', onClick: async () => add(await invoke('pick_audio_files')) }, t('Choose files…')),
  );
  onFileDrop(panel, drop, (paths) => add(paths));

  function add(paths) {
    if (running) return;
    const known = new Set(files.map((f) => f.path));
    files.push(...paths.filter((p) => !known.has(p)).map((path) => ({ path, state: 'waiting' })));
    render();
  }

  function renderQuality() {
    const choices = QUALITY[format.value];
    qualityField.hidden = !choices;
    quality.replaceChildren(...(choices ?? []).map(([value, label]) => h('option', { value }, label)));
    // Opus is always 48 kHz.
    rate.disabled = format.value === 'opus';
  }

  const stateText = (f) =>
    ({
      waiting: '',
      running: f.progress != null ? `${Math.round(f.progress * 100)}%` : t('Converting…'),
      done: t('Done'),
      skipped: t('Skipped: the file already exists.'),
      cancelled: t('Cancelled'),
      error: t('Failed: {error}', { error: f.error }),
    })[f.state];

  function render() {
    startButton.textContent = running ? t('Cancel') : t('Convert {n} files', { n: files.length });
    startButton.disabled = !running && !files.length;
    clearButton.disabled = running || !files.length;
    folderLabel.textContent = folder ?? t('Next to each file');
    list.replaceChildren(
      ...files.map((f) =>
        h(
          'li',
          { class: `convert-item ${f.state}` },
          h('span', { class: 'convert-name', title: f.path }, baseName(f.path)),
          h('span', { class: 'hint' }, stateText(f)),
          f.state === 'running' ? h('progress', { max: 1, value: f.progress ?? 0, 'aria-label': baseName(f.path) }) : '',
        ),
      ),
    );
  }

  async function start() {
    files = files.map((f) => ({ path: f.path, state: 'waiting' }));
    running = true;
    message.textContent = '';
    render();
    const events = new Channel();
    events.onmessage = (e) => {
      if (e.finished) {
        running = false;
        const done = files.filter((f) => f.state === 'done').length;
        const failed = files.filter((f) => f.state === 'error').length;
        message.textContent = t('{done} converted, {failed} failed.', { done, failed });
      } else {
        Object.assign(files[e.index], e);
      }
      render();
    };
    try {
      await invoke('convert_start', {
        request: {
          files: files.map((f) => f.path),
          format: format.value,
          quality: quality.value ? Number(quality.value) : null,
          rate: rate.value && !rate.disabled ? Number(rate.value) : null,
          channels: channels.value ? Number(channels.value) : null,
          normalize: normalize.checked,
          folder,
          onExists: onExists.value,
        },
        onEvent: events,
      });
    } catch (error) {
      running = false;
      message.textContent = t('Something went wrong: {error}', { error: String(error) });
      render();
    }
  }

  function cancel() {
    invoke('convert_cancel');
  }

  panel.append(
    h('fieldset', {}, h('h3', {}, t('Files')), drop, list, h('div', { class: 'row-between' }, clearButton, startButton), message),
    h(
      'fieldset',
      {},
      h('h3', {}, t('Output')),
      field('convert-format', t('Format'), format),
      qualityField,
      field('convert-rate', t('Sample rate'), rate),
      field('convert-channels', t('Channels'), channels),
      h(
        'label',
        { class: 'toggle-row' },
        normalize,
        h('span', { class: 'toggle-text' }, h('span', { class: 'toggle-title' }, t('Normalize')), h('span', { class: 'hint' }, t('Brings the loudest moment to −1 dBFS.'))),
      ),
      h(
        'div',
        { class: 'field' },
        h('span', { class: 'field-label' }, t('Save to')),
        h(
          'div',
          { class: 'row-between' },
          folderLabel,
          h('div', { class: 'button-row' },
            h('button', { type: 'button', class: 'btn btn-secondary', onClick: async () => ((folder = (await invoke('pick_folder')) ?? folder), render()) }, t('Choose folder…')),
            h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => ((folder = null), render()) }, t('Next to each file')),
          ),
        ),
      ),
      field('convert-exists', t('If the file already exists'), onExists),
    ),
  );

  import('@tauri-apps/api/core').then((core) => {
    ({ invoke, Channel } = core);
  });
  renderQuality();
  render();
  return panel;
}
