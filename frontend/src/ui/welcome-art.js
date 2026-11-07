// Illustrations for the "Welcome!" tour (ui/welcome-modal.js), one small scene per step, drawn the
// way Jay-C's are (ui/welcome-art.js there). Trusted static SVG (like icons.js). Colors come from
// the theme: `currentColor` is the accent, the .wa-* classes map to theme variables. Motion lives in
// app.css (.wa-* keyframes) and only runs with <html data-motion="full">.

const svg = (body) =>
  `<svg viewBox="0 0 240 140" fill="none" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

/** A microphone: capsule, cradle and stand. */
const mic = (x, y, s = 1) =>
  `<rect x="${x - 9 * s}" y="${y - 24 * s}" width="${18 * s}" height="${30 * s}" rx="${9 * s}" class="wa-fill-accent"/>` +
  `<path d="M${x - 16 * s} ${y - 4 * s} a${16 * s} ${16 * s} 0 0 0 ${32 * s} 0 M${x} ${y + 12 * s} v${10 * s} M${x - 10 * s} ${y + 22 * s} h${20 * s}" class="wa-stroke-text"/>`;

/** Level bars that bounce, one after the other. */
const bars = (x, y, n, gap = 9) =>
  Array.from({ length: n }, (_, i) => {
    const h = [14, 26, 18, 32, 22, 12, 28][i % 7];
    return `<rect x="${x + i * gap}" y="${y - h / 2}" width="5" height="${h}" rx="2.5" class="wa-fill-accent wa-bounce" style="--d:${i * 110}ms"/>`;
  }).join('');

/** A small window with a title bar. */
const card = (x, y, w, h, label = '') =>
  `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="7" class="wa-card"/>` +
  (label ? `<text x="${x + w / 2}" y="${y + h / 2 + 4}" text-anchor="middle" class="wa-label">${label}</text>` : '');

export const WELCOME_ART = {
  // A microphone, its voice going out as clean bars.
  welcome: svg(
    `<rect x="40" y="24" width="160" height="92" rx="12" class="wa-card"/>` +
      `<circle cx="88" cy="68" r="30" class="wa-ring wa-pulse"/>` +
      mic(88, 70) +
      `<path d="M122 52 q8 16 0 32" class="wa-stroke-accent wa-wave"/>` +
      `<path d="M130 46 q12 22 0 44" class="wa-stroke-accent wa-wave" style="--d:200ms"/>` +
      bars(146, 68, 5, 10),
  ),
  // Noise goes into the chain of effects and comes out a clean wave.
  filters: svg(
    `<path d="M14 70 l6 -14 l5 26 l6 -30 l6 34 l5 -22 l6 12" class="wa-stroke-muted"/>` +
      `<path d="M58 70 H182" class="wa-link wa-flow"/>` +
      [0, 1, 2].map((i) => `<rect x="${70 + i * 38}" y="52" width="30" height="36" rx="7" class="wa-card wa-pop" style="--d:${i * 140}ms"/>` + `<circle cx="${85 + i * 38}" cy="70" r="6" class="wa-fill-accent"/>`).join('') +
      `<path d="M190 70 q8 -18 16 0 t16 0" class="wa-stroke-accent wa-float"/>`,
  ),
  // Jay-A and Jay-C linked; the voice flows across, a green light says it's on.
  jayc: svg(
    card(20, 40, 76, 60, 'Jay-A') +
      card(144, 40, 76, 60, 'Jay-C') +
      `<path d="M96 70 H144" class="wa-link wa-flow"/>` +
      `<circle cx="120" cy="70" r="5" class="wa-fill-accent wa-slide-x"/>` +
      `<circle cx="210" cy="50" r="4" class="wa-fill-green wa-blink"/>`,
  ),
  // A grid of pads; one is pressed and plays.
  soundboard: svg(
    [0, 1, 2]
      .flatMap((r) => [0, 1, 2, 3].map((c) => [r, c]))
      .map(([r, c], i) => `<rect x="${48 + c * 38}" y="${22 + r * 34}" width="30" height="26" rx="6" class="${i === 5 ? 'wa-fill-accent wa-pulse' : 'wa-card wa-pop'}" style="--d:${i * 40}ms"/>`)
      .join('') +
      `<path d="M206 50 q8 12 0 24" class="wa-stroke-accent wa-wave"/>` +
      `<path d="M214 44 q12 18 0 36" class="wa-stroke-accent wa-wave" style="--d:200ms"/>`,
  ),
  // The microphone through a cable into other apps.
  apps: svg(
    mic(40, 72, 0.8) +
      `<path d="M58 70 C 90 70 90 40 120 40 M58 70 C 90 70 90 100 120 100" class="wa-link wa-flow"/>` +
      card(120, 24, 92, 32) +
      `<circle cx="134" cy="40" r="6" class="wa-fill-accent wa-pop"/>` +
      `<path d="M146 40 h52" class="wa-stroke-muted"/>` +
      card(120, 84, 92, 32) +
      `<circle cx="134" cy="100" r="6" class="wa-fill-accent wa-pop" style="--d:150ms"/>` +
      `<path d="M146 100 h52" class="wa-stroke-muted"/>`,
  ),
  // A waveform with a selection sliding over it, and a file turning into another.
  files: svg(
    `<rect x="18" y="28" width="150" height="84" rx="10" class="wa-card"/>` +
      `<rect x="44" y="34" width="44" height="72" rx="4" class="wa-select wa-slide-x"/>` +
      Array.from({ length: 22 }, (_, i) => {
        const h = 10 + Math.round(Math.abs(Math.sin(i * 0.9)) * 44);
        return `<rect x="${26 + i * 6.4}" y="${70 - h / 2}" width="3" height="${h}" rx="1.5" class="wa-fill-accent"/>`;
      }).join('') +
      `<rect x="186" y="34" width="36" height="44" rx="5" class="wa-card wa-float"/>` +
      `<path d="M194 48 h20 M194 58 h20 M194 68 h12" class="wa-stroke-muted"/>` +
      `<path d="M204 88 v14 m-6 -6 l6 6 l6 -6" class="wa-stroke-accent wa-bounce"/>`,
  ),
  // The taskbar; Jay-A's icon by the clock, still lit.
  tray: svg(
    `<rect x="16" y="92" width="208" height="30" rx="8" class="wa-card"/>` +
      [0, 1, 2].map((i) => `<rect x="${30 + i * 26}" y="99" width="16" height="16" rx="4" class="wa-fill-muted"/>`).join('') +
      `<circle cx="170" cy="107" r="9" class="wa-fill-accent wa-pulse"/>` +
      `<path d="M188 103 h24 M188 111 h16" class="wa-stroke-muted"/>` +
      `<rect x="60" y="18" width="120" height="58" rx="10" class="wa-card wa-faded"/>` +
      `<path d="M150 30 l12 12 m0 -12 l-12 12" class="wa-stroke-muted"/>` +
      `<path d="M120 76 v10" class="wa-link wa-flow"/>`,
  ),
};
