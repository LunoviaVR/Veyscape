import { useId } from 'react';
import styles from './controls.module.css';

interface Props {
  label: string;
  description?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}

/** Labelled on/off switch (role="switch"); the whole row is clickable. */
export function Switch({ label, description, checked, disabled, onChange }: Props) {
  const id = useId();
  return (
    <div className={styles.switchRow}>
      <div className={styles.switchText}>
        <label htmlFor={id}>{label}</label>
        {description && (
          <p id={`${id}-d`} className={styles.description}>
            {description}
          </p>
        )}
      </div>
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        aria-describedby={description ? `${id}-d` : undefined}
        disabled={disabled}
        className={styles.switch}
        onClick={() => onChange(!checked)}
      >
        <span className={styles.thumb} />
      </button>
    </div>
  );
}
