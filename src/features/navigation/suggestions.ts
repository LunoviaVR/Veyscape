import type { SuggestionRow } from '../../lib/ipc';
import type { SearchEngine, Suggestion } from '../../types/browser';

export const SEARCH_ENGINE_NAMES: Record<SearchEngine, string> = {
  duckDuckGo: 'DuckDuckGo',
  brave: 'Brave Search',
  startpage: 'Startpage',
  google: 'Google',
  bing: 'Bing',
};

/** Heuristic for the first row's label only; the backend makes the real URL-vs-search decision. */
export function looksLikeAddress(text: string): boolean {
  const t = text.trim();
  if (!t || /\s/.test(t)) return false;
  return /^[a-z][a-z0-9+.-]*:\/\//i.test(t) || /^localhost(:\d+)?(\/|$)/i.test(t) || /^[^./]+\.[a-z]{2,}(:\d+)?(\/.*)?$/i.test(t);
}

export function buildRows(text: string, suggestions: Suggestion[], engine: SearchEngine): SuggestionRow[] {
  const t = text.trim();
  if (!t) return [];
  const first: SuggestionRow = looksLikeAddress(t)
    ? { key: 'typed', kind: 'url', primary: t, secondary: 'Open address', value: t, historyId: null }
    : { key: 'typed', kind: 'search', primary: t, secondary: `Search with ${SEARCH_ENGINE_NAMES[engine]}`, value: t, historyId: null };
  const rest = suggestions
    .filter((s) => s.url !== t)
    .map<SuggestionRow>((s) => ({
      key: s.url,
      kind: s.bookmarked ? 'bookmark' : 'history',
      primary: s.title || s.url,
      secondary: s.url,
      value: s.url,
      historyId: s.historyId,
    }));
  return [first, ...rest];
}
