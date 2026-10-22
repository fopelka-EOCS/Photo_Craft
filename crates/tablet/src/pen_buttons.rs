//! The pen's barrel (side) button, from the per-event flags a platform reports: whether it is
//! held, whether it went down since the UI last looked, and a click with it (pressed and released
//! within [`CLICK_MS`]). Tablet drivers send that click for a side switch set to Right Click: with
//! Windows Ink, Wacom's driver reports it as a short contact with the secondary button, and the
//! window gets no press of its own for it. Pure, so it is tested on every platform;
//! [`crate::windows`] feeds it. (The click timing follows storytold/photocraft#7616.)

/// Longest press of the button that still counts as a click, in milliseconds. A longer hold is
/// the button held for something else (a right-drag), not a click.
pub const CLICK_MS: u32 = 600;

/// What the barrel button did since the UI last asked ([`Tracker::take`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    /// The button is held now.
    pub held: bool,
    /// It went down since the last [`Tracker::take`] (it may be up again already).
    pub pressed: bool,
    /// It was clicked since the last [`Tracker::take`]: pressed and released within [`CLICK_MS`].
    pub clicked: bool,
}

/// Tracks the barrel button across pen events.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tracker {
    /// When the button went down (the event's time in milliseconds), while it is held.
    down_at: Option<u32>,
    pressed: bool,
    clicked: bool,
}

impl Tracker {
    /// Nothing held (usable in a `static`).
    pub const DEFAULT: Tracker = Tracker { down_at: None, pressed: false, clicked: false };

    /// One pen event: the button's state at `time_ms` (a millisecond clock that may wrap, like
    /// Windows' message time).
    pub fn update(&mut self, barrel: bool, time_ms: u32) {
        match (self.down_at, barrel) {
            (None, true) => {
                self.down_at = Some(time_ms);
                self.pressed = true;
            }
            (Some(at), false) => {
                self.down_at = None;
                self.clicked |= time_ms.wrapping_sub(at) <= CLICK_MS;
            }
            _ => {}
        }
    }

    /// The pen left the window or its range: the button can't still be held (no click).
    pub fn leave(&mut self) {
        self.down_at = None;
    }

    /// The button is held.
    pub fn held(&self) -> bool {
        self.down_at.is_some()
    }

    /// What happened since the last call; forgets the press and the click.
    pub fn take(&mut self) -> Buttons {
        Buttons { held: self.held(), pressed: std::mem::take(&mut self.pressed), clicked: std::mem::take(&mut self.clicked) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_press_and_release_is_a_click() {
        let mut t = Tracker::default();
        t.update(false, 0);
        assert_eq!(t.take(), Buttons::default(), "nothing yet");
        t.update(true, 1000);
        assert_eq!(t.take(), Buttons { held: true, pressed: true, clicked: false });
        t.update(true, 1050);
        assert_eq!(t.take(), Buttons { held: true, pressed: false, clicked: false }, "the press is reported once");
        t.update(false, 1120);
        assert_eq!(t.take(), Buttons { held: false, pressed: false, clicked: true });
        assert_eq!(t.take(), Buttons::default(), "the click is reported once");
    }

    #[test]
    fn a_click_between_two_takes_reports_both_the_press_and_the_click() {
        let mut t = Tracker::default();
        t.update(true, 10);
        t.update(false, 40);
        assert_eq!(t.take(), Buttons { held: false, pressed: true, clicked: true });
    }

    #[test]
    fn a_long_hold_is_no_click() {
        let mut t = Tracker::default();
        t.update(true, 0);
        t.update(false, CLICK_MS + 1);
        assert_eq!(t.take(), Buttons { held: false, pressed: true, clicked: false });
    }

    #[test]
    fn the_clock_may_wrap() {
        let mut t = Tracker::default();
        t.update(true, u32::MAX - 10);
        t.update(false, 50);
        assert!(t.take().clicked);
    }

    #[test]
    fn leaving_releases_the_button_without_a_click() {
        let mut t = Tracker::default();
        t.update(true, 0);
        t.leave();
        assert_eq!(t.take(), Buttons { held: false, pressed: true, clicked: false });
        t.update(false, 10);
        assert_eq!(t.take(), Buttons::default());
    }
}
