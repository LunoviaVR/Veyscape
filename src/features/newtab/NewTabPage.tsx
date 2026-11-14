import { Search, X } from 'lucide-react';
import { useEffect, useState, type FormEvent } from 'react';
import { api, errorMessage, onEvent } from '../../lib/ipc';
import { announce } from '../../lib/ui';
import { useBrowserState, useSettings } from '../../lib/store';
import { usePageTab } from '../../lib/pageTab';
import { displayHost } from '../../lib/format';
import type { HistoryEntry } from '../../types/browser';
import { SEARCH_ENGINE_NAMES } from '../navigation/suggestions';
import styles from './NewTabPage.module.css';

interface Site {
  url: string;
  title: string;
  favicon?: string | null;
}

/** Minimal new tab: a search field plus optional, dismissible shortcuts. No feeds, no ads. */
export function NewTabPage() {
  const tab = usePageTab();
  const { privateWindow } = useBrowserState();
  const settings = useSettings();
  const [query, setQuery] = useState('');
  const [top, setTop] = useState<HistoryEntry[]>([]);
  const [bookmarks, setBookmarks] = useState<Site[]>([]);

  // Top sites come from history, which every window shares: refresh when any window adds to it.
  useEffect(() => {
    if (!settings?.newtabShowTopSites || tab?.private) return;
    const refresh = () => void api.library.topSites().then(setTop);
    refresh();
    const un = onEvent('history:changed', refresh);
    return () => void un.then((f) => f());
  }, [settings?.newtabShowTopSites, tab?.private]);

  // Refreshes when bookmarks change in any window (including a site icon learned from a visit).
  useEffect(() => {
    if (!settings?.newtabShowBookmarks || tab?.private) return;
    const refresh = () =>
      void api.library.bookmarks().then((all) => {
        // One tile per address: the same page may be bookmarked in several folders.
        const sites = new Map<string, Site>();
        for (const b of all) {
          if (b.kind === 'bookmark' && b.url && !sites.has(b.url)) sites.set(b.url, { url: b.url, title: b.title, favicon: b.favicon });
        }
        setBookmarks([...sites.values()].slice(0, 8));
      });
    refresh();
    const un = onEvent('bookmarks:changed', refresh);
    return () => void un.then((f) => f());
  }, [settings?.newtabShowBookmarks, tab?.private]);

  if (!tab || !settings) return null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (query.trim()) void api.nav.go(tab.id, query);
  };

  const hide = (key: 'newtabShowTopSites' | 'newtabShowBookmarks') =>
    void api.settings.update({ ...settings, [key]: false }).catch((e) => announce(errorMessage(e)));

  return (
    <main className={styles.page} data-background={settings.newtabBackground} aria-label="New tab">
      <div className={styles.center}>
        {tab.private && (
          <p className={styles.privateNote}>
            {privateWindow ? 'Private window: every tab here is private. Nothing' : 'Private tab: nothing'} you do here is saved to history, and cookies, site data and site choices are discarded
            when you close your last private tab. Tracking prevention is strict. Downloaded files stay on disk.
          </p>
        )}
        <form className={styles.search} role="search" onSubmit={submit}>
          <Search size={18} aria-hidden className={styles.searchIcon} />
          <input
            className={styles.searchInput}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={`Search with ${SEARCH_ENGINE_NAMES[settings.searchEngine]} or enter address`}
            aria-label="Search or enter address"
          />
        </form>
        {!tab.private && settings.newtabShowTopSites && top.length > 0 && (
          <SiteGrid title="Frequently visited" sites={top} onHide={() => hide('newtabShowTopSites')} tabId={tab.id} />
        )}
        {!tab.private && settings.newtabShowBookmarks && bookmarks.length > 0 && (
          <SiteGrid title="Bookmarks" sites={bookmarks} onHide={() => hide('newtabShowBookmarks')} tabId={tab.id} />
        )}
      </div>
    </main>
  );
}

function SiteGrid({ title, sites, onHide, tabId }: { title: string; sites: Site[]; onHide: () => void; tabId: number }) {
  return (
    <section className={styles.section} aria-label={title}>
      <div className={styles.sectionHeader}>
        <h2>{title}</h2>
        <button type="button" className={styles.hide} onClick={onHide} aria-label={`Hide ${title.toLowerCase()}`} title="Hide">
          <X size={14} />
        </button>
      </div>
      <ul className={styles.grid}>
        {sites.map((s) => (
          <li key={s.url}>
            <button type="button" className={styles.site} onClick={() => void api.nav.go(tabId, s.url)} title={s.url}>
              <span className={styles.letter} aria-hidden>
                {s.favicon ? <img src={s.favicon} alt="" width={20} height={20} draggable={false} /> : displayHost(s.url).charAt(0).toUpperCase()}
              </span>
              <span className={styles.siteTitle}>{s.title || displayHost(s.url)}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
