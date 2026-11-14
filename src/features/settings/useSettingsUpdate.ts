import { useState } from 'react';
import { api, errorMessage } from '../../lib/ipc';
import { settingsStore, useSettings } from '../../lib/store';
import type { Settings } from '../../types/browser';

/** Current settings plus a patch function that persists through the backend (which validates). */
export function useSettingsUpdate() {
  const settings = useSettings();
  const [error, setError] = useState('');
  const update = async (patch: Partial<Settings>) => {
    const current = settingsStore.get();
    if (!current) return;
    setError('');
    try {
      settingsStore.set(await api.settings.update({ ...current, ...patch }));
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return { settings, update, error };
}
