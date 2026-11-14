import { Check, ChevronDown, ChevronRight, FolderPlus, Pencil, Trash2, Upload } from 'lucide-react';
import { SiteIcon } from '../../components/SiteIcon';
import { useCallback, useEffect, useMemo, useState, type ChangeEvent } from 'react';
import { IconButton } from '../../components/IconButton';
import { EmptyState, Page } from '../../components/Page';
import { FolderIcon } from '../../lib/folderIcons';
import { groupColor } from '../../lib/groupColor';
import { ColorPicker, IconPicker } from './StylePickers';
import { api, errorMessage, onEvent } from '../../lib/ipc';
import { usePageTab } from '../../lib/pageTab';
import { announce } from '../../lib/ui';
import type { Bookmark, ImportSource, ImportSummary } from '../../types/browser';
import controls from '../../components/controls.module.css';
import page from '../../components/page.module.css';
import styles from './BookmarksPage.module.css';

const BAR = 'bar';
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

/** Bookmark manager: folders (shown on the bookmarks bar at the top level), icons, moves, import. */
export function BookmarksPage() {
  const tab = usePageTab();
  const [items, setItems] = useState<Bookmark[]>([]);
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState<Set<number>>(new Set());
  const [editing, setEditing] = useState<{ id: number; title: string } | null>(null);
  const [iconFor, setIconFor] = useState<number | null>(null);
  const [showImport, setShowImport] = useState(false);

  const reload = useCallback(() => void api.library.bookmarks().then(setItems), []);
  useEffect(() => {
    reload();
    const un = onEvent('bookmarks:changed', reload);
    return () => void un.then((f) => f());
  }, [reload]);

  const children = useMemo(() => {
    const map = new Map<number | null, Bookmark[]>();
    for (const b of items) {
      const siblings = map.get(b.parentId);
      if (siblings) siblings.push(b);
      else map.set(b.parentId, [b]);
    }
    return map;
  }, [items]);

  const folders = useMemo(() => {
    const out: { id: number; path: string }[] = [];
    const walk = (parent: number | null, prefix: string) => {
      for (const f of (children.get(parent) ?? []).filter((b) => b.kind === 'folder')) {
        const path = prefix ? `${prefix} / ${f.title}` : f.title;
        out.push({ id: f.id, path });
        walk(f.id, path);
      }
    };
    walk(null, '');
    return out;
  }, [children]);

  const run = async (p: Promise<unknown>, done?: string) => {
    try {
      await p;
      if (done) announce(done);
    } catch (e) {
      announce(errorMessage(e));
    }
  };

  const save = (b: Bookmark) => {
    if (!editing) return;
    void run(api.library.updateBookmark(b.id, editing.title, { icon: b.icon, color: b.color }), 'Saved');
    setEditing(null);
  };

  const remove = (b: Bookmark) => {
    const count = b.kind === 'folder' ? items.filter((x) => x.parentId === b.id).length : 0;
    if (b.kind === 'folder' && count > 0 && !window.confirm(`Delete “${b.title}” and everything in it?`)) return;
    void run(api.library.removeBookmark(b.id), b.kind === 'folder' ? 'Folder deleted' : 'Bookmark deleted');
  };

  const newFolder = async () => {
    const id = await api.library.createFolder(null, 'New folder', 'folder').catch((e) => void announce(errorMessage(e)));
    if (typeof id === 'number') setEditing({ id, title: 'New folder' });
  };

  const q = query.trim().toLowerCase();
  const matches = q
    ? items.filter((b) => b.kind === 'bookmark' && (b.title.toLowerCase().includes(q) || (b.url ?? '').toLowerCase().includes(q)))
    : [];

  const row = (b: Bookmark, depth: number) => {
    const isFolder = b.kind === 'folder';
    const expanded = open.has(b.id);
    // Untitled bookmarks (icon-only on the bar) are named by their address.
    const name = b.title || b.url || '';
    return (
      <li key={b.id}>
        <div className={page.item} style={{ paddingLeft: `calc(${depth} * 1.25rem + var(--space-2))` }}>
          {isFolder ? (
            <IconButton
              label={expanded ? `Collapse ${name}` : `Expand ${name}`}
              aria-expanded={expanded}
              onClick={() => setOpen((s) => new Set(s.has(b.id) ? [...s].filter((x) => x !== b.id) : [...s, b.id]))}
            >
              {expanded ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
            </IconButton>
          ) : (
            <span className={styles.leafIcon} aria-hidden>
              <SiteIcon src={b.favicon} />
            </span>
          )}
          {isFolder && (
            <IconButton label={`Change icon and colour of ${name}`} pressed={iconFor === b.id} onClick={() => setIconFor(iconFor === b.id ? null : b.id)}>
              <span className={styles.tint} style={b.color ? { color: groupColor(b.color) } : undefined}>
                <FolderIcon name={b.icon} />
              </span>
            </IconButton>
          )}
          {editing?.id === b.id ? (
            <form className={page.itemMain} onSubmit={(e) => (e.preventDefault(), save(b))}>
              <input
                className={controls.input}
                value={editing.title}
                autoFocus
                aria-label={isFolder ? 'Folder name' : 'Bookmark title'}
                onChange={(e) => setEditing({ id: b.id, title: e.target.value })}
                onKeyDown={(e) => e.key === 'Escape' && setEditing(null)}
              />
            </form>
          ) : isFolder ? (
            <span className={page.itemMain}>
              <span className={page.itemTitle}>{b.title}</span>
              <span className={page.itemSub}>{plural((children.get(b.id) ?? []).length, 'item')}{depth === 0 ? ' · on bookmarks bar' : ''}</span>
            </span>
          ) : (
            <button type="button" className={page.itemMain} onClick={() => tab && b.url && void api.nav.go(tab.id, b.url)}>
              <span className={page.itemTitle}>{b.title || 'No name'}</span>
              <span className={page.itemSub}>{b.url}</span>
            </button>
          )}
          <select
            className={`${controls.select} ${styles.moveSelect}`}
            aria-label={`Move ${name} to`}
            value={b.parentId ?? BAR}
            onChange={(e: ChangeEvent<HTMLSelectElement>) =>
              void run(api.library.moveBookmark(b.id, e.target.value === BAR ? null : Number(e.target.value)), 'Moved')
            }
          >
            <option value={BAR}>Bookmarks bar</option>
            {folders
              .filter((f) => f.id !== b.id)
              .map((f) => (
                <option key={f.id} value={f.id}>
                  {f.path}
                </option>
              ))}
          </select>
          {editing?.id === b.id ? (
            <IconButton label="Save" onClick={() => save(b)}>
              <Check size={15} />
            </IconButton>
          ) : (
            <IconButton label={`Rename ${name}`} onClick={() => setEditing({ id: b.id, title: b.title })}>
              <Pencil size={15} />
            </IconButton>
          )}
          <IconButton label={`Delete ${name}`} onClick={() => remove(b)}>
            <Trash2 size={15} />
          </IconButton>
        </div>
        {iconFor === b.id && (
          <div className={styles.iconGrid} aria-label={`Icon and colour for ${name}`}>
            <IconPicker
              value={b.icon ?? 'folder'}
              color={b.color}
              onChange={(icon) => void run(api.library.updateBookmark(b.id, b.title, { icon, color: b.color }), 'Icon changed')}
            />
            <ColorPicker
              value={b.color}
              onChange={(color) => void run(api.library.updateBookmark(b.id, b.title, { icon: b.icon, color }), 'Colour changed')}
            />
          </div>
        )}
        {isFolder && expanded && <ul className={page.list}>{(children.get(b.id) ?? []).map((c) => row(c, depth + 1))}</ul>}
      </li>
    );
  };

  return (
    <Page
      title="Bookmarks"
      actions={
        <>
          <button type="button" className={controls.button} onClick={() => void newFolder()}>
            <FolderPlus size={15} aria-hidden /> New folder
          </button>
          <button type="button" className={controls.button} aria-expanded={showImport} onClick={() => setShowImport((v) => !v)}>
            <Upload size={15} aria-hidden /> Import
          </button>
        </>
      }
    >
      {showImport && <ImportPanel onDone={() => setShowImport(false)} />}
      <input
        type="search"
        className={`${controls.input} ${page.search}`}
        placeholder="Search bookmarks"
        aria-label="Search bookmarks"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {q ? (
        matches.length ? (
          <ul className={page.list}>{matches.map((b) => row(b, 0))}</ul>
        ) : (
          <EmptyState>No matching bookmarks.</EmptyState>
        )
      ) : items.length ? (
        <ul className={page.list}>{(children.get(null) ?? []).map((b) => row(b, 0))}</ul>
      ) : (
        <EmptyState>Press Ctrl+D or the star in the address bar to bookmark a page, or import bookmarks from another browser.</EmptyState>
      )}
    </Page>
  );
}

function ImportPanel({ onDone }: { onDone: () => void }) {
  const [sources, setSources] = useState<ImportSource[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => void api.library.importSources().then(setSources), []);

  const finish = async (p: Promise<ImportSummary>, from: string) => {
    setBusy(true);
    setStatus(null);
    try {
      const { imported, skipped } = await p;
      const skippedText = skipped > 0 ? `; ${skipped.toLocaleString()} skipped` : '';
      if (imported === 0 && skipped === 0) {
        setStatus({ ok: false, text: `No bookmarks found in ${from}.` });
      } else {
        setStatus({ ok: true, text: `Imported ${imported.toLocaleString()} bookmarks${skippedText} from ${from} into a new folder on the bookmarks bar.` });
        announce(`Imported ${imported.toLocaleString()} bookmarks${skippedText}`);
      }
    } catch (e) {
      setStatus({ ok: false, text: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const fromFile = (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (!file) return;
    if (file.size > 32 * 1024 * 1024) return setStatus({ ok: false, text: 'That file is too large.' });
    void finish(file.text().then((text) => api.library.importFile(file.name, text)), file.name);
  };

  return (
    <section className={styles.import} aria-label="Import bookmarks">
      <h2 className={styles.importTitle}>Import bookmarks</h2>
      <p className={page.itemSub}>Reads the other browser&apos;s bookmark file on this computer. Nothing is uploaded anywhere.</p>
      <div className={styles.importRow}>
        {sources === null ? (
          <span className={page.itemSub}>Looking for browsers…</span>
        ) : sources.length === 0 ? (
          <span className={page.itemSub}>No other browsers found.</span>
        ) : (
          sources.map((s) => (
            <button key={s.id} type="button" className={controls.button} disabled={busy} onClick={() => void finish(api.library.importBrowser(s.id), s.name)}>
              {s.name}
            </button>
          ))
        )}
        <label className={`${controls.button} ${styles.fileButton}`}>
          From file (HTML or JSON)…
          <input type="file" accept=".html,.htm,.json" className="visually-hidden" disabled={busy} onChange={fromFile} />
        </label>
        <button type="button" className={controls.button} onClick={onDone}>
          Close
        </button>
      </div>
      {status && (
        <p className={status.ok ? styles.ok : styles.error} role="status">
          {status.text}
        </p>
      )}
    </section>
  );
}
