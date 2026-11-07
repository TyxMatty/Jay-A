// Illustrations for the "Welcome!" tour (ui/welcome-modal.js), one small scene per step, drawn the
// way Jay-C's are (ui/welcome-art.js there): every scene keeps moving. Trusted static SVG (like
// icons.js). Colors come from the theme: `currentColor` is the accent, the .wa-* classes map to
// theme variables. Motion lives in app.css (.wa-* keyframes; `--d` staggers) and only runs with
// <html data-motion="full">; otherwise each scene is a still picture.

const svg = (body) =>
  `<svg viewBox="0 0 240 140" fill="none" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

/** A microphone: capsule, cradle and stand. */
const mic = (x, y, s = 1, cls = '') =>
  `<g class="${cls}"><rect x="${x - 9 * s}" y="${y - 24 * s}" width="${18 * s}" height="${30 * s}" rx="${9 * s}" class="wa-fill-accent"/>` +
  `<path d="M${x - 16 * s} ${y - 4 * s} a${16 * s} ${16 * s} 0 0 0 ${32 * s} 0 M${x} ${y + 12 * s} v${10 * s} M${x - 10 * s} ${y + 22 * s} h${20 * s}" class="wa-stroke-text"/></g>`;

/** Level bars that bounce one after the other. */
const bars = (x, y, n, gap = 9, w = 5) =>
  Array.from({ length: n }, (_, i) => {
    const h = [14, 26, 18, 32, 22, 12, 28][i % 7];
    return `<rect x="${x + i * gap}" y="${y - h / 2}" width="${w}" height="${h}" rx="${w / 2}" class="wa-fill-accent wa-level" style="--d:${i * 120}ms"/>`;
  }).join('');

/** Sound waves going out to the right of (x, y). */
const waves = (x, y) =>
  `<path d="M${x} ${y - 10} q7 10 0 20" class="wa-stroke-accent wa-wave"/>` +
  `<path d="M${x + 8} ${y - 18} q12 18 0 36" class="wa-stroke-accent wa-wave" style="--d:250ms"/>`;

/** A small window with a title. */
const card = (x, y, w, h, label = '', cls = 'wa-pop', d = 0) =>
  `<g class="${cls}" style="--d:${d}ms"><rect x="${x}" y="${y}" width="${w}" height="${h}" rx="7" class="wa-card"/>` +
  (label ? `<text x="${x + w / 2}" y="${y + h / 2 + 4}" text-anchor="middle" class="wa-label">${label}</text>` : '') +
  `</g>`;

export const WELCOME_ART = {
  // A microphone that pulses as it picks you up, its voice going out as dancing bars.
  welcome: svg(
    `<rect x="40" y="24" width="160" height="92" rx="12" class="wa-card wa-pop"/>` +
      `<circle cx="88" cy="68" r="30" class="wa-ring wa-pulse"/>` +
      mic(88, 70, 1, 'wa-float') +
      waves(122, 68) +
      bars(148, 68, 5, 10),
  ),
  // Noise shakes on the way in, each effect lights up in turn, a smooth wave comes out.
  filters: svg(
    `<path d="M10 70 l6 -14 l5 26 l6 -30 l6 34 l5 -22 l6 12" class="wa-stroke-muted wa-shake"/>` +
      `<path d="M56 70 H186" class="wa-link wa-flow"/>` +
      [0, 1, 2]
        .map(
          (i) =>
            `<g class="wa-pop" style="--d:${i * 140}ms"><rect x="${70 + i * 38}" y="52" width="30" height="36" rx="7" class="wa-card"/>` +
            `<circle cx="${85 + i * 38}" cy="70" r="7" class="wa-fill-accent wa-glow" style="--d:${i * 400}ms"/></g>`,
        )
        .join('') +
      `<path d="M192 70 q6 -16 12 0 t12 0 t12 0" class="wa-stroke-accent wa-float"/>`,
  ),
  // Jay-A and Jay-C linked: the voice travels across, Jay-C's light is on and it plays.
  jayc: svg(
    card(16, 40, 76, 60, 'Jay-A', 'wa-walk-in', 0) +
      card(148, 40, 76, 60, 'Jay-C', 'wa-walk-in', 200) +
      `<path d="M92 70 H148" class="wa-link wa-flow"/>` +
      `<circle cx="120" cy="70" r="5" class="wa-fill-accent wa-travel"/>` +
      `<circle cx="214" cy="50" r="4" class="wa-fill-green wa-blink"/>` +
      bars(170, 84, 4, 8, 4),
  ),
  // A grid of pads popping in, then lighting up one after another as they play.
  soundboard: svg(
    [0, 1, 2]
      .flatMap((r) => [0, 1, 2, 3].map((c) => [r, c]))
      .map(([r, c], i) => `<g class="wa-pop" style="--d:${i * 40}ms"><rect x="${44 + c * 38}" y="${22 + r * 34}" width="30" height="26" rx="6" class="wa-card"/><rect x="${44 + c * 38}" y="${22 + r * 34}" width="30" height="26" rx="6" class="wa-fill-accent wa-glow" style="--d:${[0, 700, 1400, 350, 1050, 1750, 200, 900, 1600, 550, 1250, 1950][i]}ms"/></g>`)
      .join('') +
      waves(204, 62),
  ),
  // The microphone through a cable into two apps, each showing the level.
  apps: svg(
    mic(38, 72, 0.8, 'wa-float') +
      `<path d="M56 70 C 88 70 88 40 118 40 M56 70 C 88 70 88 100 118 100" class="wa-link wa-flow"/>` +
      card(118, 22, 100, 36, '', 'wa-walk-in', 0) +
      `<circle cx="132" cy="40" r="6" class="wa-fill-accent wa-pulse"/>` +
      bars(148, 40, 6, 9, 4) +
      card(118, 82, 100, 36, '', 'wa-walk-in', 200) +
      `<circle cx="132" cy="100" r="6" class="wa-fill-accent wa-pulse" style="--d:300ms"/>` +
      bars(148, 100, 6, 9, 4),
  ),
  // A waveform with a selection sliding over it; the file below turns into another.
  files: svg(
    `<rect x="16" y="26" width="152" height="88" rx="10" class="wa-card wa-pop"/>` +
      `<rect x="44" y="32" width="44" height="76" rx="4" class="wa-select wa-slide-x"/>` +
      Array.from({ length: 22 }, (_, i) => {
        const h = 10 + Math.round(Math.abs(Math.sin(i * 0.9)) * 44);
        return `<rect x="${25 + i * 6.4}" y="${70 - h / 2}" width="3" height="${h}" rx="1.5" class="wa-fill-accent wa-level" style="--d:${(i % 6) * 90}ms"/>`;
      }).join('') +
      `<g class="wa-float"><rect x="184" y="30" width="38" height="46" rx="5" class="wa-card"/>` +
      `<path d="M192 44 h22 M192 54 h22 M192 64 h14" class="wa-stroke-muted"/></g>` +
      `<path d="M203 88 v16 m-7 -7 l7 7 l7 -7" class="wa-stroke-accent wa-bounce"/>`,
  ),
  // The window going down into the tray, where Jay-A's icon stays lit by the clock.
  tray: svg(
    `<rect x="16" y="94" width="208" height="30" rx="8" class="wa-card"/>` +
      [0, 1, 2].map((i) => `<rect x="${30 + i * 26}" y="101" width="16" height="16" rx="4" class="wa-fill-muted"/>`).join('') +
      `<circle cx="170" cy="109" r="9" class="wa-fill-accent wa-pulse"/>` +
      `<path d="M188 105 h24 M188 113 h16" class="wa-stroke-muted wa-blink"/>` +
      `<g class="wa-to-tray"><rect x="60" y="14" width="120" height="62" rx="10" class="wa-card"/>` +
      `<path d="M72 28 h40" class="wa-stroke-muted"/>` +
      bars(80, 54, 6, 12, 5) +
      `</g>`,
  ),
};
