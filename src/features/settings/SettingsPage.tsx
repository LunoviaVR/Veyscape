import { useId } from 'react';
import { usePageTab } from '../../lib/pageTab';
import { api } from '../../lib/ipc';
import { AdblockSettings } from '../adblock/AdblockSettings';
import { PasswordsSection } from '../passwords/PasswordsSection';
import { PrivacySettings } from '../privacy/PrivacySettings';
import { AboutSection } from './AboutSection';
import { AppearanceSection, GeneralSection } from './GeneralSections';
import styles from './Settings.module.css';

const SECTIONS = [
  { id: 'general', label: 'General', Component: GeneralSection },
  { id: 'appearance', label: 'Appearance', Component: AppearanceSection },
  { id: 'privacy', label: 'Privacy & security', Component: PrivacySettings },
  { id: 'passwords', label: 'Passwords', Component: PasswordsSection },
  { id: 'adblock', label: 'Ad blocking', Component: AdblockSettings },
  { id: 'about', label: 'About', Component: AboutSection },
] as const;

/** Categorized settings. The section comes from the URL (veyscape://settings/<section>). */
export function SettingsPage() {
  const tab = usePageTab();
  const sectionId = tab?.url.replace('veyscape://settings', '').replace(/^\//, '').split(/[/?#]/)[0] || 'general';
  const section = SECTIONS.find((s) => s.id === sectionId) ?? SECTIONS[0];
  const Section = section.Component;
  const titleId = useId();

  return (
    <div className={styles.layout}>
      <nav className={styles.nav} aria-label="Settings sections">
        <h1 className={styles.navTitle}>Settings</h1>
        <ul>
          {SECTIONS.map((s) => (
            <li key={s.id}>
              <button
                type="button"
                className={styles.navItem}
                aria-current={s.id === section.id ? 'page' : undefined}
                onClick={() => tab && void api.nav.go(tab.id, `veyscape://settings/${s.id}`)}
              >
                {s.label}
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <main className={styles.main} aria-labelledby={titleId}>
        <div className={styles.column}>
          <h2 id={titleId} className={styles.sectionTitle}>
            {section.label}
          </h2>
          <Section />
        </div>
      </main>
    </div>
  );
}
