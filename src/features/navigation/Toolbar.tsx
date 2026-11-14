import { ArrowLeft, ArrowRight, Columns2, Download, EllipsisVertical, House, RotateCw, X } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/ipc';
import { useActiveTab, useBrowserState, useDownloads, useSettings } from '../../lib/store';
import { ShieldButton } from '../adblock/ShieldButton';
import { hideOverlay, onOverlayAction, overlayKind, showOverlay } from '../overlay/client';
import { runAction } from '../shortcuts/actions';
import { AddressBar } from './AddressBar';
import styles from './Toolbar.module.css';

const SESSION_START = Date.now();
// Split picker size without the "Close split view" button (which adds 44px).
const SPLIT_PICKER_WIDTH = 360;
const SPLIT_PICKER_HEIGHT = 404;

export function Toolbar() {
  const tab = useActiveTab();
  const downloads = useDownloads();
  const { canReopenClosed, split } = useBrowserState();
  const settings = useSettings();
  const splitRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLButtonElement>(null);
  const pickerWasOpen = useRef(false);
  const active = downloads.filter((d) => d.state === 'inProgress').length;
  // "Downloads when relevant": only after something was downloaded in this session.
  const showDownloads = downloads.some((d) => d.startedAt >= SESSION_START - 1000);

  useEffect(
    () =>
      onOverlayAction((a) => {
        if (a.type === 'closed' && a.reason === 'escape' && a.kind === 'split') splitRef.current?.focus();
      }),
    [],
  );

  return (
    <div className={styles.toolbar} role="toolbar" aria-label="Navigation">
      <IconButton label="Back (Alt+Left)" disabled={!tab?.canGoBack} onClick={() => tab && void api.nav.back(tab.id)}>
        <ArrowLeft size={17} />
      </IconButton>
      <IconButton label="Forward (Alt+Right)" disabled={!tab?.canGoForward} onClick={() => tab && void api.nav.forward(tab.id)}>
        <ArrowRight size={17} />
      </IconButton>
      {tab?.loading ? (
        <IconButton label="Stop loading" onClick={() => void api.nav.stop(tab.id)}>
          <X size={17} />
        </IconButton>
      ) : (
        <IconButton label="Reload (Ctrl+R)" disabled={!tab || !!tab.internal} onClick={() => tab && void api.nav.reload(tab.id)}>
          <RotateCw size={16} />
        </IconButton>
      )}
      {settings?.showHomeButton && (
        <IconButton label="Home (Alt+Home)" disabled={!tab} onClick={() => void runAction('goHome')}>
          <House size={16} />
        </IconButton>
      )}
      <AddressBar />
      <ShieldButton />
      <IconButton
        ref={splitRef}
        label={split?.shown ? 'Split view (on)' : 'Split view'}
        aria-haspopup="dialog"
        pressed={!!split?.shown}
        // The picker closes itself on blur before the click lands; remember it was open so the click toggles.
        onPointerDown={() => (pickerWasOpen.current = overlayKind() === 'split')}
        onClick={() => {
          if (pickerWasOpen.current || overlayKind() === 'split') {
            pickerWasOpen.current = false;
            return void hideOverlay(true);
          }
          const r = splitRef.current!.getBoundingClientRect();
          const current = split?.shown ? { count: split.tabs.length, layout: split.layout } : null;
          const size = { width: SPLIT_PICKER_WIDTH, height: current ? SPLIT_PICKER_HEIGHT + 44 : SPLIT_PICKER_HEIGHT };
          void showOverlay({ kind: 'split', current }, { x: Math.max(8, r.right - size.width), y: r.bottom + 4, ...size }, true);
        }}
      >
        <Columns2 size={17} />
      </IconButton>
      {showDownloads && (
        <IconButton label={active ? `Downloads, ${active} in progress (Ctrl+J)` : 'Downloads (Ctrl+J)'} onClick={() => void runAction('openDownloads')}>
          <span className={styles.badgeWrap}>
            <Download size={17} />
            {active > 0 && <span className={styles.activityDot} aria-hidden />}
          </span>
        </IconButton>
      )}
      <IconButton
        ref={menuRef}
        label="Menu"
        aria-haspopup="menu"
        onClick={() => {
          const r = menuRef.current!.getBoundingClientRect();
          void api.layout.mainMenu(canReopenClosed, r.right - 220, r.bottom + 2);
        }}
      >
        <EllipsisVertical size={17} />
      </IconButton>
    </div>
  );
}
