import { KeyRound } from 'lucide-react';
import { useState } from 'react';
import { api, errorMessage } from '../../lib/ipc';
import { useActiveTab, useBrowserState } from '../../lib/store';
import { announce } from '../../lib/ui';
import { displayHost } from '../../lib/format';
import type { PermissionKind, Prompt } from '../../types/browser';
import controls from '../../components/controls.module.css';
import styles from './PromptBar.module.css';

const KIND_TEXT: Record<PermissionKind, string> = {
  camera: 'use your camera',
  microphone: 'use your microphone',
  location: 'know your location',
  notifications: 'show notifications',
  popups: 'open pop-up windows',
  automaticDownloads: 'download multiple files',
};

/** Infobars for the active tab's pending requests. They push content down rather than cover it. */
export function PromptBar() {
  const { prompts } = useBrowserState();
  const tab = useActiveTab();
  const mine = prompts.filter((p) => p.tabId === tab?.id);
  if (!tab || !mine.length) return null;
  return (
    <div className={styles.stack}>
      {mine.map((p) => (
        <PromptRow key={p.id} prompt={p} privateTab={tab.private} />
      ))}
    </div>
  );
}

function PromptRow({ prompt, privateTab }: { prompt: Prompt; privateTab: boolean }) {
  const canRemember = !privateTab;
  // Allowing a pop-up for good is a bigger step than the other prompts: opt in.
  const [remember, setRemember] = useState(canRemember && prompt.type !== 'popupBlocked');
  const respond = (allow: boolean) => {
    const call =
      prompt.type === 'externalProtocol'
        ? api.prompts.respondExternal(prompt.id, allow)
        : api.prompts.respond(prompt.id, allow, remember && canRemember);
    call.catch((e) => announce(errorMessage(e)));
  };

  let message: string;
  let allowLabel = 'Allow';
  let denyLabel = 'Block';
  if (prompt.type === 'permission') {
    message = `${displayHost(prompt.origin)} wants to ${KIND_TEXT[prompt.kind]}.`;
  } else if (prompt.type === 'externalProtocol') {
    const scheme = prompt.uri.split(':')[0];
    message = `This page wants to open a “${scheme}:” link in another application.`;
    allowLabel = 'Open app';
    denyLabel = 'Cancel';
  } else if (prompt.type === 'popupBlocked') {
    message = `Pop-up blocked on ${displayHost(prompt.origin)}.`;
    allowLabel = 'Open pop-up';
    denyLabel = 'Dismiss';
  } else {
    return <SavePasswordRow prompt={prompt} />;
  }

  return (
    <div className={styles.bar} role="alertdialog" aria-label="Site request" aria-describedby={`prompt-${prompt.id}`}>
      <p id={`prompt-${prompt.id}`} className={styles.message} data-selectable>
        {message}
      </p>
      {prompt.type !== 'externalProtocol' && canRemember && (
        <label className={styles.remember}>
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          {prompt.type === 'popupBlocked' ? 'Always allow on this site' : 'Remember'}
        </label>
      )}
      <button type="button" className={controls.button} onClick={() => respond(false)}>
        {denyLabel}
      </button>
      <button type="button" className={`${controls.button} ${controls.primary}`} onClick={() => respond(true)}>
        {allowLabel}
      </button>
    </div>
  );
}

type SavePasswordPrompt = Extract<Prompt, { type: 'savePassword' }>;

/** "Save password?" after a login: Save, Never for this site, or Not now. */
function SavePasswordRow({ prompt }: { prompt: SavePasswordPrompt }) {
  // allow = save; remember without allow = never offer for this site.
  const respond = (allow: boolean, never: boolean) => api.prompts.respond(prompt.id, allow, never).catch((e) => announce(errorMessage(e)));
  const host = displayHost(prompt.origin);
  const who = prompt.username ? ` for ${prompt.username}` : '';
  return (
    <div className={styles.bar} role="alertdialog" aria-label="Save password" aria-describedby={`prompt-${prompt.id}`}>
      <KeyRound size={16} aria-hidden />
      <p id={`prompt-${prompt.id}`} className={styles.message} data-selectable>
        {prompt.update ? `Update the saved password${who} on ${host}?` : `Save the password${who} on ${host}?`}
      </p>
      {!prompt.update && (
        <button type="button" className={controls.button} onClick={() => void respond(false, true)}>
          Never for this site
        </button>
      )}
      <button type="button" className={controls.button} onClick={() => void respond(false, false)}>
        Not now
      </button>
      <button type="button" className={`${controls.button} ${controls.primary}`} onClick={() => void respond(true, false)}>
        {prompt.update ? 'Update' : 'Save'}
      </button>
    </div>
  );
}
