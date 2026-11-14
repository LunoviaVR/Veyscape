import { useId, type ReactNode } from 'react';
import styles from './page.module.css';

/** Layout for internal pages (History, Settings, ...): a centered, readable column. */
export function Page({ title, actions, children, wide }: { title: string; actions?: ReactNode; children: ReactNode; wide?: boolean }) {
  // useId: several pages can be on screen at once (split view).
  const titleId = useId();
  return (
    <main className={styles.page} aria-labelledby={titleId}>
      <div className={`${styles.column} ${wide ? styles.wide : ''}`}>
        <header className={styles.header}>
          <h1 id={titleId} className={styles.title}>
            {title}
          </h1>
          {actions && <div className={styles.actions}>{actions}</div>}
        </header>
        {children}
      </div>
    </main>
  );
}

export function EmptyState({ children }: { children: ReactNode }) {
  return <p className={styles.empty}>{children}</p>;
}
