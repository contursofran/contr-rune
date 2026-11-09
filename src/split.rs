//! Two terminals side by side: which tabs the panes show, which pane has focus, and the width
//! math of the divider.

/// A pane keeps at least this share of the panel's width.
pub const MIN_FRAC: f32 = 0.2;

/// The split panel: two panes, one focused.
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    /// The session ids shown in the left and the right pane.
    pub panes: [u64; 2],
    /// The focused pane (0 left, 1 right); its tab is the app's active tab.
    pub focus: usize,
    /// The left pane's share of the panel's width.
    pub frac: f32,
    /// The project both tabs belong to.
    pub project: usize,
}

impl Split {
    pub fn new(left: u64, right: u64, project: usize) -> Split {
        Split { panes: [left, right], focus: 1, frac: 0.5, project }
    }
}

/// Keep the left pane's share inside `[MIN_FRAC, 1 - MIN_FRAC]` (a non-finite share becomes half).
pub fn clamp_frac(f: f32) -> f32 {
    if f.is_finite() { f.clamp(MIN_FRAC, 1.0 - MIN_FRAC) } else { 0.5 }
}

/// The share after a divider drag of `dx` points that started at `start`, in a panel `total` wide.
pub fn dragged_frac(start: f32, dx: f32, total: f32) -> f32 {
    if total <= 0.0 { clamp_frac(start) } else { clamp_frac(start + dx / total) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_each_pane_at_a_fifth() {
        assert_eq!(clamp_frac(0.5), 0.5);
        assert_eq!(clamp_frac(0.0), MIN_FRAC);
        assert_eq!(clamp_frac(-3.0), MIN_FRAC);
        assert_eq!(clamp_frac(1.0), 1.0 - MIN_FRAC);
        assert_eq!(clamp_frac(f32::NAN), 0.5);
        assert_eq!(clamp_frac(0.2), 0.2);
        assert_eq!(clamp_frac(0.8), 0.8);
    }

    #[test]
    fn a_drag_moves_the_divider_by_its_share_of_the_panel() {
        assert_eq!(dragged_frac(0.5, 100.0, 1000.0), 0.6);
        assert_eq!(dragged_frac(0.5, -100.0, 1000.0), 0.4);
        assert_eq!(dragged_frac(0.5, 5000.0, 1000.0), 0.8, "stops at the right limit");
        assert_eq!(dragged_frac(0.5, -5000.0, 1000.0), 0.2, "stops at the left limit");
        assert_eq!(dragged_frac(0.5, 10.0, 0.0), 0.5, "no width yet: no move");
    }

    #[test]
    fn a_new_split_focuses_the_right_pane() {
        let s = Split::new(3, 4, 0);
        assert_eq!((s.panes, s.focus, s.frac), ([3, 4], 1, 0.5));
    }
}
