import { Trash2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { IconButton } from '../../components/IconButton';
import { EmptyState, Page } from '../../components/Page';
import { api, errorMessage, onEvent } from '../../lib/ipc';
import { usePageTab } from '../../lib/pageTab';
import { announce } from '../../lib/ui';
import { formatDay, formatTime } from '../../lib/format';
import type { HistoryEntry } from '../../types/browser';
import controls from '../../components/controls.module.css';
import page from '../../components/page.module.css';

const PAGE_SIZE = 100;
/** The most rows the backend returns per request (`history_list` clamps to this). */
const MAX_LIMIT = 1000;

/** Local browsing history, grouped by day. Nothing here ever leaves the device. */
export function HistoryPage() {
  const tab = usePageTab();
  const [query, setQuery] = useState('');
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [hasMore, setHasMore] = useState(false);

  const loaded = useRef(0);
  useEffect(() => {
    loaded.current = entries.length;
  }, [entries.length]);

  // Replies can arrive out of order; only the latest request may write state.
  const generation = useRef(0);
  /** Loads `limit` rows at `offset`; a reload (offset 0) passes as many as are shown. */
  const load = useCallback(async (q: string, offset = 0, limit = PAGE_SIZE) => {
    const mine = ++generation.current;
    try {
      const rows = await api.library.history(q, offset, limit);
      if (mine !== generation.current) return;
      setEntries((prev) => (offset ? [...prev, ...rows] : rows));
      setHasMore(rows.length === limit);
    } catch (e) {
      if (mine === generation.current) announce(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    const t = window.setTimeout(() => void load(query), 120);
    return () => window.clearTimeout(t);
  }, [query, load]);

  // History is shared by all windows: reload when any of them records, deletes or clears visits.
  useEffect(() => {
    let t = 0;
    const un = onEvent('history:changed', () => {
      window.clearTimeout(t);
      t = window.setTimeout(() => void load(query, 0, Math.min(MAX_LIMIT, Math.max(PAGE_SIZE, loaded.current))), 300);
    });
    return () => {
      window.clearTimeout(t);
      void un.then((f) => f());
    };
  }, [query, load]);

  const groups = new Map<string, HistoryEntry[]>();
  for (const e of entries) {
    const day = formatDay(e.lastVisit);
    groups.set(day, [...(groups.get(day) ?? []), e]);
  }

  const remove = async (e: HistoryEntry) => {
    try {
      await api.library.deleteHistory(e.id);
      setEntries((prev) => prev.filter((x) => x.id !== e.id));
      announce('Removed from history');
    } catch (err) {
      announce(errorMessage(err));
    }
  };

  const clearAll = async () => {
    if (!window.confirm('Delete all browsing history? This cannot be undone.')) return;
    try {
      await api.privacy.clear({ history: true, downloads: false, cookies: false, cache: false, permissions: false, range: 'allTime' });
      setEntries([]);
      announce('History cleared');
    } catch (e) {
      announce(errorMessage(e));
    }
  };

  return (
    <Page
      title="History"
      actions={
        <button type="button" className={`${controls.button} ${controls.danger}`} onClick={() => void clearAll()} disabled={!entries.length}>
          Clear history
        </button>
      }
    >
      <input
        type="search"
        className={`${controls.input} ${page.search}`}
        placeholder="Search history"
        aria-label="Search history"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {!entries.length && <EmptyState>{query ? 'No matching pages.' : 'Pages you visit will appear here. Private tabs are never recorded.'}</EmptyState>}
      {[...groups].map(([day, items]) => (
        <section key={day} aria-label={day}>
          <h2 className={page.groupHeading}>{day}</h2>
          <ul className={page.list}>
            {items.map((e) => (
              <li key={e.id} className={page.item}>
                <span className={page.meta}>{formatTime(e.lastVisit)}</span>
                <button type="button" className={page.itemMain} onClick={() => tab && void api.nav.go(tab.id, e.url)}>
                  <span className={page.itemTitle}>{e.title || e.url}</span>
                  <span className={page.itemSub}>{e.url}</span>
                </button>
                <IconButton label={`Remove ${e.title || e.url} from history`} onClick={() => void remove(e)}>
                  <Trash2 size={15} />
                </IconButton>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {hasMore && (
        <button type="button" className={controls.button} onClick={() => void load(query, entries.length)}>
          Show more
        </button>
      )}
    </Page>
  );
}
