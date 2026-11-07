//! Remaining runtime estimate from the battery drops of each bud (Phase 12).

use std::mem::{Discriminant, discriminant};
use std::time::{Duration, Instant};

use crate::app::status::{DeviceStatus, Source};

const MAX_POINTS: usize = 10;
const MIN_POINTS: usize = 4;
const MIN_SPAN: Duration = Duration::from_secs(10 * 60);
const MIN_PER_PCT: Duration = Duration::from_secs(20);
const MIN_STALE: Duration = Duration::from_secs(10 * 60);
/// Shown while the estimator still collects data.
pub const LEARNING: &str = "- h left";

#[derive(Debug, Default)]
struct Series {
    last_value: Option<u8>,
    points: Vec<(Instant, u8)>,
}

impl Series {
    fn clear(&mut self) {
        self.last_value = None;
        self.points.clear();
    }

    fn feed(&mut self, now: Instant, value: Option<u8>) {
        let Some(v) = value else {
            self.clear();
            return;
        };
        match self.last_value {
            Some(last) if v < last => {
                self.points.push((now, v));
                if self.points.len() > MAX_POINTS {
                    self.points.remove(0);
                }
            }
            Some(last) if v > last => self.points.clear(),
            _ => {}
        }
        self.last_value = Some(v);
    }

    fn estimate(&self, now: Instant) -> Option<Duration> {
        if self.points.len() < MIN_POINTS {
            return None;
        }
        let (t_first, v_first) = self.points[0];
        let (t_last, v_last) = *self.points.last()?;
        let span = t_last.duration_since(t_first);
        if span < MIN_SPAN || v_first <= v_last {
            return None;
        }
        let per_pct = span / u32::from(v_first - v_last);
        if per_pct < MIN_PER_PCT {
            return None;
        }
        let since = now.saturating_duration_since(t_last);
        if since > (per_pct * 3).max(MIN_STALE) {
            return None;
        }
        Some((per_pct * u32::from(v_last)).saturating_sub(since))
    }
}

#[derive(Debug, Default)]
pub struct RuntimeEstimator {
    source: Option<Discriminant<Source>>,
    left: Series,
    right: Series,
}

impl RuntimeEstimator {
    pub fn on_status(&mut self, now: Instant, status: &DeviceStatus) {
        let DeviceStatus::Connected {
            view: Some(view),
            source,
        } = status
        else {
            self.clear();
            return;
        };
        let kind = discriminant(source);
        if self.source != Some(kind) {
            self.clear();
            self.source = Some(kind);
        }
        self.left.feed(now, view.left);
        self.right.feed(now, view.right);
    }

    /// Remaining time of the bud that runs out first, if known.
    pub fn estimate(&self, now: Instant) -> Option<Duration> {
        [self.left.estimate(now), self.right.estimate(now)]
            .into_iter()
            .flatten()
            .min()
    }

    /// Text for the runtime line: the estimate, or `LEARNING` while data is
    /// still collected. `None` when no bud value is shown at all.
    pub fn line(&self, now: Instant, status: &DeviceStatus) -> Option<String> {
        let view = status.view()?;
        if view.left.is_none() && view.right.is_none() {
            return None;
        }
        Some(
            self.estimate(now)
                .map(format_remaining)
                .unwrap_or_else(|| LEARNING.to_string()),
        )
    }

    fn clear(&mut self) {
        self.source = None;
        self.left.clear();
        self.right.clear();
    }
}

/// `~40 min left` below one hour (10 minute steps, at least 10),
/// `~1.5 h left` from one hour on (half hour steps).
pub fn format_remaining(d: Duration) -> String {
    let minutes = d.as_secs_f64() / 60.0;
    if minutes < 60.0 {
        let rounded = ((minutes / 10.0).round() * 10.0).max(10.0) as u32;
        if rounded < 60 {
            return format!("~{rounded} min left");
        }
    }
    let half_hours = (minutes / 30.0).round() as u32;
    if half_hours.is_multiple_of(2) {
        format!("~{} h left", half_hours / 2)
    } else {
        format!("~{}.5 h left", half_hours / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::status::BatteryView;

    fn min(m: u64) -> Duration {
        Duration::from_secs(m * 60)
    }

    fn vendor(left: Option<u8>, right: Option<u8>) -> DeviceStatus {
        DeviceStatus::Connected {
            view: Some(BatteryView {
                left,
                right,
                case: None,
            }),
            source: Source::Vendor(crate::app::tiers::Vendor::Edifier),
        }
    }

    fn fast_pair(left: u8, right: u8, case_last_known: bool) -> DeviceStatus {
        DeviceStatus::Connected {
            view: Some(BatteryView {
                left: Some(left),
                right: Some(right),
                case: Some(50),
            }),
            source: Source::FastPair {
                model_id: None,
                case_last_known,
            },
        }
    }

    /// Feeds left only (right None) values at the given minutes.
    fn feed_left(e: &mut RuntimeEstimator, t0: Instant, steps: &[(u64, u8)]) {
        for &(m, v) in steps {
            e.on_status(t0 + min(m), &vendor(Some(v), None));
        }
    }

    fn approx(d: Duration, expected_secs: f64, tol_secs: f64) {
        let got = d.as_secs_f64();
        assert!(
            (got - expected_secs).abs() <= tol_secs,
            "got {got} s, expected {expected_secs} s"
        );
    }

    #[test]
    fn four_drops_four_minutes_apart() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        // Baseline 81 at 0, then drops at 4, 8, 12, 16 min to 80..77.
        feed_left(&mut e, t0, &[(0, 81), (4, 80), (8, 79), (12, 78), (16, 77)]);
        // per_pct = 12 min / 3 = 4 min, remaining = 77 * 4 min.
        approx(e.estimate(t0 + min(16)).unwrap(), 77.0 * 240.0, 1.0);
    }

    #[test]
    fn first_reading_is_baseline_only() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        // Long plateau at 100, then drops every 4 minutes.
        feed_left(
            &mut e,
            t0,
            &[(0, 100), (30, 100), (60, 99), (64, 98), (68, 97), (72, 96)],
        );
        approx(e.estimate(t0 + min(72)).unwrap(), 96.0 * 240.0, 1.0);
    }

    #[test]
    fn not_enough_points_or_span() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        feed_left(&mut e, t0, &[(0, 81), (4, 80), (8, 79), (12, 78)]);
        assert_eq!(e.estimate(t0 + min(12)), None, "3 points");

        let mut e = RuntimeEstimator::default();
        // 4 points but only 9 minutes from first to last point.
        feed_left(&mut e, t0, &[(0, 81), (1, 80), (4, 79), (7, 78), (10, 77)]);
        assert_eq!(e.estimate(t0 + min(10)), None, "span under 10 min");
    }

    #[test]
    fn rise_clears_and_starts_new_baseline() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        feed_left(&mut e, t0, &[(0, 81), (4, 80), (8, 79), (12, 78), (16, 77)]);
        assert!(e.estimate(t0 + min(16)).is_some());
        e.on_status(t0 + min(17), &vendor(Some(78), None));
        assert_eq!(e.estimate(t0 + min(17)), None);
        // New baseline 78; drops from 20 min on every 5 min.
        feed_left(&mut e, t0, &[(20, 77), (25, 76), (30, 75), (35, 74)]);
        approx(e.estimate(t0 + min(35)).unwrap(), 74.0 * 300.0, 1.0);
    }

    #[test]
    fn remaining_goes_down_between_drops() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        feed_left(&mut e, t0, &[(0, 81), (4, 80), (8, 79), (12, 78), (16, 77)]);
        approx(e.estimate(t0 + min(18)).unwrap(), 77.0 * 240.0 - 120.0, 1.0);
    }

    #[test]
    fn smaller_bud_wins() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        let steps = [
            (0, 81, 61),
            (4, 80, 60),
            (8, 79, 59),
            (12, 78, 58),
            (16, 77, 57),
        ];
        for (m, l, r) in steps {
            e.on_status(t0 + min(m), &vendor(Some(l), Some(r)));
        }
        approx(e.estimate(t0 + min(16)).unwrap(), 57.0 * 240.0, 1.0);
    }

    #[test]
    fn frozen_bud_is_ignored_when_stale() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        // Both drop every 4 min until 16 min; right is lower.
        for (m, l, r) in [
            (0, 81, 61),
            (4, 80, 60),
            (8, 79, 59),
            (12, 78, 58),
            (16, 77, 57),
        ] {
            e.on_status(t0 + min(m), &vendor(Some(l), Some(r)));
        }
        // Right goes into the case and freezes at 57, left keeps dropping.
        for (m, l) in [(20, 76), (24, 75), (28, 74), (32, 73)] {
            e.on_status(t0 + min(m), &vendor(Some(l), Some(57)));
        }
        // Right's last drop was at 16 min, stale limit max(12, 10) min: stale at 32.
        approx(e.estimate(t0 + min(32)).unwrap(), 73.0 * 240.0, 1.0);
    }

    #[test]
    fn disconnect_clears() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        feed_left(&mut e, t0, &[(0, 81), (4, 80), (8, 79), (12, 78), (16, 77)]);
        e.on_status(t0 + min(17), &DeviceStatus::Disconnected);
        assert_eq!(e.estimate(t0 + min(17)), None);
        e.on_status(t0 + min(18), &vendor(Some(77), None));
        assert_eq!(e.estimate(t0 + min(18)), None);
    }

    #[test]
    fn value_none_clears_that_series() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        feed_left(&mut e, t0, &[(0, 81), (4, 80), (8, 79), (12, 78), (16, 77)]);
        e.on_status(t0 + min(17), &vendor(None, None));
        e.on_status(t0 + min(18), &vendor(Some(77), None));
        assert_eq!(e.estimate(t0 + min(18)), None);
    }

    #[test]
    fn source_variant_change_clears_but_field_change_does_not() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        let mut flip = false;
        for (m, v) in [(0, 81), (4, 80), (8, 79), (12, 78), (16, 77)] {
            flip = !flip;
            e.on_status(t0 + min(m), &fast_pair(v, v, flip));
        }
        assert!(e.estimate(t0 + min(16)).is_some(), "case_last_known flip");
        e.on_status(t0 + min(17), &vendor(Some(77), Some(77)));
        assert_eq!(e.estimate(t0 + min(17)), None, "variant change");
    }

    #[test]
    fn rejects_implausibly_fast_drops() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        // Jumps of 20 % every 4 minutes: 12 s per %, under the 20 s floor.
        feed_left(
            &mut e,
            t0,
            &[(0, 100), (4, 80), (8, 60), (12, 40), (16, 20)],
        );
        assert_eq!(e.estimate(t0 + min(16)), None);
    }

    #[test]
    fn replay_of_real_log() {
        // Release log 2026-11-01, 19:48:26 to 20:25:47 (left, right).
        let log = [
            ("19:48:26", 70, 72),
            ("19:48:33", 69, 72),
            ("19:52:14", 69, 71),
            ("19:52:48", 68, 71),
            ("19:56:33", 68, 70),
            ("19:56:40", 67, 70),
            ("19:59:41", 67, 69),
            ("20:00:24", 66, 69),
            ("20:03:35", 65, 69),
            ("20:04:46", 65, 68),
            ("20:07:39", 65, 67),
            ("20:08:31", 64, 67),
            ("20:11:10", 63, 67),
            ("20:12:05", 63, 66),
            ("20:15:21", 63, 65),
            ("20:15:58", 62, 65),
            ("20:18:30", 61, 65),
            ("20:19:39", 61, 64),
            ("20:20:29", 60, 64),
            ("20:22:55", 60, 63),
            ("20:24:13", 59, 63),
            ("20:25:47", 59, 62),
        ];
        let secs = |s: &str| -> u64 {
            let p: Vec<u64> = s.split(':').map(|x| x.parse().unwrap()).collect();
            p[0] * 3600 + p[1] * 60 + p[2]
        };
        let t0 = Instant::now();
        let base = secs(log[0].0);
        let mut e = RuntimeEstimator::default();
        for (t, l, r) in log {
            let at = t0 + Duration::from_secs(secs(t) - base);
            e.on_status(at, &vendor(Some(l), Some(r)));
        }
        let end = t0 + Duration::from_secs(secs("20:25:47") - base);
        let h = e.estimate(end).unwrap().as_secs_f64() / 3600.0;
        assert!((3.0..=4.0).contains(&h), "estimate {h} h");
    }

    #[test]
    fn format_edges() {
        let cases = [
            (9, "~10 min left"),
            (14, "~10 min left"),
            (55, "~1 h left"),
            (59, "~1 h left"),
            (60, "~1 h left"),
            (74, "~1 h left"),
            (75, "~1.5 h left"),
            (89, "~1.5 h left"),
            (90, "~1.5 h left"),
            (240, "~4 h left"),
            (0, "~10 min left"),
            (34, "~30 min left"),
            (35, "~40 min left"),
        ];
        for (m, want) in cases {
            assert_eq!(format_remaining(min(m)), want, "{m} min");
        }
    }

    #[test]
    fn line_shows_placeholder_while_learning() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        let s = vendor(Some(80), Some(82));
        e.on_status(t0, &s);
        assert_eq!(e.line(t0, &s), Some(LEARNING.to_string()));
    }

    #[test]
    fn line_shows_estimate_once_known() {
        let t0 = Instant::now();
        let mut e = RuntimeEstimator::default();
        feed_left(
            &mut e,
            t0,
            &[(0, 100), (30, 100), (60, 99), (64, 98), (68, 97), (72, 96)],
        );
        let s = vendor(Some(96), None);
        let want = format_remaining(e.estimate(t0 + min(72)).unwrap());
        assert_eq!(e.line(t0 + min(72), &s), Some(want));
    }

    #[test]
    fn line_hidden_without_battery_values() {
        let t0 = Instant::now();
        let e = RuntimeEstimator::default();
        assert_eq!(e.line(t0, &DeviceStatus::Disconnected), None);
        assert_eq!(e.line(t0, &DeviceStatus::Connecting), None);
        assert_eq!(e.line(t0, &vendor(None, None)), None);
    }
}
