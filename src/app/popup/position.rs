//! Where the popup goes, in physical screen pixels.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxRect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// Top left corner of the visible popup of `size` (width, height), placed next
/// to the taskbar of the monitor that was clicked on. The taskbar edge is the
/// side where the work area is smaller than the monitor (none: auto hide,
/// treated as bottom). Along the taskbar the popup is centered on the click,
/// and it always stays `gap` pixels inside the work area.
pub fn popup_origin(
    click: (i32, i32),
    monitor: PxRect,
    work: PxRect,
    size: (i32, i32),
    gap: i32,
) -> (i32, i32) {
    let (w, h) = size;
    let along_x = clamp_in(click.0 - w / 2, work.left + gap, work.right - w - gap);
    let along_y = clamp_in(click.1 - h / 2, work.top + gap, work.bottom - h - gap);
    if work.bottom < monitor.bottom {
        (along_x, work.bottom - h - gap)
    } else if work.top > monitor.top {
        (along_x, work.top + gap)
    } else if work.left > monitor.left {
        (work.left + gap, along_y)
    } else if work.right < monitor.right {
        (work.right - w - gap, along_y)
    } else {
        (along_x, work.bottom - h - gap)
    }
}

/// Like `clamp`, but never panics: if the range is empty the lower bound wins.
fn clamp_in(v: i32, lo: i32, hi: i32) -> i32 {
    if hi < lo { lo } else { v.clamp(lo, hi) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: PxRect = PxRect::new(0, 0, 1920, 1080);
    const SIZE: (i32, i32) = (300, 176);

    #[test]
    fn bottom_taskbar_centered_on_click() {
        let work = PxRect::new(0, 0, 1920, 1032);
        assert_eq!(
            popup_origin((1000, 1050), MON, work, SIZE, 12),
            (850, 1032 - 176 - 12)
        );
    }

    #[test]
    fn bottom_taskbar_clamped_at_right_edge() {
        let work = PxRect::new(0, 0, 1920, 1032);
        assert_eq!(
            popup_origin((1900, 1050), MON, work, SIZE, 12),
            (1920 - 300 - 12, 844)
        );
    }

    #[test]
    fn bottom_taskbar_clamped_at_left_edge() {
        let work = PxRect::new(0, 0, 1920, 1032);
        assert_eq!(popup_origin((20, 1050), MON, work, SIZE, 12), (12, 844));
    }

    #[test]
    fn top_taskbar() {
        let work = PxRect::new(0, 48, 1920, 1080);
        assert_eq!(popup_origin((1000, 20), MON, work, SIZE, 12), (850, 60));
    }

    #[test]
    fn left_taskbar_centered_vertically() {
        let work = PxRect::new(60, 0, 1920, 1080);
        assert_eq!(popup_origin((30, 600), MON, work, SIZE, 12), (72, 512));
    }

    #[test]
    fn right_taskbar_clamped_at_bottom() {
        let work = PxRect::new(0, 0, 1860, 1080);
        assert_eq!(
            popup_origin((1890, 1070), MON, work, SIZE, 12),
            (1860 - 300 - 12, 1080 - 176 - 12)
        );
    }

    #[test]
    fn auto_hide_taskbar_treated_as_bottom() {
        assert_eq!(
            popup_origin((1000, 1075), MON, MON, SIZE, 12),
            (850, 1080 - 176 - 12)
        );
    }

    #[test]
    fn second_monitor_with_offset() {
        let mon = PxRect::new(1920, -200, 4480, 1240);
        let work = PxRect::new(1920, -200, 4480, 1192);
        assert_eq!(
            popup_origin((4400, 1220), mon, work, SIZE, 12),
            (4480 - 300 - 12, 1192 - 176 - 12)
        );
    }

    #[test]
    fn work_area_smaller_than_popup_does_not_panic() {
        let mon = PxRect::new(0, 0, 200, 100);
        let work = PxRect::new(0, 0, 200, 60);
        let (x, y) = popup_origin((100, 80), mon, work, SIZE, 12);
        assert_eq!(x, 12);
        assert_eq!(y, 60 - 176 - 12);
    }
}
