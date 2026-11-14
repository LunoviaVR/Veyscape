import { Bookmark, Clock, Globe, Search, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { ShieldPopup } from '../features/adblock/ShieldPopup';
import { BookmarkEditor } from '../features/bookmarks/BookmarkEditor';
import { PasswordsPopup } from '../features/passwords/PasswordsPopup';
import { SplitPicker } from '../features/split/SplitPicker';
import { GroupEditor } from '../features/tabs/GroupEditor';
import { api, emitToWebview, onEvent, type OverlayContent, type SuggestionRow } from '../lib/ipc';
import styles from './OverlayApp.module.css';

// What is showing; read by close() so the shell knows which popup ended (to return focus to its button).
let shown: OverlayContent | null = null;

const close = async (reason: 'escape' | 'blur' | 'done' | 'page') => {
  // Focus goes back to the browser UI unless it already moved elsewhere (a click outside, the page).
  await api.layout.overlayHide(reason === 'escape' || reason === 'done');
  await emitToWebview('shell', 'overlay:action', { type: 'closed', reason, kind: shown?.kind });
};

/** Root of the overlay webview: popups that must draw above web content. */
export function OverlayApp() {
  const [content, setContent] = useState<OverlayContent | null>(null);
  // True while the system colour dialog is open: it takes focus, which must not close the popup.
  const keepOpenRef = useRef(false);

  useEffect(() => {
    const un = onEvent('overlay:render', (c) => {
      shown = c;
      setContent(c);
    });
    void un.then(() => emitToWebview('shell', 'overlay:ready', null));
    const onBlur = () => {
      // Suggestions never take focus; interactive popups close when focus leaves them.
      if (shown && shown.kind !== 'suggestions' && !keepOpenRef.current) void close('blur');
    };
    const onFocus = () => (keepOpenRef.current = false);
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && void close('escape');
    window.addEventListener('blur', onBlur);
    window.addEventListener('focus', onFocus);
    window.addEventListener('keydown', onKey);
    return () => {
      void un.then((f) => f());
      window.removeEventListener('blur', onBlur);
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('keydown', onKey);
    };
  }, []);

  if (!content) return null;
  return (
    <div className={styles.frame}>
      {content.kind === 'suggestions' ? (
        <Suggestions items={content.items} selected={content.selected} />
      ) : content.kind === 'shield' ? (
        <ShieldPopup key={content.tabId} tabId={content.tabId} />
      ) : content.kind === 'passwords' ? (
        <PasswordsPopup key={content.tabId} tabId={content.tabId} onFilled={() => void close('page')} />
      ) : content.kind === 'split' ? (
        <SplitPicker current={content.current} onClose={() => void close('done')} />
      ) : content.kind === 'groupEditor' ? (
        <GroupEditor
          key={content.groupId}
          request={content}
          onClose={() => void close('done')}
          onPickerOpen={() => (keepOpenRef.current = true)}
        />
      ) : (
        <BookmarkEditor
          key={`${content.mode}-${content.id}-${content.parentId}`}
          request={content}
          pageUrl={content.pageUrl}
          pageTitle={content.pageTitle}
          onClose={() => void close('done')}
          onPickerOpen={() => (keepOpenRef.current = true)}
        />
      )}
    </div>
  );
}

const ICONS = { search: Search, url: Globe, history: Clock, bookmark: Bookmark } as const;

function Suggestions({ items, selected }: { items: SuggestionRow[]; selected: number }) {
  return (
    <ul className={styles.list} role="listbox" aria-label="Suggestions">
      {items.map((row, i) => {
        const Icon = ICONS[row.kind];
        return (
          <li
            key={row.key}
            role="option"
            aria-selected={i === selected}
            className={styles.row}
            // mousedown fires before the address bar loses focus.
            onMouseDown={(e) => {
              e.preventDefault();
              void emitToWebview('shell', 'overlay:action', { type: 'pick', index: i });
            }}
          >
            <Icon size={15} aria-hidden className={styles.rowIcon} />
            <span className={styles.primary}>{row.primary}</span>
            <span className={styles.secondary}>{row.secondary}</span>
            {row.historyId !== null && (
              // Mouse only: focus stays in the address bar, where Shift+Delete does the same.
              <button
                type="button"
                className={styles.remove}
                tabIndex={-1}
                aria-hidden
                title="Remove from history (Shift+Delete)"
                onMouseDown={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  void emitToWebview('shell', 'overlay:action', { type: 'removeSuggestion', index: i });
                }}
              >
                <X size={14} />
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
}
