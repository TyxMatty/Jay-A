// UI preferences persisted in localStorage. public/theme-init.js reads the same key.
// The engine's settings (filters, soundboard) will live in its own config.json (docs/ARQUITETURA.md).
const STORAGE_KEY = 'jaya.settings';

export const DEFAULT_SETTINGS = Object.freeze({
  /** 'system' | 'light' | 'dark' */
  theme: 'system',
  /** The look: 'calm', 'neon' or 'soft' (core/style.js, styles/theme.css). */
  style: 'calm',
  /** Interface language: 'system' (the OS language), 'en' or 'pt-BR' (core/i18n.js). */
  language: 'system',
  // shortcut: the mic settings live here while only the page uses them; they move to the engine's
  // config.json in stage 3, when the engine must run them with the window closed (Jay Link).
  /** Microphone endpoint id (the engine's list_mics), or null = the Windows default. */
  micDevice: null,
  noiseSuppression: true,
  autoGainControl: true,
  /** "Hear my microphone" during the test. */
  monitor: false,
});

function load() {
  try {
    return { ...DEFAULT_SETTINGS, ...JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

let state = load();
const listeners = new Set();

export const settings = {
  get() {
    return state;
  },

  update(patch) {
    const prev = state;
    state = { ...state, ...patch };
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch {
      // Storage unavailable (private mode / quota): keep working in memory.
    }
    for (const listener of listeners) listener(state, prev);
  },

  /** Calls `listener(next, prev)` on every update. Returns an unsubscribe function. */
  subscribe(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
};
