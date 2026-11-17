//! Platform-independent input snapshot. Keys are Windows virtual-key codes,
//! which is what Fusion stores in key parameters and player controls.

use std::collections::HashSet;

pub mod vk {
    pub const LBUTTON: u16 = 0x01;
    pub const RBUTTON: u16 = 0x02;
    pub const MBUTTON: u16 = 0x04;
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Keys currently held (includes mouse buttons as VK 1/2/4).
    pub down: HashSet<u16>,
    /// Keys that went down since the previous tick.
    pub pressed: HashSet<u16>,
    /// Mouse position in frame coordinates.
    pub mouse: (f64, f64),
    /// Buttons clicked since the previous tick: 0 left, 1 middle, 2 right.
    pub clicked: [bool; 3],
    pub double_clicked: [bool; 3],
    pub wheel: f32,
    /// Text typed since the previous tick (for edit boxes).
    pub typed: Vec<char>,
}

/// Tells double clicks apart: a click soon after the previous one with the same button,
/// close to where it was, is a double click (as Windows decides). A third click starts over.
#[derive(Debug, Clone, Default)]
pub struct DoubleClicks {
    /// Per button: time (seconds) and position of the last single click.
    last: [Option<(f64, (f32, f32))>; 3],
}

impl DoubleClicks {
    /// Windows' default double-click time.
    pub const TIME: f64 = 0.5;
    /// How far apart (in window pixels) the two clicks may be.
    pub const DISTANCE: f32 = 6.0;

    /// A click of `button` at time `now` (seconds) and window position `at`; true if it
    /// completes a double click.
    pub fn click(&mut self, button: usize, now: f64, at: (f32, f32)) -> bool {
        let Some(slot) = self.last.get_mut(button) else { return false };
        let double = slot.is_some_and(|(t, p)| {
            now - t <= Self::TIME && (at.0 - p.0).abs() <= Self::DISTANCE && (at.1 - p.1).abs() <= Self::DISTANCE
        });
        *slot = if double { None } else { Some((now, at)) };
        double
    }
}

impl Input {
    /// Clear the edge-triggered parts after a tick consumed them.
    pub fn consume_edges(&mut self) {
        self.pressed.clear();
        self.clicked = [false; 3];
        self.double_clicked = [false; 3];
        self.wheel = 0.0;
        self.typed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::DoubleClicks;

    #[test]
    fn double_clicks_need_two_quick_close_clicks() {
        let mut d = DoubleClicks::default();
        assert!(!d.click(0, 1.0, (10.0, 10.0)));
        assert!(d.click(0, 1.3, (12.0, 11.0)));
        // A third click is a new first click.
        assert!(!d.click(0, 1.4, (12.0, 11.0)));
        // Too slow, too far, or another button.
        assert!(!d.click(0, 2.0, (12.0, 11.0)));
        assert!(!d.click(0, 2.1, (40.0, 11.0)));
        assert!(!d.click(2, 2.2, (40.0, 11.0)));
    }
}
