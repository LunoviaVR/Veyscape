/** CSS colour for a tab group: preset names map to theme tokens, custom values are #rrggbb. */
export function groupColor(color: string): string {
  return /^#[0-9a-f]{6}$/i.test(color) ? color : `var(--group-${color}, var(--group-grey))`;
}

/** Text colour with enough contrast on a custom group colour; presets are handled in CSS. */
export function groupInk(color: string): string | undefined {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(color);
  if (!m) return undefined;
  const [r, g, b] = m.slice(1).map((h) => {
    const c = parseInt(h, 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.18 ? '#111111' : '#ffffff';
}

/** Preset colours shared by tab groups and bookmark folders (names map to theme tokens). */
export const PRESET_COLORS = ['grey', 'blue', 'red', 'yellow', 'green', 'pink', 'purple', 'cyan', 'orange'] as const;

/** Light-theme hex of each preset, used to seed custom colour pickers. */
export const PRESET_HEX: Record<string, string> = {
  grey: '#6b6f78',
  blue: '#2f6fe4',
  red: '#d23b32',
  yellow: '#b98900',
  green: '#1f8a4c',
  pink: '#cf3f87',
  purple: '#7a4bd6',
  cyan: '#08869b',
  orange: '#d0681a',
};
