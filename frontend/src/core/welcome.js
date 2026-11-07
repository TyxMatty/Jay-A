// The "Welcome!" tour shown on first launch (ui/welcome-modal.js) and from Settings › About.
// Content and navigation live here so they can be tested without a DOM. Same tour as Jay-C's
// (core/welcome.js there), with Jay-A's steps.

/** One card per step. `art` names the illustration (ui/welcome-art.js). */
export const WELCOME_STEPS = Object.freeze([
  {
    key: 'welcome',
    art: 'welcome',
    title: 'Welcome to Jay-A!',
    body: "The Jay line's audio app: your microphone cleaned up, a soundboard, an audio editor and a converter. Here's how it works, in a few quick steps.",
  },
  {
    key: 'filters',
    art: 'filters',
    title: 'Clean up your microphone',
    body: 'In Filters, pick your microphone and build an effect chain: AI noise removal takes out keyboards, clicks and fans, and a preset gets you started. Test it and hear yourself with headphones.',
  },
  {
    key: 'jayc',
    art: 'jayc',
    title: 'Use it in Jay-C',
    body: 'In Jay-C, choose Jay-A in Settings › Audio › Microphone filter. Your calls get this microphone, with echo cancellation. If Jay-A stops, Jay-C takes over at once.',
  },
  {
    key: 'soundboard',
    art: 'soundboard',
    title: 'Play clips',
    body: 'Import short clips to the Soundboard and play them with a click or a global shortcut, even with a game in focus: in the call and in your headphones.',
  },
  {
    key: 'apps',
    art: 'apps',
    title: 'In other apps too',
    body: 'Filters › Microphone for other apps sends the filtered microphone to a virtual cable. In Discord, OBS and others, pick the other end of the cable as the microphone.',
  },
  {
    key: 'files',
    art: 'files',
    title: 'Edit and convert',
    body: 'Cut, fade and normalize audio files in the Editor, without touching the original. Convert batches between WAV, FLAC, MP3, OGG and Opus.',
  },
  {
    key: 'tray',
    art: 'tray',
    title: 'It stays in the tray',
    body: 'Closing the window keeps Jay-A next to the clock, so your microphone keeps its filters during calls. To quit, right-click its icon there.',
  },
]);

/** Keeps a step index inside the tour. */
export function clampStep(index, count = WELCOME_STEPS.length) {
  if (!Number.isFinite(index)) return 0;
  return Math.min(Math.max(Math.trunc(index), 0), count - 1);
}

/** The tour opens by itself once (ui/app-shell.js marks it seen as it opens). */
export function shouldShowWelcome(prefs) {
  return prefs?.welcomeSeen !== true;
}
