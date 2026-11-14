import { describe, expect, it } from 'vitest';
import { displayHost, displayUrl, formatBytes, formatDay } from './format';

describe('format', () => {
  it('formats byte counts', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(50 * 1024 * 1024)).toBe('50 MB');
    expect(formatBytes(-1)).toBe('');
  });

  it('shows hosts and hides the new-tab URL', () => {
    expect(displayHost('https://www.example.com/a')).toBe('example.com');
    expect(displayHost('not a url')).toBe('not a url');
    expect(displayUrl('veyscape://newtab')).toBe('');
    expect(displayUrl('https://a.example/')).toBe('https://a.example/');
  });

  it('labels days relative to now', () => {
    const now = new Date(2026, 10, 12, 15).getTime();
    expect(formatDay(new Date(2026, 10, 12, 9).getTime(), now)).toBe('Today');
    expect(formatDay(new Date(2026, 10, 11, 23).getTime(), now)).toBe('Yesterday');
  });
});
