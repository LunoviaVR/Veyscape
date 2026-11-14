import { useEffect, useId, useMemo, useState, type FormEvent } from 'react';
import { api, errorMessage, type BookmarkEditorRequest } from '../../lib/ipc';
import { announce } from '../../lib/ui';
import type { Bookmark } from '../../types/browser';
import { ColorPicker, IconPicker } from './StylePickers';
import controls from '../../components/controls.module.css';
import styles from '../../components/popupForm.module.css';

const BAR = 'bar';

/** Adds a bookmark or folder: name, address (bookmarks), location, icon and colour (folders). Rendered in the overlay. */
export function BookmarkEditor({
  request,
  pageUrl,
  pageTitle,
  onClose,
  onPickerOpen,
}: {
  request: BookmarkEditorRequest;
  pageUrl: string;
  pageTitle: string;
  onClose: () => void;
  onPickerOpen: () => void;
}) {
  const id = useId();
  const isFolder = request.mode === 'folder';
  const editing = request.id !== null;
  const [all, setAll] = useState<Bookmark[]>([]);
  const [title, setTitle] = useState('');
  const [url, setUrl] = useState('');
  const [parent, setParent] = useState<number | null>(request.parentId);
  const [icon, setIcon] = useState('folder');
  const [color, setColor] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    void api.library.bookmarks().then((items) => {
      setAll(items);
      const existing = items.find((b) => b.id === request.id);
      if (existing) {
        setTitle(existing.title);
        setUrl(existing.url ?? '');
        setIcon(existing.icon ?? 'folder');
        setColor(existing.color);
        setParent(existing.parentId);
      } else if (!isFolder && /^https?:\/\//.test(pageUrl)) {
        // New bookmarks start from the current page; the user can change both fields.
        setTitle(pageTitle);
        setUrl(pageUrl);
      } else if (isFolder) {
        setTitle('New folder');
      }
      setLoaded(true);
    });
  }, [request.id, isFolder, pageUrl, pageTitle]);

  // Folders this item may live in (a folder cannot go inside itself or its descendants).
  const locations = useMemo(() => {
    const out: { id: number; path: string }[] = [];
    const walk = (p: number | null, prefix: string) => {
      for (const f of all.filter((b) => b.parentId === p && b.kind === 'folder' && b.id !== request.id)) {
        const path = prefix ? `${prefix} / ${f.title}` : f.title;
        out.push({ id: f.id, path });
        walk(f.id, path);
      }
    };
    walk(null, '');
    return out;
  }, [all, request.id]);

  const save = async (e: FormEvent) => {
    e.preventDefault();
    setError('');
    const name = title.trim();
    let address = url.trim();
    if (!isFolder && address && !/^[a-z][a-z0-9+.-]*:/i.test(address)) address = `https://${address}`;
    try {
      if (!isFolder && !address) throw new Error('Enter the address to bookmark.');
      if (isFolder && !name) throw new Error('Enter a folder name.');
      if (editing) {
        const current = all.find((b) => b.id === request.id);
        await api.library.updateBookmark(request.id!, name, isFolder ? { icon, color } : { url: address });
        if (current && current.parentId !== parent) await api.library.moveBookmark(request.id!, parent);
      } else if (isFolder) {
        await api.library.createFolder(parent, name, icon, color);
      } else {
        await api.library.addBookmark(parent, name, address);
      }
      announce(editing ? 'Saved' : isFolder ? 'Folder created' : 'Bookmark added');
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : errorMessage(err));
    }
  };

  if (!loaded) return <div className={styles.editor} aria-busy="true" />;
  const heading = `${editing ? 'Edit' : 'New'} ${isFolder ? 'folder' : 'bookmark'}`;

  return (
    <form className={styles.editor} onSubmit={(e) => void save(e)} aria-labelledby={`${id}-h`}>
      <h2 id={`${id}-h`} className={styles.heading}>
        {heading}
      </h2>
      <label className={styles.field}>
        <span>Name</span>
        <input className={controls.input} value={title} autoFocus maxLength={1024} onChange={(e) => setTitle(e.target.value)} />
      </label>
      {!isFolder && (
        <label className={styles.field}>
          <span>Address</span>
          <input
            className={controls.input}
            value={url}
            inputMode="url"
            spellCheck={false}
            placeholder="https://"
            onChange={(e) => setUrl(e.target.value)}
          />
        </label>
      )}
      <label className={styles.field}>
        <span>Location</span>
        <select className={controls.select} value={parent ?? BAR} onChange={(e) => setParent(e.target.value === BAR ? null : Number(e.target.value))}>
          <option value={BAR}>Bookmarks bar</option>
          {locations.map((f) => (
            <option key={f.id} value={f.id}>
              {f.path}
            </option>
          ))}
        </select>
      </label>
      {isFolder && (
        <>
          <div className={styles.field}>
            <span id={`${id}-icon`}>Icon</span>
            <IconPicker value={icon} color={color} onChange={(i) => i && setIcon(i)} />
          </div>
          <div className={styles.field}>
            <span>Colour</span>
            <ColorPicker value={color} onChange={setColor} onPickerOpen={onPickerOpen} />
          </div>
        </>
      )}
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      <div className={styles.actions}>
        <button type="button" className={controls.button} onClick={onClose}>
          Cancel
        </button>
        <button type="submit" className={`${controls.button} ${controls.primary}`}>
          {editing ? 'Save' : isFolder ? 'Create folder' : 'Add bookmark'}
        </button>
      </div>
    </form>
  );
}
