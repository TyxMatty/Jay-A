// The window: top bar, then the sections (Filters, Soundboard, Editor, Convert, Settings) as tabs.
// Sections not built yet are placeholders until their stage in docs/ARQUITETURA.md.
import { h } from './dom.js';
import { icons } from './icons.js';
import { createTabs } from './tabs.js';
import { createFiltersPanel } from './filters.js';
import { createSoundboardPanel } from './soundboard.js';
import { createConvertPanel } from './convert.js';
import { createEditorPanel } from './editor.js';
import { createWelcomeModal } from './welcome-modal.js';
import { shouldShowWelcome } from '../core/welcome.js';
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

function createSettingsPanel(openWelcome) {
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
        h('div', { class: 'button-row' }, h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => openWelcome() }, t('Show the tour'))),
      ),
    ),
  );
}

const tabLabel = (icon, text) => h('span', { class: 'tab-label' }, h('span', { class: 'tab-icon', html: icon }), text);

export function mountApp(root) {
  const welcome = createWelcomeModal();
  const items = [
    { key: 'filters', label: tabLabel(icons.mic, t('Filters')), panel: createFiltersPanel() },
    {
      key: 'soundboard',
      label: tabLabel(icons.pads, t('Soundboard')),
      panel: createSoundboardPanel(),
    },
    {
      key: 'editor',
      label: tabLabel(icons.waveform, t('Editor')),
      panel: createEditorPanel(),
    },
    {
      key: 'convert',
      label: tabLabel(icons.convert, t('Convert')),
      panel: createConvertPanel(),
    },
    { key: 'settings', label: tabLabel(icons.settings, t('Settings')), panel: createSettingsPanel(() => welcome.open()) },
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

  root.replaceChildren(header, h('div', { class: 'app-body' }, tabs.list, h('div', { class: 'app-panels' }, items.map((i) => i.panel))), welcome.element);
  tabs.select('filters');
  // The tour opens once by itself, marked seen as it opens (like Jay-C). Started with --background,
  // it waits in the hidden window until the user opens it.
  if (shouldShowWelcome(settings.get())) {
    settings.update({ welcomeSeen: true });
    welcome.open();
  }
}
