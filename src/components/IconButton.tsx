import type { ComponentPropsWithRef, ReactNode } from 'react';
import styles from './controls.module.css';

interface Props extends ComponentPropsWithRef<'button'> {
  /** Accessible name; also shown as the tooltip. Icons alone are never the only label. */
  label: string;
  children: ReactNode;
  pressed?: boolean;
}

export function IconButton({ label, children, pressed, className, ...rest }: Props) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      className={`${styles.iconButton} ${className ?? ''}`}
      {...rest}
    >
      {children}
    </button>
  );
}
