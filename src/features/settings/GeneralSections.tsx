import { useId, useState, type ReactNode } from 'react';
import { Switch } from '../../components/Switch';
import type { Density, NewTabBackground, SearchEngine, StartupBehavior, ThemeSetting } from '../../types/browser';
import { SEARCH_ENGINE_NAMES } from '../navigation/suggestions';
import { useSettingsUpdate } from './useSettingsUpdate';
import controls from '../../components/controls.module.css';
import styles from './Settings.module.css';

export function SelectField<T extends string>({ label, value, options, onChange }: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  const id = useId();
  return (
    <div className={styles.field}>
      <label htmlFor={id}>{label}</label>
      <select id={id} className={controls.select} value={value} onChange={(e) => onChange(e.target.value as T)}>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </div>
  );
}

/** Text setting saved on blur/Enter, so half-typed values are never applied. Re-key it to reset. */
function TextField({ label, value, placeholder, hint, onSave }: { label: string; value: string; placeholder?: string; hint?: ReactNode; onSave: (v: string) => void }) {
  const id = useId();
  const [draft, setDraft] = useState(value);
  return (
    <div className={styles.stack}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <input
        id={id}
        className={controls.input}
        value={draft}
        placeholder={placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => draft !== value && onSave(draft)}
        onKeyDown={(e) => e.key === 'Enter' && draft !== value && onSave(draft)}
      />
      {hint && <p className={styles.hint}>{hint}</p>}
    </div>
  );
}

export function GeneralSection() {
  const { settings, update, error } = useSettingsUpdate();
  if (!settings) return null;
  return (
    <>
      <div className={styles.group}>
        <SelectField<StartupBehavior>
          label="On startup"
          value={settings.startup}
          options={[
            { value: 'restoreSession', label: 'Restore previous tabs' },
            { value: 'newTab', label: 'Open a new tab' },
            { value: 'homepage', label: 'Open the homepage' },
          ]}
          onChange={(startup) => void update({ startup })}
        />
        <TextField key={settings.homepage} label="Homepage" value={settings.homepage} placeholder="https://" onSave={(homepage) => void update({ homepage })} />
        <SelectField<SearchEngine>
          label="Search engine"
          value={settings.searchEngine}
          options={(Object.keys(SEARCH_ENGINE_NAMES) as SearchEngine[]).map((value) => ({ value, label: SEARCH_ENGINE_NAMES[value] }))}
          onChange={(searchEngine) => void update({ searchEngine })}
        />
        <TextField
          key={settings.downloadDir ?? ''}
          label="Download folder"
          value={settings.downloadDir ?? ''}
          placeholder="Windows Downloads folder"
          hint="Leave empty to use your Downloads folder. Must be a full path, for example D:\Downloads."
          onSave={(dir) => void update({ downloadDir: dir.trim() || null })}
        />
      </div>
      <p className={`${styles.hint} ${styles.note}`}>
        Address-bar suggestions come only from your local history and bookmarks; nothing you type is sent to a search engine until you press Enter.
      </p>
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
    </>
  );
}

export function AppearanceSection() {
  const { settings, update } = useSettingsUpdate();
  if (!settings) return null;
  return (
    <>
      <div className={styles.group}>
        <SelectField<ThemeSetting>
          label="Theme"
          value={settings.theme}
          options={[
            { value: 'system', label: 'Follow system' },
            { value: 'light', label: 'Light' },
            { value: 'dark', label: 'Dark' },
          ]}
          onChange={(theme) => void update({ theme })}
        />
        <Switch label="Show home button" checked={settings.showHomeButton} onChange={(v) => void update({ showHomeButton: v })} />
        <Switch
          label="Show bookmarks bar"
          description="Ctrl+Shift+B"
          checked={settings.showBookmarkBar}
          onChange={(v) => void update({ showBookmarkBar: v })}
        />
        <SelectField<Density>
          label="Interface density"
          value={settings.density}
          options={[
            { value: 'comfortable', label: 'Comfortable' },
            { value: 'compact', label: 'Compact' },
          ]}
          onChange={(density) => void update({ density })}
        />
      </div>
      <h3 className={styles.groupTitle}>New tab page</h3>
      <div className={styles.group}>
        <Switch label="Show frequently visited sites" checked={settings.newtabShowTopSites} onChange={(v) => void update({ newtabShowTopSites: v })} />
        <Switch label="Show bookmarks" checked={settings.newtabShowBookmarks} onChange={(v) => void update({ newtabShowBookmarks: v })} />
        <SelectField<NewTabBackground>
          label="Background"
          value={settings.newtabBackground}
          options={[
            { value: 'none', label: 'Plain' },
            { value: 'mist', label: 'Mist' },
            { value: 'dusk', label: 'Dusk' },
            { value: 'forest', label: 'Forest' },
          ]}
          onChange={(newtabBackground) => void update({ newtabBackground })}
        />
      </div>
    </>
  );
}
