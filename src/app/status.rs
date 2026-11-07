use crate::drivers::edifier::query::BatteryF2;

use super::tiers::Vendor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Vendor(Vendor),
    /// The single Windows value, shown while a vendor listener has no data yet.
    VendorHfp(Vendor),
    FastPair {
        model_id: Option<[u8; 3]>,
        case_last_known: bool,
    },
    FastPairHfp,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FastPairBatteryTracker {
    last_known_case: Option<u8>,
}

impl FastPairBatteryTracker {
    pub fn new() -> Self {
        Self {
            last_known_case: None,
        }
    }

    pub fn reset(&mut self) {
        self.last_known_case = None;
    }

    pub fn last_known_case(&self) -> Option<u8> {
        self.last_known_case
    }

    pub fn update(
        &mut self,
        left: Option<u8>,
        right: Option<u8>,
        case: Option<u8>,
    ) -> (BatteryView, bool) {
        let (resolved_case, is_last_known) = match case {
            Some(c) => {
                self.last_known_case = Some(c);
                (Some(c), false)
            }
            None => match self.last_known_case {
                Some(cached) => (Some(cached), true),
                None => (None, false),
            },
        };

        (
            BatteryView {
                left,
                right,
                case: resolved_case,
            },
            is_last_known,
        )
    }

    pub fn status_for(
        &mut self,
        left: Option<u8>,
        right: Option<u8>,
        case: Option<u8>,
        model_id: Option<[u8; 3]>,
    ) -> DeviceStatus {
        let (view, case_last_known) = self.update(left, right, case);
        DeviceStatus::Connected {
            view: Some(view),
            source: Source::FastPair {
                model_id,
                case_last_known,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryView {
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceStatus {
    Connecting,
    Connected {
        view: Option<BatteryView>,
        source: Source,
    },
    Disconnected,
}

impl DeviceStatus {
    pub fn view(&self) -> Option<&BatteryView> {
        match self {
            Self::Connected { view: Some(v), .. } => Some(v),
            _ => None,
        }
    }

    pub fn source(&self) -> Option<Source> {
        match self {
            Self::Connected { source, .. } => Some(*source),
            _ => None,
        }
    }

    pub fn connected_vendor(view: Option<BatteryView>) -> Self {
        Self::Connected {
            view,
            source: Source::Vendor(Vendor::Edifier),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Unknown,
    Low,
    Mid,
    High,
}

pub fn view_from_f2(b: &BatteryF2) -> Option<BatteryView> {
    let left = b.left.filter(|&v| v <= 100);
    let right = b.right.filter(|&v| v <= 100);
    let case = b.case.filter(|&v| v <= 100);
    if left.is_none() && right.is_none() {
        None
    } else {
        Some(BatteryView { left, right, case })
    }
}

/// Whether a device is only supported as a beta. Only the tested Edifier
/// NeoBuds Pro 3 and the Fast Pair and Windows tiers are not beta. A renamed
/// NeoBuds Pro 3 counts as beta.
pub fn is_beta(source: Source, name: &str) -> bool {
    match source {
        Source::Vendor(Vendor::Edifier) | Source::VendorHfp(Vendor::Edifier) => {
            !name.to_lowercase().contains("neobuds pro 3")
        }
        Source::FastPair { .. } | Source::FastPairHfp | Source::Windows => false,
        Source::Vendor(_) | Source::VendorHfp(_) => true,
    }
}

fn beta_name(name: &str, source: Source) -> String {
    if is_beta(source, name) {
        format!("{name} (beta)")
    } else {
        name.to_string()
    }
}

pub fn tooltip(device_name: &str, s: &DeviceStatus) -> String {
    match s {
        DeviceStatus::Connecting => format!("{device_name}: connecting..."),
        DeviceStatus::Disconnected => format!("{device_name}: not connected"),
        DeviceStatus::Connected { view: None, source } => match source {
            Source::Windows => format!(
                "{device_name}: connected\nYour earbuds are not yet supported! Please raise an issue on GitHub"
            ),
            _ => format!("{device_name}: connected"),
        },
        DeviceStatus::Connected {
            view: Some(b),
            source,
        } => match source {
            Source::Windows => {
                let pct_str = b
                    .left
                    .map(|v| format!("{v}%"))
                    .unwrap_or_else(|| "?".to_string());
                format!(
                    "{device_name}: {pct_str}\nYour earbuds are not yet supported! Please raise an issue on GitHub"
                )
            }
            Source::FastPairHfp | Source::VendorHfp(_) => {
                let pct_str = b
                    .left
                    .map(|v| format!("{v}%"))
                    .unwrap_or_else(|| "?".to_string());
                format!("{}: {pct_str}", beta_name(device_name, *source))
            }
            Source::Vendor(_) => {
                let left_str = b
                    .left
                    .map(|v| format!("{v}%"))
                    .unwrap_or_else(|| "?".to_string());
                let right_str = b
                    .right
                    .map(|v| format!("{v}%"))
                    .unwrap_or_else(|| "?".to_string());
                let case_str = b.case.map(|v| format!(" | Case {v}%")).unwrap_or_default();
                format!(
                    "{}\nL {left_str} | R {right_str}{case_str}",
                    beta_name(device_name, *source)
                )
            }
            Source::FastPair {
                case_last_known, ..
            } => {
                let left_str = b
                    .left
                    .map(|v| format!("{v}%"))
                    .unwrap_or_else(|| "?".to_string());
                let right_str = b
                    .right
                    .map(|v| format!("{v}%"))
                    .unwrap_or_else(|| "?".to_string());
                let case_str = match b.case {
                    Some(v) if *case_last_known => format!(" | Case {v}% (last known)"),
                    Some(v) => format!(" | Case {v}%"),
                    None => String::new(),
                };
                format!("{device_name}\nL {left_str} | R {right_str}{case_str}")
            }
        },
    }
}

pub fn level_for(pct: u8) -> Level {
    if pct < 20 {
        Level::Low
    } else if pct < 50 {
        Level::Mid
    } else {
        Level::High
    }
}

pub fn level(s: &DeviceStatus) -> Level {
    match s {
        DeviceStatus::Connected { view: Some(b), .. } => {
            let buds: Vec<u8> = [b.left, b.right].into_iter().flatten().collect();
            if let Some(&min_battery) = buds.iter().min() {
                level_for(min_battery)
            } else {
                Level::Unknown
            }
        }
        _ => Level::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_from_f2_normal() {
        let f2 = BatteryF2 {
            left: Some(86),
            right: Some(89),
            case: Some(52),
            bud_in_case: true,
            raw: vec![],
        };
        assert_eq!(
            view_from_f2(&f2),
            Some(BatteryView {
                left: Some(86),
                right: Some(89),
                case: Some(52),
            })
        );
    }

    #[test]
    fn test_view_from_f2_drops_over_100() {
        let f2 = BatteryF2 {
            left: Some(101),
            right: Some(89),
            case: Some(150),
            bud_in_case: false,
            raw: vec![],
        };
        assert_eq!(
            view_from_f2(&f2),
            Some(BatteryView {
                left: None,
                right: Some(89),
                case: None,
            })
        );
    }

    #[test]
    fn test_view_from_f2_returns_none_if_both_buds_none() {
        let f2 = BatteryF2 {
            left: None,
            right: None,
            case: Some(52),
            bud_in_case: true,
            raw: vec![],
        };
        assert_eq!(view_from_f2(&f2), None);

        let f2_over = BatteryF2 {
            left: Some(120),
            right: Some(150),
            case: Some(52),
            bud_in_case: true,
            raw: vec![],
        };
        assert_eq!(view_from_f2(&f2_over), None);
    }

    #[test]
    fn test_tooltip_connecting() {
        assert_eq!(
            tooltip("NeoBuds Pro 3", &DeviceStatus::Connecting),
            "NeoBuds Pro 3: connecting..."
        );
    }

    #[test]
    fn test_tooltip_disconnected() {
        assert_eq!(
            tooltip("NeoBuds Pro 3", &DeviceStatus::Disconnected),
            "NeoBuds Pro 3: not connected"
        );
    }

    #[test]
    fn test_tooltip_connected_none() {
        assert_eq!(
            tooltip("NeoBuds Pro 3", &DeviceStatus::connected_vendor(None)),
            "NeoBuds Pro 3: connected"
        );
    }

    #[test]
    fn test_tooltip_connected_with_case() {
        let status = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(86),
            right: Some(89),
            case: Some(52),
        }));
        assert_eq!(
            tooltip("NeoBuds Pro 3", &status),
            "NeoBuds Pro 3\nL 86% | R 89% | Case 52%"
        );
    }

    #[test]
    fn test_tooltip_connected_without_case() {
        let status = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(86),
            right: Some(89),
            case: None,
        }));
        assert_eq!(
            tooltip("NeoBuds Pro 3", &status),
            "NeoBuds Pro 3\nL 86% | R 89%"
        );
    }

    #[test]
    fn test_tooltip_connected_one_side_none() {
        let status_left_none = DeviceStatus::connected_vendor(Some(BatteryView {
            left: None,
            right: Some(89),
            case: None,
        }));
        assert_eq!(
            tooltip("NeoBuds Pro 3", &status_left_none),
            "NeoBuds Pro 3\nL ? | R 89%"
        );

        let status_right_none = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(86),
            right: None,
            case: Some(52),
        }));
        assert_eq!(
            tooltip("NeoBuds Pro 3", &status_right_none),
            "NeoBuds Pro 3\nL 86% | R ? | Case 52%"
        );
    }

    #[test]
    fn test_tooltip_windows_tier() {
        let s_pct = DeviceStatus::Connected {
            view: Some(BatteryView {
                left: Some(75),
                right: None,
                case: None,
            }),
            source: Source::Windows,
        };
        assert_eq!(
            tooltip("Generic Headset", &s_pct),
            "Generic Headset: 75%\nYour earbuds are not yet supported! Please raise an issue on GitHub"
        );

        let s_none = DeviceStatus::Connected {
            view: None,
            source: Source::Windows,
        };
        assert_eq!(
            tooltip("Generic Headset", &s_none),
            "Generic Headset: connected\nYour earbuds are not yet supported! Please raise an issue on GitHub"
        );
    }

    #[test]
    fn test_tooltip_fastpair_hfp() {
        let s = DeviceStatus::Connected {
            view: Some(BatteryView {
                left: Some(85),
                right: None,
                case: None,
            }),
            source: Source::FastPairHfp,
        };
        assert_eq!(tooltip("Pixel Buds Pro", &s), "Pixel Buds Pro: 85%");
    }

    #[test]
    fn test_tooltip_fastpair_frames() {
        let s = DeviceStatus::Connected {
            view: Some(BatteryView {
                left: Some(80),
                right: Some(90),
                case: Some(100),
            }),
            source: Source::FastPair {
                model_id: Some([0xAA, 0xBB, 0xCC]),
                case_last_known: false,
            },
        };
        assert_eq!(
            tooltip("Pixel Buds Pro", &s),
            "Pixel Buds Pro\nL 80% | R 90% | Case 100%"
        );
    }

    #[test]
    fn test_tooltip_fastpair_frames_with_last_known_case() {
        let s = DeviceStatus::Connected {
            view: Some(BatteryView {
                left: Some(80),
                right: Some(90),
                case: Some(100),
            }),
            source: Source::FastPair {
                model_id: Some([0xAA, 0xBB, 0xCC]),
                case_last_known: true,
            },
        };
        assert_eq!(
            tooltip("Pixel Buds Pro", &s),
            "Pixel Buds Pro\nL 80% | R 90% | Case 100% (last known)"
        );
    }

    #[test]
    fn test_tooltip_fastpair_frames_without_case() {
        let s = DeviceStatus::Connected {
            view: Some(BatteryView {
                left: Some(80),
                right: Some(90),
                case: None,
            }),
            source: Source::FastPair {
                model_id: Some([0xAA, 0xBB, 0xCC]),
                case_last_known: false,
            },
        };
        assert_eq!(
            tooltip("Pixel Buds Pro", &s),
            "Pixel Buds Pro\nL 80% | R 90%"
        );
    }

    #[test]
    fn test_tooltip_vendor_with_case() {
        let s = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(80),
            right: Some(90),
            case: Some(100),
        }));
        assert_eq!(
            tooltip("NeoBuds Pro 3", &s),
            "NeoBuds Pro 3\nL 80% | R 90% | Case 100%"
        );
    }

    #[test]
    fn test_tracker_initial_none() {
        let mut tracker = FastPairBatteryTracker::new();
        let (view, last_known) = tracker.update(Some(70), Some(80), None);
        assert_eq!(view.case, None);
        assert!(!last_known);
        assert_eq!(tracker.last_known_case(), None);
    }

    #[test]
    fn test_tracker_with_case() {
        let mut tracker = FastPairBatteryTracker::new();
        let (view, last_known) = tracker.update(Some(70), Some(80), Some(100));
        assert_eq!(view.case, Some(100));
        assert!(!last_known);
        assert_eq!(tracker.last_known_case(), Some(100));
    }

    #[test]
    fn test_tracker_retains_case_on_subsequent_none() {
        let mut tracker = FastPairBatteryTracker::new();
        let (view1, last_known1) = tracker.update(Some(70), Some(80), Some(100));
        assert_eq!(view1.case, Some(100));
        assert!(!last_known1);

        let (view2, last_known2) = tracker.update(Some(68), Some(78), None);
        assert_eq!(view2.case, Some(100));
        assert!(last_known2);
        assert_eq!(tracker.last_known_case(), Some(100));
    }

    #[test]
    fn test_tracker_updates_case_when_new_value_arrives() {
        let mut tracker = FastPairBatteryTracker::new();
        tracker.update(Some(70), Some(80), Some(100));
        tracker.update(Some(68), Some(78), None);
        let (view3, last_known3) = tracker.update(Some(65), Some(75), Some(90));
        assert_eq!(view3.case, Some(90));
        assert!(!last_known3);
        assert_eq!(tracker.last_known_case(), Some(90));
    }

    #[test]
    fn test_tracker_reset_clears_case() {
        let mut tracker = FastPairBatteryTracker::new();
        tracker.update(Some(70), Some(80), Some(100));
        tracker.reset();
        assert_eq!(tracker.last_known_case(), None);
        let (view, last_known) = tracker.update(Some(65), Some(75), None);
        assert_eq!(view.case, None);
        assert!(!last_known);
    }

    #[test]
    fn test_tracker_status_for() {
        let mut tracker = FastPairBatteryTracker::new();
        let s1 = tracker.status_for(Some(70), Some(80), Some(100), Some([1, 2, 3]));
        assert_eq!(
            s1,
            DeviceStatus::Connected {
                view: Some(BatteryView {
                    left: Some(70),
                    right: Some(80),
                    case: Some(100),
                }),
                source: Source::FastPair {
                    model_id: Some([1, 2, 3]),
                    case_last_known: false,
                },
            }
        );

        let s2 = tracker.status_for(Some(68), Some(78), None, Some([1, 2, 3]));
        assert_eq!(
            s2,
            DeviceStatus::Connected {
                view: Some(BatteryView {
                    left: Some(68),
                    right: Some(78),
                    case: Some(100),
                }),
                source: Source::FastPair {
                    model_id: Some([1, 2, 3]),
                    case_last_known: true,
                },
            }
        );
    }

    #[test]
    fn test_level_for() {
        assert_eq!(level_for(0), Level::Low);
        assert_eq!(level_for(19), Level::Low);
        assert_eq!(level_for(20), Level::Mid);
        assert_eq!(level_for(49), Level::Mid);
        assert_eq!(level_for(50), Level::High);
        assert_eq!(level_for(100), Level::High);
    }

    #[test]
    fn test_level_disconnected_or_no_data() {
        assert_eq!(level(&DeviceStatus::Connecting), Level::Unknown);
        assert_eq!(level(&DeviceStatus::Disconnected), Level::Unknown);
        assert_eq!(level(&DeviceStatus::connected_vendor(None)), Level::Unknown);

        let no_buds = DeviceStatus::connected_vendor(Some(BatteryView {
            left: None,
            right: None,
            case: Some(100),
        }));
        assert_eq!(level(&no_buds), Level::Unknown);
    }

    #[test]
    fn test_level_min_of_known_buds() {
        // Low: < 20
        let status_low = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(19),
            right: Some(80),
            case: Some(100),
        }));
        assert_eq!(level(&status_low), Level::Low);

        // Mid: 20..50
        let status_mid_20 = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(20),
            right: Some(80),
            case: Some(10), // case is ignored for bud level
        }));
        assert_eq!(level(&status_mid_20), Level::Mid);

        let status_mid_49 = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(49),
            right: None,
            case: None,
        }));
        assert_eq!(level(&status_mid_49), Level::Mid);

        // High: >= 50
        let status_high_50 = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(50),
            right: Some(50),
            case: None,
        }));
        assert_eq!(level(&status_high_50), Level::High);

        let status_high_100 = DeviceStatus::connected_vendor(Some(BatteryView {
            left: Some(100),
            right: Some(80),
            case: Some(10),
        }));
        assert_eq!(level(&status_high_100), Level::High);
    }

    fn galaxy_view(l: Option<u8>, r: Option<u8>, c: Option<u8>) -> Option<BatteryView> {
        Some(BatteryView {
            left: l,
            right: r,
            case: c,
        })
    }

    #[test]
    fn test_is_beta_table() {
        let fp = Source::FastPair {
            model_id: None,
            case_last_known: false,
        };
        let cases: [(Source, &str, bool); 12] = [
            (
                Source::Vendor(Vendor::Edifier),
                "EDIFIER NeoBuds Pro 3",
                false,
            ),
            (
                Source::VendorHfp(Vendor::Edifier),
                "edifier neobuds pro 3",
                false,
            ),
            (Source::Vendor(Vendor::Edifier), "EDIFIER W820NB", true),
            (Source::VendorHfp(Vendor::Edifier), "EDIFIER W820NB", true),
            (Source::Vendor(Vendor::Edifier), "My Buds", true),
            (Source::Vendor(Vendor::Galaxy), "Galaxy Buds2 Pro", true),
            (Source::VendorHfp(Vendor::Galaxy), "Galaxy Buds2 Pro", true),
            (Source::Vendor(Vendor::Galaxy), "NeoBuds Pro 3", true),
            (fp, "Pixel Buds Pro", false),
            (Source::FastPairHfp, "Pixel Buds Pro", false),
            (Source::Windows, "Cheap Headset", false),
            (Source::Windows, "NeoBuds Pro 3", false),
        ];
        for (source, name, expected) in cases {
            assert_eq!(is_beta(source, name), expected, "{source:?} {name}");
        }
    }

    #[test]
    fn test_tooltip_galaxy_is_beta() {
        let status = DeviceStatus::Connected {
            view: galaxy_view(Some(100), Some(95), Some(88)),
            source: Source::Vendor(Vendor::Galaxy),
        };
        assert_eq!(
            tooltip("Galaxy Buds2 Pro", &status),
            "Galaxy Buds2 Pro (beta)
L 100% | R 95% | Case 88%"
        );
    }

    #[test]
    fn test_tooltip_galaxy_without_case() {
        let status = DeviceStatus::Connected {
            view: galaxy_view(Some(70), Some(95), None),
            source: Source::Vendor(Vendor::Galaxy),
        };
        assert_eq!(
            tooltip("Galaxy Buds", &status),
            "Galaxy Buds (beta)
L 70% | R 95%"
        );
    }

    #[test]
    fn test_tooltip_galaxy_hfp_is_beta() {
        let status = DeviceStatus::Connected {
            view: galaxy_view(Some(60), None, None),
            source: Source::VendorHfp(Vendor::Galaxy),
        };
        assert_eq!(
            tooltip("Galaxy Buds2 Pro", &status),
            "Galaxy Buds2 Pro (beta): 60%"
        );
    }

    #[test]
    fn test_tooltip_other_edifier_is_beta_neobuds_is_not() {
        let status = DeviceStatus::Connected {
            view: galaxy_view(Some(80), Some(70), Some(50)),
            source: Source::Vendor(Vendor::Edifier),
        };
        assert_eq!(
            tooltip("EDIFIER NeoBuds Pro 3", &status),
            "EDIFIER NeoBuds Pro 3
L 80% | R 70% | Case 50%"
        );
        assert_eq!(
            tooltip("EDIFIER W820NB", &status),
            "EDIFIER W820NB (beta)
L 80% | R 70% | Case 50%"
        );
        let hfp = DeviceStatus::Connected {
            view: galaxy_view(Some(60), None, None),
            source: Source::VendorHfp(Vendor::Edifier),
        };
        assert_eq!(
            tooltip("EDIFIER NeoBuds Pro 3", &hfp),
            "EDIFIER NeoBuds Pro 3: 60%"
        );
    }
}
