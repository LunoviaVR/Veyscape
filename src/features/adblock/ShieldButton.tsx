import { Shield, ShieldOff } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/ipc';
import { useActiveTab, useSettings } from '../../lib/store';
import { hideOverlay, onOverlayAction, overlayKind, showOverlay } from '../overlay/client';
import { openInternal } from '../shortcuts/actions';
import styles from './Shield.module.css';

const SHIELD_POPUP_SIZE = { width: 320, height: 256 };

/** Privacy shield beside the address bar; opens the compact protection popup. */
export function ShieldButton() {
  const tab = useActiveTab();
  const settings = useSettings();
  const ref = useRef<HTMLButtonElement>(null);
  const enabled = settings?.adblockEnabled ?? true;

  useEffect(
    () =>
      onOverlayAction((a) => {
        if (a.type === 'reloadTab') void api.nav.reload(a.tabId);
        if (a.type === 'openSettings') void openInternal(`veyscape://settings/${a.section}`);
        if (a.type === 'closed' && a.reason === 'escape' && a.kind === 'shield') ref.current?.focus();
      }),
    [],
  );

  const toggle = () => {
    if (!tab) return;
    if (overlayKind() === 'shield') return void hideOverlay(true);
    const r = ref.current!.getBoundingClientRect();
    const x = Math.max(8, r.right - SHIELD_POPUP_SIZE.width);
    // Private tabs show one more note (site choices are not saved).
    const height = SHIELD_POPUP_SIZE.height + (tab.private ? 48 : 0);
    void showOverlay({ kind: 'shield', tabId: tab.id }, { x, y: r.bottom + 4, width: SHIELD_POPUP_SIZE.width, height }, true);
  };

  const label = !enabled
    ? 'Protection is off'
    : `Protection: ${tab?.blocked ?? 0} ${tab?.blocked === 1 ? 'request' : 'requests'} blocked on this page`;

  return (
    <IconButton ref={ref} label={label} aria-haspopup="dialog" onClick={toggle} className={enabled ? styles.shieldOn : ''}>
      {enabled ? <Shield size={17} /> : <ShieldOff size={17} />}
    </IconButton>
  );
}
