import { Columns2, Globe, Loader, Pin, Plus, VenetianMask, X } from 'lucide-react';
import { Fragment, useEffect, useRef, useState, type CSSProperties, type DragEvent, type KeyboardEvent } from 'react';
import { IconButton } from '../../components/IconButton';
import { FolderIcon } from '../../lib/folderIcons';
import { groupColor, groupInk } from '../../lib/groupColor';
import { api, errorMessage, onEvent } from '../../lib/ipc';
import { browserStore, useBrowserState, useFavicons } from '../../lib/store';
import { showOverlay } from '../overlay/client';
import { announce } from '../../lib/ui';
import { runAction } from '../shortcuts/actions';
import type { Tab, TabGroup } from '../../types/browser';
import { WindowControls } from './WindowControls';
import styles from './TabStrip.module.css';

// Overlay size of the group editor (name, icon grid, colours), sized so it fits without scrolling.
const GROUP_EDITOR_SIZE = { width: 360, height: 456 } as const;

/** Opens the group editor under the group's chip. A group just created may not be drawn yet, so
 *  this waits briefly for its chip. */
function openGroupEditor(groupId: number, tries = 0) {
  const group = browserStore.get().groups.find((g) => g.id === groupId);
  const chip = document.querySelector<HTMLElement>(`[data-group-id="${groupId}"]`);
  if (!group || !chip) {
    if (tries < 20) window.setTimeout(() => openGroupEditor(groupId, tries + 1), 50);
    return;
  }
  const r = chip.getBoundingClientRect();
  const x = Math.min(Math.max(8, r.left), window.innerWidth - GROUP_EDITOR_SIZE.width - 8);
  void showOverlay(
    { kind: 'groupEditor', groupId, title: group.title, color: group.color, icon: group.icon },
    { x, y: r.bottom + 4, ...GROUP_EDITOR_SIZE },
    true,
  );
}

// Marks drags of our tabs, also arriving from another window's strip. The tab itself is registered
// in Rust at drag start (`tab_drag_start`), so a page faking this type cannot move tabs.
const TAB_DRAG_TYPE = 'application/x-veyscape-tab';
// Also set for private tabs: a private window takes only those (Rust checks again on drop).
const PRIVATE_TAB_DRAG_TYPE = 'application/x-veyscape-private-tab';

/** Compact tab strip with tab groups; empty space drags the window (it doubles as the title bar). */
export function TabStrip() {
  const { privateWindow, tabs, activeTabId, canReopenClosed, groups, split } = useBrowserState();
  const favicons = useFavicons();
  const listRef = useRef<HTMLDivElement>(null);
  const [dragId, setDragId] = useState<number | null>(null);
  // The group a dragged tab would join if dropped now (highlights its chip).
  const [dropGroup, setDropGroup] = useState<number | null>(null);

  const groupById = new Map(groups.map((g) => [g.id, g]));
  const splitTabs = split?.tabs ?? [];

  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [activeTabId, tabs.length]);

  // "Edit group…" in the native group menu, and new groups, open the editor.
  useEffect(() => {
    const un = onEvent('group:editor', (groupId) => openGroupEditor(groupId));
    return () => void un.then((f) => f());
  }, []);

  const run = (p: Promise<unknown>) => void p.catch((e) => announce(errorMessage(e)));

  // Positions are DOM positions: tabs of collapsed groups are not rendered, so they differ from tab indexes.
  const focusTab = (from: HTMLElement, pick: (pos: number, count: number) => number) => {
    const items = Array.from(listRef.current?.querySelectorAll<HTMLElement>('[role="tab"]') ?? []);
    if (!items.length) return;
    items[(pick(items.indexOf(from), items.length) + items.length) % items.length]?.focus();
  };

  const menuInfo = (tab: Tab) => ({
    pinned: tab.pinned,
    canReopen: canReopenClosed,
    groupId: tab.groupId,
    groups: groups.map((g) => ({ id: g.id, title: g.title })),
    inSplit: splitTabs.includes(tab.id),
    onlyTab: tabs.length === 1,
  });

  // Roving focus per the WAI-ARIA tabs pattern; Delete closes, Ctrl+Shift+Arrow moves.
  const onKeyDown = (e: KeyboardEvent, tab: Tab, index: number) => {
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      const delta = e.key === 'ArrowRight' ? 1 : -1;
      if (e.ctrlKey && e.shiftKey) run(api.tabs.move(tab.id, Math.max(0, index + delta)));
      else focusTab(e.currentTarget as HTMLElement, (pos) => pos + delta);
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      focusTab(e.currentTarget as HTMLElement, (_, count) => (e.key === 'Home' ? 0 : count - 1));
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      run(api.tabs.activate(tab.id));
    } else if (e.key === 'Delete') {
      e.preventDefault();
      run(api.tabs.close(tab.id));
    } else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault();
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      run(api.tabs.contextMenu(tab.id, menuInfo(tab), r.left, r.bottom));
    }
  };

  const isTabDrag = (e: DragEvent) =>
    e.dataTransfer.types.includes(TAB_DRAG_TYPE) && (!privateWindow || e.dataTransfer.types.includes(PRIVATE_TAB_DRAG_TYPE));

  // Undefined while a tab from another window is dragged over this strip.
  const dragged = tabs.find((t) => t.id === dragId);
  /** The group a drop onto `target` adds the dragged tab to (pinned tabs never join groups). */
  const joinGroup = (target: Tab) =>
    target.groupId != null && !dragged?.pinned && target.groupId !== dragged?.groupId ? target.groupId : null;

  const endDrag = () => {
    setDragId(null);
    setDropGroup(null);
  };

  const onDrop = (e: DragEvent, index: number, target: Tab) => {
    e.preventDefault();
    run(api.tabs.drop(index, joinGroup(target)));
    endDrag();
  };

  const dropOnGroup = (e: DragEvent, g: TabGroup) => {
    e.preventDefault();
    const first = tabs.findIndex((t) => t.groupId === g.id);
    if (first >= 0) run(api.tabs.drop(first, g.id));
    endDrag();
  };

  // Dropped where nothing took it, away from the tab strip (on the page or outside the window):
  // the tab moves to a new window there. dropEffect is also "none" after Esc cancels a drag.
  // Set once the dragged tab went to a new window that follows the pointer (see onDrag).
  const tornOff = useRef(false);
  const awayFromStrip = (e: DragEvent) => {
    const strip = listRef.current?.getBoundingClientRect();
    return !!strip && (e.clientY > strip.bottom + 24 || e.clientY < 0 || e.clientX < 0 || e.clientX > window.innerWidth);
  };

  // Dragged away from the strip: the tab moves into a new window that follows the pointer until
  // the drop (the backend moves it; over another window's strip it hides so the tab can join it).
  const onDrag = (e: DragEvent, tab: Tab) => {
    // The last drag event reports (0, 0).
    if (tornOff.current || (e.screenX === 0 && e.screenY === 0) || tabs.length < 2 || !awayFromStrip(e)) return;
    tornOff.current = true;
    run(api.tabs.tearOff(tab.id, e.screenX, e.screenY, true));
  };

  // Fallback without drag positions: dropped where nothing took it, away from the strip, the tab
  // moves to a new window there. dropEffect is also "none" after Esc cancels a drag.
  const onDragEnd = (e: DragEvent, tab: Tab) => {
    if (!tornOff.current && e.dataTransfer.dropEffect === 'none' && awayFromStrip(e) && tabs.length > 1) {
      run(api.tabs.tearOff(tab.id, e.screenX, e.screenY));
    }
    endDrag();
  };

  const groupChip = (g: TabGroup) => {
    const count = tabs.filter((t) => t.groupId === g.id).length;
    const name = g.title || `${count} ${count === 1 ? 'tab' : 'tabs'}`;
    const style = { '--group-color': groupColor(g.color), color: groupInk(g.color) } as CSSProperties;
    return (
      <button
        type="button"
        className={`${styles.groupChip} ${g.collapsed ? styles.groupCollapsed : ''} ${dropGroup === g.id ? styles.groupDrop : ''}`}
        style={style}
        aria-expanded={!g.collapsed}
        aria-label={`Tab group ${name}, ${count} tabs, ${g.collapsed ? 'minimized' : 'expanded'}`}
        title={`${name} — click to ${g.collapsed ? 'expand' : 'minimize'}, double-click to edit`}
        data-group-id={g.id}
        onClick={() => run(api.groups.update(g.id, { collapsed: !g.collapsed }))}
        onDoubleClick={() => openGroupEditor(g.id)}
        onDragOver={(e) => {
          if (!isTabDrag(e) || dragged?.pinned || dragged?.groupId === g.id) return;
          e.preventDefault();
          setDropGroup(g.id);
        }}
        onDrop={(e) => dropOnGroup(e, g)}
        onContextMenu={(e) => {
          e.preventDefault();
          run(api.groups.menu(g.id, g.collapsed, e.clientX, e.clientY));
        }}
      >
        {g.icon && <FolderIcon name={g.icon} size={13} />}
        <span className={styles.groupName}>{name}</span>
      </button>
    );
  };

  return (
    <div className={`${styles.strip} ${privateWindow ? styles.privateStrip : ''}`} data-tauri-drag-region>
      <div
        ref={listRef}
        className={styles.tabs}
        role="tablist"
        aria-label="Tabs"
        data-tauri-drag-region
        // Empty strip space takes a dropped tab at the end (tabs and chips handle their own drops).
        onDragOver={(e) => isTabDrag(e) && e.preventDefault()}
        onDrop={(e) => {
          if (e.defaultPrevented || !isTabDrag(e)) return;
          e.preventDefault();
          run(api.tabs.drop(tabs.length, null));
          endDrag();
        }}
        onDragLeave={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && setDropGroup(null)}
      >
        {tabs.map((tab, index) => {
          const group = tab.groupId != null ? groupById.get(tab.groupId) : undefined;
          const firstOfGroup = !!group && tabs[index - 1]?.groupId !== tab.groupId;
          if (group?.collapsed) return <Fragment key={tab.id}>{firstOfGroup && groupChip(group)}</Fragment>;
          const active = tab.id === activeTabId;
          const title = tab.title || tab.url || 'New Tab';
          const inSplit = splitTabs.includes(tab.id);
          return (
            <Fragment key={tab.id}>
              {firstOfGroup && groupChip(group)}
              <div
                role="tab"
                aria-selected={active}
                aria-label={`${title}${tab.private ? ', private' : ''}${tab.pinned ? ', pinned' : ''}${group ? `, in group ${group.title || 'unnamed'}` : ''}${inSplit ? ', in split view' : ''}${tab.loading ? ', loading' : ''}`}
                tabIndex={active ? 0 : -1}
                title={title}
                draggable
                className={`${styles.tab} ${tab.pinned ? styles.pinned : ''} ${tab.private ? styles.private : ''} ${group ? styles.grouped : ''}`}
                style={group ? ({ '--group-color': groupColor(group.color) } as CSSProperties) : undefined}
                onClick={() => run(api.tabs.activate(tab.id))}
                onMouseDown={(e) => e.button === 1 && e.preventDefault()}
                onAuxClick={(e) => e.button === 1 && run(api.tabs.close(tab.id))}
                onContextMenu={(e) => {
                  e.preventDefault();
                  run(api.tabs.contextMenu(tab.id, menuInfo(tab), e.clientX, e.clientY));
                }}
                onKeyDown={(e) => onKeyDown(e, tab, index)}
                onDragStart={(e) => {
                  tornOff.current = false;
                  setDragId(tab.id);
                  e.dataTransfer.setData(TAB_DRAG_TYPE, '1');
                  if (tab.private) e.dataTransfer.setData(PRIVATE_TAB_DRAG_TYPE, '1');
                  e.dataTransfer.effectAllowed = 'move';
                  run(api.tabs.dragStart(tab.id));
                }}
                onDragOver={(e) => {
                  if (!isTabDrag(e)) return;
                  e.preventDefault();
                  setDropGroup(joinGroup(tab));
                }}
                onDrop={(e) => onDrop(e, index, tab)}
                onDrag={(e) => onDrag(e, tab)}
                onDragEnd={(e) => onDragEnd(e, tab)}
              >
                <span className={styles.icon} aria-hidden>
                  {tab.loading ? (
                    <Loader size={14} className={styles.spin} />
                  ) : favicons[tab.id] ? (
                    <img src={favicons[tab.id]} alt="" width={16} height={16} />
                  ) : tab.private ? (
                    <VenetianMask size={14} />
                  ) : (
                    <Globe size={14} />
                  )}
                </span>
                {!tab.pinned && <span className={styles.title}>{title}</span>}
                {inSplit && !tab.pinned && <Columns2 size={12} aria-hidden className={styles.splitMark} />}
                {tab.pinned && <Pin size={10} className={styles.pinMark} aria-hidden />}
                {!tab.pinned && (
                  <button
                    type="button"
                    className={styles.close}
                    aria-label={`Close ${title}`}
                    title="Close tab (Ctrl+W)"
                    tabIndex={-1}
                    onClick={(e) => {
                      e.stopPropagation();
                      run(api.tabs.close(tab.id));
                    }}
                  >
                    <X size={14} />
                  </button>
                )}
              </div>
            </Fragment>
          );
        })}
        <IconButton
          label={privateWindow ? 'New private tab (Ctrl+T)' : 'New tab (Ctrl+T)'}
          className={styles.newTab}
          onClick={() => void runAction('newTab')}
        >
          <Plus size={16} />
        </IconButton>
      </div>
      {privateWindow && (
        <span className={styles.privateBadge} data-tauri-drag-region>
          <VenetianMask size={14} aria-hidden /> Private window
        </span>
      )}
      <WindowControls />
    </div>
  );
}
