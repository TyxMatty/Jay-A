// The update pill's states, without the DOM (ui/update.js draws them):
// hidden -> available -> confirm -> downloading -> installing, or error (then retry).

export const HIDDEN = { name: 'hidden' };

/** The percent of a download, 0..100 (an unknown total counts as 0). */
export const percentOf = (done, total) => (total > 0 ? Math.min(100, Math.max(0, Math.floor((done * 100) / total))) : 0);

/** The next state after `event` ({ type, ... }); an event that doesn't apply returns `state` itself. */
export function next(state, event) {
  switch (event.type) {
    case 'available':
      return state.name === 'hidden' && typeof event.version === 'string' && event.version.trim() ? { name: 'available', version: event.version } : state;
    case 'open':
      return state.name === 'available' ? { ...state, name: 'confirm' } : state;
    case 'close':
      return state.name === 'confirm' || state.name === 'error' ? { name: 'available', version: state.version } : state;
    case 'start': // also the retry from an error
      return state.name === 'confirm' || state.name === 'error' ? { name: 'downloading', version: state.version, percent: 0 } : state;
    case 'progress': {
      if (state.name !== 'downloading') return state;
      const percent = percentOf(event.done, event.total);
      return event.total > 0 && event.done >= event.total ? { name: 'installing', version: state.version } : { ...state, percent };
    }
    case 'error':
      return state.name === 'hidden' ? state : { name: 'error', version: state.version, code: event.code ?? null };
    default:
      return state;
  }
}
