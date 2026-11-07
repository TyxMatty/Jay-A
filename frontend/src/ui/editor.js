// Editor (stage 6, docs/ARQUITETURA.md section 8): one audio file, its waveform, a selection (drag
// on the waveform, or type start and end), the edits, undo, a preview in the headphones and the
// export. The engine keeps the original untouched and the edits as a list (src-tauri/src/editor.rs);
// every command answers with the new view, and the page redraws.
import { cardTitle } from './card-title.js';
import { icons } from './icons.js';
import { h } from './dom.js';
import { t } from '../core/i18n.js';
import { onFileDrop } from './file-drop.js';

const isTauri = '__TAURI_INTERNALS__' in window;
const FORMATS = [
  ['wav16', 'WAV 16-bit'],
  ['wav24', 'WAV 24-bit'],
  ['wav32', 'WAV 32-bit float'],
  ['flac', 'FLAC'],
  ['mp3', 'MP3 192 kbps'],
  ['ogg', 'OGG Vorbis'],
  ['opus', 'Opus 96 kbps'],
];

export function createEditorPanel() {
  const panel = h('section', { class: 'app-panel editor-panel' }, h('h1', {}, t('Editor')));
  if (!isTauri) {
    panel.append(h('p', { class: 'hint' }, t('The editor only works in the Jay-A app, not in the browser.')));
    return panel;
  }

  let invoke = null;
  /** { name, rate, channels, frames, edits, peaks } or null */
  let view = null;
  /** Selected frames, or null for none (the operations then take the whole file). */
  let selection = null;
  let dragFrom = null;

  const message = h('p', { class: 'hint', 'aria-live': 'polite' });
  const fail = (error) => (message.textContent = t('Something went wrong: {error}', { error: String(error) }));
  const canvas = h('canvas', { class: 'editor-wave', height: 160, 'aria-label': t('Waveform') });
  const info = h('p', { class: 'hint' });
  const start = h('input', { type: 'number', class: 'input', id: 'editor-start', min: 0, step: 0.01, onChange: () => typed() });
  const end = h('input', { type: 'number', class: 'input', id: 'editor-end', min: 0, step: 0.01, onChange: () => typed() });
  const gainDb = h('input', { type: 'number', class: 'input', id: 'editor-gain', value: 3, min: -30, max: 30, step: 1 });
  const format = h('select', { class: 'input', id: 'editor-format' }, FORMATS.map(([value, label]) => h('option', { value }, label)));
  const columns = () => Math.max(100, Math.round(canvas.clientWidth || 800));
  const seconds = (frames) => (view ? frames / view.rate : 0);

  async function open(path) {
    // Shown first, so columns() measures the real width.
    editing.hidden = false;
    message.textContent = t('Opening…');
    try {
      view = await invoke('editor_open', { path, columns: columns() });
      selection = null;
      message.textContent = '';
    } catch (error) {
      message.textContent = t("Couldn't open it: {error}", { error });
    }
    render();
  }

  /** Runs an edit on the selection (or everything) and redraws. */
  async function edit(op, extra = {}) {
    const [from, to] = selection ?? [0, view.frames];
    try {
      view = await invoke('editor_edit', { edit: { op, from, to, ...extra }, columns: columns() });
      // After a cut the old positions mean something else: start over.
      selection = op === 'delete' || op === 'keep' || op === 'trimSilence' ? null : selection;
      message.textContent = '';
    } catch (error) {
      fail(error);
    }
    render();
  }

  async function undo() {
    try {
      view = await invoke('editor_undo', { columns: columns() });
      selection = null;
    } catch (error) {
      fail(error);
    }
    render();
  }

  function typed() {
    if (!view) return;
    const a = Math.round(Number(start.value) * view.rate);
    const b = Math.round(Number(end.value) * view.rate);
    selection = b > a ? [Math.max(0, a), Math.min(view.frames, b)] : null;
    render();
  }

  // Selecting with the mouse (or pen, or touch).
  const frameAt = (e) => {
    const r = canvas.getBoundingClientRect();
    return Math.round(Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) * view.frames);
  };
  canvas.addEventListener('pointerdown', (e) => {
    if (!view) return;
    canvas.setPointerCapture(e.pointerId);
    dragFrom = frameAt(e);
    selection = null;
    render();
  });
  canvas.addEventListener('pointermove', (e) => {
    if (dragFrom == null) return;
    const at = frameAt(e);
    selection = at === dragFrom ? null : [Math.min(dragFrom, at), Math.max(dragFrom, at)];
    render();
  });
  canvas.addEventListener('pointerup', () => (dragFrom = null));

  function draw() {
    const width = canvas.clientWidth || 800;
    const ratio = devicePixelRatio || 1;
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(160 * ratio);
    const g = canvas.getContext('2d');
    g.scale(ratio, ratio);
    const style = getComputedStyle(document.documentElement);
    const accent = style.getPropertyValue('--accent').trim() || '#7048e8';
    g.clearRect(0, 0, width, 160);
    if (!view) return;
    if (selection) {
      const [a, b] = selection.map((f) => (f / view.frames) * width);
      g.fillStyle = `color-mix(in srgb, ${accent} 22%, transparent)`;
      g.fillRect(a, 0, Math.max(1, b - a), 160);
    }
    g.fillStyle = accent;
    const step = width / view.peaks.length;
    view.peaks.forEach(([lo, hi], i) => {
      const y1 = 80 - hi * 76;
      const y2 = 80 - lo * 76;
      g.fillRect(i * step, y1, Math.max(1, step - 0.5), Math.max(1, y2 - y1));
    });
  }

  const button = (label, onClick, needsSelection = false) => {
    const b = h('button', { type: 'button', class: 'btn btn-secondary', onClick });
    b.textContent = label;
    b.dataset.selection = needsSelection ? '1' : '';
    return b;
  };
  const tools = [
    button(t('Play'), () => invoke('editor_play', { from: selection?.[0] ?? null, to: selection?.[1] ?? null }).catch(fail)),
    button(t('Stop'), () => invoke('editor_stop').catch(fail)),
    button(t('Delete selection'), () => edit('delete'), true),
    button(t('Keep only the selection'), () => edit('keep'), true),
    button(t('Fade in'), () => edit('fadeIn')),
    button(t('Fade out'), () => edit('fadeOut')),
    button(t('Normalize'), () => edit('normalize')),
    button(t('Reverse'), () => edit('reverse')),
    button(t('Trim silence'), () => edit('trimSilence', { db: -50 })),
  ];
  const undoButton = button(t('Undo'), () => undo());
  const gainButton = button(t('Apply gain'), () => edit('gain', { db: Number(gainDb.value) || 0 }));
  const exportButton = h('button', {
    type: 'button',
    class: 'btn',
    onClick: async () => {
      try {
        const path = await invoke('editor_export', { format: format.value, quality: null });
        if (path) message.textContent = t('Saved to {path}.', { path });
      } catch (error) {
        fail(error);
      }
    },
  }, t('Export…'));

  function render() {
    const has = !!view;
    // Without a file, only the drop zone and the hint: no empty waveform, no greyed-out tools.
    editing.hidden = !has;
    for (const b of [...tools, gainButton, exportButton]) b.disabled = !has || (b.dataset.selection === '1' && !selection);
    undoButton.disabled = !has || !view.edits;
    if (has) {
      const sel = selection ? t('Selection: {from} s to {to} s.', { from: seconds(selection[0]).toFixed(2), to: seconds(selection[1]).toFixed(2) }) : t('No selection: the edits take the whole file.');
      info.textContent = `${view.name} · ${seconds(view.frames).toFixed(2)} s · ${t('{n} edits', { n: view.edits })} · ${sel}`;
      start.value = selection ? seconds(selection[0]).toFixed(2) : '';
      end.value = selection ? seconds(selection[1]).toFixed(2) : '';
    } else {
      info.textContent = t('Open an audio file (up to 30 minutes) to edit it. The original is never changed.');
    }
    draw();
  }

  const drop = h(
    'div',
    { class: 'clip-drop' },
    h('p', { class: 'hint' }, t('Drop an audio file here, or')),
    h('button', { type: 'button', class: 'btn', onClick: async () => { const [path] = await invoke('pick_audio_files'); if (path) open(path); } }, t('Open file…')),
  );
  onFileDrop(panel, drop, (paths) => open(paths[0]));

  const editing = h(
    'div',
    { class: 'editor-editing' },
    canvas,
    h(
      'div',
      { class: 'editor-range' },
      h('div', { class: 'field' }, h('label', { for: 'editor-start' }, t('Start (s)')), start),
      h('div', { class: 'field' }, h('label', { for: 'editor-end' }, t('End (s)')), end),
    ),
    h('div', { class: 'button-row' }, ...tools),
    h('div', { class: 'button-row' }, h('div', { class: 'field' }, h('label', { for: 'editor-gain' }, t('Gain (dB)')), gainDb), gainButton, undoButton),
  );

  panel.append(
    h(
      'fieldset',
      { class: 'editor-main' },
      cardTitle(icons.waveform, t('Audio')),
      drop,
      info,
      editing,
      message,
    ),
    h('fieldset', {}, cardTitle(icons.download, t('Export')), h('div', { class: 'field' }, h('label', { for: 'editor-format' }, t('Format')), format), exportButton),
  );

  addEventListener('resize', () => {
    if (view && !panel.hidden) invoke('editor_view', { columns: columns() }).then((v) => ((view = v), render()));
  });
  import('@tauri-apps/api/core').then((core) => {
    invoke = core.invoke;
    render();
  });
  render();
  return panel;
}
