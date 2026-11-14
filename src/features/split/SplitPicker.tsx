import { X } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { api, errorMessage, type SplitChoice } from '../../lib/ipc';
import { PREVIEW, SPLIT_OPTIONS, previewRects } from './layouts';
import controls from '../../components/controls.module.css';
import styles from './SplitPicker.module.css';

/** Visual split-view layout chooser (rendered in the overlay, above web content). */
export function SplitPicker({ current, onClose }: { current: SplitChoice | null; onClose: () => void }) {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    const root = rootRef.current;
    (root?.querySelector<HTMLElement>('[aria-current="true"]') ?? root?.querySelector<HTMLElement>('button'))?.focus();
  }, []);

  const run = async (fn: () => Promise<void>) => {
    try {
      await fn();
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div ref={rootRef} className={styles.picker} role="dialog" aria-labelledby={`${id}-h`}>
      <div>
        <h2 id={`${id}-h`} className={styles.heading}>
          Split view
        </h2>
        <p className={styles.hint}>The current tab is always one of the pages.</p>
      </div>
      {[2, 3, 4].map((count) => (
        <section key={count} className={styles.group} aria-labelledby={`${id}-${count}`}>
          <h3 id={`${id}-${count}`} className={styles.groupLabel}>
            {count} pages
          </h3>
          <div className={styles.tiles}>
            {SPLIT_OPTIONS.filter((o) => o.count === count).map((o) => {
              const selected = current?.count === o.count && current.layout === o.layout;
              const name = `${o.label}: ${o.description}`;
              return (
                <button
                  key={o.layout}
                  type="button"
                  className={styles.tile}
                  aria-label={name}
                  aria-current={selected || undefined}
                  title={name}
                  onClick={() => void run(() => api.split.set(o.count, o.layout))}
                >
                  <svg className={styles.preview} viewBox={`0 0 ${PREVIEW.width} ${PREVIEW.height}`} aria-hidden>
                    {previewRects(o.layout, o.count).map(([x, y, w, h], i) => (
                      <rect key={i} className={styles.pane} x={x} y={y} width={w} height={h} rx={2} />
                    ))}
                  </svg>
                  <span>{o.label}</span>
                </button>
              );
            })}
          </div>
        </section>
      ))}
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      {current && (
        <button type="button" className={`${controls.button} ${styles.close}`} onClick={() => void run(() => api.split.close())}>
          <X size={15} aria-hidden /> Close split view
        </button>
      )}
    </div>
  );
}
