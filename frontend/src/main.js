import { initTheme } from './ui/theme.js';
import { mountApp } from './ui/app-shell.js';
import { settings } from './store/settings.js';
import { setLanguage, resolveLanguage, t } from './core/i18n.js';
import { resolveMotion } from './core/motion.js';

setLanguage(resolveLanguage(settings.get().language));
// Animations (the tour's scenes, its slides): Settings › Appearance › Animations, on by default,
// like Jay-C (core/motion.js).
const applyMotion = () => (document.documentElement.dataset.motion = resolveMotion(settings.get().motion));
applyMotion();
settings.subscribe(applyMotion);
initTheme();
mountApp(document.getElementById('app'));

if ('__TAURI_INTERNALS__' in window) {
  // The tray menu is built in Rust, which doesn't know the page's language.
  import('@tauri-apps/api/core')
    .then(({ invoke }) => invoke('set_tray_labels', { open: t('Open Jay-A'), stop: t('Stop soundboard'), quit: t('Quit') }))
    .catch((error) => console.warn('[tray]', error));
}

// No browser menu (Back, Reload, Print, Inspect…) outside text fields, which keep copy/paste.
document.addEventListener('contextmenu', (e) => {
  if (!e.target.closest?.('input, textarea, [contenteditable="true"]')) e.preventDefault();
});
