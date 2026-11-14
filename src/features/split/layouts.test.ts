import { describe, expect, it } from 'vitest';
import { PREVIEW, SPLIT_OPTIONS, previewRects, type PreviewRect } from './layouts';

describe('split layout previews', () => {
  it('draws one box per pane for every option', () => {
    for (const o of SPLIT_OPTIONS) expect(previewRects(o.layout, o.count)).toHaveLength(o.count);
  });

  it('keeps every box inside the thumbnail', () => {
    for (const o of SPLIT_OPTIONS) {
      for (const [x, y, w, h] of previewRects(o.layout, o.count)) {
        expect(x).toBeGreaterThanOrEqual(0);
        expect(y).toBeGreaterThanOrEqual(0);
        expect(x + w).toBeLessThanOrEqual(PREVIEW.width + 1e-9);
        expect(y + h).toBeLessThanOrEqual(PREVIEW.height + 1e-9);
      }
    }
  });

  it('puts the main pane full height on the left', () => {
    const [main, a, b] = previewRects('mainLeft', 3) as [PreviewRect, PreviewRect, PreviewRect];
    expect(main[3]).toBe(PREVIEW.height);
    expect(a[0]).toBe(b[0]);
    expect(b[1]).toBeGreaterThan(a[1]);
  });
});
