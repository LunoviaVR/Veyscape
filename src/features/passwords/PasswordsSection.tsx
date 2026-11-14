import { Copy, Eye, EyeOff, Pencil, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { IconButton } from '../../components/IconButton';
import { Switch } from '../../components/Switch';
import { api, errorMessage, onEvent } from '../../lib/ipc';
import { displayHost } from '../../lib/format';
import { announce } from '../../lib/ui';
import type { Login } from '../../types/browser';
import { useSettingsUpdate } from '../settings/useSettingsUpdate';
import controls from '../../components/controls.module.css';
import page from '../settings/Settings.module.css';
import styles from './Passwords.module.css';

/** Settings → Passwords: saved logins (search, show, copy, edit, delete) and never-saved sites. */
export function PasswordsSection() {
  const { settings, update, error: settingsError } = useSettingsUpdate();
  const [logins, setLogins] = useState<Login[]>([]);
  const [never, setNever] = useState<string[]>([]);
  const [query, setQuery] = useState('');
  // Passwords shown after Windows Hello; kept only while this page is open.
  const [shown, setShown] = useState<Record<number, string>>({});
  const [editing, setEditing] = useState<number | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const [error, setError] = useState('');

  const reload = useCallback(() => {
    api.passwords.list().then(setLogins, (e) => setError(errorMessage(e)));
    api.passwords.neverList().then(setNever, (e) => setError(errorMessage(e)));
  }, []);

  useEffect(() => {
    reload();
    const un = onEvent('passwords:changed', reload);
    return () => void un.then((f) => f());
  }, [reload]);

  const attempt = async <T,>(fn: () => Promise<T>): Promise<T | undefined> => {
    setError('');
    try {
      return await fn();
    } catch (e) {
      setError(errorMessage(e));
      return undefined;
    }
  };

  const reveal = async (id: number) => {
    if (id in shown) return shown[id];
    const password = await attempt(() => api.passwords.reveal(id));
    if (password !== undefined) setShown((s) => ({ ...s, [id]: password }));
    return password;
  };

  const hide = (id: number) => setShown((s) => Object.fromEntries(Object.entries(s).filter(([k]) => Number(k) !== id)));

  const copy = async (l: Login) => {
    const wasShown = l.id in shown;
    const password = await reveal(l.id);
    if (password === undefined) return;
    if (!wasShown) hide(l.id);
    try {
      await navigator.clipboard.writeText(password);
      announce('Password copied');
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const startEdit = async (id: number) => {
    if ((await reveal(id)) !== undefined) setEditing(id);
  };

  const remove = async (l: Login) => {
    // attempt() yields undefined on failure, so only a real success is announced.
    const deleted = await attempt(async () => {
      await api.passwords.remove(l.id);
      return true;
    });
    setConfirmDelete(null);
    if (deleted) announce(`Deleted the password for ${l.username || displayHost(l.origin)}`);
  };

  const q = query.trim().toLowerCase();
  const visible = q ? logins.filter((l) => l.origin.toLowerCase().includes(q) || l.username.toLowerCase().includes(q)) : logins;

  return (
    <>
      <div className={page.group}>
        <Switch
          label="Offer to save passwords"
          description="After you sign in to a site, Veyscape asks whether to save the password. Never in private tabs."
          checked={settings?.offerToSavePasswords ?? true}
          onChange={(v) => void update({ offerToSavePasswords: v })}
        />
      </div>
      {settingsError && (
        <p className={page.error} role="alert">
          {settingsError}
        </p>
      )}
      <p className={`${page.hint} ${page.note}`}>
        Passwords are encrypted with your Windows account and never leave this device. Showing, copying or editing one asks for Windows Hello
        first, if it is set up.
      </p>

      <h3 className={page.groupTitle}>Saved passwords</h3>
      {logins.length > 0 && (
        <input
          type="search"
          className={`${controls.input} ${styles.search}`}
          placeholder="Search passwords"
          aria-label="Search passwords"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      )}
      {error && (
        <p className={page.error} role="alert">
          {error}
        </p>
      )}
      <div className={page.group}>
        {visible.length === 0 ? (
          <p className={`${page.hint} ${page.field}`}>{logins.length ? 'No matching passwords.' : 'No saved passwords yet.'}</p>
        ) : (
          <ul className={page.tableList}>
            {visible.map((l) =>
              editing === l.id ? (
                <li key={l.id}>
                  <EditLogin
                    login={l}
                    password={shown[l.id] ?? ''}
                    onDone={(saved) => {
                      setEditing(null);
                      hide(l.id);
                      if (saved) announce('Password saved');
                    }}
                  />
                </li>
              ) : (
                <li key={l.id}>
                  <span className={`${page.grow} ${styles.site}`}>
                    <span data-selectable>{displayHost(l.origin)}</span>
                    <span className={page.hint} data-selectable>
                      {l.username || 'No username'}
                    </span>
                  </span>
                  <span className={styles.secret} data-selectable={l.id in shown || undefined} aria-label={l.id in shown ? undefined : 'Password hidden'}>
                    {shown[l.id] ?? '••••••••••'}
                  </span>
                  {confirmDelete === l.id ? (
                    <>
                      <button type="button" className={`${controls.button} ${controls.danger}`} onClick={() => void remove(l)}>
                        Delete
                      </button>
                      {/* The trash button is gone: keep keyboard focus on the safe choice. */}
                      <button type="button" className={controls.button} onClick={() => setConfirmDelete(null)} autoFocus>
                        Cancel
                      </button>
                    </>
                  ) : (
                    <>
                      <IconButton
                        label={l.id in shown ? 'Hide password' : `Show password for ${l.username || displayHost(l.origin)}`}
                        onClick={() => (l.id in shown ? hide(l.id) : void reveal(l.id))}
                      >
                        {l.id in shown ? <EyeOff size={16} /> : <Eye size={16} />}
                      </IconButton>
                      <IconButton label="Copy password" onClick={() => void copy(l)}>
                        <Copy size={16} />
                      </IconButton>
                      <IconButton label="Edit" onClick={() => void startEdit(l.id)}>
                        <Pencil size={16} />
                      </IconButton>
                      <IconButton label="Delete" onClick={() => setConfirmDelete(l.id)}>
                        <Trash2 size={16} />
                      </IconButton>
                    </>
                  )}
                </li>
              ),
            )}
          </ul>
        )}
      </div>

      <h3 className={page.groupTitle}>Never saved</h3>
      <p className={page.hint}>Sites where you chose “Never”. Veyscape does not offer to save passwords there.</p>
      <div className={page.group}>
        {never.length === 0 ? (
          <p className={`${page.hint} ${page.field}`}>No sites.</p>
        ) : (
          <ul className={page.tableList}>
            {never.map((origin) => (
              <li key={origin}>
                <span className={page.grow} data-selectable>
                  {origin}
                </span>
                <button type="button" className={controls.button} onClick={() => void attempt(() => api.passwords.neverRemove(origin))}>
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

function EditLogin({ login, password, onDone }: { login: Login; password: string; onDone: (saved: boolean) => void }) {
  const [username, setUsername] = useState(login.username);
  const [value, setValue] = useState(password);
  const [error, setError] = useState('');
  const save = async () => {
    try {
      await api.passwords.update(login.id, username, value);
      onDone(true);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <form
      className={styles.edit}
      aria-label={`Edit the password for ${displayHost(login.origin)}`}
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label htmlFor={`user-${login.id}`}>Username</label>
      <input id={`user-${login.id}`} className={controls.input} value={username} maxLength={256} onChange={(e) => setUsername(e.target.value)} autoFocus />
      <label htmlFor={`pass-${login.id}`}>Password</label>
      <input id={`pass-${login.id}`} className={controls.input} value={value} maxLength={1024} required onChange={(e) => setValue(e.target.value)} />
      {error && (
        <p className={page.error} role="alert">
          {error}
        </p>
      )}
      <div className={styles.editActions}>
        <button type="submit" className={`${controls.button} ${controls.primary}`}>
          Save
        </button>
        <button type="button" className={controls.button} onClick={() => onDone(false)}>
          Cancel
        </button>
      </div>
    </form>
  );
}
