import { useEffect, useState } from 'react';
import logo from '../../../assets/logo.png';
import { api, errorMessage } from '../../lib/ipc';
import { announce } from '../../lib/ui';
import type { AppInfo, UpdateInfo } from '../../types/browser';
import controls from '../../components/controls.module.css';
import styles from './Settings.module.css';

const LICENSES = [
  ['Tauri', 'MIT / Apache-2.0'],
  ['adblock-rust (Brave)', 'MPL-2.0'],
  ['rusqlite / SQLite', 'MIT / Public domain'],
  ['webview2-com, windows-rs', 'MIT / Apache-2.0'],
  ['React', 'MIT'],
  ['Lucide icons', 'ISC'],
  ['EasyList, EasyPrivacy, Fanboy lists', 'GPL-3.0 / CC BY-SA 3.0 (downloaded from easylist.to)'],
  ['Malicious URL Blocklist (URLhaus)', 'CC0 / MIT'],
] as const;

export function AboutSection() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => void api.appInfo().then(setInfo), []);
  const [checking, setChecking] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState('');

  const check = async () => {
    setChecking(true);
    setError('');
    try {
      const result = await api.checkForUpdates();
      setUpdate(result);
      announce(result.available ? `Veyscape ${result.latest} is available` : 'Veyscape is up to date');
    } catch (e) {
      setUpdate(null);
      setError(`Could not check for updates: ${errorMessage(e)}`);
    } finally {
      setChecking(false);
    }
  };

  return (
    <>
      <div className={styles.brand}>
        <img src={logo} alt="" width={72} height={72} draggable={false} />
        <div>
          <p className={styles.brandName}>Veyscape</p>
          <p className={styles.hint}>A privacy-first browser</p>
        </div>
      </div>
      <div className={styles.group}>
        <div className={styles.field}>
          <span className={styles.label}>Veyscape</span>
          <span data-selectable>{info?.version ?? '…'}</span>
        </div>
        <div className={styles.field}>
          <span className={styles.label}>Tauri</span>
          <span data-selectable>{info?.tauriVersion ?? '…'}</span>
        </div>
        <div className={styles.field}>
          <span className={styles.label}>Web engine</span>
          <span data-selectable>{info?.engine ?? '…'}</span>
        </div>
        <div className={styles.stack}>
          <div className={styles.row}>
            <span className={`${styles.label} ${styles.grow}`}>Updates</span>
            <button type="button" className={controls.button} disabled={checking} onClick={() => void check()}>
              {checking ? 'Checking…' : 'Check for updates'}
            </button>
          </div>
          <p className={styles.hint}>
            Veyscape never checks in the background. The button asks GitHub for the latest release, nothing else is sent.
          </p>
          {update &&
            (update.available ? (
              <div className={styles.row}>
                <span className={styles.grow}>Veyscape {update.latest} is available.</span>
                <button
                  type="button"
                  className={`${controls.button} ${controls.primary}`}
                  onClick={() => void api.tabs.create({ url: update.url })}
                >
                  Open download page
                </button>
              </div>
            ) : (
              <p className={styles.success}>You have the latest version ({update.latest}).</p>
            ))}
          {error && (
            <p className={styles.error} role="alert">
              {error}
            </p>
          )}
        </div>
      </div>
      <p className={`${styles.hint} ${styles.note}`}>
        No telemetry, no accounts, no sync. The web engine (the system WebView2 runtime) is a Windows component whose own
        diagnostic data follows your Windows privacy settings.
      </p>
      <h3 className={styles.groupTitle}>Open-source licenses</h3>
      <ul className={`${styles.group} ${styles.tableList}`}>
        {LICENSES.map(([name, license]) => (
          <li key={name}>
            <span className={styles.grow}>{name}</span>
            <span className={styles.hint}>{license}</span>
          </li>
        ))}
      </ul>
    </>
  );
}
