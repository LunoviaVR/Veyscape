import { AlertTriangle, LockOpen } from 'lucide-react';
import { useId } from 'react';
import { BookmarksPage } from '../features/bookmarks/BookmarksPage';
import { DownloadsPage } from '../features/downloads/DownloadsPage';
import { HistoryPage } from '../features/history/HistoryPage';
import { NewTabPage } from '../features/newtab/NewTabPage';
import { SettingsPage } from '../features/settings/SettingsPage';
import { api } from '../lib/ipc';
import { useActiveTab, useBrowserState } from '../lib/store';
import { PageTabContext } from '../lib/pageTab';
import { displayHost } from '../lib/format';
import type { LoadFailure, Tab } from '../types/browser';
import controls from '../components/controls.module.css';
import styles from './ShellApp.module.css';

/**
 * The content area. Web pages are native engine views drawn on top of this element by the
 * backend; this component renders only internal pages and error screens.
 */
export function Content() {
  const tab = useActiveTab();
  const { split, tabs, activeTabId } = useBrowserState();
  if (!tab) return <div className={styles.content} />;
  if (split?.shown && split.panes.length) {
    return (
      <div className={`${styles.content} ${styles.splitArea}`}>
        {split.panes.map(({ tabId, rect }) => {
          const paneTab = tabs.find((t) => t.id === tabId);
          if (!paneTab) return null;
          const active = tabId === activeTabId;
          return (
            <section
              key={tabId}
              className={`${styles.pane} ${active ? styles.activePane : ''}`}
              style={{ left: rect.x, top: rect.y, width: rect.width, height: rect.height }}
              aria-label={`Split pane: ${paneTab.title || 'New Tab'}${active ? ' (active)' : ''}`}
              onMouseDownCapture={() => !active && void api.tabs.activate(tabId)}
            >
              <PageTabContext.Provider value={paneTab}>
                {paneTab.error ? <ErrorScreen tab={paneTab} /> : <InternalPage tab={paneTab} />}
              </PageTabContext.Provider>
            </section>
          );
        })}
      </div>
    );
  }
  return (
    <div className={styles.content} key={tab.id}>
      {tab.error ? <ErrorScreen tab={tab} /> : <InternalPage tab={tab} />}
    </div>
  );
}

function InternalPage({ tab }: { tab: Tab }) {
  switch (tab.internal) {
    case 'newTab':
      return <NewTabPage />;
    case 'settings':
      return <SettingsPage />;
    case 'history':
      return <HistoryPage />;
    case 'bookmarks':
      return <BookmarksPage />;
    case 'downloads':
      return <DownloadsPage />;
    default:
      return null;
  }
}

const LOAD_FAILURES: Record<LoadFailure, { title: string; body: (host: string) => string }> = {
  certificate: {
    title: 'Connection is not secure',
    body: (h) => `${h} presented a certificate that could not be verified. Veyscape stopped loading the page so your data stays protected.`,
  },
  notFound: { title: 'Site not found', body: (h) => `${h} could not be found. Check the address for typos.` },
  timeout: { title: 'The site took too long to respond', body: (h) => `${h} did not answer in time. It may be busy or down.` },
  unreachable: { title: 'Can’t connect to the site', body: (h) => `${h} refused or dropped the connection.` },
  offline: { title: 'You’re offline', body: () => 'Check your network connection, then try again.' },
  failed: { title: 'This page could not be loaded', body: (h) => `Something went wrong while loading ${h}.` },
};

function ErrorScreen({ tab }: { tab: Tab }) {
  const error = tab.error!;
  const titleId = useId();
  if (error.kind === 'httpsUnavailable') {
    return (
      <main className={styles.errorPage} aria-labelledby={titleId}>
        <LockOpen size={36} aria-hidden className={styles.errorIcon} />
        <h1 id={titleId}>Secure connection not available</h1>
        <p>
          {displayHost(error.httpUrl)} does not support HTTPS. HTTPS-only mode stopped the page from loading over an insecure
          connection, where others on the network could read or change it.
        </p>
        <div className={styles.errorActions}>
          <button type="button" className={`${controls.button} ${controls.primary}`} onClick={() => void api.nav.back(tab.id)} disabled={!tab.canGoBack}>
            Go back
          </button>
          <button type="button" className={controls.button} onClick={() => void api.nav.httpsContinue(tab.id)}>
            Continue to insecure site
          </button>
        </div>
      </main>
    );
  }
  if (error.kind === 'loadFailed') {
    const copy = LOAD_FAILURES[error.reason] ?? LOAD_FAILURES.failed;
    return (
      <main className={styles.errorPage} aria-labelledby={titleId}>
        <AlertTriangle size={36} aria-hidden className={styles.errorIcon} />
        <h1 id={titleId}>{copy.title}</h1>
        <p data-selectable>{copy.body(displayHost(error.url))}</p>
        <div className={styles.errorActions}>
          <button type="button" className={`${controls.button} ${controls.primary}`} onClick={() => void api.nav.reload(tab.id)}>
            Try again
          </button>
          {tab.canGoBack && (
            <button type="button" className={controls.button} onClick={() => void api.nav.back(tab.id)}>
              Go back
            </button>
          )}
        </div>
      </main>
    );
  }
  const crashed = error.kind === 'crashed';
  return (
    <main className={styles.errorPage} aria-labelledby={titleId}>
      <AlertTriangle size={36} aria-hidden className={styles.errorIcon} />
      <h1 id={titleId}>{crashed ? 'This page stopped working' : 'This page could not be shown'}</h1>
      <p data-selectable>{crashed ? 'The page’s process ended unexpectedly.' : error.message}</p>
      <div className={styles.errorActions}>
        <button type="button" className={`${controls.button} ${controls.primary}`} onClick={() => void api.nav.reload(tab.id)}>
          Reload
        </button>
      </div>
    </main>
  );
}
