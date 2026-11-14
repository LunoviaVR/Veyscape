import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import styles from './TabStrip.module.css';

/** Minimize / maximize / close for the undecorated main window. */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const w = getCurrentWindow();
    const sync = () => void w.isMaximized().then(setMaximized);
    sync();
    const unlisten = w.onResized(sync);
    return () => void unlisten.then((f) => f());
  }, []);

  const w = getCurrentWindow();
  return (
    <div className={styles.windowControls}>
      <button type="button" className={styles.windowButton} aria-label="Minimize" title="Minimize" onClick={() => void w.minimize()}>
        <Minus size={16} />
      </button>
      <button
        type="button"
        className={styles.windowButton}
        aria-label={maximized ? 'Restore' : 'Maximize'}
        title={maximized ? 'Restore' : 'Maximize'}
        onClick={() => void w.toggleMaximize()}
      >
        {maximized ? <Copy size={13} /> : <Square size={13} />}
      </button>
      <button type="button" className={`${styles.windowButton} ${styles.closeWindow}`} aria-label="Close Veyscape" title="Close" onClick={() => void w.close()}>
        <X size={16} />
      </button>
    </div>
  );
}
