import { useEffect } from 'react';
import { useSettings } from '../lib/store';

/** Applies theme and density from settings to <html>; "system" follows the OS live. */
export function useTheme() {
  const settings = useSettings();
  const theme = settings?.theme ?? 'system';
  const density = settings?.density ?? 'comfortable';

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.density = density;
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      root.dataset.theme = theme === 'system' ? (media.matches ? 'dark' : 'light') : theme;
    };
    apply();
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, [theme, density]);
}
