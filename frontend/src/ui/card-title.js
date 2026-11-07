// A card's title with its icon in a small accent-tinted square, like the feature list of Jay-C's
// lobby (.lobby-feature-icon there): the Jay line's look for a section.
import { h } from './dom.js';

export const cardTitle = (icon, text) => h('h3', { class: 'card-title' }, h('span', { class: 'card-icon', html: icon, 'aria-hidden': 'true' }), text);
