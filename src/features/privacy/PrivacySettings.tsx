import { useEffect, useState } from 'react';
import { Switch } from '../../components/Switch';
import { api, errorMessage } from '../../lib/ipc';
import { announce } from '../../lib/ui';
import type { ClearDataRequest, PermissionKind, SitePermission, TimeRange, TrackingPrevention } from '../../types/browser';
import { SelectField } from '../settings/GeneralSections';
import { useSettingsUpdate } from '../settings/useSettingsUpdate';
import controls from '../../components/controls.module.css';
import styles from '../settings/Settings.module.css';

const PERMISSION_LABELS: Record<PermissionKind, string> = {
  camera: 'Camera',
  microphone: 'Microphone',
  location: 'Location',
  notifications: 'Notifications',
  popups: 'Pop-ups',
  automaticDownloads: 'Automatic downloads',
};

export function PrivacySettings() {
  const { settings, update, error } = useSettingsUpdate();
  // Remounting the permissions list reloads it after Delete data cleared them.
  const [permissionsKey, setPermissionsKey] = useState(0);
  if (!settings) return null;
  return (
    <>
      <div className={styles.group}>
        <Switch
          label="Block ads and trackers"
          description="Blocks requests to known ad, tracker and malware domains before they load."
          checked={settings.adblockEnabled}
          onChange={(v) => void update({ adblockEnabled: v })}
        />
        <SelectField<TrackingPrevention>
          label="Tracking prevention (engine)"
          value={settings.trackingPrevention}
          options={[
            { value: 'strict', label: 'Strict — may break some sites' },
            { value: 'balanced', label: 'Balanced (recommended)' },
            { value: 'basic', label: 'Basic' },
            { value: 'off', label: 'Off' },
          ]}
          onChange={(trackingPrevention) => void update({ trackingPrevention })}
        />
        <Switch
          label="HTTPS-only mode"
          description="Upgrades every site to a secure connection and asks before loading a site that only offers insecure HTTP."
          checked={settings.httpsOnly}
          onChange={(v) => void update({ httpsOnly: v })}
        />
        <Switch
          label="Delete cookies and site data when Veyscape closes"
          description="Signs you out of websites every time you quit."
          checked={settings.clearCookiesOnExit}
          onChange={(v) => void update({ clearCookiesOnExit: v })}
        />
      </div>
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      <ClearData onPermissionsCleared={() => setPermissionsKey((k) => k + 1)} />
      <SitePermissions key={permissionsKey} />
      <p className={`${styles.hint} ${styles.note}`}>
        Third-party cookie blocking is handled by the engine&apos;s tracking prevention; WebView2 does not offer a separate switch for it.
        Private tabs use an in-memory InPrivate session that is wiped when the last private tab closes.
      </p>
    </>
  );
}

function ClearData({ onPermissionsCleared }: { onPermissionsCleared: () => void }) {
  const [req, setReq] = useState<ClearDataRequest>({ history: true, downloads: false, cookies: true, cache: true, permissions: false, range: 'allTime' });
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const toggle = (key: keyof Omit<ClearDataRequest, 'range'>, label: string) => (
    <label className={styles.row}>
      <input type="checkbox" checked={req[key]} onChange={(e) => setReq({ ...req, [key]: e.target.checked })} />
      {label}
    </label>
  );

  const clear = async () => {
    setBusy(true);
    setStatus(null);
    try {
      await api.privacy.clear(req);
      if (req.permissions) onPermissionsCleared();
      setStatus({ ok: true, text: 'Browsing data deleted.' });
      announce('Browsing data deleted');
    } catch (e) {
      setStatus({ ok: false, text: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <h3 className={styles.groupTitle}>Delete browsing data</h3>
      <div className={styles.group}>
        <div className={styles.stack}>
          <SelectField<TimeRange>
            label="History time range"
            value={req.range}
            options={[
              { value: 'lastHour', label: 'Last hour' },
              { value: 'lastDay', label: 'Last 24 hours' },
              { value: 'lastWeek', label: 'Last 7 days' },
              { value: 'allTime', label: 'All time' },
            ]}
            onChange={(range) => setReq({ ...req, range })}
          />
          {toggle('history', req.range === 'allTime' ? 'Browsing history' : 'Sites visited in this period (all their entries are removed)')}
          {toggle('cookies', 'Cookies and site data (all time)')}
          {toggle('cache', 'Cached images and files (all time)')}
          {toggle('downloads', 'Download list (files stay on disk)')}
          {toggle('permissions', 'Site permissions')}
          <div className={styles.row}>
            <button type="button" className={`${controls.button} ${controls.primary}`} disabled={busy || !Object.entries(req).some(([k, v]) => k !== 'range' && v)} onClick={() => void clear()}>
              {busy ? 'Deleting…' : 'Delete data'}
            </button>
          </div>
          {status && (
            <p className={status.ok ? styles.success : styles.error} role="status">
              {status.text}
            </p>
          )}
        </div>
      </div>
    </>
  );
}

function SitePermissions() {
  const [items, setItems] = useState<SitePermission[]>([]);
  const reload = () => void api.privacy.permissions().then(setItems);
  useEffect(reload, []);

  const set = async (p: SitePermission, state: 'allow' | 'deny' | null) => {
    try {
      await api.privacy.setPermission(p.origin, p.kind, state);
      reload();
    } catch (e) {
      announce(errorMessage(e));
    }
  };

  return (
    <>
      <h3 className={styles.groupTitle}>Site permissions</h3>
      <p className={styles.hint}>Sites must ask before using your camera, microphone, location or notifications. Decisions you chose to remember are listed here.</p>
      <div className={styles.group}>
        {items.length === 0 ? (
          <p className={`${styles.hint} ${styles.field}`}>No saved site permissions.</p>
        ) : (
          <ul className={styles.tableList}>
            {items.map((p) => (
              <li key={`${p.origin}|${p.kind}`}>
                <span className={styles.grow} data-selectable>
                  {p.origin}
                </span>
                <span>{PERMISSION_LABELS[p.kind] ?? p.kind}</span>
                <select
                  className={controls.select}
                  aria-label={`${PERMISSION_LABELS[p.kind]} for ${p.origin}`}
                  value={p.state}
                  onChange={(e) => void set(p, e.target.value as 'allow' | 'deny')}
                >
                  <option value="allow">Allow</option>
                  <option value="deny">Block</option>
                </select>
                <button type="button" className={controls.button} onClick={() => void set(p, null)}>
                  Reset
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}
