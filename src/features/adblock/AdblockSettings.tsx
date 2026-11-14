import { Trash2 } from 'lucide-react';
import { useCallback, useEffect, useId, useState } from 'react';
import { IconButton } from '../../components/IconButton';
import { Switch } from '../../components/Switch';
import { api, errorMessage, onEvent } from '../../lib/ipc';
import { announce } from '../../lib/ui';
import type { FilterList, ProtectionStatus } from '../../types/browser';
import { useSettingsUpdate } from '../settings/useSettingsUpdate';
import controls from '../../components/controls.module.css';
import styles from '../settings/Settings.module.css';

export function AdblockSettings() {
  const { settings, update } = useSettingsUpdate();
  const [lists, setLists] = useState<FilterList[]>([]);
  const [status, setStatus] = useState<ProtectionStatus | null>(null);
  const [exceptions, setExceptions] = useState<string[]>([]);
  const [error, setError] = useState('');

  const reload = useCallback(() => {
    void api.protection.lists().then(setLists);
    void api.protection.status(null).then(setStatus);
    void api.protection.exceptions().then(setExceptions);
  }, []);

  useEffect(() => {
    reload();
    const un = onEvent('protection:changed', reload);
    return () => void un.then((f) => f());
  }, [reload]);

  /** Resolves true when `fn` succeeded. */
  const run = async (fn: () => Promise<unknown>, done?: string) => {
    setError('');
    try {
      await fn();
      if (done) announce(done);
      return true;
    } catch (e) {
      setError(errorMessage(e));
      return false;
    } finally {
      reload();
    }
  };

  if (!settings) return null;
  const engine = status?.engine;

  return (
    <>
      <div className={styles.group}>
        <Switch label="Block ads and trackers" checked={settings.adblockEnabled} onChange={(v) => void update({ adblockEnabled: v })} />
        <Switch
          label="Hide leftover ad space (cosmetic filtering)"
          description="Removes empty ad frames using CSS rules from the same lists. Applies from the next page load."
          checked={settings.cosmeticFiltering}
          onChange={(v) => void update({ cosmeticFiltering: v })}
        />
        <div className={styles.field}>
          <span className={styles.label}>
            {engine?.updating
              ? 'Updating filter lists…'
              : engine?.ready
                ? `${engine.ruleListsActive} lists active · ${engine.totalBlocked} requests blocked since start`
                : 'No filter lists loaded yet'}
          </span>
          <button type="button" className={controls.button} disabled={engine?.updating} onClick={() => void run(() => api.protection.updateLists(), 'Filter lists updated')}>
            Update now
          </button>
        </div>
      </div>
      {(error || engine?.lastError) && (
        <p className={styles.error} role="alert">
          {error || engine?.lastError}
        </p>
      )}

      <h3 className={styles.groupTitle}>Filter lists</h3>
      <p className={styles.hint}>Lists are downloaded directly from their maintainers over HTTPS and refreshed every few days. Cached copies keep protection working offline.</p>
      <ul className={`${styles.group} ${styles.tableList}`}>
        {lists.map((l) => (
          <li key={l.id}>
            <input
              type="checkbox"
              checked={l.enabled}
              aria-label={`Use ${l.title}`}
              onChange={(e) => void run(() => api.protection.setListEnabled(l.id, e.target.checked))}
            />
            <span className={styles.grow}>
              {l.title}
              <br />
              <span className={styles.hint}>
                {l.lastError ? `Update failed: ${l.lastError}` : l.lastUpdated ? `Updated ${new Date(l.lastUpdated).toLocaleDateString()}` : 'Not downloaded yet'} · {l.license}
              </span>
            </span>
            {!l.builtin && (
              <IconButton label={`Remove ${l.title}`} onClick={() => void run(() => api.protection.removeList(l.id), 'List removed')}>
                <Trash2 size={15} />
              </IconButton>
            )}
          </li>
        ))}
      </ul>
      <AddList onAdd={(url, title) => run(() => api.protection.addList(url, title), 'List added')} />

      <details className={styles.details}>
        <summary>Advanced: custom rules and site exceptions</summary>
        <CustomRules onSaved={reload} />
        <h3 className={styles.groupTitle}>Sites with protection disabled</h3>
        <div className={styles.group}>
          {exceptions.length === 0 ? (
            <p className={`${styles.hint} ${styles.field}`}>None. Use the shield next to the address bar to disable protection for a site.</p>
          ) : (
            <ul className={styles.tableList}>
              {exceptions.map((host) => (
                <li key={host}>
                  <span className={styles.grow} data-selectable>
                    {host}
                  </span>
                  <button type="button" className={controls.button} onClick={() => void run(() => api.protection.setSiteEnabled(host, true), `Protection re-enabled for ${host}`)}>
                    Re-enable
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </details>
    </>
  );
}

function AddList({ onAdd }: { onAdd: (url: string, title: string) => Promise<boolean> }) {
  const [url, setUrl] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <form
      className={`${styles.row} ${styles.note}`}
      onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        void onAdd(url.trim(), '').then((ok) => {
          setBusy(false);
          if (ok) setUrl('');
        });
      }}
    >
      <input
        className={`${controls.input} ${styles.wideInput}`}
        type="url"
        value={url}
        placeholder="https://example.com/filters.txt"
        aria-label="Filter list URL"
        onChange={(e) => setUrl(e.target.value)}
      />
      <button type="submit" className={controls.button} disabled={!url.trim() || busy}>
        {busy ? 'Adding…' : 'Add list'}
      </button>
    </form>
  );
}

function CustomRules({ onSaved }: { onSaved: () => void }) {
  const id = useId();
  const [rules, setRules] = useState('');
  const [saved, setSaved] = useState('');
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => {
    void api.protection.customRules().then((r) => {
      setRules(r);
      setSaved(r);
    });
  }, []);

  const save = async () => {
    try {
      await api.protection.setCustomRules(rules);
      setSaved(rules);
      setMessage({ ok: true, text: 'Rules saved and applied.' });
      onSaved();
    } catch (e) {
      setMessage({ ok: false, text: errorMessage(e) });
    }
  };

  return (
    <div className={styles.stack}>
      <label htmlFor={id} className={styles.groupTitle}>
        Custom rules
      </label>
      <p id={`${id}-hint`} className={styles.hint}>
        One rule per line in Adblock Plus syntax. Block: <code>||ads.example.com^</code> · Allow: <code>@@||example.com/ad-free.js</code> · Hide: <code>example.com##.banner</code>
      </p>
      <textarea
        id={id}
        className={controls.textarea}
        rows={8}
        spellCheck={false}
        aria-describedby={`${id}-hint`}
        value={rules}
        onChange={(e) => setRules(e.target.value)}
      />
      <div className={styles.row}>
        <button type="button" className={`${controls.button} ${controls.primary}`} disabled={rules === saved} onClick={() => void save()}>
          Save rules
        </button>
      </div>
      {message && (
        <p className={message.ok ? styles.success : styles.error} role="status">
          {message.text}
        </p>
      )}
    </div>
  );
}
