import { Settings2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Switch } from '../../components/Switch';
import { api, emitToWebview, errorMessage, onEvent } from '../../lib/ipc';
import type { ProtectionStatus, TabId } from '../../types/browser';
import styles from './Shield.module.css';
import controls from '../../components/controls.module.css';

const TRACKING_LABEL = { off: 'Off', basic: 'Basic', balanced: 'Balanced', strict: 'Strict' } as const;

/** Compact protection popup (rendered in the overlay webview). */
export function ShieldPopup({ tabId }: { tabId: TabId }) {
  const [status, setStatus] = useState<ProtectionStatus | null>(null);
  const [error, setError] = useState('');
  const firstRef = useRef<HTMLDivElement>(null);

  const load = useCallback(() => {
    api.protection.status(tabId).then(setStatus, (e) => setError(errorMessage(e)));
  }, [tabId]);

  useEffect(() => {
    load();
    const un = onEvent('protection:changed', load);
    return () => void un.then((f) => f());
  }, [load]);

  useEffect(() => {
    if (status) firstRef.current?.querySelector<HTMLElement>('button')?.focus();
  }, [status]);

  const act = async (fn: () => Promise<void>, reload = false) => {
    try {
      await fn();
      load();
      if (reload) await emitToWebview('shell', 'overlay:action', { type: 'reloadTab', tabId });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  if (!status) return <div className={styles.popup} aria-busy="true" />;
  const siteOn = status.siteEnabled;
  const engineNote = status.engine.updating
    ? 'Updating filter lists…'
    : !status.engine.ready
      ? status.engine.lastError
        ? 'Filter lists unavailable — network blocking is not active yet.'
        : 'Downloading filter lists…'
      : null;

  return (
    <div className={styles.popup} role="dialog" aria-label="Protection" ref={firstRef}>
      <h2 className={styles.heading}>{status.host ?? 'Protection'}</h2>
      <Switch
        label="Ad and tracker blocker"
        description={status.adblockEnabled ? 'Enabled on all sites' : 'Disabled everywhere'}
        checked={status.adblockEnabled}
        onChange={(v) => void act(() => api.protection.setEnabled(v), true)}
      />
      <dl className={styles.stats}>
        <div>
          <dt>Blocked on this page</dt>
          <dd>{status.adblockEnabled && siteOn ? status.blockedOnPage : '—'}</dd>
        </div>
        <div>
          <dt>Tracking protection</dt>
          <dd>{TRACKING_LABEL[status.trackingPrevention]}</dd>
        </div>
      </dl>
      {engineNote && <p className={styles.note}>{engineNote}</p>}
      {status.private && status.adblockEnabled && status.host && (
        <p className={styles.note}>Private tab: this site choice is not saved and ends with your last private tab.</p>
      )}
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      <div className={styles.actions}>
        {status.host && status.adblockEnabled && (
          <button
            type="button"
            className={`${controls.button} ${siteOn ? '' : controls.primary}`}
            onClick={() => void act(() => api.protection.setSiteEnabled(status.host!, !siteOn, tabId), true)}
          >
            {siteOn ? 'Disable protection for this site' : 'Re-enable protection for this site'}
          </button>
        )}
        <button
          type="button"
          className={`${controls.button} ${styles.linkButton}`}
          onClick={() => void emitToWebview('shell', 'overlay:action', { type: 'openSettings', section: 'adblock' })}
        >
          <Settings2 size={15} aria-hidden /> Protection settings
        </button>
      </div>
    </div>
  );
}
