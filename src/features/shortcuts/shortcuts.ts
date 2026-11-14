// Keyboard shortcuts while the browser UI has focus. The table is shared with the Rust side
// (shared/shortcuts.json), which handles the same keys while a web page has focus.
import table from '../../../shared/shortcuts.json';

export interface Binding {
  action: string;
  code: string;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

export const bindings: Binding[] = table;

export interface KeyLike {
  code: string;
  key?: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

/**
 * Letters follow the layout (`key`), like the Rust side's virtual keys: on AZERTY Ctrl+Z reports code KeyW.
 * Other keys use the physical `code`, also for non-Latin layouts where `key` is no ASCII letter; `key` is the
 * fallback when the input carried no scan code (synthetic input, some remote sessions).
 */
export function codeOf(e: KeyLike): string {
  const k = e.key ?? '';
  if (/^[a-z]$/i.test(k)) return `Key${k.toUpperCase()}`;
  // Rust maps zoom from the layout's +/- characters; on German or Nordic layouts they sit on other physical keys.
  const symbol = ({ '=': 'Equal', '+': 'Equal', '-': 'Minus' } as Record<string, string | undefined>)[k];
  if (symbol && !/^(Digit|Numpad)/.test(e.code)) return symbol;
  if (e.code) return e.code;
  if (/^[0-9]$/.test(k)) return `Digit${k}`;
  return k;
}

export function actionForKey(e: KeyLike): string | undefined {
  if (e.metaKey) return undefined;
  const code = codeOf(e);
  return bindings.find((b) => b.code === code && b.ctrl === e.ctrlKey && b.shift === e.shiftKey && b.alt === e.altKey)?.action;
}

/** Shortcuts that must keep working while typing in a text field (e.g. Ctrl+T in the address bar). */
export function allowedWhileTyping(action: string): boolean {
  return action !== 'goBack' && action !== 'goForward';
}
