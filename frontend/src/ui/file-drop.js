// Files dropped on the window, as paths. On Windows, Tauri takes the drag and drop over (the page's
// own drop events get no files), so the panels listen here; only the visible panel gets the drop.
const listeners = new Set();

if ('__TAURI_INTERNALS__' in window) {
  import('@tauri-apps/api/webview').then(({ getCurrentWebview }) =>
    getCurrentWebview().onDragDropEvent(({ payload }) => {
      for (const l of listeners) {
        if (l.panel.hidden) continue;
        if (payload.type === 'over' || payload.type === 'enter') l.zone.classList.add('over');
        else l.zone.classList.remove('over');
        if (payload.type === 'drop' && payload.paths.length) l.onDrop(payload.paths);
      }
    }),
  );
}

/** Calls `onDrop(paths)` for files dropped while `panel` is showing; `zone` is highlighted meanwhile. */
export function onFileDrop(panel, zone, onDrop) {
  listeners.add({ panel, zone, onDrop });
}
