// The window: top bar, then the sections (Filters, Soundboard, Editor, Convert, Settings) as tabs.
// The audio sections are placeholders until their stage in docs/ARQUITETURA.md.
import { h } from './dom.js';
import { icons } from './icons.js';
import { createTabs } from './tabs.js';
import { toggleTheme, currentTheme } from './theme.js';
import { settings } from '../store/settings.js';
import { resolveStyle } from '../core/style.js';
import { t } from '../core/i18n.js';

/** Each language in its own name, so it can be found whichever one is showing. */
const LANGUAGES = [
  ['system', null],
  ['en', 'English'],
  ['pt-BR', 'Português (Brasil)'],
];

function createThemeToggle() {
  const button = h('button', { type: 'button', class: 'icon-btn' });
  const render = () => {
    const dark = currentTheme() === 'dark';
    button.innerHTML = dark ? icons.sun : icons.moon;
    button.setAttribute('aria-label', dark ? t('Switch to light mode') : t('Switch to dark mode'));
    button.title = button.getAttribute('aria-label');
  };
  button.addEventListener('click', toggleTheme);
  // The theme can also change from Settings or the OS.
  new MutationObserver(render).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  render();
  return button;
}

/**
 * Radios for the setting `key`, kept checked on its current value, also when it changes elsewhere
 * (the header's theme button). `renderOption(input, value, label)` builds each option's <label>.
 */
function radioGroup(key, attrs, options, renderOption) {
  const group = h(
    'div',
    { role: 'radiogroup', ...attrs },
    options.map(([value, label]) =>
      renderOption(h('input', { type: 'radio', name: key, value, onChange: () => settings.update({ [key]: value }) }), value, label),
    ),
  );
  const sync = () => {
    const current = key === 'style' ? resolveStyle(settings.get().style) : settings.get()[key];
    for (const input of group.querySelectorAll('input')) input.checked = input.value === current;
  };
  settings.subscribe(sync);
  sync();
  return group;
}

function createSettingsPanel() {
  // Each style option is a small preview drawn with that style's own tokens (styles/theme.css).
  const styleGroup = radioGroup(
    'style',
    { class: 'style-picker', 'aria-labelledby': 'style-label' },
    [['calm', t('Calm')], ['neon', t('Neon')], ['soft', t('Soft')]],
    (input, value, label) =>
      h(
        'label',
        { class: 'style-option' },
        input,
        h(
          'span',
          { class: 'style-preview', 'data-style-preview': value, 'aria-hidden': 'true' },
          h('span', { class: 'style-preview-tile' }, h('span', { class: 'style-preview-avatar' })),
          h('span', { class: 'style-preview-bar' }, h('span', {}), h('span', {}), h('span', { class: 'is-danger' })),
        ),
        h('span', { class: 'style-option-name' }, label),
      ),
  );
  const themeGroup = radioGroup(
    'theme',
    { class: 'segmented', 'aria-labelledby': 'theme-label' },
    [['system', t('System')], ['light', t('Light')], ['dark', t('Dark')]],
    (input, value, label) => h('label', {}, input, label),
  );
  const languageSelect = h(
    'select',
    { class: 'input', id: 'settings-language', 'aria-describedby': 'language-hint', onChange: (e) => settings.update({ language: e.target.value }) },
    LANGUAGES.map(([value, name]) => h('option', { value, selected: settings.get().language === value }, name ?? t('System'))),
  );

  return h(
    'section',
    { class: 'app-panel' },
    h('h1', {}, t('Settings')),
    h(
      'fieldset',
      {},
      h('h3', {}, t('Appearance')),
      h('div', { class: 'field' }, h('span', { class: 'field-label', id: 'style-label' }, t('Style')), styleGroup),
      h('div', { class: 'field' }, h('span', { class: 'field-label', id: 'theme-label' }, t('Theme')), themeGroup),
      h(
        'div',
        { class: 'field' },
        h('label', { for: 'settings-language' }, t('Language')),
        languageSelect,
        h('p', { class: 'hint', id: 'language-hint' }, t('Jay-A reloads to change the language.')),
      ),
    ),
    h(
      'fieldset',
      {},
      h('h3', {}, t('About')),
      h(
        'div',
        { class: 'field' },
        h('p', { class: 'hint' }, t('Jay-A {version}', { version: __APP_VERSION__ })),
        h('p', { class: 'hint' }, t('Closing the window keeps Jay-A in the tray, next to the clock. To quit, right-click its icon there.')),
      ),
    ),
  );
}

function placeholder(title, text, stage) {
  return h(
    'section',
    { class: 'app-panel' },
    h('h1', {}, title),
    h('div', { class: 'placeholder-card' }, h('p', {}, text), h('p', { class: 'hint' }, t('Coming in stage {n}.', { n: stage }))),
  );
}

const tabLabel = (icon, text) => h('span', { class: 'tab-label' }, h('span', { class: 'tab-icon', html: icon }), text);

export function mountApp(root) {
  const items = [
    {
      key: 'filters',
      label: tabLabel(icons.mic, t('Filters')),
      panel: placeholder(t('Filters'), t('Echo, noise and gain cleanup, volume, sensitivity, EQ and presets for your microphone. Jay-C will use them in calls.'), 1),
    },
    {
      key: 'soundboard',
      label: tabLabel(icons.pads, t('Soundboard')),
      panel: placeholder(t('Soundboard'), t('Clips that play in the call and in your headphones, with a click or a global shortcut.'), 4),
    },
    {
      key: 'editor',
      label: tabLabel(icons.waveform, t('Editor')),
      panel: placeholder(t('Editor'), t('Cut, fade, normalize and filter audio files, without touching the original.'), 6),
    },
    {
      key: 'convert',
      label: tabLabel(icons.convert, t('Convert')),
      panel: placeholder(t('Convert'), t('Convert audio files between WAV, FLAC, MP3, OGG and Opus, in batches.'), 5),
    },
    { key: 'settings', label: tabLabel(icons.settings, t('Settings')), panel: createSettingsPanel() },
  ];
  const tabs = createTabs({ idPrefix: 'section', label: t('Sections'), className: 'app-nav', vertical: true, items });

  const header = h(
    'header',
    { class: 'topbar' },
    h('div', { class: 'brand' }, h('span', { class: 'brand-mark wolf-mark', 'aria-hidden': 'true' }), h('span', { class: 'brand-name' }, 'Jay', h('span', {}, '-'), 'A')),
    createThemeToggle(),
  );

  // A new language applies on a fresh page.
  settings.subscribe((next, prev) => {
    if (next.language !== prev.language) location.reload();
  });

  root.replaceChildren(header, h('div', { class: 'app-body' }, tabs.list, h('div', { class: 'app-panels' }, items.map((i) => i.panel))));
  tabs.select('filters');
}
