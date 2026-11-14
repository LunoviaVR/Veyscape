import { useEffect, useLayoutEffect, useRef } from 'react';
import { FindBar } from '../features/find/FindBar';
import { Toolbar } from '../features/navigation/Toolbar';
import { initOverlayClient } from '../features/overlay/client';
import { PromptBar } from '../features/prompts/PromptBar';
import { runAction } from '../features/shortcuts/actions';
import { actionForKey, allowedWhileTyping } from '../features/shortcuts/shortcuts';
import { TabStrip } from '../features/tabs/TabStrip';
import { BookmarkBar } from '../features/bookmarks/BookmarkBar';
import { useTheme } from '../hooks/useTheme';
import { api, onEvent } from '../lib/ipc';
import { useBrowserState } from '../lib/store';
import { Content } from './Content';
import styles from './ShellApp.module.css';

/** Root of the trusted browser UI (tab strip, toolbar, bars, internal pages). */
export function ShellApp() {
  const { fullscreen, tabs } = useBrowserState();
  const chromeRef = useRef<HTMLDivElement>(null);
  const hasTabs = tabs.length > 0;
  useTheme();

  useEffect(() => {
    const unOverlay = initOverlayClient();
    const un = onEvent('shell:action', (action) => void runAction(action));
    const onKey = (e: KeyboardEvent) => {
      const action = actionForKey(e);
      if (!action) return;
      const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
      if (typing && !allowedWhileTyping(action)) return;
      e.preventDefault();
      void runAction(action);
    };
    window.addEventListener('keydown', onKey);
    return () => {
      unOverlay();
      void un.then((f) => f());
      window.removeEventListener('keydown', onKey);
    };
  }, []);

  // Report the chrome height so the backend can place web content directly below it.
  useLayoutEffect(() => {
    const el = chromeRef.current;
    if (!el) return;
    const report = () => void api.layout.update(fullscreen ? 0 : el.getBoundingClientRect().height);
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    return () => ro.disconnect();
  }, [fullscreen, hasTabs]);

  return (
    <div className={styles.app}>
      <div id="chrome" ref={chromeRef} className={styles.chrome} hidden={fullscreen}>
        <TabStrip />
        <Toolbar />
        <BookmarkBar />
        <PromptBar />
        <FindBar />
      </div>
      <Content />
      <div id="live-region" className="visually-hidden" aria-live="polite" aria-atomic="true" />
    </div>
  );
}
