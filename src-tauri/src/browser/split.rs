//! Split view: 2–4 tabs shown side by side. Geometry is computed here (pure) in CSS pixels and
//! sent to the shell, then converted to physical pixels for the engine, so both agree exactly.

use crate::platform::TabId;
use serde::{Deserialize, Serialize};

pub const MAX_PANES: usize = 4;
/// Space between panes (CSS px); the shell draws the active-pane outline in it.
pub const GAP: f64 = 6.0;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SplitLayout {
    /// Side by side.
    Columns,
    /// Stacked top to bottom.
    Rows,
    /// One large pane on the left, the rest stacked on the right.
    MainLeft,
    /// One large pane on top, the rest side by side below.
    MainTop,
    /// 2×2 (four panes).
    Grid,
}

impl SplitLayout {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "columns" => Self::Columns,
            "rows" => Self::Rows,
            "mainLeft" => Self::MainLeft,
            "mainTop" => Self::MainTop,
            "grid" => Self::Grid,
            _ => return None,
        })
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub tabs: Vec<TabId>,
    pub layout: SplitLayout,
}

/// The pane "Open in split view" replaces when all panes are taken: the last one that is not the
/// active tab, so the tab the user is looking at never leaves the split.
pub fn pane_to_replace(panes: &[TabId], active: TabId) -> Option<usize> {
    panes.iter().rposition(|t| *t != active)
}

/// Splits `len` into `n` equal segments separated by `gap`; returns (offset, size) pairs.
fn segments(start: f64, len: f64, n: usize, gap: f64) -> Vec<(f64, f64)> {
    let size = ((len - gap * (n as f64 - 1.0)) / n as f64).max(0.0);
    (0..n)
        .map(|i| (start + i as f64 * (size + gap), size))
        .collect()
}

/// Pane rectangles inside `area` for `n` panes (2..=4).
pub fn pane_rects(layout: SplitLayout, n: usize, area: RectF, gap: f64) -> Vec<RectF> {
    let n = n.clamp(1, MAX_PANES);
    let cols = |a: RectF, k: usize| {
        segments(a.x, a.width, k, gap)
            .into_iter()
            .map(|(x, w)| RectF {
                x,
                y: a.y,
                width: w,
                height: a.height,
            })
            .collect::<Vec<_>>()
    };
    let rows = |a: RectF, k: usize| {
        segments(a.y, a.height, k, gap)
            .into_iter()
            .map(|(y, h)| RectF {
                x: a.x,
                y,
                width: a.width,
                height: h,
            })
            .collect::<Vec<_>>()
    };
    match layout {
        _ if n == 1 => vec![area],
        SplitLayout::Columns => cols(area, n),
        SplitLayout::Rows => rows(area, n),
        SplitLayout::MainLeft if n >= 3 => {
            let halves = cols(area, 2);
            let mut out = vec![halves[0]];
            out.extend(rows(halves[1], n - 1));
            out
        }
        SplitLayout::MainTop if n >= 3 => {
            let halves = rows(area, 2);
            let mut out = vec![halves[0]];
            out.extend(cols(halves[1], n - 1));
            out
        }
        SplitLayout::MainLeft => cols(area, n),
        SplitLayout::MainTop => rows(area, n),
        SplitLayout::Grid if n == 4 => rows(area, 2).into_iter().flat_map(|r| cols(r, 2)).collect(),
        SplitLayout::Grid => cols(area, n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: RectF = RectF {
        x: 0.0,
        y: 100.0,
        width: 1006.0,
        height: 606.0,
    };

    #[test]
    fn columns_and_rows_divide_with_gaps() {
        let c = pane_rects(SplitLayout::Columns, 2, AREA, 6.0);
        assert_eq!(
            c[0],
            RectF {
                x: 0.0,
                y: 100.0,
                width: 500.0,
                height: 606.0
            }
        );
        assert_eq!(c[1].x, 506.0);
        let r = pane_rects(SplitLayout::Rows, 2, AREA, 6.0);
        assert_eq!((r[0].height, r[1].y), (300.0, 406.0));
    }

    #[test]
    fn main_left_and_grid() {
        let m = pane_rects(SplitLayout::MainLeft, 3, AREA, 6.0);
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].height, 606.0);
        assert_eq!((m[1].x, m[2].y), (506.0, 406.0));
        let g = pane_rects(SplitLayout::Grid, 4, AREA, 6.0);
        assert_eq!(g.len(), 4);
        assert_eq!((g[3].x, g[3].y), (506.0, 406.0));
        assert_eq!(
            pane_rects(SplitLayout::Grid, 3, AREA, 6.0).len(),
            3,
            "grid falls back for 3 panes"
        );
    }

    #[test]
    fn a_full_split_keeps_the_active_pane() {
        assert_eq!(pane_to_replace(&[1, 2, 3, 4], 2), Some(3));
        assert_eq!(pane_to_replace(&[1, 2, 3, 4], 4), Some(2));
        assert_eq!(pane_to_replace(&[4], 4), None);
    }

    #[test]
    fn pane_count_is_clamped() {
        assert_eq!(
            pane_rects(SplitLayout::Columns, 9, AREA, 6.0).len(),
            MAX_PANES
        );
        assert_eq!(SplitLayout::parse("mainTop"), Some(SplitLayout::MainTop));
        assert_eq!(SplitLayout::parse("diagonal"), None);
    }
}
