import type { SplitLayout } from '../../types/browser';

export interface SplitOption {
  count: number;
  layout: SplitLayout;
  /** Short label under the preview. */
  label: string;
  /** Spoken and shown as a tooltip after the label. */
  description: string;
}

export const SPLIT_OPTIONS: SplitOption[] = [
  { count: 2, layout: 'columns', label: 'Side by side', description: 'two pages next to each other' },
  { count: 2, layout: 'rows', label: 'Stacked', description: 'two pages, one above the other' },
  { count: 3, layout: 'columns', label: 'Columns', description: 'three pages next to each other' },
  { count: 3, layout: 'rows', label: 'Rows', description: 'three pages stacked' },
  { count: 3, layout: 'mainLeft', label: 'Main left', description: 'one large page on the left, two on the right' },
  { count: 3, layout: 'mainTop', label: 'Main top', description: 'one large page on top, two below' },
  { count: 4, layout: 'grid', label: 'Grid', description: 'four pages in a 2 by 2 grid' },
  { count: 4, layout: 'columns', label: 'Columns', description: 'four pages next to each other' },
  { count: 4, layout: 'mainLeft', label: 'Main left', description: 'one large page on the left, three on the right' },
  { count: 4, layout: 'mainTop', label: 'Main top', description: 'one large page on top, three below' },
];

/** [x, y, width, height] */
export type PreviewRect = [number, number, number, number];

export const PREVIEW = { width: 48, height: 32, gap: 2.5 };

const segments = (start: number, len: number, n: number) => {
  const size = (len - PREVIEW.gap * (n - 1)) / n;
  return Array.from({ length: n }, (_, i) => [start + i * (size + PREVIEW.gap), size] as const);
};
const cols = ([x, y, w, h]: PreviewRect, n: number) => segments(x, w, n).map(([sx, sw]): PreviewRect => [sx, y, sw, h]);
const rows = ([x, y, w, h]: PreviewRect, n: number) => segments(y, h, n).map(([sy, sh]): PreviewRect => [x, sy, w, sh]);

/** Pane boxes for the layout thumbnail; mirrors `browser/split.rs::pane_rects` at preview scale. */
export function previewRects(layout: SplitLayout, count: number): PreviewRect[] {
  const area: PreviewRect = [0, 0, PREVIEW.width, PREVIEW.height];
  switch (layout) {
    case 'columns':
      return cols(area, count);
    case 'rows':
      return rows(area, count);
    case 'mainLeft': {
      const [main, rest] = cols(area, 2) as [PreviewRect, PreviewRect];
      return [main, ...rows(rest, count - 1)];
    }
    case 'mainTop': {
      const [main, rest] = rows(area, 2) as [PreviewRect, PreviewRect];
      return [main, ...cols(rest, count - 1)];
    }
    case 'grid':
      return rows(area, 2).flatMap((r) => cols(r, 2));
  }
}
