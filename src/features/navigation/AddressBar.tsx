import { Info, Lock, LockOpen, Search, Star } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import { api, errorMessage, type SuggestionRow } from '../../lib/ipc';
import { useActiveTab, useSettings } from '../../lib/store';
import { announce, useUi } from '../../lib/ui';
import { displayUrl } from '../../lib/format';
import { hideOverlay, onOverlayAction, overlayKind, showOverlay, updateOverlay } from '../overlay/client';
import { runAction } from '../shortcuts/actions';
import { buildRows } from './suggestions';
import styles from './Toolbar.module.css';
import { KeyButton } from '../passwords/KeyButton';

const ROW_HEIGHT = 40;

/** Unified address and search field with local-only suggestions. */
export function AddressBar() {
  const tab = useActiveTab();
  const settings = useSettings();
  const { focusAddressBar } = useUi();
  const inputRef = useRef<HTMLInputElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  // Typed text belongs to the tab it was typed in; switching tabs shows that tab's URL again.
  const [draft, setDraft] = useState<{ tabId: number; text: string } | null>(null);
  const [expanded, setExpanded] = useState(false);
  const editing = draft !== null && draft.tabId === tab?.id;
  const text = editing ? draft.text : '';
  const setEditing = (on: boolean) => !on && setDraft(null);
  const rowsRef = useRef<SuggestionRow[]>([]);
  const selectedRef = useRef(0);
  const queryRef = useRef(0);
  // What the user typed, without the inline completion after it.
  const typedRef = useRef('');
  // Selection to apply once React has rendered an inline completion.
  const selectionRef = useRef<[number, number] | null>(null);

  const shown = editing ? text : displayUrl(tab?.url ?? '');

  useEffect(() => {
    if (focusAddressBar === 0) return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [focusAddressBar]);

  useEffect(() => {
    void hideOverlay();
  }, [tab?.id]);

  useLayoutEffect(() => {
    const sel = selectionRef.current;
    selectionRef.current = null;
    if (sel && document.activeElement === inputRef.current) inputRef.current?.setSelectionRange(sel[0], sel[1]);
  }, [text]);

  const go = async (value: string, newTab = false) => {
    if (!tab || !value.trim()) return;
    // A suggestion query still in flight must not re-open the popup after this navigation.
    queryRef.current++;
    await hideOverlay();
    setEditing(false);
    inputRef.current?.blur();
    try {
      if (newTab) {
        const id = await api.tabs.create();
        await api.nav.go(id, value);
      } else {
        await api.nav.go(tab.id, value);
      }
    } catch (e) {
      announce(String(e));
    }
  };

  const render = async (rows: SuggestionRow[], refocusShell = false) => {
    rowsRef.current = rows;
    selectedRef.current = Math.min(selectedRef.current, Math.max(rows.length - 1, 0));
    setExpanded(rows.length > 0);
    if (!rows.length || document.activeElement !== inputRef.current) return void hideOverlay();
    const content = { kind: 'suggestions' as const, items: rows, selected: selectedRef.current };
    const r = boxRef.current!.getBoundingClientRect();
    const rect = { x: r.left, y: r.bottom + 4, width: r.width, height: rows.length * ROW_HEIGHT + 12 };
    if (overlayKind() === 'suggestions') {
      await updateOverlay(content);
      await api.layout.overlayShow(rect, false, refocusShell);
    } else {
      await showOverlay(content, rect, false);
    }
  };

  /** Fetches suggestions for `value`; `complete` allows an inline completion (only while typing). */
  const suggest = (value: string, complete: boolean, refocusShell = false) => {
    typedRef.current = value;
    const query = ++queryRef.current;
    const engine = settings?.searchEngine ?? 'duckDuckGo';
    void api.library.suggest(value).then((s) => {
      if (query !== queryRef.current) return;
      const input = inputRef.current;
      // Only while the caret sits at the end of what was just typed.
      const completion = complete && s.completion && input?.value === value && input.selectionStart === value.length ? s.completion : null;
      if (completion && tab) {
        selectionRef.current = [value.length, completion.length];
        setDraft({ tabId: tab.id, text: completion });
      }
      void render(buildRows(completion ?? value, s.items, engine), refocusShell);
    });
  };

  const onChange = (value: string, typing: boolean) => {
    if (tab) setDraft({ tabId: tab.id, text: value });
    selectedRef.current = 0;
    suggest(value, typing);
  };

  /** Removes a history suggestion from history and refreshes the list for the typed text. */
  const removeRow = async (index: number) => {
    const row = rowsRef.current[index];
    if (row?.historyId == null) return;
    try {
      await api.library.deleteHistory(row.historyId);
    } catch (e) {
      return announce(errorMessage(e));
    }
    announce('Removed from history');
    const typed = typedRef.current;
    if (tab) setDraft({ tabId: tab.id, text: typed });
    // The click on the X focused the overlay; typing should go on in the address bar.
    suggest(typed, true, true);
  };

  useEffect(
    () =>
      onOverlayAction((a) => {
        if (a.type === 'pick') {
          const row = rowsRef.current[a.index];
          if (row) void go(row.value);
        }
        if (a.type === 'removeSuggestion') void removeRow(a.index);
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab?.id],
  );

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const rows = rowsRef.current;
    if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && rows.length && overlayKind() === 'suggestions') {
      e.preventDefault();
      const delta = e.key === 'ArrowDown' ? 1 : -1;
      selectedRef.current = (selectedRef.current + delta + rows.length) % rows.length;
      const row = rows[selectedRef.current];
      if (row) announce(`${row.primary}, ${row.secondary}`);
      void updateOverlay({ kind: 'suggestions', items: rows, selected: selectedRef.current });
    } else if (e.key === 'Delete' && e.shiftKey && overlayKind() === 'suggestions' && rows[selectedRef.current]?.historyId != null) {
      e.preventDefault();
      void removeRow(selectedRef.current);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const row = overlayKind() === 'suggestions' ? rows[selectedRef.current] : undefined;
      void go(row?.value ?? text ?? '', e.altKey);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      if (overlayKind()) {
        setExpanded(false);
        void hideOverlay();
      } else {
        setEditing(false);
        inputRef.current?.blur();
        void api.nav.focusContent();
      }
    }
  };

  const toggleBookmark = () => void runAction('bookmarkPage');
  const isWeb = !!tab && !tab.internal;
  // A failed certificate must never look secure, whatever the scheme says.
  const certFailed = tab?.error?.kind === 'loadFailed' && tab.error.reason === 'certificate';
  const security = certFailed ? 'insecure' : tab?.security;
  const SecurityIcon = security === 'secure' ? Lock : security === 'insecure' ? LockOpen : tab?.internal ? Info : Search;
  const securityLabel =
    security === 'secure'
      ? 'Connection is secure'
      : security === 'insecure'
        ? 'Connection is not secure'
        : tab?.internal
          ? 'Veyscape page'
          : 'Search or enter address';

  return (
    <div ref={boxRef} className={`${styles.address} ${tab?.private ? styles.privateAddress : ''}`}>
      <span
        className={`${styles.security} ${security === 'insecure' ? styles.insecure : ''}`}
        role="img"
        aria-label={securityLabel}
        title={securityLabel}
      >
        <SecurityIcon size={14} />
        {security === 'insecure' && !editing && <span className={styles.notSecure}>Not secure</span>}
      </span>
      <input
        ref={inputRef}
        className={styles.input}
        value={shown}
        placeholder={tab?.private ? 'Search privately or enter address' : 'Search or enter address'}
        aria-label="Address and search bar"
        aria-autocomplete="both"
        aria-expanded={editing && expanded}
        spellCheck={false}
        autoComplete="off"
        // Inline completion only for typed characters: not after deleting, pasting or IME input.
        onChange={(e) => onChange(e.target.value, (e.nativeEvent as InputEvent).inputType === 'insertText')}
        // Select all when entering the field, not when focus returns from the open suggestion list.
        onFocus={(e) => overlayKind() !== 'suggestions' && e.target.select()}
        onBlur={() => {
          // Leave time for a click in the suggestion overlay to arrive first.
          window.setTimeout(() => {
            if (document.activeElement !== inputRef.current) {
              setEditing(false);
              if (overlayKind() === 'suggestions') void hideOverlay();
            }
          }, 200);
        }}
        onKeyDown={onKeyDown}
      />
      {tab && Math.abs(tab.zoom - 1) > 0.001 && (
        <button type="button" className={styles.chip} title="Reset zoom (Ctrl+0)" aria-label={`Zoom ${Math.round(tab.zoom * 100)} percent, reset`} onClick={() => void runAction('zoomReset')}>
          {Math.round(tab.zoom * 100)}%
        </button>
      )}
      {isWeb && <KeyButton className={styles.star} />}
      {isWeb && (
        <button
          type="button"
          className={`${styles.star} ${tab.bookmarked ? styles.starred : ''}`}
          aria-label={tab.bookmarked ? 'Remove bookmark (Ctrl+D)' : 'Bookmark this page (Ctrl+D)'}
          aria-pressed={tab.bookmarked}
          title={tab.bookmarked ? 'Remove bookmark (Ctrl+D)' : 'Bookmark this page (Ctrl+D)'}
          onClick={toggleBookmark}
        >
          <Star size={15} fill={tab.bookmarked ? 'currentColor' : 'none'} />
        </button>
      )}
    </div>
  );
}
