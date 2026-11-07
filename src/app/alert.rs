use super::status::DeviceStatus;

pub const LOW_THRESHOLD: u8 = 15; // warn when value < 15
pub const REARM_THRESHOLD: u8 = 20; // arm again when value >= 20

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bud {
    Left,
    Right,
}

#[derive(Debug, Default)]
pub struct LowBatteryAlert {
    left_warned: bool,
    right_warned: bool,
}

impl LowBatteryAlert {
    /// Returns the buds that just crossed below the threshold, with their value.
    pub fn update(&mut self, status: &DeviceStatus) -> Vec<(Bud, u8)> {
        let mut triggered = Vec::new();
        if let DeviceStatus::Connected {
            view: Some(view), ..
        } = status
        {
            if let Some(left) = view.left {
                if left < LOW_THRESHOLD {
                    if !self.left_warned {
                        self.left_warned = true;
                        triggered.push((Bud::Left, left));
                    }
                } else if left >= REARM_THRESHOLD {
                    self.left_warned = false;
                }
            }

            if let Some(right) = view.right {
                if right < LOW_THRESHOLD {
                    if !self.right_warned {
                        self.right_warned = true;
                        triggered.push((Bud::Right, right));
                    }
                } else if right >= REARM_THRESHOLD {
                    self.right_warned = false;
                }
            }
        }
        triggered
    }
}

/// Title and body of the toast.
pub fn alert_text(device_name: &str, low: &[(Bud, u8)]) -> (String, String) {
    let title = format!("{device_name}: battery low");
    let parts: Vec<String> = low
        .iter()
        .map(|(bud, pct)| {
            let name = match bud {
                Bud::Left => "Left",
                Bud::Right => "Right",
            };
            format!("{name} {pct}%")
        })
        .collect();
    (title, parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::status::BatteryView;

    #[test]
    fn test_first_warning() {
        let mut alert = LowBatteryAlert::default();
        let status = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(14),
            right: Some(80),
            case: None,
        }));
        let triggered = alert.update(&status);
        assert_eq!(triggered, vec![(Bud::Left, 14)]);
    }

    #[test]
    fn test_no_repeat() {
        let mut alert = LowBatteryAlert::default();
        let status1 = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(14),
            right: Some(80),
            case: None,
        }));
        let status2 = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(13),
            right: Some(80),
            case: None,
        }));
        let _ = alert.update(&status1);
        let triggered = alert.update(&status2);
        assert!(
            triggered.is_empty(),
            "Should not repeat warning when already warned"
        );
    }

    #[test]
    fn test_rearm_only_at_20() {
        let mut alert = LowBatteryAlert::default();
        let s_warn = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(14),
            right: Some(80),
            case: None,
        }));
        assert_eq!(alert.update(&s_warn), vec![(Bud::Left, 14)]);

        // 15..19 should NOT rearm
        for pct in 15..20 {
            let s_mid = DeviceStatus::connected_vendor(Some(BatteryView {
                left: Some(pct),
                right: Some(80),
                case: None,
            }));
            assert!(alert.update(&s_mid).is_empty());

            // Dropping back down to 14 still should not warn because it wasn't rearmed
            let s_low = DeviceStatus::connected_vendor(Some(BatteryView {
                left: Some(14),
                right: Some(80),
                case: None,
            }));
            assert!(alert.update(&s_low).is_empty(), "Must not rearm at {pct}%");
        }

        // At 20: rearm happens (no warning reported)
        let s_rearm = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(20),
            right: Some(80),
            case: None,
        }));
        assert!(alert.update(&s_rearm).is_empty());

        // Now dropping below 15 should warn again!
        let s_warn_again = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(10),
            right: Some(80),
            case: None,
        }));
        assert_eq!(alert.update(&s_warn_again), vec![(Bud::Left, 10)]);
    }

    #[test]
    fn test_both_buds_in_one_update() {
        let mut alert = LowBatteryAlert::default();
        let status = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(14),
            right: Some(12),
            case: None,
        }));
        let triggered = alert.update(&status);
        assert_eq!(triggered, vec![(Bud::Left, 14), (Bud::Right, 12)]);
    }

    #[test]
    fn test_disconnect_keeps_warned_state() {
        let mut alert = LowBatteryAlert::default();
        let s_warn = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(14),
            right: Some(80),
            case: None,
        }));
        assert_eq!(alert.update(&s_warn), vec![(Bud::Left, 14)]);

        // Disconnect and connecting
        assert!(alert.update(&DeviceStatus::Disconnected).is_empty());
        assert!(alert.update(&DeviceStatus::Connecting).is_empty());
        assert!(
            alert
                .update(&DeviceStatus::connected_vendor(None))
                .is_empty()
        );

        // Reconnect with low battery: should NOT warn again
        let s_reconnect = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(14),
            right: Some(80),
            case: None,
        }));
        assert!(alert.update(&s_reconnect).is_empty());
    }

    #[test]
    fn test_none_for_one_bud_keeps_state() {
        let mut alert = LowBatteryAlert::default();
        let s_warn = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(14),
            right: Some(80),
            case: None,
        }));
        assert_eq!(alert.update(&s_warn), vec![(Bud::Left, 14)]);

        // Left is None in next view
        let s_none = DeviceStatus::connected_vendor(Some(BatteryView {
            left: None,
            right: Some(80),
            case: None,
        }));
        assert!(alert.update(&s_none).is_empty());

        // Left reports 14 again: still warned
        assert!(alert.update(&s_warn).is_empty());
    }

    #[test]
    fn test_case_below_15_gives_nothing() {
        let mut alert = LowBatteryAlert::default();
        let status = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(80),
            right: Some(80),
            case: Some(10),
        }));
        assert!(alert.update(&status).is_empty());
    }

    #[test]
    fn test_alert_text_formatting() {
        let (title, body_left) = alert_text("NeoBuds Pro 3", &[(Bud::Left, 14)]);
        assert_eq!(title, "NeoBuds Pro 3: battery low");
        assert_eq!(body_left, "Left 14%");

        let (title, body_right) = alert_text("NeoBuds Pro 3", &[(Bud::Right, 9)]);
        assert_eq!(title, "NeoBuds Pro 3: battery low");
        assert_eq!(body_right, "Right 9%");

        let (title, body_both) = alert_text("NeoBuds Pro 3", &[(Bud::Left, 14), (Bud::Right, 12)]);
        assert_eq!(title, "NeoBuds Pro 3: battery low");
        assert_eq!(body_both, "Left 14%, Right 12%");
    }
}
