// Shell side of the popup overlay: a second trusted webview stacked above web content.
// The shell decides what to show and where; the overlay renders it and reports actions.
import { api, emitToWebview, onEvent, type OverlayAction, type OverlayContent } from '../../lib/ipc';

let current: OverlayContent | null = null;
let visible = false;
const listeners = new Set<(a: OverlayAction) => void>();

export function initOverlayClient(): () => void {
  const unReady = onEvent('overlay:ready', () => {
    if (current) void emitToWebview('overlay', 'overlay:render', current);
  });
  const unAction = onEvent('overlay:action', (action) => {
    if (action.type === 'closed') {
      visible = false;
      current = null;
      // Unmount the popup so the next one starts fresh (focus, form state).
      void emitToWebview('overlay', 'overlay:render', null);
    }
    listeners.forEach((l) => l(action));
  });
  // Chaining on the promise also unlistens when cleanup runs before listen() resolved.
  return () => {
    void unReady.then((f) => f());
    void unAction.then((f) => f());
  };
}

export interface OverlayRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export async function showOverlay(content: OverlayContent, rect: OverlayRect, focus: boolean) {
  current = content;
  await emitToWebview('overlay', 'overlay:render', content);
  await api.layout.overlayShow(rect, focus);
  visible = true;
}

export async function updateOverlay(content: OverlayContent) {
  current = content;
  if (visible) await emitToWebview('overlay', 'overlay:render', content);
}

export async function hideOverlay(refocusShell = false) {
  if (!visible) return;
  visible = false;
  current = null;
  await api.layout.overlayHide(refocusShell);
  await emitToWebview('overlay', 'overlay:render', null);
}

export function overlayKind(): OverlayContent['kind'] | null {
  return visible ? (current?.kind ?? null) : null;
}

export function onOverlayAction(listener: (a: OverlayAction) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
