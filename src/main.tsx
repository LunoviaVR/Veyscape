import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { OverlayApp } from './app/OverlayApp';
import { ShellApp } from './app/ShellApp';
import { useTheme } from './hooks/useTheme';
import { api, onEvent } from './lib/ipc';
import { connectShellStores, settingsStore } from './lib/store';
import './styles/global.css';

// One bundle, two trusted webviews: the shell (default) and the popup overlay (?view=overlay).
const isOverlay = new URLSearchParams(window.location.search).get('view') === 'overlay';

function Overlay() {
  useTheme();
  return <OverlayApp />;
}

async function start() {
  const root = createRoot(document.getElementById('root')!);
  if (isOverlay) {
    document.documentElement.dataset.view = 'overlay';
    settingsStore.set(await api.settings.get());
    void onEvent('settings:changed', (s) => settingsStore.set(s));
    root.render(
      <StrictMode>
        <Overlay />
      </StrictMode>,
    );
    return;
  }
  await connectShellStores();
  root.render(
    <StrictMode>
      <ShellApp />
    </StrictMode>,
  );
}

void start();
