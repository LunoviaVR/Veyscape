import { describe, expect, it } from 'vitest';
import { groupColor, groupInk } from './groupColor';

describe('group colours', () => {
  it('maps presets to tokens and keeps custom hex', () => {
    expect(groupColor('blue')).toBe('var(--group-blue, var(--group-grey))');
    expect(groupColor('#12ab9f')).toBe('#12ab9f');
  });
  it('picks readable text for custom colours', () => {
    expect(groupInk('#ffff00')).toBe('#111111');
    expect(groupInk('#202060')).toBe('#ffffff');
    expect(groupInk('blue')).toBeUndefined();
  });
});
