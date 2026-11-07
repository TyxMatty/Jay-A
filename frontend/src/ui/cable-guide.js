// The virtual cable setup guide (docs/ARQUITETURA.md D4), laid out like Jay-C's server setup guide
// (ui/setup-guide-modal.js there): a wide dialog, numbered steps, Done. Its steps act: install
// VB-CABLE (VB-Audio's own setup, downloaded and checked by src-tauri/src/cable_setup.rs), turn the
// bridge on (the filtered microphone and the soundboard into "CABLE Input"), then pick "CABLE Output"
// as the microphone in Discord, OBS and the rest.
import { h } from './dom.js';
import { icons } from './icons.js';
import { t } from '../core/i18n.js';

/** The cable's input, as Windows names it ("CABLE Input (VB-Audio Virtual Cable)"). */
const isCableInput = (name) => /^CABLE Input\b/i.test(name);

/**
 * @param {{ invoke: Function, onBridge: () => void }} deps `onBridge`: the bridge was turned on or
 *   off here (the Filters tab redraws its picker).
 */
export function createCableGuide({ invoke, onBridge }) {
  const installStatus = h('p', { class: 'hint', 'aria-live': 'polite' });
  const bridgeStatus = h('p', { class: 'hint', 'aria-live': 'polite' });
  const installButton = h('button', { type: 'button', class: 'btn', onClick: () => install() }, t('Install VB-CABLE'));
  const siteButton = h('button', { type: 'button', class: 'btn btn-secondary', onClick: () => invoke('cable_site') }, t('Download it myself'));
  const bridgeButton = h('button', { type: 'button', class: 'btn', onClick: () => toggleBridge() });
  let cable = null; // the cable's input endpoint, once installed
  let bridgeOn = false;

  const step = (n, title, ...content) => h('section', { class: 'guide-step' }, h('h3', {}, h('span', { class: 'guide-step-n' }, String(n)), title), ...content);

  const body = h(
    'div',
    { class: 'modal-body cable-guide' },
    h('p', {}, t('Jay-A becomes a microphone for every app: it sends your filtered voice, and the soundboard, into a virtual cable, and Discord, OBS and others listen to the other end of the cable. The cable is VB-CABLE, from VB-Audio: free (donationware), and installed once.')),
    step(
      1,
      t('Install VB-CABLE'),
      h('p', {}, t('Jay-A downloads the setup from vb-audio.com, checks it is signed by VB-Audio and opens it. Click "Install Driver" there and allow it to make changes (it is a sound driver). If the cable does not show up afterwards, restart the PC.')),
      h('div', { class: 'button-row' }, installButton, siteButton),
      installStatus,
    ),
    step(
      2,
      t('Turn the bridge on'),
      h('p', {}, t('Jay-A sends the filtered microphone and the soundboard to "CABLE Input". The microphone stays open while the bridge is on, like in Voicemod.')),
      h('div', { class: 'button-row' }, bridgeButton),
      bridgeStatus,
    ),
    step(
      3,
      t('Pick the cable in your apps'),
      h(
        'ul',
        {},
        h('li', {}, t('Discord: User Settings › Voice & Video › Input Device: "CABLE Output (VB-Audio Virtual Cable)". Turn the noise suppression of Discord off: Jay-A already does it.')),
        h('li', {}, t('OBS: Sources › + › Audio Input Capture › Device: "CABLE Output (VB-Audio Virtual Cable)".')),
        h('li', {}, t('Any other app: choose "CABLE Output" where it asks for the microphone.')),
      ),
      h('p', { class: 'hint' }, t('Jay-C does not need the cable: it gets Jay-A directly (Settings › Audio › Microphone filter › Jay-A).')),
    ),
  );

  async function refresh() {
    const [outputs, current] = await Promise.all([invoke('list_outputs').catch(() => []), invoke('cable').catch(() => null)]);
    cable = outputs.find((o) => isCableInput(o.name)) ?? null;
    bridgeOn = !!current && current === cable?.id;
    installStatus.textContent = cable ? t('VB-CABLE is installed.') : t('VB-CABLE is not installed yet.');
    installButton.disabled = !!cable;
    bridgeButton.disabled = !cable;
    bridgeButton.textContent = bridgeOn ? t('Turn the bridge off') : t('Turn the bridge on');
    bridgeStatus.textContent = bridgeOn ? t('On: your microphone goes to "{name}".', { name: cable.name }) : cable ? t('Off.') : t('Install VB-CABLE first.');
  }

  async function install() {
    installButton.disabled = true;
    installStatus.textContent = t('Downloading VB-CABLE and opening its setup… Allow it in the window Windows shows.');
    try {
      await invoke('cable_install');
      // The new device takes a few seconds to appear.
      for (let i = 0; i < 10 && !cable; i++) {
        await refresh();
        if (!cable) await new Promise((r) => setTimeout(r, 1500));
      }
      if (cable && !bridgeOn) await toggleBridge();
      if (!cable) installStatus.textContent = t('VB-CABLE does not show up yet. Restart the PC, then open this guide again.');
    } catch (error) {
      installStatus.textContent = t("Couldn't install VB-CABLE: {error}", { error });
      installButton.disabled = false;
    }
  }

  async function toggleBridge() {
    try {
      await invoke('set_cable', { device: bridgeOn ? null : cable.id });
    } catch (error) {
      bridgeStatus.textContent = t("Couldn't open the cable: {error}", { error });
    }
    await refresh();
    onBridge();
  }

  const dialog = h(
    'dialog',
    { class: 'modal modal-wide', 'aria-labelledby': 'cable-guide-title' },
    h(
      'form',
      { method: 'dialog' },
      h(
        'header',
        { class: 'modal-header' },
        h('h2', { id: 'cable-guide-title' }, t('Microphone for other apps')),
        h('button', { type: 'submit', class: 'icon-btn', 'aria-label': t('Close guide'), html: icons.close }),
      ),
      body,
      h('footer', { class: 'modal-footer' }, h('button', { type: 'submit', class: 'btn' }, t('Done'))),
    ),
  );
  dialog.addEventListener('click', (e) => {
    if (e.target === dialog) dialog.close();
  });

  return {
    element: dialog,
    open() {
      dialog.showModal();
      body.scrollTop = 0;
      refresh();
    },
  };
}
