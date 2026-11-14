import { KeyRound } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { useActiveTab } from '../../lib/store';
import { hideOverlay, onOverlayAction, overlayKind, showOverlay } from '../overlay/client';

const WIDTH = 320;
// Popup chrome (heading, "Manage passwords") plus one row per login, up to five before it scrolls.
const popupHeight = (logins: number) => 112 + 44 * Math.min(logins, 5);

/** Key in the address bar when the page has a login form and logins are saved for the site. */
export function KeyButton({ className }: { className?: string }) {
  const tab = useActiveTab();
  const ref = useRef<HTMLButtonElement>(null);
  const wasOpen = useRef(false);

  useEffect(
    () =>
      onOverlayAction((a) => {
        if (a.type === 'closed' && a.reason === 'escape' && a.kind === 'passwords') ref.current?.focus();
      }),
    [],
  );

  if (!tab?.savedLogins) return null;
  const toggle = () => {
    // The popup closes itself on blur before the click lands; the pointerdown saw it open.
    if (wasOpen.current || overlayKind() === 'passwords') {
      wasOpen.current = false;
      return void hideOverlay(true);
    }
    const r = ref.current!.getBoundingClientRect();
    const height = popupHeight(tab.savedLogins);
    void showOverlay({ kind: 'passwords', tabId: tab.id }, { x: Math.max(8, r.right - WIDTH), y: r.bottom + 8, width: WIDTH, height }, true);
  };

  const label = tab.savedLogins === 1 ? 'Saved password for this site' : `${tab.savedLogins} saved passwords for this site`;
  return (
    <button
      ref={ref}
      type="button"
      className={className}
      aria-label={label}
      title={label}
      aria-haspopup="dialog"
      onPointerDown={() => (wasOpen.current = overlayKind() === 'passwords')}
      onClick={toggle}
    >
      <KeyRound size={15} />
    </button>
  );
}
