//! Popup geometry in device independent pixels (DIP).
use super::model::{PopupModel, ShortcutRow};

pub const WIDTH: f32 = 300.0;
const PAD: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowRects {
    pub label: RectF,
    pub bar: RectF,
    pub pct: RectF,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShortcutRects {
    /// The whole row, used for the recording text.
    pub text: RectF,
    pub label: RectF,
    pub value: RectF,
    /// Only when the row is idle.
    pub change: Option<RectF>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub height: f32,
    pub title: RectF,
    pub source: RectF,
    pub status_line: Option<RectF>,
    pub rows: Vec<RowRects>,
    pub note: Option<RectF>,
    pub hint: Option<RectF>,
    pub button: RectF,
    pub runtime: Option<RectF>,
    pub shortcut: Option<ShortcutRects>,
}

impl Layout {
    pub fn hit_button(&self, x: f32, y: f32) -> bool {
        self.button.contains(x, y)
    }

    pub fn hit_change(&self, x: f32, y: f32) -> bool {
        self.shortcut
            .and_then(|s| s.change)
            .is_some_and(|c| c.contains(x, y))
    }
}

pub fn layout(model: &PopupModel) -> Layout {
    let right = WIDTH - PAD;
    let mut y = 48.0;
    let status_line = model.status_line.map(|_| {
        let r = RectF::new(PAD, y, right, y + 20.0);
        y += 24.0;
        r
    });
    let rows = model
        .rows
        .iter()
        .map(|_| {
            let r = RowRects {
                label: RectF::new(PAD, y, 72.0, y + 20.0),
                bar: RectF::new(72.0, y + 7.0, 236.0, y + 13.0),
                pct: RectF::new(236.0, y, right, y + 20.0),
            };
            y += 24.0;
            r
        })
        .collect();
    let runtime = model.runtime.as_ref().map(|_| {
        let r = RectF::new(PAD, y, right, y + 18.0);
        y += 20.0;
        r
    });
    let note = model.note.map(|_| {
        let r = RectF::new(PAD, y, right, y + 18.0);
        y += 20.0;
        r
    });
    let hint = model.hint.map(|_| {
        let r = RectF::new(PAD, y, right, y + 34.0);
        y += 36.0;
        r
    });
    y += 8.0;
    let button = RectF::new(PAD, y, right, y + 32.0);
    let mut bottom = button.bottom;
    let shortcut = model.shortcut.as_ref().map(|row| {
        let top = button.bottom + 8.0;
        bottom = top + 32.0;
        ShortcutRects {
            text: RectF::new(PAD, top, right, bottom),
            label: RectF::new(PAD, top, 90.0, bottom),
            value: RectF::new(90.0, top, 220.0, bottom),
            change: matches!(row, ShortcutRow::Idle(_))
                .then(|| RectF::new(220.0, top, right, bottom)),
        }
    });
    Layout {
        height: bottom + PAD,
        title: RectF::new(PAD, 12.0, 212.0, 36.0),
        source: RectF::new(212.0, 15.0, right, 36.0),
        status_line,
        rows,
        note,
        hint,
        button,
        runtime,
        shortcut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::popup::model::{Button, Row, ShortcutRow};
    use crate::app::status::Level;

    fn model(rows: usize, status: bool, note: bool, hint: bool) -> PopupModel {
        PopupModel {
            title: "X".into(),
            source: None,
            status_line: status.then_some("Connecting..."),
            rows: (0..rows)
                .map(|_| Row {
                    label: "Left",
                    pct: Some(50),
                    level: Level::High,
                    stale: false,
                })
                .collect(),
            note: note.then_some("n"),
            hint: hint.then_some("h"),
            button: Button {
                label: "Connect",
                enabled: true,
            },
            runtime: None,
            shortcut: None,
        }
    }

    #[test]
    fn runtime_line_sits_between_rows_and_note() {
        let mut m = model(3, false, true, false);
        m.runtime = Some("~3 h left".into());
        let l = layout(&m);
        // Rows end at 48 + 3 * 24 = 120.
        assert_eq!(l.runtime, Some(RectF::new(16.0, 120.0, 284.0, 138.0)));
        assert_eq!(l.note, Some(RectF::new(16.0, 140.0, 284.0, 158.0)));
        assert_eq!(l.button, RectF::new(16.0, 168.0, 284.0, 200.0));
        assert_eq!(l.height, 216.0);
    }

    #[test]
    fn no_runtime_no_shortcut_keeps_old_layout() {
        let l = layout(&model(3, false, false, false));
        assert_eq!(l.runtime, None);
        assert_eq!(l.shortcut, None);
        assert!(!l.hit_change(250.0, 190.0));
    }

    #[test]
    fn idle_shortcut_row_below_button() {
        let mut m = model(3, false, false, false);
        m.shortcut = Some(ShortcutRow::Idle("Ctrl+Alt+B".into()));
        let l = layout(&m);
        // Button 128..160, row 168..200.
        let s = l.shortcut.expect("shortcut row");
        assert_eq!(s.label, RectF::new(16.0, 168.0, 90.0, 200.0));
        assert_eq!(s.value, RectF::new(90.0, 168.0, 220.0, 200.0));
        assert_eq!(s.change, Some(RectF::new(220.0, 168.0, 284.0, 200.0)));
        assert_eq!(l.height, 216.0);
        assert!(l.hit_change(250.0, 184.0));
        assert!(!l.hit_change(150.0, 184.0));
        assert!(!l.hit_button(250.0, 184.0));
    }

    #[test]
    fn recording_shortcut_row_has_no_change_link() {
        let mut m = model(3, false, false, false);
        m.shortcut = Some(ShortcutRow::Recording(None));
        let l = layout(&m);
        let s = l.shortcut.expect("shortcut row");
        assert_eq!(s.text, RectF::new(16.0, 168.0, 284.0, 200.0));
        assert_eq!(s.change, None);
        assert!(!l.hit_change(250.0, 184.0));
    }

    #[test]
    fn three_rows_match_spike_geometry() {
        let l = layout(&model(3, false, false, false));
        assert_eq!(l.height, 176.0);
        assert_eq!(l.rows.len(), 3);
        assert_eq!(l.rows[0].label, RectF::new(16.0, 48.0, 72.0, 68.0));
        assert_eq!(l.rows[0].bar, RectF::new(72.0, 55.0, 236.0, 61.0));
        assert_eq!(l.rows[0].pct, RectF::new(236.0, 48.0, 284.0, 68.0));
        assert_eq!(l.rows[2].label.top, 96.0);
        assert_eq!(l.button, RectF::new(16.0, 128.0, 284.0, 160.0));
        assert_eq!(l.status_line, None);
        assert_eq!(l.note, None);
        assert_eq!(l.hint, None);
    }

    #[test]
    fn title_and_source_share_the_header() {
        let l = layout(&model(1, false, false, false));
        assert_eq!(l.title, RectF::new(16.0, 12.0, 212.0, 36.0));
        assert_eq!(l.source, RectF::new(212.0, 15.0, 284.0, 36.0));
    }

    #[test]
    fn status_line_takes_one_row() {
        let l = layout(&model(0, true, false, false));
        assert_eq!(l.status_line, Some(RectF::new(16.0, 48.0, 284.0, 68.0)));
        assert_eq!(l.button.top, 80.0);
        assert_eq!(l.height, 128.0);
    }

    #[test]
    fn note_and_hint_push_the_button_down() {
        let base = layout(&model(2, false, false, false));
        let with_note = layout(&model(2, false, true, false));
        let with_hint = layout(&model(2, false, false, true));
        let both = layout(&model(2, false, true, true));
        assert_eq!(with_note.note, Some(RectF::new(16.0, 96.0, 284.0, 114.0)));
        assert_eq!(with_note.height, base.height + 20.0);
        assert_eq!(with_hint.hint, Some(RectF::new(16.0, 96.0, 284.0, 130.0)));
        assert_eq!(with_hint.height, base.height + 36.0);
        assert_eq!(both.hint.unwrap().top, 116.0);
        assert_eq!(both.height, base.height + 56.0);
        assert_eq!(both.button.bottom + 16.0, both.height);
    }

    #[test]
    fn hit_button_only_inside() {
        let l = layout(&model(3, false, false, false));
        assert!(l.hit_button(150.0, 144.0));
        assert!(l.hit_button(16.0, 128.0));
        assert!(!l.hit_button(150.0, 120.0));
        assert!(!l.hit_button(10.0, 144.0));
        assert!(!l.hit_button(150.0, 170.0));
    }
}
