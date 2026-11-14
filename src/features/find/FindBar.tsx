import { ChevronDown, ChevronUp, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { IconButton } from '../../components/IconButton';
import { api, errorMessage, onEvent } from '../../lib/ipc';
import { useActiveTab } from '../../lib/store';
import { patchUi, useUi } from '../../lib/ui';
import styles from './FindBar.module.css';

/** Find in page (WebView2 Find API; highlights all matches). */
export function FindBar() {
  const { findOpen } = useUi();
  const tab = useActiveTab();
  const inputRef = useRef<HTMLInputElement>(null);
  const [text, setText] = useState('');
  const [result, setResult] = useState<{ matches: number; active: number } | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!findOpen) return;
    inputRef.current?.focus();
    inputRef.current?.select();
    const un = onEvent('find:result', (r) => {
      if (r.tabId === tab?.id) setResult({ matches: r.matches, active: r.active });
    });
    return () => void un.then((f) => f());
  }, [findOpen, tab?.id]);

  // Switching tabs or leaving web content closes the bar (the effect only reruns when those change).
  useEffect(() => {
    if (findOpen) close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab?.id, tab?.internal]);

  if (!findOpen) return null;

  function close() {
    patchUi({ findOpen: false });
    setResult(null);
    setError('');
    void api.nav.findStop();
    void api.nav.focusContent();
  }

  const find = (value: string, forward = true) => {
    setError('');
    api.nav.find(value, forward).catch((e) => setError(errorMessage(e)));
    if (!value) setResult(null);
  };

  const status = error || (result && text ? (result.matches ? `${result.active} of ${result.matches}` : 'No matches') : '');

  return (
    <div className={styles.bar} role="search" aria-label="Find in page">
      <input
        ref={inputRef}
        className={styles.input}
        value={text}
        placeholder="Find in page"
        aria-label="Find in page"
        aria-describedby="find-status"
        onChange={(e) => {
          setText(e.target.value);
          find(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            find(text, !e.shiftKey);
          } else if (e.key === 'Escape') {
            e.preventDefault();
            close();
          }
        }}
      />
      <span id="find-status" className={styles.status} aria-live="polite">
        {status}
      </span>
      <IconButton label="Previous match (Shift+Enter)" disabled={!text} onClick={() => find(text, false)}>
        <ChevronUp size={16} />
      </IconButton>
      <IconButton label="Next match (Enter)" disabled={!text} onClick={() => find(text, true)}>
        <ChevronDown size={16} />
      </IconButton>
      <IconButton label="Close find bar (Escape)" onClick={close}>
        <X size={16} />
      </IconButton>
    </div>
  );
}
