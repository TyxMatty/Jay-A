// Whether the interface animates: Settings › Appearance › Animations, On (default) or Off. It
// doesn't follow Windows' "reduce motion" (the user chose that on 05/11/2026). ui/theme.js puts
// the result on <html data-motion="full|reduced">, which the CSS animations key off.

export const MOTION_PREFERENCES = ['on', 'off'];

/** Only 'off' turns animations off; anything else (including an old saved 'system') is on. */
export const normalizeMotion = (preference) => (preference === 'off' ? 'off' : 'on');

/** @returns {'full' | 'reduced'} */
export const resolveMotion = (preference) => (normalizeMotion(preference) === 'off' ? 'reduced' : 'full');
