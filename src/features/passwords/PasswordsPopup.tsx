import { KeyRound, Settings2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, emitToWebview, errorMessage } from '../../lib/ipc';
import { displayHost } from '../../lib/format';
import type { Login, TabId } from '../../types/browser';
import controls from '../../components/controls.module.css';
import shield from '../adblock/Shield.module.css';
import styles from './Passwords.module.css';

/** Saved logins for the page (rendered in the overlay webview); picking one fills the form. */
export function PasswordsPopup({ tabId, onFilled }: { tabId: TabId; onFilled: () => void }) {
  const [logins, setLogins] = useState<Login[] | null>(null);
  const [error, setError] = useState('');
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.passwords.forTab(tabId).then(setLogins, (e) => setError(errorMessage(e)));
  }, [tabId]);

  useEffect(() => {
    if (logins) rootRef.current?.querySelector<HTMLElement>('button')?.focus();
  }, [logins]);

  const fill = (id: number) => api.passwords.fill(tabId, id).then(onFilled, (e) => setError(errorMessage(e)));

  const host = logins?.[0] ? displayHost(logins[0].origin) : '';
  return (
    <div className={shield.popup} role="dialog" aria-label={`Saved passwords for ${host}`} ref={rootRef} aria-busy={!logins}>
      <h2 className={shield.heading}>Saved passwords for {host}</h2>
      {logins && logins.length === 0 && <p className={shield.note}>No saved passwords for this site.</p>}
      <ul className={styles.list}>
        {logins?.map((l) => (
          <li key={l.id}>
            <button type="button" className={styles.login} aria-label={`Fill in ${l.username || 'password'}`} onClick={() => void fill(l.id)}>
              <KeyRound size={15} aria-hidden />
              <span className={styles.user}>{l.username || 'No username'}</span>
              <span className={styles.dots} aria-hidden>
                ••••••••
              </span>
            </button>
          </li>
        ))}
      </ul>
      {error && (
        <p className={shield.error} role="alert">
          {error}
        </p>
      )}
      <div className={shield.actions}>
        <button
          type="button"
          className={`${controls.button} ${shield.linkButton}`}
          onClick={() => void emitToWebview('shell', 'overlay:action', { type: 'openSettings', section: 'passwords' })}
        >
          <Settings2 size={15} aria-hidden /> Manage passwords
        </button>
      </div>
    </div>
  );
}
