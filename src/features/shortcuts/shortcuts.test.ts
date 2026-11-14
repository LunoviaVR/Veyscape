import { describe, expect, it } from 'vitest';
import { actionForKey, bindings } from './shortcuts';

const key = (code: string, mods: Partial<{ ctrl: boolean; shift: boolean; alt: boolean }> = {}) => ({
  code,
  ctrlKey: !!mods.ctrl,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
  metaKey: false,
});

describe('shortcuts', () => {
  it('maps the required browser shortcuts', () => {
    expect(actionForKey(key('KeyT', { ctrl: true }))).toBe('newTab');
    expect(actionForKey(key('KeyW', { ctrl: true }))).toBe('closeTab');
    expect(actionForKey(key('KeyT', { ctrl: true, shift: true }))).toBe('reopenClosedTab');
    expect(actionForKey(key('KeyL', { ctrl: true }))).toBe('focusAddressBar');
    expect(actionForKey(key('KeyR', { ctrl: true }))).toBe('reload');
    expect(actionForKey(key('Tab', { ctrl: true }))).toBe('nextTab');
    expect(actionForKey(key('Tab', { ctrl: true, shift: true }))).toBe('previousTab');
    expect(actionForKey(key('Digit1', { ctrl: true }))).toBe('selectTab1');
    expect(actionForKey(key('Digit9', { ctrl: true }))).toBe('selectLastTab');
    expect(actionForKey(key('KeyF', { ctrl: true }))).toBe('findInPage');
    expect(actionForKey(key('KeyD', { ctrl: true }))).toBe('bookmarkPage');
    expect(actionForKey(key('KeyH', { ctrl: true }))).toBe('openHistory');
    expect(actionForKey(key('KeyJ', { ctrl: true }))).toBe('openDownloads');
    expect(actionForKey(key('Equal', { ctrl: true }))).toBe('zoomIn');
    expect(actionForKey(key('Minus', { ctrl: true }))).toBe('zoomOut');
    expect(actionForKey(key('Digit0', { ctrl: true }))).toBe('zoomReset');
  });

  it('requires exact modifiers', () => {
    expect(actionForKey(key('KeyT'))).toBeUndefined();
    expect(actionForKey(key('KeyT', { ctrl: true, alt: true }))).toBeUndefined();
    expect(actionForKey({ ...key('KeyT', { ctrl: true }), metaKey: true })).toBeUndefined();
  });

  it('falls back to key when code is missing', () => {
    expect(actionForKey({ ...key('', { ctrl: true }), key: 't' })).toBe('newTab');
    expect(actionForKey({ ...key('', { ctrl: true }), key: '3' })).toBe('selectTab3');
    expect(actionForKey({ ...key('', { alt: true }), key: 'Home' })).toBe('goHome');
  });

  it('follows the layout for Latin letters', () => {
    // AZERTY: the physical W key types "z"; Ctrl+Z must not close the tab.
    expect(actionForKey({ ...key('KeyW', { ctrl: true }), key: 'z' })).not.toBe('closeTab');
    expect(actionForKey({ ...key('KeyW', { ctrl: true }), key: 'Z' })).toBeUndefined();
    // Ctrl+W (physical Z) closes.
    expect(actionForKey({ ...key('KeyZ', { ctrl: true }), key: 'w' })).toBe('closeTab');
  });

  it('uses the physical key on non-Latin layouts', () => {
    expect(actionForKey({ ...key('KeyW', { ctrl: true }), key: 'ц' })).toBe('closeTab');
    expect(actionForKey({ ...key('Digit1', { ctrl: true }), key: '1' })).toBe('selectTab1');
  });

  it('maps zoom keys by the layout character', () => {
    // German: '+' sits on BracketRight, '-' on Slash.
    expect(actionForKey({ ...key('BracketRight', { ctrl: true }), key: '+' })).toBe('zoomIn');
    expect(actionForKey({ ...key('Slash', { ctrl: true }), key: '-' })).toBe('zoomOut');
    // AZERTY: Ctrl+1 types '&' but is still tab 1.
    expect(actionForKey({ ...key('Digit1', { ctrl: true }), key: '&' })).toBe('selectTab1');
  });

  it('has no duplicate chords', () => {
    const chords = bindings.map((b) => `${b.code}|${b.ctrl}|${b.shift}|${b.alt}`);
    expect(new Set(chords).size).toBe(chords.length);
  });
});
