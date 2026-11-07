//! Open/close decision for a left click on the tray icon.
//!
//! Clicking the icon while the popup is open first takes the focus away from
//! the popup (it hides), then the click arrives. Without a guard that click
//! would open it again at once.
use std::time::{Duration, Instant};

pub const REOPEN_GUARD: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    Show,
    Hide,
    Ignore,
}

#[derive(Debug, Default)]
pub struct PopupToggle {
    hidden_by_focus_at: Option<Instant>,
}

impl PopupToggle {
    pub fn on_click(&mut self, now: Instant, visible: bool) -> ToggleAction {
        let hidden_at = self.hidden_by_focus_at.take();
        if visible {
            return ToggleAction::Hide;
        }
        match hidden_at {
            Some(t) if now.saturating_duration_since(t) <= REOPEN_GUARD => ToggleAction::Ignore,
            _ => ToggleAction::Show,
        }
    }

    pub fn on_hidden_by_focus(&mut self, now: Instant) {
        self.hidden_by_focus_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_while_hidden_shows() {
        let mut t = PopupToggle::default();
        assert_eq!(t.on_click(Instant::now(), false), ToggleAction::Show);
    }

    #[test]
    fn click_while_visible_hides() {
        let mut t = PopupToggle::default();
        assert_eq!(t.on_click(Instant::now(), true), ToggleAction::Hide);
    }

    #[test]
    fn click_right_after_focus_loss_is_ignored() {
        let mut t = PopupToggle::default();
        let t0 = Instant::now();
        t.on_hidden_by_focus(t0);
        assert_eq!(
            t.on_click(t0 + Duration::from_millis(120), false),
            ToggleAction::Ignore
        );
    }

    #[test]
    fn click_after_guard_shows_again() {
        let mut t = PopupToggle::default();
        let t0 = Instant::now();
        t.on_hidden_by_focus(t0);
        assert_eq!(
            t.on_click(t0 + REOPEN_GUARD + Duration::from_millis(1), false),
            ToggleAction::Show
        );
    }

    #[test]
    fn guard_is_used_once() {
        let mut t = PopupToggle::default();
        let t0 = Instant::now();
        t.on_hidden_by_focus(t0);
        assert_eq!(
            t.on_click(t0 + Duration::from_millis(100), false),
            ToggleAction::Ignore
        );
        assert_eq!(
            t.on_click(t0 + Duration::from_millis(200), false),
            ToggleAction::Show
        );
    }
}
