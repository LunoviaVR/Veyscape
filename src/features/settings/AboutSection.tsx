import { useEffect, useState } from 'react';
import logo from '../../../assets/logo.png';
import { api } from '../../lib/ipc';
import type { AppInfo } from '../../types/browser';
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
        <div className={styles.field}>
          <span className={styles.label}>Updates</span>
          <span>Veyscape never checks for updates in the background. New versions are published on GitHub.</span>
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
