import { Ban, Pipette } from 'lucide-react';
import { useRef } from 'react';
import { FOLDER_ICONS } from '../../lib/folderIcons';
import { groupColor, PRESET_COLORS, PRESET_HEX } from '../../lib/groupColor';
import styles from './StylePickers.module.css';

/** Icon choice (radio group of icon buttons), for bookmark folders and tab groups. */
export function IconPicker({
  value,
  color,
  onChange,
  allowNone = false,
  label = 'Folder icon',
}: {
  value: string | null;
  color: string | null;
  onChange: (icon: string | null) => void;
  /** Offer "No icon" first (tab groups may have none; folders always show one). */
  allowNone?: boolean;
  label?: string;
}) {
  const tint = (selected: boolean) => (color && selected ? { color: groupColor(color) } : undefined);
  return (
    <div className={styles.icons} role="radiogroup" aria-label={label}>
      {allowNone && (
        <button type="button" role="radio" aria-checked={value === null} aria-label="No icon" title="No icon" className={styles.iconChoice} onClick={() => onChange(null)}>
          <Ban size={18} aria-hidden />
        </button>
      )}
      {Object.entries(FOLDER_ICONS).map(([key, { icon: Icon, label: name }]) => (
        <button
          key={key}
          type="button"
          role="radio"
          aria-checked={value === key}
          aria-label={name}
          title={name}
          className={styles.iconChoice}
          style={tint(value === key)}
          onClick={() => onChange(key)}
        >
          <Icon size={18} aria-hidden />
        </button>
      ))}
    </div>
  );
}

/**
 * Colour: a preset, a custom colour from the system colour dialog, or (when allowed) none.
 * `onPickerOpen` lets a popup stay open while the (separate) colour dialog has focus.
 */
export function ColorPicker({
  value,
  onChange,
  onPickerOpen,
  allowNone = true,
  label = 'Folder colour',
}: {
  value: string | null;
  onChange: (color: string | null) => void;
  onPickerOpen?: () => void;
  /** Tab groups always have a colour, so they hide "No colour". */
  allowNone?: boolean;
  label?: string;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const custom = value?.startsWith('#') ? value : null;
  return (
    <div className={styles.colors} role="radiogroup" aria-label={label}>
      {allowNone && (
        <button type="button" role="radio" aria-checked={value === null} aria-label="No colour" title="No colour" className={styles.swatch} onClick={() => onChange(null)}>
          <Ban size={14} aria-hidden />
        </button>
      )}
      {PRESET_COLORS.map((c) => (
        <button
          key={c}
          type="button"
          role="radio"
          aria-checked={value === c}
          aria-label={c}
          title={c[0]!.toUpperCase() + c.slice(1)}
          className={styles.swatch}
          style={{ background: groupColor(c) }}
          onClick={() => onChange(c)}
        />
      ))}
      <button
        type="button"
        role="radio"
        aria-checked={!!custom}
        aria-label="Custom colour"
        title="Custom colour…"
        className={`${styles.swatch} ${styles.customSwatch}`}
        style={custom ? { background: custom } : undefined}
        onClick={() => {
          onPickerOpen?.();
          inputRef.current?.click();
        }}
      >
        {!custom && <Pipette size={14} aria-hidden />}
      </button>
      <input
        ref={inputRef}
        type="color"
        className="visually-hidden"
        tabIndex={-1}
        aria-hidden
        defaultValue={custom ?? PRESET_HEX[value ?? 'blue'] ?? '#2f6fe4'}
        onChange={(e) => onChange(e.target.value.toLowerCase())}
      />
    </div>
  );
}
