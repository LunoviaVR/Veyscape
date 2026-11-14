import { describe, expect, it } from 'vitest';
import { buildRows, looksLikeAddress } from './suggestions';

describe('address bar suggestions', () => {
  it('labels addresses and searches', () => {
    expect(looksLikeAddress('example.com')).toBe(true);
    expect(looksLikeAddress('https://a.example/x y')).toBe(false);
    expect(looksLikeAddress('localhost:3000')).toBe(true);
    expect(looksLikeAddress('rust borrow checker')).toBe(false);
  });

  it('puts the typed entry first and de-duplicates it', () => {
    const rows = buildRows(
      'docs',
      [
        { url: 'https://docs.example/', title: 'Docs', bookmarked: true, historyId: null },
        { url: 'https://docs.example/a', title: 'A', bookmarked: false, historyId: 7 },
      ],
      'duckDuckGo',
    );
    expect(rows.map((r) => r.kind)).toEqual(['search', 'bookmark', 'history']);
    // Only history rows can be removed.
    expect(rows.map((r) => r.historyId)).toEqual([null, null, 7]);
    expect(rows[0]?.secondary).toBe('Search with DuckDuckGo');
    expect(buildRows('  ', [], 'duckDuckGo')).toEqual([]);
  });
});
