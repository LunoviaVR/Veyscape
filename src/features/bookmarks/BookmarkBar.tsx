import { ChevronDown, ChevronsRight } from 'lucide-react';
import { SiteIcon } from '../../components/SiteIcon';
import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from 'react';
import { FolderIcon } from '../../lib/folderIcons';
import { api, onEvent } from '../../lib/ipc';
import { activeTabId, browserStore, useSettings } from '../../lib/store';
import { groupColor } from '../../lib/groupColor';
import { showOverlay } from '../overlay/client';
import type { Bookmark } from '../../types/browser';
import styles from './BookmarkBar.module.css';

// Overlay size of the bookmark editor, sized so each mode fits without scrolling.
const EDITOR_SIZE = { bookmark: { width: 360, height: 318 }, folder: { width: 360, height: 476 } } as const;

/** Root bookmarks and folders under the toolbar. Folders open native dropdown menus. */
export function BookmarkBar() {
  const settings = useSettings();
  const shown = !!settings?.showBookmarkBar;
  const [items, setItems] = useState<Bookmark[]>([]);
  const [hiddenFrom, setHiddenFrom] = useState<number | null>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const barRef = useRef<HTMLElement>(null);
  // Where the last bar menu was opened; the editor appears there.
  const lastClick = useRef({ x: 16, y: 0 });

  useEffect(() => {
    const load = () => void api.library.bookmarks().then((all) => setItems(all.filter((b) => b.parentId === null)));
    load();
    const un = onEvent('bookmarks:changed', load);
    // Opened from the native bar/item menus: show the editor in the overlay, above web content.
    const unEditor = onEvent('bookmark:editor', (req) => {
      const state = browserStore.get();
      const tab = state.tabs.find((t) => t.id === state.activeTabId);
      const size = EDITOR_SIZE[req.mode];
      const bar = barRef.current?.getBoundingClientRect();
      const x = Math.min(Math.max(8, lastClick.current.x - 24), window.innerWidth - size.width - 8);
      const y = (bar?.bottom ?? 120) + 4;
      void showOverlay(
        { kind: 'bookmarkEditor', ...req, pageUrl: tab && !tab.internal ? tab.url : '', pageTitle: tab?.title ?? '' },
        { x, y, ...size },
        true,
      );
    });
    return () => {
      void un.then((f) => f());
      void unEditor.then((f) => f());
    };
  }, []);

  // Items that do not fit move into the overflow menu.
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return setHiddenFrom(null);
    const measure = () => {
      const right = list.getBoundingClientRect().right;
      const children = Array.from(list.children) as HTMLElement[];
      const first = children.findIndex((c) => c.getBoundingClientRect().right > right + 1);
      setHiddenFrom(first >= 0 ? first : null);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(list);
    return () => ro.disconnect();
  }, [items, shown]);

  if (!shown) return null;

  const open = (b: Bookmark, e: MouseEvent) => {
    const tab = activeTabId();
    if (!b.url || tab == null) return;
    if (e.button === 1 || e.ctrlKey) void api.tabs.create({ url: b.url, background: true });
    else void api.nav.go(tab, b.url);
  };

  const openFolder = (b: Bookmark, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    void api.library.folderMenu(b.id, r.left, r.bottom + 2);
  };

  const contextMenu = (b: Bookmark, e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    lastClick.current = { x: e.clientX, y: e.clientY };
    void api.library.itemMenu(b.id, e.clientX, e.clientY);
  };

  return (
    <nav
      ref={barRef}
      className={styles.bar}
      aria-label="Bookmarks bar"
      onContextMenu={(e) => {
        e.preventDefault();
        lastClick.current = { x: e.clientX, y: e.clientY };
        const state = browserStore.get();
        const tab = state.tabs.find((t) => t.id === state.activeTabId);
        void api.library.barMenu(!!tab && !tab.internal && /^https?:/.test(tab.url), e.clientX, e.clientY);
      }}
    >
      {items.length === 0 ? (
        <p className={styles.empty}>Right-click here to add bookmarks and folders, or press Ctrl+D to bookmark this page.</p>
      ) : (
        <ul ref={listRef} className={styles.list}>
          {items.map((b, i) => (
            <li key={b.id} className={hiddenFrom !== null && i >= hiddenFrom ? styles.clipped : undefined}>
              {b.kind === 'folder' ? (
                <button
                  type="button"
                  className={styles.item}
                  aria-haspopup="menu"
                  title={b.title}
                  onClick={(e) => openFolder(b, e.currentTarget)}
                  onContextMenu={(e) => contextMenu(b, e)}
                >
                  <span className={styles.folderIcon} style={b.color ? { color: groupColor(b.color) } : undefined}>
                    <FolderIcon name={b.icon} size={14} />
                  </span>
                  <span className={styles.label}>{b.title}</span>
                  <ChevronDown size={12} aria-hidden className={styles.chevron} />
                </button>
              ) : (
                <button
                  type="button"
                  className={styles.item}
                  title={`${b.title}\n${b.url}`}
                  onClick={(e) => open(b, e)}
                  onAuxClick={(e) => e.button === 1 && open(b, e)}
                  onContextMenu={(e) => contextMenu(b, e)}
                >
                  <SiteIcon src={b.favicon} />
                  <span className={styles.label}>{b.title}</span>
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      {hiddenFrom !== null && (
        <button
          type="button"
          className={styles.item}
          aria-label="More bookmarks"
          title="More bookmarks"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            // The overflow menu lists the whole bar via the same native folder menu (root = 0).
            void api.library.folderMenu(0, r.left, r.bottom + 2);
          }}
        >
          <ChevronsRight size={14} aria-hidden />
        </button>
      )}
    </nav>
  );
}
