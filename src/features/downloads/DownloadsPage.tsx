import { FolderOpen, Play, Trash2, X } from 'lucide-react';
import { IconButton } from '../../components/IconButton';
import { EmptyState, Page } from '../../components/Page';
import { api, errorMessage } from '../../lib/ipc';
import { useDownloads } from '../../lib/store';
import { announce } from '../../lib/ui';
import { displayHost, formatBytes, formatDay } from '../../lib/format';
import type { DownloadItem } from '../../types/browser';
import controls from '../../components/controls.module.css';
import page from '../../components/page.module.css';

const STATE_TEXT = { inProgress: 'Downloading', completed: 'Done', interrupted: 'Failed', cancelled: 'Cancelled' } as const;

export function DownloadsPage() {
  const downloads = useDownloads();

  const run = (p: Promise<void>) => p.catch((e) => announce(errorMessage(e)));

  const open = (d: DownloadItem) => {
    if (d.risky && !window.confirm(`“${d.fileName}” can run code on your computer. Open it only if you trust ${displayHost(d.url)}. Open anyway?`)) {
      return;
    }
    void run(api.downloads.open(d.id));
  };

  return (
    <Page
      title="Downloads"
      actions={
        <button type="button" className={controls.button} onClick={() => void run(api.downloads.clear())} disabled={!downloads.some((d) => d.state !== 'inProgress')}>
          Clear list
        </button>
      }
    >
      {!downloads.length && <EmptyState>Files you download will appear here.</EmptyState>}
      <ul className={page.list}>
        {downloads.map((d) => {
          const pct = d.total ? Math.round((d.received / d.total) * 100) : null;
          const size = d.total ? `${formatBytes(d.received)} of ${formatBytes(d.total)}` : formatBytes(d.received);
          return (
            <li key={d.id} className={page.item}>
              <div className={page.itemMain}>
                {d.state === 'completed' ? (
                  <button type="button" className={`${page.itemTitle} ${page.linkTitle}`} onClick={() => open(d)}>
                    {d.fileName || d.url}
                  </button>
                ) : (
                  <span className={page.itemTitle}>{d.fileName || d.url}</span>
                )}
                <span className={page.itemSub}>
                  {STATE_TEXT[d.state]} · {size} · {displayHost(d.url)} · {formatDay(d.startedAt)}
                  {d.private ? ' · private' : ''}
                  {d.risky ? ' · executable file' : ''}
                </span>
                {d.state === 'inProgress' && (
                  <progress className={page.progress} max={100} value={pct ?? undefined} aria-label={`Download progress for ${d.fileName}`} />
                )}
              </div>
              {d.state === 'inProgress' && (
                <IconButton label={`Cancel download of ${d.fileName}`} onClick={() => void run(api.downloads.cancel(d.id))}>
                  <X size={15} />
                </IconButton>
              )}
              {d.state === 'interrupted' && (
                <IconButton label={`Retry download of ${d.fileName}`} onClick={() => void run(api.downloads.resume(d.id))}>
                  <Play size={15} />
                </IconButton>
              )}
              {d.state === 'completed' && (
                <IconButton label={`Show ${d.fileName} in folder`} onClick={() => void run(api.downloads.show(d.id))}>
                  <FolderOpen size={15} />
                </IconButton>
              )}
              {d.state !== 'inProgress' && (
                <IconButton label={`Remove ${d.fileName} from list`} onClick={() => void run(api.downloads.remove(d.id))}>
                  <Trash2 size={15} />
                </IconButton>
              )}
            </li>
          );
        })}
      </ul>
      <p className={page.footnote}>
        Removing an entry keeps the file on disk.
      </p>
    </Page>
  );
}
